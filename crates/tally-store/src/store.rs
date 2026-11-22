//! [`Store`] – владелец соединения; чтение через [`Reader`], запись – только
//! внутри транзакции через [`Writer`].

use std::path::Path;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use jiff::civil::Date;
use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, ErrorCode, TransactionBehavior};
use serde::{Deserialize, Serialize};
use tally_core::lists::Lists;
use tally_core::{Area, AreaId, Heading, HeadingId, Project, ProjectId, Tag, TagId, Task, TaskId};
use uuid::Uuid;

use crate::backup;
use crate::error::{Kind, Result, StoreError};
use crate::migrate;
use crate::search::{self, Hit};
use crate::settings::{self, Settings};
use crate::table::history::{self, HistoryEntry};
use crate::table::{self, area, heading, project, reminder, tag, task};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Включить WAL. Смена режима журнала не ждёт блокировку через
/// `busy_timeout` (SQLite сразу отвечает SQLITE_BUSY, если файл открывает
/// второй процесс), поэтому ждём сами, не дольше [`BUSY_TIMEOUT`].
fn enable_wal(conn: &Connection) -> Result<()> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        match conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0)) {
            Ok(_) => return Ok(()),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == ErrorCode::DatabaseBusy && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Хранилище Tally поверх одного соединения SQLite.
///
/// `Store` можно передать в другой поток (`Send`), но не делить между
/// потоками: им владеет один поток (`tally_app::Worker`) или один
/// синхронный `tally_app::App`.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Открыть (или создать) базу в файле и довести схему до текущей версии.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        // Ожидание блокировки – до первого обращения к файлу: второй процесс,
        // открывающий ту же новую базу, ждёт, а не падает с SQLITE_BUSY.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // WAL: читатели не ждут писателя; в базе в памяти режим остаётся «memory».
        enable_wal(&conn)?;
        Self::setup(conn)
    }

    /// Новая пустая база в памяти – для тестов и примеров.
    pub fn open_in_memory() -> Result<Self> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        migrate::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn settings(&self) -> Result<Settings> {
        settings::load(&self.conn)
    }

    pub fn save_settings(&mut self, value: Settings) -> Result<()> {
        self.write(|writer| settings::save(writer.conn, value))
    }

    /// Копирует согласованное состояние SQLite, включая записи в WAL.
    /// Путь должен указывать на выделенный для копии временный файл.
    pub fn backup_into(&self, path: &Path) -> Result<()> {
        let mut destination = Connection::open(path)?;
        let backup = Backup::new(&self.conn, &mut destination)?;
        let deadline = Instant::now() + BUSY_TIMEOUT;
        loop {
            match backup.step(-1)? {
                StepResult::Done => return Ok(()),
                _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                        None,
                    )
                    .into());
                }
            }
        }
    }

    /// Проверить целостность самостоятельной копии и её схему без записи в файл.
    pub fn verify_backup(&self, path: &Path) -> Result<()> {
        backup::verify(&self.conn, path)
    }

    /// Текущая версия схемы в базе.
    pub fn schema_version(&self) -> Result<i64> {
        migrate::version(&self.conn)
    }

    /// Взять наступившие напоминания. До подтверждения показа они доступны
    /// повторно, в том числе после перезапуска.
    pub fn claim_reminders(&mut self, now: Timestamp) -> Result<Vec<reminder::Reminder>> {
        if reminder::due(&self.conn, now)?.is_empty() {
            return Ok(Vec::new());
        }
        self.write(|writer| reminder::claim(writer.conn, now))
    }

    pub fn confirm_reminders(&mut self, shown: &[reminder::Reminder]) -> Result<()> {
        self.write(|writer| reminder::confirm(writer.conn, shown))
    }

    /// Чтение.
    pub fn read(&self) -> Reader<'_> {
        Reader { conn: &self.conn }
    }

    /// Запись в одной транзакции: `f` вернул `Ok` – изменения зафиксированы,
    /// `Err` – откатываются все до одного.
    pub fn write<T>(&mut self, f: impl FnOnce(&Writer<'_>) -> Result<T>) -> Result<T> {
        self.write_with(f)
    }

    /// Как [`Store::write`], но с ошибкой вызывающего кода: проверки
    /// приложения идут в той же транзакции, что и запись, и их ошибка тоже
    /// откатывает всё. Транзакция – `BEGIN IMMEDIATE`: блокировка записи
    /// берётся сразу, поэтому всё прочитанное через [`Writer::read`] другое
    /// соединение до фиксации не изменит.
    pub fn write_with<T, E: From<StoreError>>(
        &mut self,
        f: impl FnOnce(&Writer<'_>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::from)?;
        let out = f(&Writer { conn: &tx })?;
        tx.commit().map_err(StoreError::from)?;
        Ok(out)
    }
}

/// Id любой записи – для удаления, возврата из корзины и очистки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecordId {
    Area(AreaId),
    Tag(TagId),
    Project(ProjectId),
    Heading(HeadingId),
    Task(TaskId),
}

