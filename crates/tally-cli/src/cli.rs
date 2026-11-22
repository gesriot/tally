//! Аргументы командной строки (clap). Подписи справки – по-русски, см.
//! [`crate::clap_ru`].

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Текстовое значение аргумента: без пробелов по краям и не пустое. Пустое
/// или из одних пробелов – ошибка командной строки (код 2), до обращения к базе.
pub fn text(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err("пустое значение".to_owned())
    } else {
        Ok(value.to_owned())
    }
}

/// Tally – дела в духе Things 3 из командной строки.
#[derive(Debug, Parser)]
#[command(
    name = "tally",
    version,
    override_usage = "tally [ПАРАМЕТРЫ] <КОМАНДА>"
)]
pub struct Cli {
    /// Файл базы. Иначе – переменная TALLY_DB или файл tally.db в папке
    /// данных пользователя
    #[arg(long, global = true, value_name = "ФАЙЛ")]
    pub db: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Добавить задачу; дата в конце текста разбирается, как в быстром вводе
    #[command(override_usage = "tally add [ПАРАМЕТРЫ] <ТЕКСТ>...")]
    Add(AddArgs),
    /// Открыть окно быстрого ввода
    #[command(override_usage = "tally quick-add [--db ФАЙЛ]")]
    QuickAdd,
    /// Показать список: сегодня, входящие, планы и другие
    #[command(override_usage = "tally list [СПИСОК]")]
    List(ListArgs),
    /// Отметить задачи выполненными
    #[command(override_usage = "tally done [ПАРАМЕТРЫ] <ЗАДАЧА>...")]
    Done(DoneArgs),
    /// Перенести задачу: на дату, в список, проект, раздел или область
    #[command(override_usage = "tally move [ПАРАМЕТРЫ] <ЗАДАЧА>")]
    Move(MoveArgs),
    /// Найти задачи, проекты, области и теги
    #[command(override_usage = "tally search <ЗАПРОС>...")]
    Search(SearchArgs),
    /// Создать проект
    #[command(override_usage = "tally project [ПАРАМЕТРЫ] <НАЗВАНИЕ>...")]
    Project(ProjectArgs),
    /// Создать область
    #[command(override_usage = "tally area <НАЗВАНИЕ>...")]
    Area(AreaArgs),
    /// Создать раздел в проекте
    #[command(override_usage = "tally heading --project <ПРОЕКТ> <НАЗВАНИЕ>...")]
    Heading(HeadingArgs),
    /// Отменить последнее действие
    #[command(override_usage = "tally undo")]
    Undo,
    /// Повторить отменённое действие
    #[command(override_usage = "tally redo")]
    Redo,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Название задачи; «завтра», «в пятницу вечером», «15 дек» в конце –
    /// дата начала
    #[arg(value_parser = text, required = true, num_args = 1.., value_name = "ТЕКСТ")]
    pub text: Vec<String>,
    /// Когда начать: «завтра», «пт», «через 3 дня», «когда-нибудь»; текст
    /// тогда не разбирается
    #[arg(value_parser = text, short, long, value_name = "КОГДА")]
    pub when: Option<String>,
    /// Крайний срок: «пт», «15 дек», «через 2 недели»
    #[arg(value_parser = text, short, long, value_name = "СРОК")]
    pub deadline: Option<String>,
    /// Проект (по названию)
    #[arg(value_parser = text, short, long, value_name = "ПРОЕКТ", conflicts_with = "area")]
    pub project: Option<String>,
    /// Раздел проекта (по названию)
    #[arg(value_parser = text, long, value_name = "РАЗДЕЛ", requires = "project")]
    pub heading: Option<String>,
    /// Область (по названию) – для задачи вне проекта
    #[arg(value_parser = text, short, long, value_name = "ОБЛАСТЬ")]
    pub area: Option<String>,
    /// Заметки
    #[arg(short, long, value_name = "ТЕКСТ")]
    pub notes: Option<String>,
    /// Тег; нового тега ещё нет – он создаётся. Можно несколько раз
    #[arg(value_parser = text, short, long = "tag", value_name = "ТЕГ")]
    pub tags: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Список: today (сегодня), inbox (входящие), upcoming (планы),
    /// anytime (любое), someday (когда-нибудь), logbook (журнал),
    /// trash (корзина), projects (проекты). Без списка – сегодня
    #[arg(
        value_enum,
        value_name = "СПИСОК",
        hide_possible_values = true,
        hide_default_value = true,
        default_value_t = ListName::Today
    )]
    pub list: ListName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ListName {
    #[value(alias = "сегодня")]
    Today,
    #[value(alias = "входящие")]
    Inbox,
    #[value(alias = "планы")]
    Upcoming,
    #[value(alias = "любое")]
    Anytime,
    #[value(alias = "когда-нибудь")]
    Someday,
    #[value(alias = "журнал")]
    Logbook,
    #[value(alias = "корзина")]
    Trash,
    #[value(alias = "проекты")]
    Projects,
}

