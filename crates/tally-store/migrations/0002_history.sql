-- Журнал действий для отмены и повтора (undo/redo, `tally-app`).
--
-- Одна строка – одно действие пользователя: `action` – его название для
-- вывода, `changes` – состояния затронутых записей до и после (JSON
-- `tally-app`, хранилище его не разбирает). `undone = 1` – действие отменено
-- и ждёт повтора. Все отменённые строки новее всех неотменённых: новое
-- действие сначала стирает отменённые.
CREATE TABLE history (
    seq     INTEGER PRIMARY KEY AUTOINCREMENT,
    at      TEXT    NOT NULL,
    action  TEXT    NOT NULL,
    changes TEXT    NOT NULL,
    undone  INTEGER NOT NULL DEFAULT 0 CHECK (undone IN (0, 1))
) STRICT;

CREATE INDEX history_undone ON history (undone, seq);
