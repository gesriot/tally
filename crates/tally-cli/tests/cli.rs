//! Программа `tally` целиком: запуск с базой во временной папке.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

struct Db {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn db() -> Db {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("tally.db");
    Db { _dir: dir, path }
}

/// `tally` с базой `db` и сегодняшним днём 19 ноября 2026.
fn tally(db: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tally"));
    cmd.env_clear()
        .env("TALLY_DB", db)
        .env("TALLY_TODAY", "2026-11-19");
    cmd
}

fn ok(db: &Path, args: &[&str]) -> String {
    let out = tally(db)
        .args(args)
        .assert()
        .success()
        .stderr("")
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap_or_else(|e| panic!("{e}"))
}

/// Номер задачи из строки «Добавлено: [abc123] …».
fn id_of(added: &str) -> String {
    let start = added.find('[').map_or(0, |i| i + 1);
    let end = added.find(']').unwrap_or(start);
    added[start..end].to_owned()
}

#[test]
fn help_is_russian() {
    let d = db();
    for args in [
        ["--help"].as_slice(),
        &["-h"],
        &["add", "--help"],
        &["move", "-h"],
    ] {
        let out = ok(&d.path, args);
        assert!(out.contains("Использование:"), "{out}");
        for english in [
            "Usage",
            "Options",
            "Commands",
            "Print help",
            "Arguments",
            "[default",
        ] {
            assert!(!out.contains(english), "{english} в\n{out}");
        }
        assert!(!out.contains('\u{2014}'), "{out}");
    }
    let out = ok(&d.path, &["--help"]);
    for command in [
        "add", "list", "done", "move", "search", "project", "area", "heading",
    ] {
        assert!(out.contains(&format!("  {command}")), "{command} в\n{out}");
    }
    assert_eq!(
        ok(&d.path, &["--version"]),
        format!("tally {}\n", env!("CARGO_PKG_VERSION"))
    );
    // Помощь не создаёт базу.
    assert!(!d.path.exists());
}

#[test]
fn no_command_shows_help_and_fails() {
    let d = db();
    tally(&d.path)
        .assert()
        .code(2)
        .stdout("")
        .stderr(contains("Использование:"));
}

#[test]
fn usage_errors_are_russian_with_code_2() {
    let d = db();
    for (args, message) in [
        (
            ["frobnicate"].as_slice(),
            "ошибка: неизвестная команда «frobnicate»",
        ),
        (&["add"], "ошибка: не хватает аргумента: <ТЕКСТ>"),
        (
            &["add", "x", "--bogus"],
            "ошибка: неизвестный параметр «--bogus»",
        ),
        (
            &["list", "вчера"],
            "ошибка: неверное значение «вчера» у [СПИСОК]; можно: today, inbox",
        ),
        (
            &["move", "x", "--inbox", "-p", "Ремонт"],
            "ошибка: --inbox нельзя вместе с --project <ПРОЕКТ>",
        ),
        (
            &["add", "x", "--heading", "Покупки"],
            "ошибка: не хватает аргумента: --project <ПРОЕКТ>",
        ),
    ] {
        tally(&d.path)
            .args(args)
            .assert()
            .code(2)
            .stdout("")
            .stderr(contains(message));
    }
}

#[test]
fn session() {
    let d = db();
    let p = d.path.as_path();
    assert_eq!(
        ok(p, &["add", "Позвонить", "маме", "сегодня"])
            .lines()
            .count(),
        1
    );
    ok(p, &["add", "Заказать ужин вечером"]);
    let bill = ok(
        p,
        &[
            "add",
            "Оплатить счёт",
            "-w",
            "сегодня",
            "-d",
            "сегодня",
            "-t",
            "дом",
        ],
    );
    assert!(bill.starts_with("Добавлено: ["), "{bill}");
    assert!(
        bill.ends_with("] Оплатить счёт – Сегодня #дом ⚑ сегодня\n"),
        "{bill}"
    );
    ok(p, &["add", "Разобрать почту"]);
    ok(p, &["add", "Купить хлеб завтра"]);
    ok(p, &["add", "Выучить испанский когда-нибудь"]);

    let today = ok(p, &["list"]);
    assert_eq!(ok(p, &["list", "today"]), today);
    assert_eq!(ok(p, &["list", "сегодня"]), today);
    let titles: Vec<&str> = today
        .lines()
        .map(|l| l.split("] ").last().unwrap_or(l))
        .collect();
    assert_eq!(
        titles,
        [
            "Сегодня – чт, 19 ноября",
            "Позвонить маме",
            "Оплатить счёт #дом ⚑ сегодня",
            "Вечер",
            "Заказать ужин"
        ]
    );
    assert!(ok(p, &["list", "inbox"]).contains("] Разобрать почту\n"));
    assert!(ok(p, &["list", "upcoming"]).starts_with("пт, 20 ноября\n  ["));
    assert!(ok(p, &["list", "someday"]).contains("] Выучить испанский\n"));

    let done = ok(p, &["done", "маме"]);
    assert!(
        done.starts_with("Выполнено: [") && done.ends_with("] Позвонить маме\n"),
        "{done}"
    );
    let canceled = ok(p, &["done", "--cancel", "ужин"]);
    assert!(canceled.starts_with("Отменено: ["), "{canceled}");
    let logbook = ok(p, &["list", "logbook"]);
    assert!(
        logbook.contains("✓ [") && logbook.contains("✕ ["),
        "{logbook}"
    );
    assert!(!ok(p, &["list"]).contains("маме"));

    // Повторно – по номеру: задача закрыта.
    let id = id_of(&done);
    tally(p)
        .args(["done", &id])
        .assert()
        .code(1)
        .stdout("")
        .stderr(format!(
            "ошибка: задача [{id}] Позвонить маме уже закрыта\n"
        ));

    ok(p, &["project", "Ремонт", "кухни"]);
    ok(p, &["heading", "Покупки", "-p", "ремонт КУХНИ"]);
    let moved = ok(
        p,
        &[
            "move",
            "почту",
            "-p",
            "Ремонт кухни",
            "--heading",
            "Покупки",
            "-w",
            "пт",
        ],
    );
    assert!(
        moved.ends_with("] Разобрать почту – пт, 20 ноября · Ремонт кухни / Покупки\n"),
        "{moved}"
    );
    assert!(ok(p, &["list", "inbox"]).contains("пусто"));
    assert!(ok(p, &["list", "projects"]).contains("проект «Ремонт кухни» – 0 из 1"));

    let found = ok(p, &["search", "ПОЧТ"]);
    assert!(found.contains("] Разобрать почту"), "{found}");
    assert_eq!(ok(p, &["search", "нет такого"]), "Ничего не найдено\n");
}

#[test]
fn runtime_errors_have_code_1() {
    let d = db();
    let p = d.path.as_path();
    ok(p, &["add", "Купить хлеб"]);
    ok(p, &["add", "Купить молоко"]);
    for (args, message) in [
        (
            ["done", "zzzz"].as_slice(),
            "ошибка: задача «zzzz» не найдена\n".to_owned(),
        ),
        (
            &["add", "Отчёт", "-w", "31 ноября"],
            "ошибка: когда: такого дня нет: 31.11\n".to_owned(),
        ),
        (
            &["add", "Отчёт", "-p", "Нет"],
            "ошибка: проекта «Нет» нет\n".to_owned(),
        ),
        (
            &["move", "хлеб"],
            "ошибка: не указано, куда перенести задачу\n".to_owned(),
        ),
    ] {
        tally(p)
            .args(args)
            .assert()
            .code(1)
            .stdout("")
            .stderr(message);
    }
    tally(p).args(["done", "купить"]).assert().code(1).stderr(
        contains("ошибка: «купить» подходит к нескольким задачам:\n  [")
            .and(contains("Купить молоко")),
    );
    // Ошибка ничего не меняет.
    assert_eq!(ok(p, &["list", "inbox"]).lines().count(), 3);
}

#[test]
fn bad_today_is_reported() {
    let d = db();
    tally(&d.path)
        .env("TALLY_TODAY", "завтра")
        .arg("list")
        .assert()
        .code(1)
        .stderr(contains("ошибка: TALLY_TODAY:"));
}

#[test]
fn db_flag_wins_over_variable() {
    let a = db();
    let b = db();
    tally(&a.path)
        .args(["--db"])
        .arg(&b.path)
        .args(["add", "В базе Б"])
        .assert()
        .success();
    tally(&a.path)
        .args(["add", "В базе А", "--db"])
        .arg(&b.path)
        .assert()
        .success();
    assert!(!a.path.exists());
    let inbox = ok(&b.path, &["list", "inbox"]);
    assert!(
        inbox.contains("В базе Б") && inbox.contains("В базе А"),
        "{inbox}"
    );
}

#[test]
fn db_flag_rejects_blank_paths_and_keeps_spaces_in_other_paths() {
    let d = db();
    for blank in ["", "  \t\u{2003} "] {
        tally(&d.path)
            .current_dir(d._dir.path())
            .args(["--db", blank, "add", "Задача"])
            .assert()
            .code(2)
            .stderr(contains(if blank.is_empty() {
                "у --db <ФАЙЛ> нет значения"
            } else {
                "пустой путь к базе в --db"
            }));
        assert!(!d.path.exists());
    }

    let name = " task notes.db";
    tally(&d.path)
        .current_dir(d._dir.path())
        .args(["add", "Задача", "--db", name])
        .assert()
        .success();
    assert!(d._dir.path().join(name).is_file());
    assert!(!d._dir.path().join(name.trim()).exists());
    assert!(!d.path.exists());
}

#[test]
fn db_folder_is_created() {
    let d = db();
    let nested = d.path.with_file_name("a").join("b").join("tally.db");
    ok(&nested, &["add", "Задача"]);
    assert!(nested.exists());
}

#[test]
fn unreadable_db_is_reported() {
    let d = db();
    std::fs::write(&d.path, "это не база").unwrap_or_else(|e| panic!("{e}"));
    tally(&d.path)
        .arg("list")
        .assert()
        .code(1)
        .stderr(contains("ошибка: не удалось открыть базу"));
}

/// Несовместимые назначения `move` – ошибка командной строки до базы:
/// задача остаётся на месте.
#[test]
fn conflicting_move_targets_change_nothing() {
    let d = db();
    let p = d.path.as_path();
    ok(p, &["area", "Дом"]);
    ok(p, &["project", "Ремонт"]);
    ok(p, &["heading", "Покупки", "-p", "Ремонт"]);
    ok(
        p,
        &["add", "Краска", "-p", "Ремонт", "--heading", "Покупки"],
    );
    let before = ok(p, &["list", "anytime"]);
    for (args, message) in [
        (
            ["--heading", "Покупки", "-a", "Дом"].as_slice(),
            "ошибка: --heading <РАЗДЕЛ> нельзя вместе с --area <ОБЛАСТЬ>",
        ),
        (
            &["--heading", "Покупки", "--no-project"],
            "ошибка: --heading <РАЗДЕЛ> нельзя вместе с --no-project",
        ),
        (
            &["--heading", "Покупки", "--inbox"],
            "ошибка: --inbox нельзя вместе с --heading <РАЗДЕЛ>",
        ),
        (
            &["--inbox", "--no-project"],
            "ошибка: --inbox нельзя вместе с --no-project",
        ),
        (
            &["-a", "Дом", "--no-project"],
            "ошибка: --area <ОБЛАСТЬ> нельзя вместе с --no-project",
        ),
        (
            &["-a", "Дом", "-p", "Ремонт"],
            "ошибка: --project <ПРОЕКТ> нельзя вместе с --area <ОБЛАСТЬ>",
        ),
        (
            &["-w", "пт", "--inbox"],
            "ошибка: --when <КОГДА> нельзя вместе с --inbox",
        ),
        (
            &["-d", "пт", "--no-deadline"],
            "ошибка: --deadline <СРОК> нельзя вместе с --no-deadline",
        ),
    ] {
        let mut cmd = tally(p);
        cmd.args(["move", "Краска"]).args(args);
        let out = cmd.assert().code(2).stdout("").get_output().stderr.clone();
        let err = String::from_utf8(out).unwrap_or_default();
        // Порядок пары в сообщении – как его называет clap.
        let (first, second) = message
            .trim_start_matches("ошибка: ")
            .split_once(" нельзя вместе с ")
            .unwrap_or_default();
        assert!(
            err.contains(first) && err.contains(second) && err.contains("нельзя вместе с"),
            "{args:?}: {err}"
        );
    }
    assert_eq!(ok(p, &["list", "anytime"]), before);
}

/// Пустые и пробельные значения – ошибка командной строки, база не меняется.
#[test]
fn blank_values_are_usage_errors() {
    let d = db();
    let p = d.path.as_path();
    ok(p, &["project", "Ремонт"]);
    ok(p, &["add", "Краска"]);
    for (args, message) in [
        (
            ["add", "Письмо", "-t", "Срочно", "-t", "   "].as_slice(),
            "ошибка: пустое значение у --tag <ТЕГ>\n",
        ),
        (&["add", "  "], "ошибка: пустое значение у <ТЕКСТ>...\n"),
        (
            &["add", "Письмо", "-p", ""],
            "ошибка: пустое значение у --project <ПРОЕКТ>\n",
        ),
        (
            &["add", "Письмо", "-p", "Ремонт", "--heading", " "],
            "ошибка: пустое значение у --heading <РАЗДЕЛ>\n",
        ),
        (
            &["add", "Письмо", "-a", "\t"],
            "ошибка: пустое значение у --area <ОБЛАСТЬ>\n",
        ),
        (
            &["add", "Письмо", "-w", " "],
            "ошибка: пустое значение у --when <КОГДА>\n",
        ),
        (
            &["add", "Письмо", "-d", ""],
            "ошибка: пустое значение у --deadline <СРОК>\n",
        ),
        (
            &["project", " "],
            "ошибка: пустое значение у <НАЗВАНИЕ>...\n",
        ),
        (&["area", ""], "ошибка: пустое значение у <НАЗВАНИЕ>...\n"),
        (
            &["heading", " ", "-p", "Ремонт"],
            "ошибка: пустое значение у <НАЗВАНИЕ>...\n",
        ),
        (&["done", " "], "ошибка: пустое значение у <ЗАДАЧА>...\n"),
        (
            &["move", "Краска", "-p", " "],
            "ошибка: пустое значение у --project <ПРОЕКТ>\n",
        ),
        (&["search", " "], "ошибка: пустое значение у <ЗАПРОС>...\n"),
    ] {
        tally(p)
            .args(args)
            .assert()
            .code(2)
            .stdout("")
            .stderr(message);
    }
    assert_eq!(ok(p, &["list", "inbox"]), ok(p, &["list", "inbox"]));
    let inbox = ok(p, &["list", "inbox"]);
    assert_eq!(inbox.lines().count(), 2, "{inbox}");
    assert_eq!(
        ok(p, &["list", "projects"]),
        "Проекты\n  проект «Ремонт» – 0 из 0\n"
    );
    assert_eq!(ok(p, &["search", "Срочно"]), "Ничего не найдено\n");
    // Заметки могут быть пустыми.
    ok(p, &["add", "Письмо", "-n", ""]);
}

/// «Планы»: семь дней с завтрашнего, пустые – одним заголовком; вечер виден.
#[test]
fn upcoming_shows_seven_days_and_evening() {
    let d = db();
    let p = d.path.as_path();
    ok(p, &["add", "Отчёт за квартал в пятницу вечером"]);
    ok(p, &["add", "Встреча в понедельник"]);
    ok(p, &["add", "Поездка 5 декабря вечером"]);
    ok(p, &["add", "Сдать отчёт", "-d", "3 дек"]);
    let out = ok(p, &["list", "upcoming"]);
    let lines: Vec<&str> = out
        .lines()
        .map(|l| l.split("] ").last().unwrap_or(l))
        .collect();
    assert_eq!(
        lines,
        [
            "пт, 20 ноября",
            "Отчёт за квартал – вечер",
            "сб, 21 ноября",
            "вс, 22 ноября",
            "пн, 23 ноября",
            "Встреча",
            "вт, 24 ноября",
            "ср, 25 ноября",
            "чт, 26 ноября",
            "Декабрь",
            "Сдать отчёт – чт, 3 декабря ⚑ срок",
            "Поездка – сб, 5 декабря, вечер",
        ],
        "{out}"
    );
    // На пустой базе – семь заголовков, без «пусто».
    let e = db();
    let empty = ok(&e.path, &["list", "upcoming"]);
    assert_eq!(empty.lines().count(), 7, "{empty}");
    assert!(!empty.contains("пусто"), "{empty}");
}

/// По названию закрытый проект не в счёт и для `move`; по номеру задачу
/// такого проекта по-прежнему можно указать.
#[test]
fn title_lookup_skips_tasks_of_closed_projects() {
    let d = db();
    let p = d.path.as_path();
    ok(p, &["project", "Старый"]);
    let child = ok(p, &["add", "Отчёт", "-p", "Старый"]);
    ok(p, &["add", "Отчёт"]);
    {
        let mut store = tally_store::Store::open(p).unwrap_or_else(|e| panic!("{e}"));
        let mut projects = store.read().projects().unwrap_or_else(|e| panic!("{e}"));
        let mut project = projects.pop().unwrap_or_else(|| panic!("нет проекта"));
        project.status = tally_core::Status::Canceled {
            at: project.sync.updated_at,
        };
        store
            .write(|w| w.update_project(&project))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let moved = ok(p, &["move", "Отчёт", "-w", "пт"]);
    assert!(moved.ends_with("] Отчёт – пт, 20 ноября\n"), "{moved}");
    let id = id_of(&child);
    let moved = ok(p, &["move", &id, "--no-project"]);
    assert!(
        moved.contains(&format!("[{id}] Отчёт – В любое время")),
        "{moved}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn db_flag_accepts_native_paths() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let d = db();
    let path = d.path.with_file_name(OsStr::from_bytes(b"\xfe-tally.db"));
    tally(&d.path)
        .arg("--db")
        .arg(&path)
        .args(["add", "Письмо"])
        .assert()
        .success();
    assert!(path.is_file());
    assert!(!d.path.exists());
}

/// Удалить запись `id` и восстановить её после `between`: так в базе
/// появляются две живые одноимённые записи.
fn twin(p: &Path, id: tally_store::RecordId, between: &[&str]) {
    let at = jiff::Timestamp::now();
    let mut store = tally_store::Store::open(p).unwrap_or_else(|e| panic!("{e}"));
    store
        .write(|w| w.delete(id, at))
        .unwrap_or_else(|e| panic!("{e}"));
    ok(p, between);
    store
        .write(|w| w.restore(id, at))
        .unwrap_or_else(|e| panic!("{e}"));
}

/// Две живые записи с одним названием – ошибка со списком, ничего не пишется:
/// ни при создании ещё одной, ни при выборе по названию.
#[test]
fn homonyms_are_errors_not_guesses() {
    use tally_store::RecordId;

    let d = db();
    let p = d.path.as_path();
    ok(p, &["area", "Дом"]);
    ok(p, &["project", "Ремонт"]);
    ok(p, &["heading", "Покупки", "-p", "Ремонт"]);
    ok(p, &["add", "Краска", "-t", "Срочно"]);
    let snapshot = || {
        let store = tally_store::Store::open(p).unwrap_or_else(|e| panic!("{e}"));
        store.read().snapshot().unwrap_or_else(|e| panic!("{e}"))
    };
    let s = snapshot();
    twin(
        p,
        RecordId::Heading(s.headings[0].id),
        &["heading", "покупки", "-p", "Ремонт"],
    );
    for args in [
        ["heading", "Покупки", "-p", "Ремонт"].as_slice(),
        &["add", "Плитка", "-p", "Ремонт", "--heading", "Покупки"],
        &["move", "Краска", "-p", "Ремонт", "--heading", "Покупки"],
    ] {
        tally(p)
            .args(args)
            .assert()
            .code(1)
            .stdout("")
            .stderr(predicate::str::starts_with(
                "ошибка: в проекте «Ремонт» разделов «Покупки» несколько:\n  [",
            ));
    }
    twin(p, RecordId::Area(s.areas[0].id), &["area", "ДОМ"]);
    twin(
        p,
        RecordId::Project(s.projects[0].id),
        &["project", "ремонт"],
    );
    twin(
        p,
        RecordId::Tag(s.tags[0].id),
        &["add", "Лак", "-t", "срочно"],
    );
    let before = snapshot();
    assert_eq!(
        (
            before.areas.len(),
            before.projects.len(),
            before.tags.len(),
            before.tasks.len()
        ),
        (2, 2, 2, 2)
    );
    let short = |id: uuid::Uuid| id.simple().to_string()[..8].to_owned();
    let areas = format!(
        "ошибка: областей «дом» несколько:\n  [{}] Дом\n  [{}] ДОМ\n",
        short(before.areas[0].id.0),
        short(before.areas[1].id.0)
    );
    tally(p)
        .args(["area", "дом"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(areas);
    for (args, start) in [
        (
            ["project", "Ремонт"].as_slice(),
            "ошибка: проектов «Ремонт» несколько:\n  [",
        ),
        (
            &["add", "Плитка", "-p", "Ремонт"],
            "ошибка: проектов «Ремонт» несколько:\n  [",
        ),
        (
            &["add", "Плитка", "-a", "Дом"],
            "ошибка: областей «Дом» несколько:\n  [",
        ),
        (
            &["add", "Плитка", "-t", "СРОЧНО"],
            "ошибка: тегов «СРОЧНО» несколько:\n  [",
        ),
        (
            &["move", "Краска", "-a", "Дом"],
            "ошибка: областей «Дом» несколько:\n  [",
        ),
        (
            &["move", "Краска", "-p", "Ремонт"],
            "ошибка: проектов «Ремонт» несколько:\n  [",
        ),
    ] {
        tally(p)
            .args(args)
            .assert()
            .code(1)
            .stdout("")
            .stderr(predicate::str::starts_with(start));
    }
    let after = snapshot();
    assert_eq!(
        (
            after.areas.len(),
            after.projects.len(),
            after.headings.len(),
            after.tags.len(),
            after.tasks.len()
        ),
        (2, 2, 2, 2, 2)
    );
    assert_eq!(after.tasks, before.tasks);
}

/// Тег в корзине не виден ни в списках, ни в подтверждениях, ни в поиске;
/// новый одноимённый тег создаётся, а не берётся удалённый.
#[test]
fn trashed_tags_are_hidden_everywhere() {
    let d = db();
    let p = d.path.as_path();
    let added = ok(p, &["add", "Письмо", "-t", "Дом", "-t", "Работа"]);
    assert!(
        added.ends_with("Письмо – Входящие #Дом #Работа\n"),
        "{added}"
    );
    let tag = {
        let store = tally_store::Store::open(p).unwrap_or_else(|e| panic!("{e}"));
        let tags = store.read().tags().unwrap_or_else(|e| panic!("{e}"));
        tags.into_iter()
            .find(|t| t.title == "Дом")
            .map(|t| t.id)
            .unwrap_or_else(|| panic!("нет тега"))
    };
    {
        let mut store = tally_store::Store::open(p).unwrap_or_else(|e| panic!("{e}"));
        let at = jiff::Timestamp::now();
        store
            .write(|w| w.delete(tally_store::RecordId::Tag(tag), at))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    assert!(ok(p, &["list", "inbox"]).contains("] Письмо #Работа\n"));
    assert!(ok(p, &["move", "Письмо", "-w", "пт"]).ends_with("] Письмо – пт, 20 ноября #Работа\n"));
    assert_eq!(ok(p, &["search", "письмо"]).matches("#Дом").count(), 0);
    assert!(ok(p, &["search", "дом"]).starts_with("Ничего не найдено"));
    let again = ok(p, &["add", "Счёт", "-t", "дом"]);
    assert!(again.ends_with("] Счёт – Входящие #дом\n"), "{again}");
}