#[derive(Debug, Args)]
pub struct DoneArgs {
    /// Задача: начало её номера из списка («3f9a2c») или часть названия
    #[arg(value_parser = text, required = true, num_args = 1.., value_name = "ЗАДАЧА")]
    pub tasks: Vec<String>,
    /// Отменить, а не выполнить
    #[arg(short, long)]
    pub cancel: bool,
}

#[derive(Debug, Args)]
pub struct MoveArgs {
    /// Задача: начало её номера из списка («3f9a2c») или часть названия
    #[arg(value_parser = text, value_name = "ЗАДАЧА")]
    pub task: String,
    /// Когда начать: «сегодня», «завтра вечером», «пт», «в любое время»,
    /// «когда-нибудь»
    #[arg(value_parser = text, short, long, value_name = "КОГДА", conflicts_with = "inbox")]
    pub when: Option<String>,
    /// Во «Входящие»: без даты, проекта и области
    #[arg(long, conflicts_with_all = ["project", "area", "heading", "no_project"])]
    pub inbox: bool,
    /// В проект (по названию)
    #[arg(value_parser = text, short, long, value_name = "ПРОЕКТ", conflicts_with_all = ["area", "no_project"])]
    pub project: Option<String>,
    /// В раздел проекта – указанного или текущего
    #[arg(value_parser = text, long, value_name = "РАЗДЕЛ", conflicts_with_all = ["area", "no_project"])]
    pub heading: Option<String>,
    /// В область (по названию); задача уходит из проекта
    #[arg(value_parser = text, short, long, value_name = "ОБЛАСТЬ", conflicts_with = "no_project")]
    pub area: Option<String>,
    /// Убрать из проекта и области
    #[arg(long)]
    pub no_project: bool,
    /// Крайний срок
    #[arg(value_parser = text, short, long, value_name = "СРОК", conflicts_with = "no_deadline")]
    pub deadline: Option<String>,
    /// Убрать крайний срок
    #[arg(long)]
    pub no_deadline: bool,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Слова запроса: каждое – часть названия или заметок
    #[arg(value_parser = text, required = true, num_args = 1.., value_name = "ЗАПРОС")]
    pub query: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ProjectArgs {
    /// Название проекта
    #[arg(value_parser = text, required = true, num_args = 1.., value_name = "НАЗВАНИЕ")]
    pub title: Vec<String>,
    /// Область проекта (по названию)
    #[arg(value_parser = text, short, long, value_name = "ОБЛАСТЬ")]
    pub area: Option<String>,
}

#[derive(Debug, Args)]
pub struct AreaArgs {
    /// Название области
    #[arg(value_parser = text, required = true, num_args = 1.., value_name = "НАЗВАНИЕ")]
    pub title: Vec<String>,
}

#[derive(Debug, Args)]
pub struct HeadingArgs {
    /// Название раздела
    #[arg(value_parser = text, required = true, num_args = 1.., value_name = "НАЗВАНИЕ")]
    pub title: Vec<String>,
    /// Проект раздела (по названию)
    #[arg(value_parser = text, short, long, required = true, value_name = "ПРОЕКТ")]
    pub project: String,
}
