-- Прежние версии разрешали убрать дату, оставив календарное правило.
-- Запланированный день неизвестен. Отключаем только неработающее правило,
-- сохраняя существующее размещение записи и время правки.
UPDATE tasks
SET repeat_interval = NULL, repeat_unit = NULL, repeat_anchor = NULL
WHERE date IS NULL AND repeat_from = 'date' AND repeat_interval IS NOT NULL;

UPDATE projects
SET repeat_interval = NULL, repeat_unit = NULL, repeat_anchor = NULL
WHERE date IS NULL AND repeat_from = 'date' AND repeat_interval IS NOT NULL;