impl RecordId {
    fn parts(self) -> (Kind, Uuid) {
        match self {
            Self::Area(id) => (Kind::Area, id.0),
            Self::Tag(id) => (Kind::Tag, id.0),
            Self::Project(id) => (Kind::Project, id.0),
            Self::Heading(id) => (Kind::Heading, id.0),
            Self::Task(id) => (Kind::Task, id.0),
        }
    }
}

/// Все записи базы, включая удалённые (они нужны «Корзине»).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub areas: Vec<Area>,
    pub tags: Vec<Tag>,
    pub projects: Vec<Project>,
    pub headings: Vec<Heading>,
    pub tasks: Vec<Task>,
}

impl Snapshot {
    /// Списки в духе Things на день `today` (правила – в `tally_core::lists`).
    pub fn lists(&self, today: Date) -> Lists<'_> {
        Lists::compute(
            &self.tasks,
            &self.projects,
            &self.areas,
            &self.headings,
            today,
        )
    }
}

/// Сколько записей удалила очистка корзины (вместе с содержимым удалённых
/// проектов, разделов и областей).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Purged {
    pub tasks: usize,
    /// Пункты чек-листов: удалённых задач и надгробия живых.
    pub checklist_items: usize,
    pub headings: usize,
    pub projects: usize,
    pub areas: usize,
    pub tags: usize,
}

/// Чтение из базы. Живёт не дольше [`Store`] (или транзакции), из которого взят.
#[derive(Clone, Copy)]
pub struct Reader<'c> {
    conn: &'c Connection,
}

impl Reader<'_> {
    /// Несколько запросов – в одной транзакции чтения: в WAL все они видят
    /// один и тот же снимок базы, даже если рядом пишет другое соединение.
    /// Внутри [`Store::write`] транзакция уже идёт – запросы выполняются в ней.
    ///
    /// Открыт и для вызывающего кода: например, поиск и снимок для его
    /// вывода – из одного состояния базы.
    pub fn consistent<T>(&self, f: impl FnOnce(Reader<'_>) -> Result<T>) -> Result<T> {
        if !self.conn.is_autocommit() {
            return f(*self);
        }
        let tx = self.conn.unchecked_transaction()?;
        let out = f(Reader { conn: &tx })?;
        tx.commit()?;
        Ok(out)
    }

    pub fn area(&self, id: AreaId) -> Result<Option<Area>> {
        self.consistent(|r| area::get(r.conn, id))
    }
    pub fn areas(&self) -> Result<Vec<Area>> {
        self.consistent(|r| area::all(r.conn))
    }
    pub fn tag(&self, id: TagId) -> Result<Option<Tag>> {
        tag::get(self.conn, id)
    }
    pub fn tags(&self) -> Result<Vec<Tag>> {
        tag::all(self.conn)
    }
    pub fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        self.consistent(|r| project::get(r.conn, id))
    }
    pub fn projects(&self) -> Result<Vec<Project>> {
        self.consistent(|r| project::all(r.conn))
    }
    pub fn heading(&self, id: HeadingId) -> Result<Option<Heading>> {
        heading::get(self.conn, id)
    }
    pub fn headings(&self) -> Result<Vec<Heading>> {
        heading::all(self.conn)
    }
    pub fn task(&self, id: TaskId) -> Result<Option<Task>> {
        self.consistent(|r| task::get(r.conn, id))
    }
    /// Все задачи (и удалённые) в порядке `index`.
    pub fn tasks(&self) -> Result<Vec<Task>> {
        self.consistent(|r| task::all(r.conn))
    }

    /// Вся база разом – для вычисления списков. Один согласованный снимок:
    /// читается в одной транзакции.
    pub fn snapshot(&self) -> Result<Snapshot> {
        self.consistent(|r| {
            Ok(Snapshot {
                areas: r.areas()?,
                tags: r.tags()?,
                projects: r.projects()?,
                headings: r.headings()?,
                tasks: r.tasks()?,
            })
        })
    }

    /// Последнее неотменённое действие журнала – его отменит undo.
    pub fn last_done(&self) -> Result<Option<HistoryEntry>> {
        history::last_done(self.conn)
    }

    /// Самое старое отменённое действие – его повторит redo.
    pub fn first_undone(&self) -> Result<Option<HistoryEntry>> {
        history::first_undone(self.conn)
    }

    /// Весь журнал действий, от старых к новым.
    pub fn history(&self) -> Result<Vec<HistoryEntry>> {
        history::all(self.conn)
    }

    /// Поиск по названиям, заметкам и чек-листам без учёта регистра и «ё».
    /// Удалённое (в том числе через проект или раздел) не находится.
    pub fn search(&self, query: &str) -> Result<Vec<Hit>> {
        self.consistent(|r| search::search(r.conn, query))
    }
}

