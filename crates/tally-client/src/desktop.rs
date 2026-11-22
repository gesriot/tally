//! Запросы к одному оконному процессу для файла базы.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::env;

#[cfg(windows)]
mod windows;

const MAGIC: &[u8] = b"Tally desktop 1\n";
const MAX_RECORD: u64 = 65_536;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(15);
// Клиент ждёт не дольше STARTUP_TIMEOUT. Даём ответу запас до уборки.
const REPLY_RETENTION: Duration = Duration::from_secs(60);
const STARTUP_TIMEOUT_MESSAGE: &str = "Tally не подтвердил открытие окна быстрого ввода";
const MAX_DB_SYMLINKS: usize = 40;
const ACTIVATION_TOKEN: &str = "XDG_ACTIVATION_TOKEN";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Show {
    Window,
    QuickAdd,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ShowRequest {
    pub show: Show,
    pub activation_token: Option<String>,
    pub reply_to: Option<Uuid>,
}

impl ShowRequest {
    pub fn new(show: Show, reply_to: Option<Uuid>) -> Self {
        Self {
            show,
            activation_token: std::env::var(ACTIVATION_TOKEN)
                .ok()
                .filter(|token| !token.is_empty() && token.len() <= 4096 && !token.contains('\0')),
            reply_to,
        }
    }
}

#[derive(Clone)]
pub struct Desktop {
    directory: PathBuf,
}

enum Route {
    Forwarded,
    Retry,
    Launch(File),
}

impl Desktop {
    pub fn backup_directory(&self) -> Result<PathBuf> {
        let key = self
            .directory
            .file_name()
            .context("некорректный ключ базы Tally")?;
        let root =
            if let Some(root) = std::env::var_os("TALLY_BACKUP_DIR").filter(|s| !s.is_empty()) {
                PathBuf::from(root)
            } else {
                let dirs = directories::ProjectDirs::from("", "", "Tally")
                    .context("не удалось найти папку данных Tally")?;
                dirs.data_dir().join("backups")
            };
        Ok(root.join(key))
    }

    pub fn for_db(path: &Path) -> Result<Self> {
        let root = if let Some(root) = std::env::var_os("TALLY_IPC_DIR").filter(|s| !s.is_empty()) {
            PathBuf::from(root)
        } else {
            let dirs = directories::ProjectDirs::from("", "", "Tally")
                .context("не удалось найти каталог приложения Tally")?;
            dirs.runtime_dir()
                .unwrap_or_else(|| dirs.cache_dir())
                .join("desktop")
        };
        Self::in_directory(path, &root)
    }

    fn in_directory(path: &Path, root: &Path) -> Result<Self> {
        let path = database_path(path)?;
        let mut key = String::with_capacity(64);
        for byte in Sha256::digest(path.as_os_str().as_encoded_bytes()) {
            write!(key, "{byte:02x}")?;
        }
        let directory = root.join(key);
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory).with_context(|| {
            format!(
                "не удалось создать каталог обмена Tally {}",
                directory.display()
            )
        })?;
        Ok(Self { directory })
    }

    /// Блокировка остаётся у первого процесса. Следующие передают ему запрос.
    pub fn claim(&self, show: Show) -> Result<Option<File>> {
        self.claim_request(&ShowRequest::new(show, None))
    }

    pub fn claim_request(&self, request: &ShowRequest) -> Result<Option<File>> {
        let gate = self.lock_file("requests.lock")?;
        gate.lock()?;
        let owner = self.lock_file("instance.lock")?;
        match owner.try_lock() {
            Ok(()) => {
                for stale in self.pending()? {
                    self.respond(&stale, Err("предыдущий экземпляр Tally завершился".into()))?;
                }
                Ok(Some(owner))
            }
            Err(TryLockError::WouldBlock) => {
                self.queue(request)?;
                Ok(None)
            }
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }

    fn route_quick_add(&self, request: &ShowRequest) -> Result<Route> {
        let gate = self.lock_file("requests.lock")?;
        match gate.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(Route::Retry),
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        // Только оконный процесс держит instance.lock за пределами requests.lock.
        // Клиенты выбирают того, кто запустит окно, отдельной блокировкой.
        let owner = self.lock_file("instance.lock")?;
        match owner.try_lock() {
            Err(TryLockError::WouldBlock) => {
                self.queue(request)?;
                Ok(Route::Forwarded)
            }
            Err(TryLockError::Error(error)) => Err(error.into()),
            Ok(()) => {
                let startup = self.lock_file("startup.lock")?;
                match startup.try_lock() {
                    Ok(()) => Ok(Route::Launch(startup)),
                    Err(TryLockError::WouldBlock) => Ok(Route::Retry),
                    Err(TryLockError::Error(error)) => Err(error.into()),
                }
            }
        }
    }

    fn queue(&self, request: &ShowRequest) -> Result<()> {
        self.write_record(&self.record("request", Uuid::new_v4()), request)
    }

    /// Забрать запросы только после завершения записи отправителем.
    pub fn receive(&self) -> Result<Vec<ShowRequest>> {
        let gate = self.lock_file("requests.lock")?;
        match gate.try_lock() {
            Ok(()) => self.pending(),
            Err(TryLockError::WouldBlock) => Ok(Vec::new()),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }

    /// Завершить работу владельца и передать причину всем, кто успел отправить запрос.
    pub fn fail_pending(&self, owner: File, cause: &str) -> Result<()> {
        let gate = self.lock_file("requests.lock")?;
        gate.lock()?;
        // Освободить instance.lock раньше requests.lock, чтобы после обхода
        // очереди ни один клиент не мог отправить запрос уходящему владельцу.
        let _owner = owner;
        for request in self.pending()? {
            if let Err(error) = self.respond(&request, Err(cause.to_owned())) {
                eprintln!("ответ на запрос Tally: {error:#}");
            }
        }
        Ok(())
    }

    fn pending(&self) -> Result<Vec<ShowRequest>> {
        let mut pending = Vec::new();
        let entries = fs::read_dir(&self.directory)?.collect::<io::Result<Vec<_>>>()?;
        for entry in entries {
            let name = entry.file_name();
            let Some((kind, id)) = name.to_str().and_then(|s| s.split_once('-')) else {
                continue;
            };
            if Uuid::parse_str(id).is_err() {
                continue;
            }
            match kind {
                "request" => match take_record::<ShowRequest>(&entry.path()) {
                    Ok(Some(request)) => pending.push(request),
                    Ok(None) => {}
                    Err(error) => eprintln!("чтение запроса Tally: {error:#}"),
                },
                "reply"
                    if entry
                        .metadata()
                        .and_then(|meta| meta.modified())
                        .is_ok_and(|time| {
                            time.elapsed().is_ok_and(|age| age >= REPLY_RETENTION)
                        }) =>
                {
                    // Удаляем только файлы с правильной сигнатурой и содержимым.
                    let _ = take_record::<Result<(), String>>(&entry.path());
                }
                _ => {}
            }
        }
        Ok(pending)
    }

    pub fn respond(&self, request: &ShowRequest, result: Result<(), String>) -> Result<()> {
        if let Some(id) = request.reply_to {
            self.write_record(&self.record("reply", id), &result)?;
        }
        Ok(())
    }

    fn record(&self, kind: &str, id: Uuid) -> PathBuf {
        self.directory.join(format!("{kind}-{id}"))
    }

    fn lock_file(&self, name: &str) -> Result<File> {
        let path = self.directory.join(name);
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("не удалось открыть {}", path.display()))
    }

    fn write_record(&self, path: &Path, value: &impl Serialize) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        file.write_all(MAGIC)?;
        serde_json::to_writer(file.as_file_mut(), value)?;
        file.persist_noclobber(path).map_err(|e| e.error)?;
        Ok(())
    }

    fn wait_for_reply(
        &self,
        id: Uuid,
        mut child: Option<&mut Child>,
        deadline: Instant,
    ) -> Result<()> {
        let path = self.record("reply", id);
        loop {
            if let Some(reply) = take_record::<Result<(), String>>(&path)? {
                return reply.map_err(anyhow::Error::msg);
            }
            if let Some(process) = &mut child
                && let Some(status) = process.try_wait()?
                && !status.success()
            {
                if let Some(reply) = take_record::<Result<(), String>>(&path)? {
                    return reply.map_err(anyhow::Error::msg);
                }
                bail!("tally-ui завершился до открытия окна ({status})");
            }
            wait_briefly(deadline)?;
        }
    }
}

