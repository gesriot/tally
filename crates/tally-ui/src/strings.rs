//! Все строки интерфейса – здесь, и только по-русски (без `@tr`).
//! В `.slint` текстов нет: всё, что видно в окне, приходит отсюда.
//! Даты («пт, 20 ноября», «До конца ноября») – общие с консолью
//! (`tally_client::show`).

use jiff::civil::Date;
use tally_app::Error;
use tally_core::lists::Group;
use tally_core::schedule::DeadlineState;

/// Заголовок окна.
pub const EDITOR_REMINDER_ERROR: &str =
    "Укажите дату и время напоминания в формате ДД.ММ.ГГГГ ЧЧ:ММ";
pub const EDITOR_REMINDER_TIME_ERROR: &str =
    "Это время неоднозначно или пропущено при переводе часов. Выберите другое время.";
pub const WINDOW_TITLE: &str = "Tally";

pub const INBOX: &str = "Входящие";
pub const TODAY: &str = "Сегодня";
pub const UPCOMING: &str = "Планы";
pub const ANYTIME: &str = "В любое время";
pub const SOMEDAY: &str = "Когда-нибудь";
pub const LOGBOOK: &str = "Журнал";
pub const TRASH: &str = "Корзина";
pub const SEARCH: &str = "Поиск";
pub const SEARCH_PLACEHOLDER: &str = "Задача, проект, область или тег";
pub const SEARCH_CLOSE: &str = "Закрыть поиск";
pub const SEARCH_EMPTY: &str = "Ничего не найдено";
pub const SEARCH_PENDING: &str = "Поиск…";
pub const SEARCH_AREA: &str = "Область";
pub const SEARCH_PROJECT: &str = "Проект";
pub const SEARCH_TAG: &str = "Тег";
pub const FILTER_TAG: &str = "Фильтр по тегу";
pub const ALL_TAGS: &str = "Все теги";
pub const FILTER_EMPTY: &str = "Нет записей с этим тегом";

/// Блок «Вечер» в «Сегодня».
pub const EVENING: &str = "Вечер";
/// Пометка записи «Планов», стоящей в этот день из-за срока.
pub const DEADLINE_DAY: &str = "срок";
/// Вечер дня в «Планах»: «вечер», «ср, 15 декабря, вечер».
pub const EVENING_SHORT: &str = "вечер";

// Карточка открытой или завершённой задачи.
pub const EDITOR_TITLE_ERROR: &str = "Введите название задачи";
pub const EDITOR_CHECK_ERROR: &str = "Заполните или удалите пустой пункт чек-листа";
pub const EDITOR_DEADLINE_ERROR: &str = "Укажите день срока без времени суток";
pub const EDITOR_FINISH_FIRST: &str = "Сохраните или закройте открытую карточку";
pub const EDITOR_REFRESH_FAILED: &str =
    "Не удалось обновить задачу. Нажмите F5 перед повторным открытием";
pub const EDITOR_WHEN: &str = "Когда";
pub const EDITOR_DEADLINE: &str = "Срок";
pub const REPEAT: &str = "Повтор";
pub const REPEAT_ERROR: &str = "Укажите интервал от 1 до 65 535";
pub const TRASH_MOVE: &str = "В корзину";
pub const TRASH_RESTORE: &str = "Восстановить";
pub const TRASH_EMPTY: &str = "Очистить корзину";
pub fn trash_empty_prompt(preview: &tally_app::TrashPreview) -> String {
    format!(
        "Окончательно удалить содержимое корзины?\nЗаписей – {}, пунктов чек-листа – {}.",
        preview.records.len(),
        preview.checklist_items.len()
    )
}
pub const TRASH_CONFIRM: &str = "Удалить навсегда";
pub const TRASH_CHANGED: &str = "Содержимое корзины изменилось. Подтвердите очистку ещё раз";
pub const TRASH_EMPTIED: &str = "Корзина очищена";
pub const PROJECT_REPEAT_SET: &str = "Настройки повтора сохранены";