/// Запись внутри транзакции [`Store::write`].
///
/// `insert_*` – новая запись (уже есть – [`crate::StoreError::AlreadyExists`]),
/// `update_*` – существующая целиком, с тегами и чек-листом
/// (нет – [`crate::StoreError::NotFound`]). `updated_at` берётся из записи:
/// отмечать изменение – дело вызывающего кода. Каждая операция атомарна
/// (SAVEPOINT): ошибка откатывает её целиком, а не только последний запрос.
pub struct Writer<'c> {
    conn: &'c Connection,
}

impl Writer<'_> {
    /// Одна операция – всё или ничего: SAVEPOINT вокруг неё. Если вызывающий
    /// код поймал ошибку операции и вернул `Ok`, транзакция зафиксирует только
    /// целые операции, без половины записи.
    fn atomic<T>(&self, op: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        self.conn.execute_batch("SAVEPOINT tally_op")?;
        match op(self.conn) {
            Ok(out) => {
                self.conn.execute_batch("RELEASE tally_op")?;
                Ok(out)
            }
            Err(e) => {
                self.conn
                    .execute_batch("ROLLBACK TO tally_op; RELEASE tally_op")?;
                Err(e)
            }
        }
    }

    /// Чтение внутри той же транзакции (видит ещё не зафиксированное).
    pub fn read(&self) -> Reader<'_> {
        Reader { conn: self.conn }
    }

    pub fn insert_area(&self, a: &Area) -> Result<()> {
        self.atomic(|c| area::save(c, a, true))
    }
    pub fn update_area(&self, a: &Area) -> Result<()> {
        self.atomic(|c| area::save(c, a, false))
    }
    pub fn insert_tag(&self, t: &Tag) -> Result<()> {
        self.atomic(|c| tag::save(c, t, true))
    }
    pub fn update_tag(&self, t: &Tag) -> Result<()> {
        self.atomic(|c| tag::save(c, t, false))
    }
    pub fn insert_project(&self, p: &Project) -> Result<()> {
        self.atomic(|c| project::save(c, p, true))
    }
    pub fn update_project(&self, p: &Project) -> Result<()> {
        self.atomic(|c| project::save(c, p, false))
    }
    pub fn insert_heading(&self, h: &Heading) -> Result<()> {
        self.atomic(|c| heading::save(c, h, true))
    }
    pub fn update_heading(&self, h: &Heading) -> Result<()> {
        self.atomic(|c| heading::save(c, h, false))
    }
    pub fn insert_task(&self, t: &Task) -> Result<()> {
        self.atomic(|c| task::save(c, t, true))
    }
    pub fn update_task(&self, t: &Task) -> Result<()> {
        self.atomic(|c| task::save(c, t, false))
    }

    pub fn confirmed_reminders(&self, task: TaskId) -> Result<Vec<Timestamp>> {
        reminder::confirmed(self.conn, task)
    }

    pub fn restore_confirmed_reminders(&self, task: TaskId, reminders: &[Timestamp]) -> Result<()> {
        self.atomic(|c| reminder::restore_confirmed(c, task, reminders))
    }

    /// Записать новое действие в журнал: отменённые действия стираются
    /// (повторять их после нового действия нельзя), всего остаётся не больше
    /// `keep` последних. Возвращает номер записи.
    pub fn push_history(
        &self,
        at: Timestamp,
        action: &str,
        changes: &str,
        keep: usize,
    ) -> Result<i64> {
        self.atomic(|c| history::push(c, at, action, changes, keep))
    }

    /// Отметить действие `seq` отменённым (`undone`) или снова выполненным и
    /// сохранить новые состояния записей. Нет такого – `false`.
    pub fn mark_history(&self, seq: i64, undone: bool, changes: &str) -> Result<bool> {
        self.atomic(|c| history::mark(c, seq, undone, changes))
    }

    /// Удалить одну запись журнала после окончательной очистки корзины.
    pub fn forget_history(&self, seq: i64) -> Result<()> {
        self.atomic(|c| {
            c.execute("DELETE FROM history WHERE seq = ?1", [seq])?;
            Ok(())
        })
    }

    /// Мягко удалить: запись уходит в «Корзину» (`deleted_at = at`).
    /// Содержимое удалённого проекта, раздела или области не меняется – в корзине
    /// оно «через» контейнер и возвращается вместе с ним.
    pub fn delete(&self, id: RecordId, at: Timestamp) -> Result<()> {
        let (kind, id) = id.parts();
        self.atomic(|c| table::set_deleted(c, kind, id, Some(at), at))
    }

    /// Вернуть из «Корзины».
    pub fn restore(&self, id: RecordId, at: Timestamp) -> Result<()> {
        let (kind, id) = id.parts();
        self.atomic(|c| table::set_deleted(c, kind, id, None, at))
    }

    /// Удалить физически запись из «Корзины» – удалённую саму или лежащую
    /// в удалённом проекте, разделе или области – вместе с зависимыми записями
    /// (`ON DELETE CASCADE`). Запись вне корзины – [`crate::StoreError::NotInTrash`].
    pub fn purge(&self, id: RecordId) -> Result<()> {
        let (kind, id) = id.parts();
        self.atomic(|c| table::purge(c, kind, id))
    }

    /// Очистить «Корзину»: физически удалить всё с `deleted_at`, а вместе
    /// с проектом, разделом или областью – и всё их содержимое (`ON DELETE CASCADE`).
    /// Надгробия пунктов чек-листа уходят и у живых задач.
    pub fn empty_trash(&self) -> Result<Purged> {
        self.atomic(|_| self.empty_trash_now())
    }

    fn empty_trash_now(&self) -> Result<Purged> {
        let total = |table: &str| -> Result<usize> {
            let n: i64 =
                self.conn
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
            Ok(usize::try_from(n).unwrap_or(0))
        };
        const TABLES: [&str; 6] = [
            "areas",
            "projects",
            "headings",
            "tasks",
            "tags",
            "checklist_items",
        ];
        let mut before = [0; 6];
        for (n, table) in before.iter_mut().zip(TABLES) {
            *n = total(table)?;
        }
        for table in TABLES {
            self.conn.execute(
                &format!("DELETE FROM {table} WHERE deleted_at IS NOT NULL"),
                [],
            )?;
        }
        let mut gone = [0; 6];
        for ((n, was), table) in gone.iter_mut().zip(before).zip(TABLES) {
            *n = was - total(table)?;
        }
        let [areas, projects, headings, tasks, tags, checklist_items] = gone;
        Ok(Purged {
            tasks,
            checklist_items,
            headings,
            projects,
            areas,
            tags,
        })
    }
}
