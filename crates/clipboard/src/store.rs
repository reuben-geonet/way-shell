use crate::{Content, TAGS};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub content: Content,
    pub title: String,
    pub pinned: bool,
    pub tag: Option<String>,
    pub copied: i64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryPolicy {
    Clear,
    #[default]
    KeepPinnedAndTagged,
    KeepAll,
}
#[derive(Clone, Copy, Debug)]
pub struct Retention {
    pub length: usize,
    pub minutes: u32,
}
impl Default for Retention {
    fn default() -> Self {
        Self {
            length: 50,
            minutes: 0,
        }
    }
}
pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(path: Option<&Path>) -> Result<Self> {
        let connection = match path {
            Some(path) => Connection::open(path),
            None => Connection::open_in_memory(),
        }
        .map_err(|e| e.to_string())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(1))
            .map_err(|e| e.to_string())?;
        if path.is_none() {
            // SQLite's temporary sort tables must also stay off disk in memory mode.
            connection
                .execute_batch("PRAGMA temp_store=MEMORY;")
                .map_err(|e| e.to_string())?;
        }
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY, fingerprint BLOB NOT NULL UNIQUE,
                content TEXT NOT NULL, payload BLOB NOT NULL,
                title TEXT NOT NULL DEFAULT '', pinned INTEGER NOT NULL DEFAULT 0,
                tag TEXT, copied INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS previews (
                entry INTEGER PRIMARY KEY REFERENCES entries(id) ON DELETE CASCADE,
                metadata TEXT NOT NULL, image BLOB);
             CREATE INDEX IF NOT EXISTS entries_by_copied ON entries(copied DESC,id DESC);
             PRAGMA user_version=1;",
            )
            .map_err(|e| e.to_string())?;
        Ok(Self { connection })
    }
    pub fn entries(&self) -> Result<Vec<Entry>> {
        let mut statement = self.connection.prepare("SELECT id,content,payload,title,pinned,tag,copied FROM entries ORDER BY copied DESC,id DESC").map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let encoded: String = row.get(1)?;
                let mut content: Content = serde_json::from_str(&encoded).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                content.bytes = row.get(2)?;
                Ok(Entry {
                    id: row.get(0)?,
                    content,
                    title: row.get(3)?,
                    pinned: row.get(4)?,
                    tag: row.get(5)?,
                    copied: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn insert(&mut self, content: Content, now: i64, retention: Retention) -> Result<i64> {
        let fingerprint = fingerprint(&content);
        let encoded = serde_json::to_string(&content).map_err(|e| e.to_string())?;
        self.connection
            .execute(
                "INSERT INTO entries(fingerprint,content,payload,copied) VALUES(?1,?2,?3,?4)
             ON CONFLICT(fingerprint) DO UPDATE SET copied=excluded.copied",
                params![fingerprint, encoded, content.bytes, now],
            )
            .map_err(|e| e.to_string())?;
        let id = self
            .connection
            .query_row(
                "SELECT id FROM entries WHERE fingerprint=?1",
                params![fingerprint],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        self.prune(retention, now)?;
        Ok(id)
    }
    pub fn update(&mut self, entry: &Entry) -> Result<()> {
        if entry
            .tag
            .as_ref()
            .is_some_and(|tag| !TAGS.contains(&tag.as_str()))
        {
            return Err("Unknown clipboard tag".into());
        }
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        let fingerprint = fingerprint(&entry.content);
        tx.execute("DELETE FROM previews WHERE entry=?1 AND EXISTS(SELECT 1 FROM entries WHERE id=?1 AND fingerprint<>?2)", params![entry.id, fingerprint])
            .map_err(|e| e.to_string())?;
        // Keep the edited entry's title, pin and tag when two entries converge.
        tx.execute(
            "DELETE FROM entries WHERE fingerprint=?1 AND id<>?2",
            params![fingerprint, entry.id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("UPDATE entries SET fingerprint=?1,content=?2,payload=?3,title=?4,pinned=?5,tag=?6,copied=?7 WHERE id=?8",
            params![fingerprint,serde_json::to_string(&entry.content).map_err(|e| e.to_string())?,entry.content.bytes,entry.title,entry.pinned,entry.tag,entry.copied,entry.id]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn delete(
        &mut self,
        id: i64,
        protect_pinned: bool,
        protect_tagged: bool,
        force: bool,
    ) -> Result<bool> {
        let count = self.connection.execute("DELETE FROM entries WHERE id=?1 AND (?2 OR ((NOT ?3 OR pinned=0) AND (NOT ?4 OR tag IS NULL)))", params![id,force,protect_pinned,protect_tagged]).map_err(|e| e.to_string())?;
        Ok(count != 0)
    }
    pub fn prune(&mut self, retention: Retention, now: i64) -> Result<()> {
        self.connection.execute("DELETE FROM entries WHERE pinned=0 AND tag IS NULL AND (id NOT IN (SELECT id FROM entries WHERE pinned=0 AND tag IS NULL ORDER BY copied DESC,id DESC LIMIT ?1) OR (?2>0 AND copied<?3))", params![retention.length as i64,retention.minutes,now-i64::from(retention.minutes)*60_000]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn clear(&mut self, policy: HistoryPolicy) -> Result<()> {
        let sql = match policy {
            HistoryPolicy::Clear => "DELETE FROM entries",
            HistoryPolicy::KeepPinnedAndTagged => {
                "DELETE FROM entries WHERE pinned=0 AND tag IS NULL"
            }
            HistoryPolicy::KeepAll => return Ok(()),
        };
        self.connection
            .execute(sql, [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn save_preview(
        &self,
        id: i64,
        metadata: &serde_json::Value,
        image: Option<&[u8]>,
    ) -> Result<()> {
        self.connection.execute("INSERT INTO previews(entry,metadata,image) SELECT ?1,?2,?3 WHERE EXISTS(SELECT 1 FROM entries WHERE id=?1) ON CONFLICT(entry) DO UPDATE SET metadata=excluded.metadata,image=excluded.image", params![id,metadata.to_string(),image]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn preview(&self, id: i64) -> Result<Option<(serde_json::Value, Option<Vec<u8>>)>> {
        let row: Option<(String, Option<Vec<u8>>)> = self
            .connection
            .query_row(
                "SELECT metadata,image FROM previews WHERE entry=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        row.map(|(meta, image)| {
            Ok((
                serde_json::from_str(&meta).map_err(|e| e.to_string())?,
                image,
            ))
        })
        .transpose()
    }
}
fn fingerprint(content: &Content) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(content.kind.name());
    hash.update([0]);
    hash.update(&content.mime);
    hash.update([0]);
    hash.update(&content.bytes);
    hash.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(value: &str) -> Content {
        Content::text(value)
    }
    #[test]
    fn duplicates_retention_protection_and_edit_conflicts() {
        let mut store = Store::open(None).unwrap();
        let retention = Retention {
            length: 1,
            minutes: 0,
        };
        let id = store.insert(text("first"), 1, retention).unwrap();
        let mut entry = store.entries().unwrap().remove(0);
        entry.pinned = true;
        store.update(&entry).unwrap();
        assert_eq!(store.insert(text("first"), 2, retention).unwrap(), id);
        store.insert(text("second"), 3, retention).unwrap();
        store.insert(text("third"), 4, retention).unwrap();
        assert_eq!(store.entries().unwrap().len(), 2);
        assert!(!store.delete(id, true, true, false).unwrap());
        entry.content = text("third");
        store.update(&entry).unwrap();
        assert_eq!(store.entries().unwrap().len(), 1);
        assert!(store.entries().unwrap()[0].pinned);
        assert!(store.delete(id, true, true, true).unwrap());
    }
    #[test]
    fn previews_cascade_and_restart_policy_preserves_tags() {
        let mut store = Store::open(None).unwrap();
        let id = store
            .insert(text("tagged"), 1, Retention::default())
            .unwrap();
        let mut entry = store.entries().unwrap().remove(0);
        entry.tag = Some("blue".into());
        store.update(&entry).unwrap();
        store
            .insert(text("ordinary"), 2, Retention::default())
            .unwrap();
        store
            .save_preview(id, &serde_json::json!({"title":"preview"}), Some(b"image"))
            .unwrap();
        entry.title = "A title".into();
        store.update(&entry).unwrap();
        assert!(store.preview(id).unwrap().is_some());
        store.clear(HistoryPolicy::KeepPinnedAndTagged).unwrap();
        assert_eq!(store.entries().unwrap().len(), 1);
        assert!(store.preview(id).unwrap().is_some());
        store.clear(HistoryPolicy::Clear).unwrap();
        assert!(store.preview(id).unwrap().is_none());
    }
    #[test]
    fn age_limits_use_minutes_and_edits_invalidate_previews_only_for_changed_content() {
        let mut store = Store::open(None).unwrap();
        let id = store
            .insert(text("old"), 1_000, Retention::default())
            .unwrap();
        store
            .save_preview(id, &serde_json::json!({"title":"cached"}), None)
            .unwrap();
        let mut entry = store.entries().unwrap().remove(0);
        entry.content = text("edited");
        store.update(&entry).unwrap();
        assert!(store.preview(id).unwrap().is_none());
        store
            .prune(
                Retention {
                    length: 50,
                    minutes: 1,
                },
                60_999,
            )
            .unwrap();
        assert_eq!(store.entries().unwrap().len(), 1);
        store
            .prune(
                Retention {
                    length: 50,
                    minutes: 1,
                },
                61_001,
            )
            .unwrap();
        assert!(store.entries().unwrap().is_empty());
    }
}
