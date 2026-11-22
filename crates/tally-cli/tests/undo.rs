//! `tally undo` и `tally redo`: сама программа, база во временной папке.
//! Каждая команда – отдельный процесс, поэтому журнал действий живёт в базе.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

struct Db {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn db() -> Db {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("tally.db");
    Db { _dir: dir, path }
}

fn tally(db: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tally"));
    cmd.env_clear()
        .env("TALLY_DB", db)
        .env("TALLY_TODAY", "2026-11-19");
    cmd
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap_or_else(|e| panic!("{e}"))
}

fn ok(db: &Path, args: &[&str]) -> String {
    let out = tally(db).args(args).assert().success().stderr("");
    text(&out.get_output().stdout)
}

/// Ошибка выполнения: код 1, пустой вывод, сообщение – в поток ошибок.
fn fails(db: &Path, args: &[&str]) -> String {
    let out = tally(db).args(args).assert().code(1).stdout("");
    text(&out.get_output().stderr)
}

fn id_of(line: &str) -> String {
    let start = line.find('[').map_or(0, |i| i + 1);
    let end = line.find(']').unwrap_or(start);
    line[start..end].to_owned()
}

/// Приёмочный сценарий: задача из раздела «Кухня» проекта «Ремонт» на пятницу
/// переносится в «Дачу» на четверг вечером. undo возвращает проект, раздел и
/// дату, redo – снова «Дачу» и четверг.
#[test]
fn move_to_another_project_is_undone_and_redone() {
    let d = db();
    ok(&d.path, &["project", "Ремонт"]);
    ok(&d.path, &["heading", "Кухня", "-p", "Ремонт"]);
    ok(&d.path, &["project", "Дача"]);
    let added = ok(
        &d.path,
        &[
            "add",
            "Купить плитку",
            "-p",
            "Ремонт",
            "--heading",
            "Кухня",
            "-w",
            "пт",
            "-d",
            "15 дек",
        ],
    );
    let id = id_of(&added);
    let before = format!("[{id}] Купить плитку – пт, 20 ноября · Ремонт / Кухня ⚑ вт, 15 декабря");
    let after = format!("[{id}] Купить плитку – чт, 26 ноября, вечер · Дача ⚑ вт, 15 декабря");
    assert_eq!(added, format!("Добавлено: {before}\n"));
    assert_eq!(
        ok(
            &d.path,
            &["move", &id, "-p", "Дача", "-w", "через неделю вечером"]
        ),
        format!("Перенесено: {after}\n")
    );
    assert_eq!(
        ok(&d.path, &["undo"]),
        format!("Отменено действие: перенос задачи «Купить плитку»\n  {before}\n")
    );
    assert_eq!(ok(&d.path, &["search", "плитку"]), format!("{before}\n"));
    assert_eq!(
        ok(&d.path, &["list", "projects"]),
        "Проекты\n  проект «Ремонт» – 0 из 1\n  проект «Дача» – 0 из 0\n"
    );
    assert_eq!(
        ok(&d.path, &["redo"]),
        format!("Повторено действие: перенос задачи «Купить плитку»\n  {after}\n")
    );
    assert_eq!(ok(&d.path, &["search", "плитку"]), format!("{after}\n"));
    assert_eq!(fails(&d.path, &["redo"]), "ошибка: нечего повторять\n");
}

#[test]
fn add_done_and_containers_are_undone_in_reverse_order() {
    let d = db();
    ok(&d.path, &["area", "Дом"]);
    ok(&d.path, &["project", "Ремонт", "-a", "Дом"]);
    let id = id_of(&ok(
        &d.path,
        &["add", "Позвонить маме сегодня", "-t", "Семья"],
    ));
    assert_eq!(
        ok(&d.path, &["done", &id]),
        format!("Выполнено: [{id}] Позвонить маме\n")
    );
    assert_eq!(
        ok(&d.path, &["undo"]),
        format!(
            "Отменено действие: выполнение задачи «Позвонить маме»\n  [{id}] Позвонить маме – Сегодня #Семья\n"
        )
    );
    assert_eq!(
        ok(&d.path, &["list"]),
        format!("Сегодня – чт, 19 ноября\n  [{id}] Позвонить маме #Семья\n")
    );
    assert_eq!(
        ok(&d.path, &["undo"]),
        "Отменено действие: добавление задачи «Позвонить маме»\n"
    );
    // Новый тег ушёл вместе с задачей.
    assert_eq!(ok(&d.path, &["search", "Семья"]), "Ничего не найдено\n");
    assert_eq!(
        ok(&d.path, &["undo"]),
        "Отменено действие: создание проекта «Ремонт»\n"
    );
    assert_eq!(
        ok(&d.path, &["undo"]),
        "Отменено действие: создание области «Дом»\n"
    );
    assert_eq!(fails(&d.path, &["undo"]), "ошибка: нечего отменять\n");
    assert_eq!(ok(&d.path, &["list", "projects"]), "Проекты\n  пусто\n");
    for action in ["создание области «Дом»", "создание проекта «Ремонт»"]
    {
        assert_eq!(
            ok(&d.path, &["redo"]),
            format!("Повторено действие: {action}\n")
        );
    }
    assert_eq!(
        ok(&d.path, &["redo"]),
        format!(
            "Повторено действие: добавление задачи «Позвонить маме»\n  [{id}] Позвонить маме – Сегодня #Семья\n"
        )
    );
    // Новая команда после отмены – повторять больше нечего.
    ok(&d.path, &["add", "Письмо"]);
    assert_eq!(fails(&d.path, &["redo"]), "ошибка: нечего повторять\n");
    assert_eq!(
        ok(&d.path, &["list", "projects"]),
        "Область «Дом»\n  проект «Ремонт» – 0 из 0\n"
    );
}

#[test]
fn empty_history_fails_with_code_1() {
    let d = db();
    assert_eq!(fails(&d.path, &["undo"]), "ошибка: нечего отменять\n");
    assert_eq!(fails(&d.path, &["redo"]), "ошибка: нечего повторять\n");
    // Лишний аргумент – ошибка командной строки.
    tally(&d.path)
        .args(["undo", "2"])
        .assert()
        .code(2)
        .stdout("");
}

/// Задачу изменили в обход журнала – undo отказывает с кодом 1 и ничего
/// не пишет.
#[test]
fn undo_after_an_outside_change_is_a_conflict() {
    let d = db();
    ok(&d.path, &["project", "Дача"]);
    let id = id_of(&ok(&d.path, &["add", "Полить грядки"]));
    ok(&d.path, &["move", &id, "-p", "Дача", "-w", "завтра"]);
    {
        let mut store = tally_store::Store::open(&d.path).unwrap_or_else(|e| panic!("{e}"));
        let mut task = store
            .read()
            .tasks()
            .unwrap_or_default()
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("нет задачи"));
        task.title = "Полить грядки и розы".into();
        task.sync
            .touch(task.sync.updated_at + jiff::SignedDuration::from_secs(1));
        let written = store.write(|w| w.update_task(&task));
        assert!(written.is_ok(), "{written:?}");
    }
    let listed = ok(&d.path, &["search", "грядки"]);
    assert_eq!(
        fails(&d.path, &["undo"]),
        format!(
            "ошибка: нельзя отменить перенос задачи «Полить грядки»: задачу [{id}] Полить грядки и розы изменили после этого действия\n"
        )
    );
    assert_eq!(ok(&d.path, &["search", "грядки"]), listed);
    assert_eq!(
        listed,
        format!("[{id}] Полить грядки и розы – пт, 20 ноября · Дача\n")
    );
}