fn database_path(path: &Path) -> Result<PathBuf> {
    let mut path = std::path::absolute(path)?;
    env::ensure_parent(&path)?;
    for _ in 0..=MAX_DB_SYMLINKS {
        match path.canonicalize() {
            Ok(path) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let parent = path
            .parent()
            .context("путь к базе без папки")?
            .canonicalize()?;
        match path.symlink_metadata() {
            // canonicalize не может пройти через ссылку до создания её цели.
            Ok(meta) if meta.file_type().is_symlink() => path = parent.join(fs::read_link(&path)?),
            Ok(_) => return Ok(path.canonicalize()?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = path.file_name().context("путь к базе без имени файла")?;
                return Ok(parent.join(name));
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!(
        "слишком много символических ссылок в пути к базе {}",
        path.display()
    )
}

fn wait_briefly(deadline: Instant) -> Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        bail!(STARTUP_TIMEOUT_MESSAGE);
    }
    std::thread::sleep(POLL_INTERVAL.min(remaining));
    Ok(())
}

fn take_record<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match path.symlink_metadata() {
        Ok(meta) if !meta.is_file() || meta.len() > MAX_RECORD => return Ok(None),
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    }
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD + 1).read_to_end(&mut bytes)?;
    let Some(value) = bytes
        .strip_prefix(MAGIC)
        .and_then(|body| serde_json::from_slice(body).ok())
    else {
        return Ok(None);
    };
    fs::remove_file(path)?;
    Ok(Some(value))
}

