ALTER TABLE tasks ADD COLUMN repeat_anchor TEXT;
ALTER TABLE projects ADD COLUMN repeat_anchor TEXT;
ALTER TABLE tasks ADD COLUMN repeat_advanced INTEGER NOT NULL DEFAULT 0 CHECK (repeat_advanced IN (0,1));
ALTER TABLE projects ADD COLUMN repeat_advanced INTEGER NOT NULL DEFAULT 0 CHECK (repeat_advanced IN (0,1));
