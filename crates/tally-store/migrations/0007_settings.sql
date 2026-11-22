CREATE TABLE settings (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    appearance          TEXT NOT NULL CHECK (appearance IN ('system', 'light', 'dark')),
    automatic_backups   INTEGER NOT NULL CHECK (automatic_backups IN (0, 1))
) STRICT;

INSERT INTO settings VALUES (1, 'system', 1);