pub(crate) fn editor_labels() -> crate::ui::EditorLabels {
    crate::ui::EditorLabels {
        edit: "Редактировать".into(),
        title: "Название задачи".into(),
        notes: "Заметки".into(),
        checklist: "Чек-лист".into(),
        check_item: "Пункт чек-листа".into(),
        new_check: "Новый пункт".into(),
        tags: "Теги".into(),
        new_tag: "Имя тега".into(),
        add: "Добавить".into(),
        add_check: "Добавить пункт".into(),
        add_tag: "Добавить тег".into(),
        remove: "Удалить".into(),
        save: "Сохранить".into(),
        reopen: "Открыть заново".into(),
        saving: "Сохранение…".into(),
        cancel: "Отмена".into(),
        apply: "Применить".into(),
        clear: "Очистить".into(),
        when_label: EDITOR_WHEN.into(),
        deadline_label: EDITOR_DEADLINE.into(),
        date_hint: "Например, завтра или 15 дек".into(),
        date_input: "Дата".into(),
        today: TODAY.into(),
        tomorrow: "Завтра".into(),
        evening: "Сегодня вечером".into(),
        anytime: ANYTIME.into(),
        someday: SOMEDAY.into(),
    }
}

pub(crate) fn organization_labels() -> crate::ui::OrganizationLabels {
    crate::ui::OrganizationLabels {
        add_area: "Новая область".into(),
        add_project: "Новый проект".into(),
        add_area_project: "Проект в области".into(),
        add_section: "Новый раздел".into(),
        rename: "Переименовать".into(),
        close_project: "Закрыть проект".into(),
        rename_area: "Переименовать область".into(),
        rename_project: "Переименовать проект".into(),
        title: "Название".into(),
        save: "Сохранить".into(),
        dismiss: "Назад".into(),
        complete: "Завершить".into(),
        cancel_project: "Отменить проект".into(),
    }
}

pub(crate) const ORGANIZATION_REFRESH_FAILED: &str =
    "Не удалось обновить данные. Нажмите F5 перед повторным открытием формы";
pub(crate) const PROJECT_CLOSE_CHANGED: &str = "Проект изменился. Подтвердите закрытие ещё раз";
pub(crate) const PROJECT_CLOSE_UNAVAILABLE: &str = "Проект больше недоступен";

pub(crate) fn project_close_prompt(title: &str) -> String {
    format!("Закрыть «{title}»?")
}

pub(crate) fn project_closed(title: &str, cancel: bool) -> String {
    let action = if cancel {
        "отменён"
    } else {
        "завершён"
    };
    format!("Проект «{title}» {action}")
}

/// День в «Планах» для группы месяца или года: «ср, 15 декабря».
pub fn date(d: Date, today: Date) -> String {
    tally_client::show::date(d, today)
}

/// Заголовок группы «Планов».
pub fn group(g: Group, today: Date) -> String {
    tally_client::show::group(g, today)
}

pub(crate) fn repeat_labels() -> crate::ui::RepeatLabels {
    use slint::{ModelRc, SharedString, VecModel};
    use std::rc::Rc;
    let model = |words: &[&str]| {
        ModelRc::from(Rc::new(VecModel::from(
            words
                .iter()
                .copied()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        )))
    };
    crate::ui::RepeatLabels {
        title: REPEAT.into(),
        interval: "Интервал".into(),
        unit: "Единица интервала".into(),
        mode: "Начало отсчёта".into(),
        units: model(&["Дни", "Недели", "Месяцы", "Годы"]),
        modes: model(&["По запланированной дате", "После выполнения"]),
        apply: "Применить".into(),
        clear: "Убрать повтор".into(),
    }
}

/// «ср, 15 декабря, вечер».
pub fn with_evening(day: &str) -> String {
    format!("{day}, {EVENING_SHORT}")
}

