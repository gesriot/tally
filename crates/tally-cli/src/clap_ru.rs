//! Справка и ошибки clap по-русски: свои заголовки разделов, флаги
//! `-h`/`-V` с русскими подписями и перевод ошибок разбора.

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Arg, ArgAction, ColorChoice, Command};

const TEMPLATE: &str = "\
{about-with-newline}
Использование: {usage}

{all-args}";

/// Русская справка для команды и всех подкоманд.
pub fn localize(cmd: Command) -> Command {
    let is_root = cmd.get_name() == "tally";
    let mut cmd = cmd
        .color(ColorChoice::Never)
        .help_template(TEMPLATE)
        .subcommand_help_heading("Команды")
        .subcommand_value_name("КОМАНДА")
        .disable_help_subcommand(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .mut_args(|a| {
            if a.get_help_heading().is_some() {
                a
            } else if a.is_positional() {
                a.help_heading("Аргументы")
            } else {
                a.help_heading("Параметры")
            }
        })
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .help("Показать справку")
                .help_heading("Параметры"),
        );
    if is_root {
        cmd = cmd.arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .action(ArgAction::Version)
                .help("Показать версию")
                .help_heading("Параметры"),
        );
    }
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_owned())
        .collect();
    for name in names {
        cmd = cmd.mut_subcommand(name, localize);
    }
    cmd
}

/// Сообщение об ошибке разбора аргументов по-русски.
pub fn message(err: &clap::Error) -> String {
    let get = |kind| err.get(kind).map(value_text);
    let arg = get(ContextKind::InvalidArg).unwrap_or_default();
    let value = get(ContextKind::InvalidValue).unwrap_or_default();
    match err.kind() {
        ErrorKind::InvalidValue => {
            let valid = get(ContextKind::ValidValue)
                .map(|v| format!("; можно: {v}"))
                .unwrap_or_default();
            if value.is_empty() {
                format!("у {arg} нет значения{valid}")
            } else {
                format!("неверное значение «{value}» у {arg}{valid}")
            }
        }
        ErrorKind::UnknownArgument => format!("неизвестный параметр «{arg}»"),
        ErrorKind::InvalidSubcommand => {
            let sub = get(ContextKind::InvalidSubcommand).unwrap_or_default();
            format!("неизвестная команда «{sub}»")
        }
        ErrorKind::MissingRequiredArgument => {
            format!("не хватает аргумента: {arg}")
        }
        ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            "не указана команда".to_owned()
        }
        ErrorKind::ArgumentConflict => {
            let other = get(ContextKind::PriorArg).unwrap_or_default();
            if other.is_empty() || other == arg {
                format!("{arg} указан несколько раз")
            } else {
                format!("{arg} нельзя вместе с {other}")
            }
        }
        ErrorKind::TooManyValues
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
        | ErrorKind::NoEquals => format!("неверное число значений у {arg}"),
        ErrorKind::ValueValidation => match std::error::Error::source(err) {
            // Своя проверка значения (`cli::text`) – её объяснение по-русски.
            Some(reason) => format!("{reason} у {arg}"),
            None => format!("неверное значение «{value}» у {arg}"),
        },
        ErrorKind::InvalidUtf8 => "аргументы – не UTF-8".to_owned(),
        _ => "неверные аргументы командной строки".to_owned(),
    }
}

fn value_text(v: &ContextValue) -> String {
    match v {
        ContextValue::String(s) => s.clone(),
        ContextValue::Strings(list) => list.join(", "),
        ContextValue::StyledStr(s) => s.to_string(),
        ContextValue::StyledStrs(list) => list
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        ContextValue::Number(n) => n.to_string(),
        ContextValue::Bool(b) => b.to_string(),
        ContextValue::None => String::new(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;
    use crate::cli::Cli;

    fn all(cmd: &Command, out: &mut Vec<Command>) {
        out.push(cmd.clone());
        for sub in cmd.get_subcommands() {
            all(sub, out);
        }
    }

    #[test]
    fn every_help_is_russian() {
        let mut cmds = Vec::new();
        all(&localize(Cli::command()), &mut cmds);
        assert_eq!(cmds.len(), 12);
        for mut cmd in cmds {
            let help = cmd.render_help().to_string();
            assert!(help.contains("Использование: tally"), "{help}");
            for english in [
                "Usage",
                "Options",
                "Commands",
                "Arguments",
                "Print",
                "default",
                "possible",
            ] {
                assert!(!help.contains(english), "{english} в\n{help}");
            }
            assert!(!help.contains('\u{2014}'), "{help}");
            for arg in cmd.get_arguments() {
                assert!(
                    arg.get_help().is_some() || arg.is_hide_set(),
                    "{:?} без описания",
                    arg.get_id()
                );
            }
        }
    }

    #[test]
    fn errors_are_russian() {
        let cmd = localize(Cli::command());
        for (args, expected) in [
            (["tally", "nope"].as_slice(), "неизвестная команда «nope»"),
            (
                &["tally", "add", "x", "--nope"],
                "неизвестный параметр «--nope»",
            ),
            (&["tally", "done"], "не хватает аргумента: <ЗАДАЧА>..."),
            (
                &["tally", "add", "x", "-p", "А", "-a", "Б"],
                "--project <ПРОЕКТ> нельзя вместе с --area <ОБЛАСТЬ>",
            ),
        ] {
            let err = cmd.clone().try_get_matches_from(args).err();
            assert_eq!(
                err.as_ref().map(message).as_deref(),
                Some(expected),
                "{args:?}"
            );
        }
    }
}
