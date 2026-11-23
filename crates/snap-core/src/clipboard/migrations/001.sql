CREATE TABLE clipboard_items (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 kind TEXT NOT NULL CHECK(kind IN ('text','image')),
 source TEXT NOT NULL DEFAULT 'clipboard' CHECK(source IN ('clipboard','capture','scroll')),
 content_hash TEXT NOT NULL,
 text_content TEXT, blob_rel_path TEXT, thumb_rel_path TEXT,
 mime_type TEXT NOT NULL, width INTEGER, height INTEGER,
 byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
 created_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
 seen_count INTEGER NOT NULL DEFAULT 1,
 is_favorite INTEGER NOT NULL DEFAULT 0 CHECK(is_favorite IN (0,1)),
 CHECK((kind='text' AND text_content IS NOT NULL AND blob_rel_path IS NULL)
 OR (kind='image' AND text_content IS NULL AND blob_rel_path IS NOT NULL AND thumb_rel_path IS NOT NULL)),
 UNIQUE(kind,content_hash)
);
CREATE INDEX idx_clipboard_recent ON clipboard_items(last_seen_at DESC,id DESC);
CREATE INDEX idx_clipboard_favorite_recent ON clipboard_items(is_favorite,last_seen_at DESC,id DESC);
CREATE INDEX idx_clipboard_kind_recent ON clipboard_items(kind,last_seen_at DESC,id DESC);
