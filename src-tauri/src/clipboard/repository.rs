//! SQLite 只保存元信息与文本。每次调用持短期连接锁，文件 IO 在返回后进行。
use super::model::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::{path::Path, sync::Mutex};

pub struct Repository(Mutex<Connection>);
pub struct NewItem<'a> {
    pub kind: Kind,
    pub hash: &'a str,
    pub text: Option<&'a str>,
    pub blob: Option<&'a str>,
    pub thumb: Option<&'a str>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bytes: u64,
}
fn err(_: impl std::fmt::Display) -> String {
    "剪贴板数据库操作失败，请检查磁盘空间及应用数据目录权限。".into()
}
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Stored> {
    let text: Option<String> = row.get("text_content")?;
    let kind = match row.get::<_, String>("kind")?.as_str() {
        "text" => Kind::Text,
        "image" => Kind::Image,
        "file" => Kind::File,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let file_paths = if kind == Kind::File {
        Some(
            super::files::from_stored(text.as_deref().unwrap_or(""))
                .map_err(|_| rusqlite::Error::InvalidQuery)?,
        )
    } else {
        None
    };
    Ok(Stored {
        item: Item {
            id: row.get("id")?,
            kind,
            file_paths,
            source: row.get("source")?,
            preview: text.as_deref().unwrap_or("").chars().take(160).collect(),
            mime_type: row.get("mime_type")?,
            width: row.get("width")?,
            height: row.get("height")?,
            byte_size: row.get("byte_size")?,
            favorite: row.get("is_favorite")?,
            created_at: row.get("created_at")?,
            last_seen_at: row.get("last_seen_at")?,
            seen_count: row.get("seen_count")?,
        },
        text,
        blob: row.get("blob_rel_path")?,
        thumb: row.get("thumb_rel_path")?,
    })
}
/// 在事务内只统计一次总量、按索引顺序计算要淘汰的记录，再复用删除语句。
/// protected 是刚插入的条目：不能靠删除它来假装插入成功；配额不足时由调用方回滚。
fn enforce_quota(
    tx: &rusqlite::Transaction<'_>,
    settings: &ClipboardSettings,
    protected: Option<i64>,
) -> Result<(), String> {
    let (mut count, mut bytes): (u64, u64) = tx
        .query_row(
            "SELECT count(*),coalesce(sum(byte_size),0) FROM clipboard_items",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    let max_count = u64::from(settings.max_items);
    let max_bytes = u64::from(settings.max_storage_mb) * 1024 * 1024;
    if count <= max_count && bytes <= max_bytes {
        return Ok(());
    }
    let mut removed = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT id,byte_size FROM clipboard_items WHERE is_favorite=0 AND (?1 IS NULL OR id<>?1) ORDER BY last_seen_at,id").map_err(err)?;
        let mut rows = stmt.query([protected]).map_err(err)?;
        while count > max_count || bytes > max_bytes {
            let Some(row) = rows.next().map_err(err)? else {
                return Err(if protected.is_some() {
                    "收藏或当前素材已占满历史配额，请清理历史或提高上限。"
                } else {
                    "收藏已超过历史配额，请先取消部分收藏或提高上限。"
                }
                .into());
            };
            removed.push(row.get::<_, i64>(0).map_err(err)?);
            count -= 1;
            bytes -= row.get::<_, u64>(1).map_err(err)?;
        }
    }
    let mut delete = tx
        .prepare("DELETE FROM clipboard_items WHERE id=?1")
        .map_err(err)?;
    for id in removed {
        delete.execute([id]).map_err(err)?;
    }
    Ok(())
}
impl Repository {
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::from_connection(Connection::open(path).map_err(err)?)
    }
    fn from_connection(mut conn: Connection) -> Result<Self, String> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=3000; PRAGMA secure_delete=ON;").map_err(err)?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(err)?;
        if version > 2 {
            return Err("历史数据库来自较新版本，请升级应用。".into());
        }
        if version == 0 {
            let tx = conn.transaction().map_err(err)?;
            tx.execute_batch(include_str!("migrations/001.sql"))
                .map_err(err)?;
            tx.pragma_update(None, "user_version", 1).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        if version < 2 {
            let tx = conn.transaction().map_err(err)?;
            tx.execute_batch(include_str!("migrations/002.sql"))
                .map_err(err)?;
            tx.pragma_update(None, "user_version", 2).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        Ok(Self(Mutex::new(conn)))
    }
    pub fn list(&self, q: &Query, revision: u64) -> Result<Page, String> {
        let (limit, time, id) = q.validate()?;
        let conn = self.0.lock().map_err(err)?;
        // 仅拼接固定 SQL 片段，用户关键词仍使用绑定参数；分类/收藏条件可直接使用索引。
        let mut sql =
            String::from("SELECT * FROM clipboard_items WHERE (last_seen_at,id) < (?4,?5)");
        if q.kind.is_some() {
            sql.push_str(" AND kind=?1");
        }
        if q.favorites_only {
            sql.push_str(" AND is_favorite=1");
        }
        if !q.keyword.is_empty() {
            sql.push_str(" AND instr(lower(coalesce(text_content,'')),lower(?3))>0");
        }
        sql.push_str(" ORDER BY last_seen_at DESC,id DESC LIMIT ?6");
        let mut stmt = conn.prepare(&sql).map_err(err)?;
        let mut items = stmt
            .query_map(
                params![
                    q.kind.map(Kind::as_str),
                    q.favorites_only,
                    q.keyword,
                    time,
                    id,
                    limit + 1
                ],
                |r| Ok(row(r)?.item),
            )
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        let more = items.len() > limit as usize;
        items.truncate(limit as usize);
        let next_cursor = if more {
            items.last().map(|i| format!("{}:{}", i.last_seen_at, i.id))
        } else {
            None
        };
        Ok(Page {
            items,
            next_cursor,
            revision,
        })
    }
    pub fn get(&self, id: i64) -> Result<Stored, String> {
        self.0
            .lock()
            .map_err(err)?
            .query_row("SELECT * FROM clipboard_items WHERE id=?1", [id], row)
            .optional()
            .map_err(err)?
            .ok_or("条目已删除，请刷新列表。".into())
    }
    pub fn refresh(&self, kind: Kind, hash: &str, now: i64) -> Result<bool, String> {
        Ok(self.0.lock().map_err(err)?.execute("UPDATE clipboard_items SET last_seen_at=?3,seen_count=seen_count+1 WHERE kind=?1 AND content_hash=?2", params![kind.as_str(),hash,now]).map_err(err)? > 0)
    }
    /// 插入与配额淘汰在同一事务内；收藏挤满容量时回滚整个插入，旧历史不会丢失。
    pub fn insert(
        &self,
        item: NewItem<'_>,
        settings: &ClipboardSettings,
        now: i64,
    ) -> Result<(), String> {
        let mut conn = self.0.lock().map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        let mime = match item.kind {
            Kind::Text => "text/plain",
            Kind::Image => "image/png",
            Kind::File => "application/x-snap-desk-file-list",
        };
        tx.execute("INSERT INTO clipboard_items(kind,content_hash,text_content,blob_rel_path,thumb_rel_path,mime_type,width,height,byte_size,created_at,last_seen_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)", params![item.kind.as_str(),item.hash,item.text,item.blob,item.thumb,mime,item.width,item.height,item.bytes,now]).map_err(err)?;
        let inserted = tx.last_insert_rowid();
        if settings.retain_days > 0 {
            tx.execute(
                "DELETE FROM clipboard_items WHERE is_favorite=0 AND last_seen_at<?1",
                [now - i64::from(settings.retain_days) * 86_400_000],
            )
            .map_err(err)?;
        }
        enforce_quota(&tx, settings, Some(inserted))?;
        tx.commit().map_err(err)
    }
    /// 启动、修改设置与定时维护共用。无法满足收藏配额时回滚，不删除收藏。
    pub fn prune(&self, settings: &ClipboardSettings, now: i64) -> Result<(), String> {
        let mut conn = self.0.lock().map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        if settings.retain_days > 0 {
            tx.execute(
                "DELETE FROM clipboard_items WHERE is_favorite=0 AND last_seen_at<?1",
                [now - i64::from(settings.retain_days) * 86_400_000],
            )
            .map_err(err)?;
        }
        enforce_quota(&tx, settings, None)?;
        tx.commit().map_err(err)
    }
    pub fn favorite(&self, id: i64, value: bool) -> Result<(), String> {
        if self
            .0
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE clipboard_items SET is_favorite=?2 WHERE id=?1",
                params![id, value],
            )
            .map_err(err)?
            == 0
        {
            return Err("条目已删除。".into());
        }
        Ok(())
    }
    pub fn delete(&self, ids: &[i64]) -> Result<(), String> {
        let mut conn = self.0.lock().map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        for id in ids {
            tx.execute("DELETE FROM clipboard_items WHERE id=?1", [id])
                .map_err(err)?;
        }
        tx.commit().map_err(err)
    }
    pub fn clear(&self, favorites: bool) -> Result<(), String> {
        self.0
            .lock()
            .map_err(err)?
            .execute(
                "DELETE FROM clipboard_items WHERE ?1 OR is_favorite=0",
                [favorites],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn references(&self) -> Result<Vec<(i64, String, String)>, String> {
        let conn = self.0.lock().map_err(err)?;
        let mut stmt = conn
            .prepare(
                "SELECT id,blob_rel_path,thumb_rel_path FROM clipboard_items WHERE kind='image'",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        Ok(rows)
    }
    pub fn totals(&self) -> Result<(u64, u64), String> {
        self.0
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT count(*),coalesce(sum(byte_size),0) FROM clipboard_items",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_removes_oldest_records_in_one_pass_and_preserves_favorites() {
        let repo = Repository::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        for value in ["favorite", "second", "third", "fourth", "newest"] {
            repo.insert(text(value), &ClipboardSettings::default(), 10)
                .unwrap();
        }
        repo.favorite(1, true).unwrap();
        repo.prune(
            &ClipboardSettings {
                max_items: 2,
                ..ClipboardSettings::default()
            },
            10,
        )
        .unwrap();
        assert!(repo.get(1).unwrap().item.favorite);
        assert!(repo.get(5).is_ok());
        assert_eq!(repo.totals().unwrap().0, 2);

        let repo = Repository::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let settings = ClipboardSettings {
            max_storage_mb: 1,
            ..ClipboardSettings::default()
        };
        repo.insert(text("favorite"), &settings, 1).unwrap();
        repo.favorite(1, true).unwrap();
        repo.insert(
            NewItem {
                bytes: 600_000,
                ..text("old")
            },
            &settings,
            2,
        )
        .unwrap();
        repo.insert(
            NewItem {
                bytes: 600_000,
                ..text("new")
            },
            &settings,
            3,
        )
        .unwrap();
        assert!(repo.get(1).unwrap().item.favorite);
        assert!(repo.get(2).is_err());
        assert!(repo.get(3).is_ok());
    }

    #[test]
    fn failed_prune_rolls_back_retention_and_filters_keep_cursor_semantics() {
        let repo = Repository::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        for value in ["Alpha", "Beta", "alpha second"] {
            repo.insert(text(value), &ClipboardSettings::default(), 10)
                .unwrap();
        }
        repo.favorite(1, true).unwrap();
        repo.favorite(2, true).unwrap();
        assert!(repo
            .prune(
                &ClipboardSettings {
                    max_items: 1,
                    retain_days: 1,
                    ..ClipboardSettings::default()
                },
                172_800_000
            )
            .is_err());
        assert_eq!(repo.totals().unwrap().0, 3);
        for kind in [None, Some(Kind::Text)] {
            let page = repo
                .list(
                    &Query {
                        kind,
                        keyword: "ALPHA".into(),
                        limit: Some(1),
                        ..Query::default()
                    },
                    7,
                )
                .unwrap();
            assert_eq!(page.items[0].id, 3);
            let next = repo
                .list(
                    &Query {
                        kind,
                        keyword: "ALPHA".into(),
                        cursor: page.next_cursor,
                        ..Query::default()
                    },
                    7,
                )
                .unwrap();
            assert_eq!(next.items[0].id, 1);
            let favorites = repo
                .list(
                    &Query {
                        kind,
                        favorites_only: true,
                        keyword: "ALPHA".into(),
                        ..Query::default()
                    },
                    7,
                )
                .unwrap();
            assert_eq!(favorites.items.len(), 1);
            assert_eq!(favorites.items[0].id, 1);
        }
    }

    #[test]
    fn migration_reopens_and_retention_preserves_favorites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        {
            let repo = Repository::open(&path).unwrap();
            repo.insert(text("keep"), &ClipboardSettings::default(), 1)
                .unwrap();
            repo.favorite(1, true).unwrap();
            repo.insert(text("expire"), &ClipboardSettings::default(), 2)
                .unwrap();
        }
        let repo = Repository::open(&path).unwrap();
        assert_eq!(repo.totals().unwrap().0, 2);
        repo.prune(
            &ClipboardSettings {
                retain_days: 1,
                ..ClipboardSettings::default()
            },
            172_800_000,
        )
        .unwrap();
        assert_eq!(repo.totals().unwrap().0, 1);
        assert_eq!(repo.get(1).unwrap().text.as_deref(), Some("keep"));
    }
    fn text<'a>(value: &'a str) -> NewItem<'a> {
        NewItem {
            kind: Kind::Text,
            hash: value,
            text: Some(value),
            blob: None,
            thumb: None,
            width: None,
            height: None,
            bytes: value.len() as u64,
        }
    }
    #[test]
    fn version_one_migration_preserves_records_and_adds_file_search() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("migrations/001.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("INSERT INTO clipboard_items(id,kind,content_hash,text_content,mime_type,byte_size,created_at,last_seen_at,is_favorite,seen_count) VALUES(8,'text','old','keep','text/plain',4,1,2,1,3)", []).unwrap();
        let repo = Repository::from_connection(conn).unwrap();
        let old = repo.get(8).unwrap();
        assert_eq!(old.text.as_deref(), Some("keep"));
        assert!(old.item.favorite);
        assert_eq!(old.item.seen_count, 3);
        let paths = "C:\\资料\\report.pdf\nD:\\image.png";
        repo.insert(
            NewItem {
                kind: Kind::File,
                ..text(paths)
            },
            &ClipboardSettings::default(),
            3,
        )
        .unwrap();
        let page = repo
            .list(
                &Query {
                    kind: Some(Kind::File),
                    keyword: "report.pdf".into(),
                    ..Query::default()
                },
                0,
            )
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].file_paths.as_ref().unwrap().len(), 2);
        assert_eq!(page.items[0].id, 9);
        assert!(repo.refresh(Kind::File, paths, 4).unwrap());
        repo.delete(&[9]).unwrap();
        assert!(repo.get(8).is_ok());
    }
    #[test]
    fn duplicate_favorites_and_cursor_keep_exact_text() {
        let repo = Repository::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let settings = ClipboardSettings::default();
        repo.insert(text(" 中文😀\n "), &settings, 10).unwrap();
        repo.favorite(1, true).unwrap();
        assert!(repo.refresh(Kind::Text, " 中文😀\n ", 20).unwrap());
        repo.insert(text("second"), &settings, 20).unwrap();
        let first = repo
            .list(
                &Query {
                    limit: Some(1),
                    ..Query::default()
                },
                0,
            )
            .unwrap();
        assert_eq!(first.items[0].id, 2);
        let second = repo
            .list(
                &Query {
                    cursor: first.next_cursor,
                    ..Query::default()
                },
                0,
            )
            .unwrap();
        assert_eq!(second.items[0].seen_count, 2);
        repo.clear(false).unwrap();
        assert_eq!(repo.get(1).unwrap().text.as_deref(), Some(" 中文😀\n "));
        assert_eq!(repo.totals().unwrap().0, 1);
    }
    #[test]
    fn full_favorite_quota_rolls_back_new_item() {
        let repo = Repository::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let settings = ClipboardSettings {
            max_items: 1,
            ..ClipboardSettings::default()
        };
        repo.insert(text("old"), &settings, 1).unwrap();
        repo.favorite(1, true).unwrap();
        assert!(repo.insert(text("new"), &settings, 2).is_err());
        assert_eq!(repo.totals().unwrap().0, 1);
        assert!(repo.get(1).unwrap().item.favorite);
    }
}
