-- 扩展类别约束；保留历史 ID、收藏、计数和素材相对引用。
CREATE TABLE clipboard_items_v2 (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 kind TEXT NOT NULL CHECK(kind IN ('text','image','file')),
 source TEXT NOT NULL DEFAULT 'clipboard' CHECK(source IN ('clipboard','capture','scroll')),
 content_hash TEXT NOT NULL,
 text_content TEXT, blob_rel_path TEXT, thumb_rel_path TEXT,
 mime_type TEXT NOT NULL, width INTEGER, height INTEGER,
 byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
 created_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
 seen_count INTEGER NOT NULL DEFAULT 1,
 is_favorite INTEGER NOT NULL DEFAULT 0 CHECK(is_favorite IN (0,1)),
 CHECK((kind IN ('text','file') AND text_content IS NOT NULL AND blob_rel_path IS NULL)
 OR (kind='image' AND text_content IS NULL AND blob_rel_path IS NOT NULL AND thumb_rel_path IS NOT NULL)),
 UNIQUE(kind,content_hash)
);
INSERT INTO clipboard_items_v2 SELECT * FROM clipboard_items;
-- 即使旧表最大 ID 已删除，也保留 AUTOINCREMENT 高水位，避免重用前端仍持有的 ID。
UPDATE sqlite_sequence SET seq = max(seq, coalesce((SELECT seq FROM sqlite_sequence WHERE name='clipboard_items'), 0)) WHERE name='clipboard_items_v2';
DROP TABLE clipboard_items;
ALTER TABLE clipboard_items_v2 RENAME TO clipboard_items;
CREATE INDEX idx_clipboard_recent ON clipboard_items(last_seen_at DESC,id DESC);
CREATE INDEX idx_clipboard_favorite_recent ON clipboard_items(is_favorite,last_seen_at DESC,id DESC);
CREATE INDEX idx_clipboard_kind_recent ON clipboard_items(kind,last_seen_at DESC,id DESC);