pub fn open_quick_add(path: &Path) -> Result<()> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let ipc = Desktop::for_db(path)?;
    let id = Uuid::new_v4();
    let request = ShowRequest::new(Show::QuickAdd, Some(id));
    while Instant::now() < deadline {
        match ipc.route_quick_add(&request)? {
            Route::Retry => wait_briefly(deadline)?,
            Route::Forwarded => return ipc.wait_for_reply(id, None, deadline),
            Route::Launch(_startup) => {
                let mut child = launch_window(path, id)?;
                return ipc.wait_for_reply(id, Some(&mut child), deadline);
            }
        }
    }
    bail!(STARTUP_TIMEOUT_MESSAGE)
}

fn launch_window(path: &Path, id: Uuid) -> Result<Child> {
    let executable = format!("tally-ui{}", std::env::consts::EXE_SUFFIX);
    let next_to_cli = std::env::current_exe()?.with_file_name(&executable);
    let program = if next_to_cli.is_file() {
        next_to_cli
    } else {
        PathBuf::from(executable)
    };
    let mut command = Command::new(program);
    command
        .arg("--db")
        .arg(path)
        .arg("--quick-add")
        .arg("--startup-id")
        .arg(id.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    let _streams = windows::StandardStreams::prevent_inheritance()?;
    command
        .spawn()
        .context("не удалось запустить tally-ui для быстрого ввода")
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use super::*;

    /// Тесты модуля проверяют блокировки файлов сразу после их освобождения и
    /// запускают дочерний процесс. В Unix дочерний процесс до `exec` держит
    /// копии всех дескрипторов родителя, а с ними и блокировки `flock`, так что
    /// запуск в одном тесте ненадолго удерживал блокировку, которую соседний
    /// тест уже отпустил. Поэтому тесты модуля идут по одному.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[test]
    fn two_cold_clients_wait_for_the_same_window() -> Result<()> {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("tally.db");
        let root = dir.path().join("ipc");
        let desktop = Desktop::in_directory(&path, &root)?;
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let first = ShowRequest::new(Show::QuickAdd, Some(first_id));
        let second = ShowRequest::new(Show::QuickAdd, Some(second_id));

        let Route::Launch(_startup) = desktop.route_quick_add(&first)? else {
            bail!("первый клиент должен запустить окно");
        };
        assert!(matches!(desktop.route_quick_add(&second)?, Route::Retry));
        assert!(desktop.receive()?.is_empty());

        let _owner = desktop.claim_request(&first)?.context("первое окно")?;
        assert!(matches!(
            desktop.route_quick_add(&second)?,
            Route::Forwarded
        ));
        desktop.respond(&first, Ok(()))?;
        let pending = desktop.receive()?;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].reply_to, Some(second_id));
        desktop.respond(&pending[0], Ok(()))?;
        desktop.wait_for_reply(first_id, None, Instant::now())?;
        desktop.wait_for_reply(second_id, None, Instant::now())?;
        Ok(())
    }

    #[test]
    fn an_expired_wait_leaves_the_child_and_other_replies_alone() -> Result<()> {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let desktop = Desktop::in_directory(&dir.path().join("tally.db"), dir.path())?;
        let expired_id = Uuid::new_v4();
        let other_id = Uuid::new_v4();

        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 5",
            ]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new("sleep");
            command.arg("5");
            command
        };
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let result = desktop.wait_for_reply(expired_id, Some(&mut child), Instant::now());
        let still_running = child.try_wait()?.is_none();
        if still_running {
            child.kill()?;
        }
        child.wait()?;
        assert!(still_running, "тайм-аут завершил оконный процесс");
        assert_eq!(
            result.err().map(|error| error.to_string()).as_deref(),
            Some("Tally не подтвердил открытие окна быстрого ввода")
        );

        desktop.respond(&ShowRequest::new(Show::QuickAdd, Some(other_id)), Ok(()))?;
        desktop.wait_for_reply(other_id, None, Instant::now())?;
        Ok(())
    }

    #[test]
    fn startup_failure_replies_to_queued_clients_before_releasing_the_owner() -> Result<()> {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let desktop = Desktop::in_directory(&dir.path().join("tally.db"), dir.path())?;
        let owner = desktop.claim(Show::Window)?.context("владелец")?;
        let ids = [Uuid::new_v4(), Uuid::new_v4()];
        let requests = ids.map(|id| ShowRequest::new(Show::QuickAdd, Some(id)));
        for request in &requests {
            assert!(matches!(
                desktop.route_quick_add(request)?,
                Route::Forwarded
            ));
        }

        desktop.fail_pending(owner, "база имеет более новую версию")?;
        for id in ids {
            let result = desktop.wait_for_reply(id, None, Instant::now());
            assert_eq!(
                result.err().map(|error| error.to_string()).as_deref(),
                Some("база имеет более новую версию")
            );
        }
        assert!(matches!(
            desktop.route_quick_add(&requests[0])?,
            Route::Launch(_)
        ));
        Ok(())
    }

    #[test]
    fn removes_only_expired_reply_records() -> Result<()> {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let desktop = Desktop::in_directory(&dir.path().join("tally.db"), dir.path())?;
        let expired_id = Uuid::new_v4();
        let fresh_id = Uuid::new_v4();
        for id in [expired_id, fresh_id] {
            desktop.respond(&ShowRequest::new(Show::QuickAdd, Some(id)), Ok(()))?;
        }

        let expired = desktop.record("reply", expired_id);
        let other = desktop.record("reply", Uuid::new_v4());
        fs::write(&other, b"unrelated data")?;
        let old_time = std::time::SystemTime::now() - REPLY_RETENTION - Duration::from_secs(5);
        for path in [&expired, &other] {
            File::options()
                .write(true)
                .open(path)?
                .set_modified(old_time)?;
        }

        assert!(desktop.receive()?.is_empty());
        assert!(!expired.exists());
        assert_eq!(fs::read(other)?, b"unrelated data");
        desktop.wait_for_reply(fresh_id, None, Instant::now())?;
        Ok(())
    }

    #[test]
    fn forwards_activation_and_acknowledges_on_the_instance_that_owns_the_database() -> Result<()> {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("tally.db");
        let root = dir.path().join("ipc");
        let first = Desktop::in_directory(&path, &root)?;
        let second = Desktop::in_directory(&path, &root)?;
        let owner = first.claim(Show::Window)?.context("первый экземпляр")?;
        let id = Uuid::new_v4();
        let request = ShowRequest {
            show: Show::QuickAdd,
            activation_token: Some("test:activation-token".into()),
            reply_to: Some(id),
        };
        assert!(second.claim_request(&request)?.is_none());
        let pending = first.receive()?;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].show, Show::QuickAdd);
        assert_eq!(pending[0].activation_token, request.activation_token);
        first.respond(&pending[0], Ok(()))?;
        second.wait_for_reply(id, None, Instant::now())?;
        assert!(first.receive()?.is_empty());

        let failure_id = Uuid::new_v4();
        let failure = ShowRequest::new(Show::QuickAdd, Some(failure_id));
        first.respond(&failure, Err("база повреждена".into()))?;
        let result = second.wait_for_reply(failure_id, None, Instant::now());
        assert_eq!(
            result.err().map(|error| error.to_string()).as_deref(),
            Some("база повреждена")
        );

        let other = Desktop::in_directory(&dir.path().join("other.db"), &root)?;
        assert!(other.claim(Show::Window)?.is_some());
        assert!(second.claim(Show::QuickAdd)?.is_none());
        drop(owner);
        let _new_owner = second.claim(Show::Window)?.context("новый экземпляр")?;
        assert!(second.receive()?.is_empty());
        Ok(())
    }
}
