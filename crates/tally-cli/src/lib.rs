//! Консольный клиент Tally: `tally add`, `list`, `done`, `move`, `search`,
//! `undo`, `redo`.
//!
//! Справка и сообщения – по-русски ([`clap_ru`]). Команды выполняет
//! `tally-app` (транзакции, журнал действий, отмена и повтор); здесь –
//! разбор аргументов и вывод. Списки – правила `tally-core`. Путь к базе и
//! сегодняшний день – `tally_client::env`.

pub mod app;
pub mod clap_ru;
pub mod cli;

use std::ffi::OsString;
use std::io::Write;

use anyhow::Result;
use clap::error::ErrorKind;
use clap::{CommandFactory, FromArgMatches};
use tally_client::{desktop, env};
use tally_store::Store;

use crate::app::App;
use crate::cli::Cli;

/// Успех.
pub const EXIT_OK: u8 = 0;
/// Ошибка выполнения: нет задачи, неверная дата, ошибка базы.
pub const EXIT_FAILED: u8 = 1;
/// Неверные аргументы командной строки.
pub const EXIT_USAGE: u8 = 2;

/// Разобрать аргументы и выполнить команду. Вывод – в `out`, ошибки – в
/// `err`; результат – код выхода.
pub fn run(
    args: impl IntoIterator<Item = OsString>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let cmd = clap_ru::localize(Cli::command());
    let parsed = cmd
        .try_get_matches_from(args)
        .and_then(|m| Cli::from_arg_matches(&m));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(e) => {
            let (text, to_out, code) = match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    (e.render().to_string(), true, EXIT_OK)
                }
                ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    (e.render().to_string(), false, EXIT_USAGE)
                }
                _ => (
                    format!("ошибка: {}\n", clap_ru::message(&e)),
                    false,
                    EXIT_USAGE,
                ),
            };
            // Вывод недоступен – сообщить больше некуда.
            let _ = if to_out {
                write!(out, "{text}")
            } else {
                write!(err, "{text}")
            };
            return code;
        }
    };
    if let Some(path) = &cli.db
        && let Err(e) = env::check_db_flag(path.as_os_str())
    {
        let _ = writeln!(err, "ошибка: {e}");
        return EXIT_USAGE;
    }
    match execute(cli, out) {
        Ok(()) => EXIT_OK,
        Err(e) => {
            let _ = writeln!(err, "ошибка: {e:#}");
            EXIT_FAILED
        }
    }
}

fn execute(cli: Cli, out: &mut dyn Write) -> Result<()> {
    let path = env::db_path(cli.db.as_deref(), env::var(env::DB_VAR).as_deref())?;
    if matches!(cli.command, cli::Command::QuickAdd) {
        return desktop::open_quick_add(&path);
    }
    let today = env::today(env::var(env::TODAY_VAR).as_deref())?;
    env::ensure_parent(&path)?;
    let store = Store::open(&path)
        .map_err(|e| anyhow::anyhow!("не удалось открыть базу {}: {e}", path.display()))?;
    let mut app = App {
        store,
        today,
        now: jiff::Timestamp::now(),
        out,
    };
    app.run(cli.command)
}
