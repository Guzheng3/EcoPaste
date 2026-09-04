-- 敏感条目自动过期清除：`sensitive_expires_at` 为敏感条目的清除截止时间，
-- 过期后由清理任务删除（收藏项豁免，见 cleanup 逻辑）；普通条目该列为 NULL，
-- 无需迁移其值。已有敏感条目未设置过期时间（NULL），不会在清理任务中被删。
ALTER TABLE clipboard_items ADD COLUMN sensitive_expires_at TEXT;

CREATE INDEX idx_clipboard_items_sensitive_expires_at
    ON clipboard_items (sensitive_expires_at);