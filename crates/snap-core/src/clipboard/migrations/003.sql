-- 文本子分类保留基础 kind，旧记录由迁移事务按完整正文回填。
ALTER TABLE clipboard_items ADD COLUMN text_category TEXT
 CHECK(text_category IS NULL OR (kind='text' AND text_category IN ('link','color','path')));
CREATE INDEX idx_clipboard_text_category_recent ON clipboard_items(text_category,last_seen_at DESC,id DESC);
