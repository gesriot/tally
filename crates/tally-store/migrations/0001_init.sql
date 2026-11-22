-- Tally, схема v1.
--
-- Соглашения:
-- * id – UUID в тексте (36 символов, нижний регистр);
-- * даты – текст YYYY-MM-DD, моменты – текст UTC фиксированной ширины
--   YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ (сравнение строк = сравнение времени);
-- * ключи порядка (sort_index, today_index) – строки tally-rank, порядок по
--   BINARY-сравнению совпадает с порядком Rank;
-- * мягкое удаление – deleted_at; физически запись удаляет только очистка корзины;
-- * search_text – нормализованный текст для поиска, его пишет Rust;
-- * физическое удаление проекта, раздела или области уносит их содержимое
--   (ON DELETE CASCADE), как очистка корзины в Things.

CREATE TABLE areas (
    id          TEXT PRIMARY KEY NOT NULL,
    title       TEXT NOT NULL,
    search_text TEXT NOT NULL,
    sort_index  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    deleted_at  TEXT
) STRICT;

CREATE TABLE tags (
    id          TEXT PRIMARY KEY NOT NULL,
    title       TEXT NOT NULL,
    search_text TEXT NOT NULL,
    parent_id   TEXT REFERENCES tags (id) ON DELETE SET NULL,
    sort_index  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    deleted_at  TEXT,
    CHECK (parent_id IS NULL OR parent_id <> id)
) STRICT;

CREATE TABLE projects (
    id              TEXT PRIMARY KEY NOT NULL,
    title           TEXT NOT NULL,
    notes           TEXT NOT NULL,
    search_text     TEXT NOT NULL,
    area_id         TEXT REFERENCES areas (id) ON DELETE CASCADE,
    start           TEXT NOT NULL CHECK (start IN ('inbox', 'anytime', 'someday')),
    date            TEXT CHECK (date IS NULL OR length(date) = 10),
    evening         INTEGER NOT NULL CHECK (evening IN (0, 1)),
    deadline        TEXT CHECK (deadline IS NULL OR length(deadline) = 10),
    status          TEXT NOT NULL CHECK (status IN ('open', 'completed', 'canceled')),
    closed_at       TEXT,
    sort_index      TEXT NOT NULL,
    today_index     TEXT NOT NULL,
    repeat_interval INTEGER CHECK (repeat_interval IS NULL OR repeat_interval > 0),
    repeat_unit     TEXT CHECK (repeat_unit IS NULL OR repeat_unit IN ('day', 'week', 'month', 'year')),
    repeat_from     TEXT NOT NULL CHECK (repeat_from IN ('date', 'completion')),
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT,
    CHECK ((status = 'open') = (closed_at IS NULL)),
    CHECK ((repeat_interval IS NULL) = (repeat_unit IS NULL))
) STRICT;

CREATE TABLE headings (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    archived    INTEGER NOT NULL CHECK (archived IN (0, 1)),
    sort_index  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    deleted_at  TEXT,
    -- Цель составной ссылки задач: раздел вместе со своим проектом.
    UNIQUE (id, project_id)
) STRICT;

CREATE TABLE tasks (
    id              TEXT PRIMARY KEY NOT NULL,
    title           TEXT NOT NULL,
    notes           TEXT NOT NULL,
    search_text     TEXT NOT NULL,
    start           TEXT NOT NULL CHECK (start IN ('inbox', 'anytime', 'someday')),
    date            TEXT CHECK (date IS NULL OR length(date) = 10),
    evening         INTEGER NOT NULL CHECK (evening IN (0, 1)),
    deadline        TEXT CHECK (deadline IS NULL OR length(deadline) = 10),
    status          TEXT NOT NULL CHECK (status IN ('open', 'completed', 'canceled')),
    closed_at       TEXT,
    project_id      TEXT REFERENCES projects (id) ON DELETE CASCADE,
    heading_id      TEXT,
    area_id         TEXT REFERENCES areas (id) ON DELETE CASCADE,
    sort_index      TEXT NOT NULL,
    today_index     TEXT NOT NULL,
    repeat_interval INTEGER CHECK (repeat_interval IS NULL OR repeat_interval > 0),
    repeat_unit     TEXT CHECK (repeat_unit IS NULL OR repeat_unit IN ('day', 'week', 'month', 'year')),
    repeat_from     TEXT NOT NULL CHECK (repeat_from IN ('date', 'completion')),
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT,
    CHECK ((status = 'open') = (closed_at IS NULL)),
    CHECK ((repeat_interval IS NULL) = (repeat_unit IS NULL)),
    -- Раздел задачи – всегда раздел её проекта: составная ссылка не даёт
    -- сохранить задачу проекта Б с разделом проекта А, а перенос раздела
    -- в другой проект переносит и его задачи (ON UPDATE CASCADE).
    CHECK (heading_id IS NULL OR project_id IS NOT NULL),
    -- Задача в проекте наследует область проекта, своей области у неё нет:
    -- перенос проекта или раздела не оставит у задачи устаревшую область.
    CHECK (project_id IS NULL OR area_id IS NULL),
    FOREIGN KEY (heading_id, project_id) REFERENCES headings (id, project_id)
        ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;

CREATE TABLE checklist_items (
    id          TEXT PRIMARY KEY NOT NULL,
    task_id     TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    done        INTEGER NOT NULL CHECK (done IN (0, 1)),
    sort_index  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    deleted_at  TEXT
) STRICT;

-- Теги записей; position хранит порядок тегов у записи.
CREATE TABLE task_tags (
    task_id  TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    tag_id   TEXT NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (task_id, tag_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE project_tags (
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    tag_id     TEXT NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    PRIMARY KEY (project_id, tag_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE area_tags (
    area_id  TEXT NOT NULL REFERENCES areas (id) ON DELETE CASCADE,
    tag_id   TEXT NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (area_id, tag_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX tasks_project   ON tasks (project_id, sort_index);
CREATE INDEX tasks_heading   ON tasks (heading_id);
CREATE INDEX tasks_area      ON tasks (area_id);
CREATE INDEX tasks_date      ON tasks (date);
CREATE INDEX tasks_deadline  ON tasks (deadline);
CREATE INDEX tasks_updated   ON tasks (updated_at);
CREATE INDEX projects_area   ON projects (area_id, sort_index);
CREATE INDEX projects_updated ON projects (updated_at);
CREATE INDEX headings_project ON headings (project_id, sort_index);
CREATE INDEX checklist_task  ON checklist_items (task_id, sort_index);
CREATE INDEX tags_parent     ON tags (parent_id);
CREATE INDEX task_tags_tag   ON task_tags (tag_id);
CREATE INDEX project_tags_tag ON project_tags (tag_id);
CREATE INDEX area_tags_tag   ON area_tags (tag_id);
