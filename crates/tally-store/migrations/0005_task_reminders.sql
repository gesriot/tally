ALTER TABLE tasks ADD COLUMN reminder_at TEXT;

CREATE INDEX task_reminders_due ON tasks (reminder_at, id)
    WHERE reminder_at IS NOT NULL AND deleted_at IS NULL AND status = 'open';

CREATE TABLE delivered_task_reminders (
    task_id TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    reminder_at TEXT NOT NULL,
    PRIMARY KEY (task_id, reminder_at)
) STRICT;
