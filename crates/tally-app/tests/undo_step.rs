//! Отмена конкретного действия ([`Command::UndoStep`]): окно снимает
//! отметку только с задачи, которую само закрыло, и только пока это
//! закрытие – последнее действие журнала. Сверка и отмена – в одной
//! транзакции; иначе ничего не пишется.

use jiff::Timestamp;
use jiff::civil::date;
use tally_app::{Clock, Close, Command, Done, Effect, Error, MoveTask, NewTask, execute};
use tally_core::TaskId;
use tally_store::{RecordId, Snapshot, Store};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn clock() -> Clock {
    Clock {
        today: date(2026, 11, 20),
        now: "2026-11-20T09:00:00Z"
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH),
    }
}

fn run(store: &mut Store, command: Command) -> Result<Done> {
    Ok(execute(store, &command, clock())?)
}

fn add(store: &mut Store, text: &str) -> Result<TaskId> {
    let done = run(
        store,
        Command::AddTask(NewTask {
            text: text.into(),
            ..NewTask::default()
        }),
    )?;
    match done.effect {
        Effect::Added(id) => Ok(id),
        other => Err(format!("ожидалось добавление, а не {other:?}").into()),
    }
}

fn close(store: &mut Store, id: TaskId) -> Result<Done> {
    run(
        store,
        Command::Close(Close {
            tasks: vec![id.to_string()],
            cancel: false,
        }),
    )
}

fn state(store: &Store) -> Result<(Snapshot, Vec<tally_store::HistoryEntry>)> {
    Ok((store.read().snapshot()?, store.read().history()?))
}

/// Номер в `Done::seq` – строка журнала, которую записала команда.
#[test]
fn done_carries_the_seq_of_its_history_row() -> Result {
    let mut store = Store::open_in_memory()?;
    let paint = add(&mut store, "Краска")?;
    let id = add(&mut store, "Плитка")?;
    let done = close(&mut store, id)?;
    let last = store.read().last_done()?.ok_or("журнал пуст")?;
    assert_eq!(done.seq, Some(last.seq));
    assert_eq!(last.action, "выполнение задачи «Плитка»");

    // Undo и redo возвращают номер строки, которую отменили и повторили.
    let undo = run(&mut store, Command::Undo)?;
    assert_eq!(undo.seq, Some(last.seq));
    let redo = run(&mut store, Command::Redo)?;
    assert_eq!(redo.seq, Some(last.seq));

    // Команда без изменений журнал не трогает – номера нет.
    let same = run(
        &mut store,
        Command::MoveTask(MoveTask {
            task: paint.to_string(),
            no_project: true,
            ..MoveTask::default()
        }),
    )?;
    assert_eq!(same.seq, None);
    Ok(())
}

#[test]
fn undo_step_undoes_exactly_that_close() -> Result {
    let mut store = Store::open_in_memory()?;
    let first = add(&mut store, "Плитка")?;
    let seq = close(&mut store, first)?.seq.ok_or("нет номера")?;
    let done = run(&mut store, Command::UndoStep(seq))?;
    let Effect::Undone(step) = done.effect else {
        return Err("ожидалась отмена".into());
    };
    assert_eq!(step.tasks, vec![first]);
    assert_eq!(step.action, "выполнение задачи «Плитка»");
    assert_eq!(done.seq, Some(seq));
    assert!(
        store
            .read()
            .task(first)?
            .ok_or("нет задачи")?
            .status
            .is_open()
    );
    // Отменённое закрытие повторяется обычным redo.
    let redo = run(&mut store, Command::Redo)?;
    assert!(matches!(redo.effect, Effect::Redone(_)));
    Ok(())
}