#[test]
fn help_lists_undo_and_redo() {
    let d = db();
    let help = ok(&d.path, &["--help"]);
    for (command, description) in [
        ("undo", "Отменить последнее действие"),
        ("redo", "Повторить отменённое действие"),
        ("quick-add", "Открыть окно быстрого ввода"),
    ] {
        assert!(
            help.lines().any(|line| {
                line.trim()
                    .split_once(char::is_whitespace)
                    .is_some_and(|(name, text)| name == command && text.trim() == description)
            }),
            "{help}"
        );
    }
    let help = ok(&d.path, &["undo", "--help"]);
    assert!(help.contains("Использование: tally undo"), "{help}");
}

/// Перенос, который ничего не меняет, – успех с обычным выводом, но без
/// записи: ветка повтора остаётся.
#[test]
fn a_move_without_changes_keeps_redo() {
    let d = db();
    let id = id_of(&ok(&d.path, &["add", "Отчёт"]));
    ok(&d.path, &["done", "Отчёт"]);
    ok(&d.path, &["undo"]);
    assert_eq!(
        ok(&d.path, &["move", "Отчёт", "--no-deadline"]),
        format!("Перенесено: [{id}] Отчёт – Входящие\n")
    );
    assert_eq!(
        ok(&d.path, &["redo"]),
        format!("Повторено действие: выполнение задачи «Отчёт»\n  [{id}] Отчёт – Журнал\n")
    );
    assert_eq!(
        ok(&d.path, &["list", "logbook"]),
        format!("Журнал\n  ✓ [{id}] Отчёт\n")
    );
}
