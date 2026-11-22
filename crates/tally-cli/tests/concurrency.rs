//! Два клиента одновременно: команды `tally` в разных потоках, у каждой своё
//! соединение с одной базой. Поиск задачи, проверка и запись должны идти в
//! одной транзакции – иначе второй клиент пишет по устаревшему снимку.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Barrier;
use std::thread;

use jiff::civil::date;
use tally_core::Status;
use tally_store::{Store, normalize};

/// `tally --db <db> <args>` в этом процессе: код выхода, вывод, ошибки.
fn tally(db: &Path, args: &[&str]) -> (u8, String, String) {
    let argv = ["tally".into(), "--db".into(), db.as_os_str().to_owned()]
        .into_iter()
        .chain(args.iter().map(OsString::from));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = tally_cli::run(argv, &mut out, &mut err);
    let text = |b: Vec<u8>| String::from_utf8(b).unwrap_or_else(|e| panic!("{e}"));
    (code, text(out), text(err))
}

/// Номер задачи из «Добавлено: [abc123] …».
fn id_of(line: &str) -> String {
    let start = line.find('[').map_or(0, |i| i + 1);
    let end = line.find(']').unwrap_or(start);
    line[start..end].to_owned()
}

/// Запустить команды одновременно – каждую в своём потоке, старт по барьеру.
fn together(db: &Path, commands: &[Vec<String>]) -> Vec<(u8, String, String)> {
    let barrier = Barrier::new(commands.len());
    thread::scope(|scope| {
        let handles: Vec<_> = commands
            .iter()
            .map(|args| {
                let barrier = &barrier;
                scope.spawn(move || {
                    let args: Vec<&str> = args.iter().map(String::as_str).collect();
                    barrier.wait();
                    tally(db, &args)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| panic!("поток упал")))
            .collect()
    })
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

/// `done` и `move` одной задачи одновременно. Если `done` сообщил об успехе,
/// задача должна остаться закрытой, чем бы ни кончился `move`.
#[test]
fn move_does_not_reopen_a_task_closed_meanwhile() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let db = dir.path().join("tally.db");
    let target = date(2030, 1, 15);
    let mut problems = Vec::new();
    // Сколько раундов move успел до done и сколько – после.
    let (mut moved_first, mut closed_first) = (0, 0);
    for round in 0..40 {
        let title = format!("Задача {round}");
        let (code, out, err) = tally(&db, &["add", &title]);
        assert_eq!(code, 0, "{err}");
        let id = id_of(&out);
        let results = together(
            &db,
            &[
                args(&["done", &id]),
                args(&["move", &id, "-w", "15 января 2030"]),
            ],
        );
        let (done, moved) = (&results[0], &results[1]);
        let store = Store::open(&db).unwrap_or_else(|e| panic!("{e}"));
        let tasks = store.read().tasks().unwrap_or_else(|e| panic!("{e}"));
        let task = tasks
            .iter()
            .find(|t| t.title == title)
            .unwrap_or_else(|| panic!("нет задачи {round}"));
        // Закрывает задачу только done – он обязан успеть в каждом раунде.
        if *done != (0, format!("Выполнено: [{id}] {title}\n"), String::new()) {
            problems.push(format!("раунд {round}: done {done:?}"));
        }
        // Итог всегда один: задача выполнена, время выполнения записано.
        if !matches!(task.status, Status::Completed { .. }) {
            problems.push(format!(
                "раунд {round}: задача не выполнена: {:?}",
                task.status
            ));
        }
        // У move два исхода: успел первым – его дата сохранилась; опоздал –
        // «уже закрыта», и дата не тронута.
        match moved.0 {
            0 if task.date == Some(target) => moved_first += 1,
            1 if moved.2 == format!("ошибка: задача [{id}] {title} уже закрыта\n")
                && task.date.is_none() =>
            {
                closed_first += 1;
            }
            _ => problems.push(format!(
                "раунд {round}: move {moved:?}, дата {:?}",
                task.date
            )),
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(moved_first + closed_first, 40);
}

/// Одновременные `add -t Работа`: один тег на всех и разные ключи порядка.
#[test]
fn parallel_adds_share_one_tag_and_distinct_ranks() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let db = dir.path().join("tally.db");
    let (code, _, err) = tally(&db, &["list"]);
    assert_eq!(code, 0, "{err}");
    let mut duplicated = Vec::new();
    for round in 0..15 {
        let commands: Vec<Vec<String>> = (0..4)
            .map(|i| {
                args(&[
                    "add",
                    &format!("Задача {round}.{i}"),
                    "-t",
                    &format!("Работа {round}"),
                ])
            })
            .collect();
        for (code, _, err) in together(&db, &commands) {
            assert_eq!(code, 0, "{err}");
        }
        let store = Store::open(&db).unwrap_or_else(|e| panic!("{e}"));
        let tags = store.read().tags().unwrap_or_else(|e| panic!("{e}"));
        let name = normalize(&format!("Работа {round}"));
        let same = tags.iter().filter(|t| normalize(&t.title) == name).count();
        if same != 1 {
            duplicated.push((round, same));
        }
    }
    assert!(
        duplicated.is_empty(),
        "одноимённые теги (раунд, сколько): {duplicated:?}"
    );
    let store = Store::open(&db).unwrap_or_else(|e| panic!("{e}"));
    let tasks = store.read().tasks().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tasks.len(), 60);
    let index: HashSet<_> = tasks.iter().map(|t| &t.index).collect();
    let today_index: HashSet<_> = tasks.iter().map(|t| &t.today_index).collect();
    assert_eq!(
        (index.len(), today_index.len()),
        (60, 60),
        "одинаковые ключи порядка"
    );
    let tags = store.read().tags().unwrap_or_else(|e| panic!("{e}"));
    let tag_index: HashSet<_> = tags.iter().map(|t| &t.index).collect();
    assert_eq!(
        tag_index.len(),
        tags.len(),
        "одинаковые ключи порядка тегов"
    );
}