/// Окно закрыло задачу, консоль добавила другую – снятие
/// отметки ничего не трогает, Ctrl+Z (обычный undo) по-прежнему отменяет
/// добавление.
#[test]
fn a_newer_action_blocks_undo_step_and_nothing_is_written() -> Result {
    let mut store = Store::open_in_memory()?;
    let tile = add(&mut store, "Купить плитку")?;
    let seq = close(&mut store, tile)?.seq.ok_or("нет номера")?;
    add(&mut store, "Заказать доставку")?;
    let before = state(&store)?;
    let err = run(&mut store, Command::UndoStep(seq)).err();
    assert!(
        matches!(
            err.as_deref().and_then(|e| e.downcast_ref::<Error>()),
            Some(Error::NotLast)
        ),
        "{err:?}"
    );
    assert_eq!(state(&store)?, before);
    assert_eq!(
        Error::NotLast.to_string(),
        "после этого действия было другое действие"
    );
    let undo = run(&mut store, Command::Undo)?;
    let Effect::Undone(step) = undo.effect else {
        return Err("ожидалась отмена".into());
    };
    assert_eq!(step.action, "добавление задачи «Заказать доставку»");
    Ok(())
}

/// Действие уже отменили (например, Ctrl+Z в консоли): вершина журнала –
/// более раннее действие, отменять его вместо этого нельзя. После redo то
/// же действие снова на вершине – и снова отменяется по номеру.
#[test]
fn an_already_undone_step_is_not_last() -> Result {
    let mut store = Store::open_in_memory()?;
    let tile = add(&mut store, "Плитка")?;
    let seq = close(&mut store, tile)?.seq.ok_or("нет номера")?;
    run(&mut store, Command::Undo)?;
    let before = state(&store)?;
    let err = run(&mut store, Command::UndoStep(seq)).err();
    assert!(matches!(
        err.as_deref().and_then(|e| e.downcast_ref::<Error>()),
        Some(Error::NotLast)
    ));
    assert_eq!(state(&store)?, before);
    run(&mut store, Command::Redo)?;
    run(&mut store, Command::UndoStep(seq))?;
    assert!(
        store
            .read()
            .task(tile)?
            .ok_or("нет задачи")?
            .status
            .is_open()
    );
    Ok(())
}

/// Номера не переиспользуются (AUTOINCREMENT): отменённое и затёртое новым
/// действием закрытие не совпадёт по номеру с этим новым действием.
#[test]
fn a_replaced_step_never_matches_the_new_one() -> Result {
    let mut store = Store::open_in_memory()?;
    let tile = add(&mut store, "Плитка")?;
    let seq = close(&mut store, tile)?.seq.ok_or("нет номера")?;
    run(&mut store, Command::Undo)?;
    let newer = add(&mut store, "Доставка")?;
    let last = store.read().last_done()?.ok_or("журнал пуст")?;
    assert!(last.seq > seq);
    let before = state(&store)?;
    assert!(run(&mut store, Command::UndoStep(seq)).is_err());
    assert_eq!(state(&store)?, before);
    assert!(store.read().task(newer)?.is_some());
    Ok(())
}

#[test]
fn empty_history_is_not_last_either() -> Result {
    let mut store = Store::open_in_memory()?;
    let err = run(&mut store, Command::UndoStep(1)).err();
    assert!(matches!(
        err.as_deref().and_then(|e| e.downcast_ref::<Error>()),
        Some(Error::NotLast)
    ));
    Ok(())
}

/// Закрытие – вершина журнала, но задачу изменили в обход журнала:
/// обычный конфликт отмены, база и журнал не меняются.
#[test]
fn undo_step_keeps_the_usual_conflict_checks() -> Result {
    let mut store = Store::open_in_memory()?;
    let tile = add(&mut store, "Плитка")?;
    let seq = close(&mut store, tile)?.seq.ok_or("нет номера")?;
    store.write(|w| w.delete(RecordId::Task(tile), clock().now))?;
    let before = state(&store)?;
    let err = run(&mut store, Command::UndoStep(seq)).err();
    let text = err.as_ref().map(ToString::to_string).unwrap_or_default();
    assert!(
        text.starts_with("нельзя отменить выполнение задачи «Плитка»: "),
        "{text}"
    );
    assert_eq!(state(&store)?, before);
    Ok(())
}
