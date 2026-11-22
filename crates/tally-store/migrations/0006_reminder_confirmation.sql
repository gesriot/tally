-- Старые записи v5 не содержат признака показа. Сохраняем прежнее
-- подавление повторной доставки для них, новые заявки пишутся с shown = 0.
ALTER TABLE delivered_task_reminders
    ADD COLUMN shown INTEGER NOT NULL DEFAULT 1 CHECK (shown IN (0, 1));
