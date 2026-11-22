//! Окружение запуска: путь к базе и сегодняшний день.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use jiff::civil::Date;

/// Переменная с путём к базе.
pub const DB_VAR: &str = "TALLY_DB";
/// Переменная с сегодняшним днём `YYYY-MM-DD` – для тестов и примеров.
pub const TODAY_VAR: &str = "TALLY_TODAY";
/// Имя файла базы в папке данных.
pub const DB_FILE: &str = "tally.db";

/// Отклонить пустой или пробельный аргумент `--db`, сохранив остальные
/// значения в исходном виде, в том числе пути вне UTF-8.
pub fn check_db_flag(path: &OsStr) -> Result<()> {
    if path.to_str().is_some_and(|p| p.trim().is_empty()) {
        bail!("пустой путь к базе в --db");
    }
    Ok(())
}

/// Путь к базе: `--db`, иначе `TALLY_DB`, иначе `tally.db` в папке данных
/// пользователя: Windows – `%APPDATA%\Tally\data`, macOS –
/// `~/Library/Application Support/Tally`, Linux – `$XDG_DATA_HOME/tally`
/// (`~/.local/share/tally`).
///
/// Путь – как его дала система (`OsStr`): на Unix в нём бывают байты вне
/// UTF-8, и такой путь используется как есть. Пустая переменная – как
/// незаданная.
pub fn db_path(flag: Option<&Path>, var: Option<&OsStr>) -> Result<PathBuf> {
    if let Some(path) = flag {
        return Ok(path.to_owned());
    }
    if let Some(path) = var.filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let dirs = directories::ProjectDirs::from("", "", "Tally")
        .context("не удалось найти папку данных пользователя; укажите базу через --db")?;
    Ok(dirs.data_dir().join(DB_FILE))
}

/// Сегодняшний день: `TALLY_TODAY`, иначе по часам и часовому поясу системы.
/// Пустая переменная – как незаданная; не дата (в том числе не UTF-8) – ошибка,
/// а не молчаливый переход на системные часы.
pub fn today(var: Option<&OsStr>) -> Result<Date> {
    let Some(raw) = var.filter(|v| !v.is_empty()) else {
        return Ok(jiff::ZonedDateTime::now().date());
    };
    let text = raw.to_string_lossy();
    match raw.to_str().map(str::parse::<Date>) {
        Some(Ok(date)) => Ok(date),
        _ => bail!("{TODAY_VAR}: ожидается дата ГГГГ-ММ-ДД, а не «{text}»"),
    }
}

/// Переменная окружения как есть, без проверки UTF-8.
pub fn var(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// Создать папку базы, если её ещё нет.
pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("не удалось создать папку {}", dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn flag_wins_over_variable() {
        let b = OsStr::new("b.db");
        let path = db_path(Some(Path::new("a.db")), Some(b)).ok();
        assert_eq!(path, Some(PathBuf::from("a.db")));
        let path = db_path(None, Some(b)).ok();
        assert_eq!(path, Some(PathBuf::from("b.db")));
        let empty = db_path(None, Some(OsStr::new(""))).ok();
        assert_eq!(empty, db_path(None, None).ok());
    }

    #[test]
    fn default_path_is_in_user_data_dir() {
        let path = db_path(None, None).ok().unwrap_or_default();
        assert!(path.ends_with(DB_FILE), "{}", path.display());
        assert!(path.is_absolute(), "{}", path.display());
    }

    #[test]
    fn today_from_variable() {
        let var = |text: &'static str| Some(OsStr::new(text));
        assert_eq!(today(var("2026-11-19")).ok(), Some(date(2026, 11, 19)));
        let err = today(var("19.11.2026")).err().map(|e| e.to_string());
        assert_eq!(
            err.as_deref(),
            Some("TALLY_TODAY: ожидается дата ГГГГ-ММ-ДД, а не «19.11.2026»")
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_values_are_kept_or_reported() {
        use std::os::unix::ffi::OsStrExt;

        let raw = OsStr::from_bytes(b"/tmp/tally-\xff.db");
        assert_eq!(db_path(None, Some(raw)).ok(), Some(PathBuf::from(raw)));
        let err = today(Some(OsStr::from_bytes(b"2026-11-\xff"))).err();
        assert_eq!(
            err.map(|e| e.to_string()).as_deref(),
            Some("TALLY_TODAY: ожидается дата ГГГГ-ММ-ДД, а не «2026-11-\u{fffd}»")
        );
    }
}