/// Срок у задачи: «сегодня», «пт, 27 ноября», «просрочено: вт, 17 ноября».
pub fn deadline(d: Date, today: Date) -> String {
    match DeadlineState::of(d, today) {
        DeadlineState::Today => "сегодня".to_owned(),
        DeadlineState::Overdue { .. } => format!("просрочено: {}", date(d, today)),
        DeadlineState::Ahead { .. } => date(d, today),
    }
}

/// Ход проекта: «2 из 5».
pub fn progress(done: usize, total: usize) -> String {
    format!("{done} из {total}")
}

/// Где задача на странице проекта или области, если не «В любое время»:
/// «Сегодня», «Сегодня, вечер», «пт, 20 ноября», «Когда-нибудь».
pub fn when_today(evening: bool) -> String {
    if evening {
        format!("{TODAY}, {EVENING_SHORT}")
    } else {
        TODAY.to_owned()
    }
}

/// Строка «Корзины» для удалённого раздела.
pub fn trashed_heading(heading: &str, project: &str) -> String {
    format!("{heading} – раздел проекта «{project}»")
}

/// Строка «Корзины» для удалённой области.
pub fn trashed_area(area: &str) -> String {
    format!("{area} – область")
}

/// Снятие отметки не прошло: после закрытия задачи в журнале появилось
/// другое действие (например, из консоли), отменять закрытие отдельно
/// нельзя.
pub const REOPEN_NOT_LAST: &str = "После выполнения задачи было другое действие";

/// После отмены и повтора: «Отменено: выполнение задачи «Отчёт»».
pub fn undone(action: &str) -> String {
    format!("Отменено: {action}")
}

pub fn redone(action: &str) -> String {
    format!("Повторено: {action}")
}

/// Ошибка команды – как её сформулировал `tally-app` (по-русски, с
/// причиной), с заглавной буквы: в консоли перед ней стоит «ошибка: », а в
/// окне она – отдельная фраза.
pub fn error(e: &Error) -> String {
    capitalized(&reason(e))
}

fn reason(e: &Error) -> String {
    match e {
        Error::Store(e) => format!("ошибка базы: {e}"),
        other => other.to_string(),
    }
}

/// Базу не удалось прочитать при запуске или обновлении.
pub fn load_failed(e: &Error) -> String {
    format!("Не удалось прочитать базу: {}", reason(e))
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date as d;

    const NOW: Date = d(2026, 11, 19);

    #[test]
    fn deadlines_read_naturally() {
        assert_eq!(deadline(NOW, NOW), "сегодня");
        assert_eq!(deadline(d(2026, 11, 20), NOW), "пт, 20 ноября");
        assert_eq!(deadline(d(2026, 11, 17), NOW), "просрочено: вт, 17 ноября");
        assert_eq!(deadline(d(2027, 1, 6), NOW), "ср, 6 января 2027");
    }

    #[test]
    fn errors_keep_the_reason_from_tally_app() {
        assert_eq!(error(&Error::NothingToUndo), "Нечего отменять");
        assert_eq!(error(&Error::NothingToRedo), "Нечего повторять");
        let conflict = Error::Conflict(
            "нельзя отменить выполнение задачи «Плитка»: проект «Ремонт» в корзине".into(),
        );
        assert_eq!(
            error(&conflict),
            "Нельзя отменить выполнение задачи «Плитка»: проект «Ремонт» в корзине"
        );
        assert_eq!(error(&Error::Stopped), "Поток базы остановлен");
        assert_eq!(
            load_failed(&Error::Stopped),
            "Не удалось прочитать базу: поток базы остановлен"
        );
        assert_eq!(capitalized(""), "");
    }

    #[test]
    fn only_en_dashes() {
        for s in [
            trashed_heading("Кухня", "Ремонт"),
            trashed_area("Дом"),
            undone("перенос задачи «Отчёт»"),
            with_evening("ср, 15 декабря"),
            when_today(true),
            progress(2, 5),
        ] {
            assert!(!s.contains('\u{2014}'), "{s}");
        }
    }
}
