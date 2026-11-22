//! `tally-ui` – окно Tally. База – та же, что у консоли: `--db <ПУТЬ>`,
//! иначе `TALLY_DB`, иначе `tally.db` в папке данных пользователя;
//! `TALLY_TODAY` подменяет сегодняшний день.

// В Windows окно запускается без консоли (двойной щелчок по exe).
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use slint::ComponentHandle;
use tally_app::App;
use tally_client::{
    desktop::{Desktop, Show, ShowRequest},
    env,
};
use tally_store::Store;
use tally_ui::backend::{Calendar, Threaded};
use tally_ui::bind::Bound;
use tally_ui::ui::{AppWindow, QuickEntry};
use uuid::Uuid;

mod desktop;

/// Неверные аргументы – код 2, как у консоли; остальные ошибки – 1.
const USAGE: u8 = 2;

fn main() -> ExitCode {
    let args = match parse_args(std::env::args_os().skip(1)) {
        Ok(flag) => flag,
        Err(e) => {
            eprintln!("ошибка: {e:#}");
            return ExitCode::from(USAGE);
        }
    };
    let reports_startup = args.startup_id.is_some();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if !reports_startup {
                eprintln!("ошибка: {e:#}");
            }
            ExitCode::FAILURE
        }
    }
}

/// Пустой путь (или
/// из одних пробелов) – ошибка: SQLite открыл бы на его месте временную
/// базу, и всё сделанное в окне пропало бы при закрытии.
struct Args {
    db: Option<PathBuf>,
    show: Show,
    startup_id: Option<Uuid>,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args> {
    let mut args = args.into_iter();
    let mut db = None;
    let mut show = Show::Window;
    let mut startup_id = None;
    while let Some(arg) = args.next() {
        if arg == "--quick-add" {
            show = Show::QuickAdd;
            continue;
        }
        if arg == "--startup-id" {
            let id = args
                .next()
                .context("после --startup-id нужен номер запроса")?;
            let id = id.to_str().context("неверный номер запроса")?;
            startup_id = Some(Uuid::parse_str(id).context("неверный номер запроса")?);
            continue;
        }
        let path = if arg == "--db" {
            args.next().context("после --db нужен путь к базе")?
        } else if let Some(path) = arg.to_str().and_then(|a| a.strip_prefix("--db=")) {
            OsString::from(path)
        } else {
            bail!("неизвестный аргумент «{}»", arg.to_string_lossy());
        };
        env::check_db_flag(&path)?;
        db = Some(PathBuf::from(path));
    }
    Ok(Args {
        db,
        show,
        startup_id,
    })
}

fn run(args: Args) -> Result<()> {
    let path = env::db_path(args.db.as_deref(), env::var(env::DB_VAR).as_deref())?;
    let ipc = Desktop::for_db(&path)?;
    let request = ShowRequest::new(args.show, args.startup_id);
    let (owner, result) = match ipc.claim_request(&request) {
        Ok(Some(owner)) => (Some(owner), run_instance(&path, &ipc, &request)),
        Ok(None) => return Ok(()),
        Err(error) => (None, Err(error)),
    };
    let cause = desktop::shutdown_cause(&result);
    if result.is_err() {
        let _ = ipc.respond(&request, Err(cause.clone()));
    }
    if let Some(owner) = owner
        && let Err(error) = ipc.fail_pending(owner, &cause)
    {
        eprintln!("ответы на запросы Tally: {error:#}");
    }
    result
}

fn run_instance(path: &std::path::Path, ipc: &Desktop, request: &ShowRequest) -> Result<()> {
    // Ошибка в TALLY_TODAY – сразу, а не молчаливые системные часы.
    let fixed_today = match env::var(env::TODAY_VAR).filter(|v| !v.is_empty()) {
        Some(v) => Some(env::today(Some(&v))?),
        None => None,
    };
    let store = Store::open(path)
        .map_err(|e| anyhow::anyhow!("не удалось открыть базу {}: {e}", path.display()))?;

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(tally_ui::RENDERER.into())
        .select()?;
    let window = AppWindow::new()?;
    let weak = window.as_weak();
    let calendar = Calendar { fixed_today };
    let backend = Threaded::spawn(App::new(store), calendar, move || {
        let weak = weak.clone();
        // Окно уже закрыто – ответ некому показать.
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = weak.upgrade() {
                w.invoke_poll();
            }
        });
    });
    let bound = Bound::attach(&window, Box::new(backend), calendar);
    let entry = QuickEntry::new()?;
    bound.attach_quick_entry(&entry);
    desktop::run(&window, &entry, &bound, ipc, request)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn db_flag(args: impl IntoIterator<Item = OsString>) -> Result<Option<PathBuf>> {
        parse_args(args).map(|args| args.db)
    }

    #[test]
    fn db_flag_in_both_forms() {
        assert_eq!(db_flag(args(&[])).ok(), Some(None));
        let a = Some(Some(PathBuf::from("a.db")));
        assert_eq!(db_flag(args(&["--db", "a.db"])).ok(), a);
        assert_eq!(db_flag(args(&["--db=a.db"])).ok(), a);
        let missing = db_flag(args(&["--db"])).err().map(|e| e.to_string());
        assert_eq!(missing.as_deref(), Some("после --db нужен путь к базе"));
        let unknown = db_flag(args(&["-x"])).err().map(|e| e.to_string());
        assert_eq!(unknown.as_deref(), Some("неизвестный аргумент «-x»"));
    }

    #[test]
    fn empty_db_path_is_rejected() {
        for list in [&["--db="][..], &["--db", ""], &["--db", "  "], &["--db= "]] {
            let e = db_flag(args(list)).err().map(|e| e.to_string());
            assert_eq!(e.as_deref(), Some("пустой путь к базе в --db"), "{list:?}");
        }
    }
}
