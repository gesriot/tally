use rusqlite::Connection;

use crate::error::{Result, StoreError};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub appearance: Appearance,
    pub automatic_backups: bool,
}

pub(crate) fn load(conn: &Connection) -> Result<Settings> {
    let (value, automatic_backups): (String, bool) = conn.query_row(
        "SELECT appearance, automatic_backups FROM settings WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let appearance = match value.as_str() {
        "system" => Appearance::System,
        "light" => Appearance::Light,
        "dark" => Appearance::Dark,
        _ => {
            return Err(StoreError::Corrupt {
                table: "settings",
                column: "appearance",
                value,
            });
        }
    };
    Ok(Settings {
        appearance,
        automatic_backups,
    })
}

pub(crate) fn save(conn: &Connection, settings: Settings) -> Result<()> {
    let appearance = match settings.appearance {
        Appearance::System => "system",
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    };
    conn.execute(
        "UPDATE settings SET appearance = ?1, automatic_backups = ?2 WHERE id = 1",
        (appearance, settings.automatic_backups),
    )?;
    Ok(())
}
