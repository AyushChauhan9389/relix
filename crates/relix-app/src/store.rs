//! Local data: profiles and per-profile records, shaped like the sync server's (ADR 0006).

use relix_proto::sync::SyncedRecord;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct Store {
    db: Connection,
}

// ponytail: one implicit profile for now (profiles UI removed); records keep the id so profiles can return.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
}

/// One synced piece of profile data, see GLOSSARY.md.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub profile: String,
    pub kind: String,
    pub key: String,
    pub value: String,
}

impl Store {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS profiles (
                 id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(8)))),
                 name TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS records (
                 profile TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 key TEXT NOT NULL,
                 value TEXT NOT NULL,
                 seq INTEGER,
                 dirty INTEGER NOT NULL DEFAULT 1,
                 PRIMARY KEY (profile, kind, key)
             );
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            // Local write order, for "most recent first" lists.
            db.execute_batch("ALTER TABLE records ADD COLUMN updated INTEGER NOT NULL DEFAULT 0; PRAGMA user_version = 1;")?;
        }
        Ok(Self { db })
    }

    pub fn create_profile(&self, name: &str) -> Result<Profile> {
        let id: String =
            self.db.query_row("INSERT INTO profiles (name) VALUES (?1) RETURNING id", params![name], |r| r.get(0))?;
        Ok(Profile { id, name: name.into() })
    }

    pub fn profiles(&self) -> Result<Vec<Profile>> {
        let mut stmt = self.db.prepare("SELECT id, name FROM profiles ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| Ok(Profile { id: r.get(0)?, name: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Writes a record and marks it for the next sync.
    pub fn put<T: Serialize>(&self, profile: &str, kind: &str, key: &str, value: &T) -> Result<()> {
        self.db.execute(
            "INSERT INTO records (profile, kind, key, value, dirty, updated)
             VALUES (?1, ?2, ?3, ?4, 1, (SELECT coalesce(max(updated), 0) + 1 FROM records))
             ON CONFLICT (profile, kind, key) DO UPDATE SET value = excluded.value, dirty = 1, updated = excluded.updated",
            params![profile, kind, key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    pub fn get<T: DeserializeOwned>(&self, profile: &str, kind: &str, key: &str) -> Result<Option<T>> {
        let value: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM records WHERE profile = ?1 AND kind = ?2 AND key = ?3",
                params![profile, kind, key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.map(|v| serde_json::from_str(&v)).transpose()?)
    }

    /// Every record of `kind` for a profile, most recently written first.
    pub fn list<T: DeserializeOwned>(&self, profile: &str, kind: &str) -> Result<Vec<(String, T)>> {
        let mut stmt = self.db.prepare(
            "SELECT key, value FROM records WHERE profile = ?1 AND kind = ?2 ORDER BY updated DESC, rowid DESC",
        )?;
        let rows = stmt.query_map(params![profile, kind], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|row| {
            let (key, value) = row?;
            Ok((key, serde_json::from_str(&value)?))
        })
        .collect()
    }

    /// Records changed locally and not yet accepted by the sync server.
    pub fn dirty(&self) -> Result<Vec<Record>> {
        let mut stmt = self.db.prepare("SELECT profile, kind, key, value FROM records WHERE dirty = 1 ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| {
            Ok(Record { profile: r.get(0)?, kind: r.get(1)?, key: r.get(2)?, value: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Device-level settings outside the synced records (account, last seen seq).
    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.db.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0)).optional()?)
    }

    pub fn set_meta(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(v) => self.db.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", params![key, v])?,
            None => self.db.execute("DELETE FROM meta WHERE key = ?1", params![key])?,
        };
        Ok(())
    }

    /// The newest server sequence number this device has pulled.
    pub fn last_seq(&self) -> Result<u64> {
        Ok(self.meta("last_seq")?.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    /// Applies records pulled from the server; returns the kinds that changed.
    /// Local edits not yet pushed are kept (they're newer), except on the first sync after
    /// signing in (`first`), when the account's copy beats this device's defaults.
    pub fn apply_pulled(&self, profile: &str, records: &[SyncedRecord], first: bool) -> Result<Vec<String>> {
        let mut changed: Vec<String> = vec![];
        for r in records {
            let dirty: bool = self
                .db
                .query_row(
                    "SELECT dirty FROM records WHERE profile = ?1 AND kind = ?2 AND key = ?3",
                    params![profile, r.kind, r.key],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(false);
            if dirty && !first {
                continue;
            }
            self.db.execute(
                "INSERT INTO records (profile, kind, key, value, seq, dirty, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, (SELECT coalesce(max(updated), 0) + 1 FROM records))
                 ON CONFLICT (profile, kind, key) DO UPDATE
                 SET value = excluded.value, seq = excluded.seq, dirty = 0, updated = excluded.updated",
                params![profile, r.kind, r.key, r.value, r.seq as i64],
            )?;
            if !changed.contains(&r.kind) {
                changed.push(r.kind.clone());
            }
        }
        if let Some(max) = records.iter().map(|r| r.seq).max().filter(|m| *m > self.last_seq().unwrap_or(0)) {
            self.set_meta("last_seq", Some(&max.to_string()))?;
        }
        Ok(changed)
    }

    /// Marks pushed records clean with their server seq, unless they changed again since being sent.
    pub fn mark_pushed(&self, sent: &[Record], seqs: &[u64]) -> Result<()> {
        for (r, seq) in sent.iter().zip(seqs) {
            self.db.execute(
                "UPDATE records SET dirty = 0, seq = ?5 WHERE profile = ?1 AND kind = ?2 AND key = ?3 AND value = ?4",
                params![r.profile, r.kind, r.key, r.value, *seq as i64],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_per_profile() {
        let store = Store::in_memory().unwrap();
        let a = store.create_profile("Ayush").unwrap();
        let b = store.create_profile("Other").unwrap();

        store.put(&a.id, "progress", "tt1:1:1", &42.5).unwrap();
        assert_eq!(store.get::<f64>(&a.id, "progress", "tt1:1:1").unwrap(), Some(42.5));
        assert_eq!(store.get::<f64>(&b.id, "progress", "tt1:1:1").unwrap(), None, "profiles must not see each other's data");

        store.put(&a.id, "progress", "tt1:1:1", &60.0).unwrap();
        assert_eq!(store.get::<f64>(&a.id, "progress", "tt1:1:1").unwrap(), Some(60.0));
    }

    #[test]
    fn local_writes_are_dirty_until_synced() {
        let store = Store::in_memory().unwrap();
        let p = store.create_profile("Ayush").unwrap();
        store.put(&p.id, "library", "tt1", &true).unwrap();
        store.put(&p.id, "library", "tt2", &true).unwrap();

        let dirty: Vec<String> = store.dirty().unwrap().into_iter().map(|r| r.key).collect();
        assert_eq!(dirty, ["tt1", "tt2"]);
    }

    #[test]
    fn lists_every_record_of_a_kind_newest_first() {
        let store = Store::in_memory().unwrap();
        let p = store.create_profile("Ayush").unwrap();
        store.put(&p.id, "library", "tt1", &"one").unwrap();
        store.put(&p.id, "library", "tt2", &"two").unwrap();
        store.put(&p.id, "progress", "tt3", &"other kind").unwrap();
        store.put(&p.id, "library", "tt1", &"one again").unwrap();

        let got = store.list::<String>(&p.id, "library").unwrap();
        assert_eq!(got, [("tt1".to_owned(), "one again".to_owned()), ("tt2".to_owned(), "two".to_owned())]);
    }

    fn synced(kind: &str, key: &str, value: &str, seq: u64) -> SyncedRecord {
        SyncedRecord { kind: kind.into(), key: key.into(), value: value.into(), seq }
    }

    #[test]
    fn pulled_records_apply_and_are_clean() {
        let store = Store::in_memory().unwrap();
        let p = store.create_profile("Me").unwrap();
        let changed = store.apply_pulled(&p.id, &[synced("library", "tt1", "\"x\"", 7)], false).unwrap();
        assert_eq!(changed, ["library"]);
        assert_eq!(store.get::<String>(&p.id, "library", "tt1").unwrap().as_deref(), Some("x"));
        assert!(store.dirty().unwrap().is_empty());
        assert_eq!(store.last_seq().unwrap(), 7);
    }

    #[test]
    fn unpushed_local_edits_survive_a_pull() {
        let store = Store::in_memory().unwrap();
        let p = store.create_profile("Me").unwrap();
        store.put(&p.id, "watched", "tt1:1:1", &true).unwrap();
        store.apply_pulled(&p.id, &[synced("watched", "tt1:1:1", "false", 3)], false).unwrap();
        assert_eq!(store.get::<bool>(&p.id, "watched", "tt1:1:1").unwrap(), Some(true));
        assert_eq!(store.dirty().unwrap().len(), 1, "still waiting to be pushed");
    }

    #[test]
    fn the_first_sync_takes_the_accounts_copy_over_local_defaults() {
        let store = Store::in_memory().unwrap();
        let p = store.create_profile("Me").unwrap();
        store.put(&p.id, "addons", "list", &"defaults").unwrap();
        store.put(&p.id, "library", "only-here", &true).unwrap();
        store.apply_pulled(&p.id, &[synced("addons", "list", "\"mine\"", 4)], true).unwrap();
        assert_eq!(store.get::<String>(&p.id, "addons", "list").unwrap().as_deref(), Some("mine"));
        let dirty: Vec<String> = store.dirty().unwrap().into_iter().map(|r| r.key).collect();
        assert_eq!(dirty, ["only-here"], "local-only records still get pushed");
    }

    #[test]
    fn a_push_only_cleans_records_unchanged_since_it_was_sent() {
        let store = Store::in_memory().unwrap();
        let p = store.create_profile("Me").unwrap();
        store.put(&p.id, "progress", "a", &1).unwrap();
        store.put(&p.id, "progress", "b", &1).unwrap();
        let sent = store.dirty().unwrap();
        store.put(&p.id, "progress", "b", &2).unwrap(); // edited while the push was in flight
        store.mark_pushed(&sent, &[10, 11]).unwrap();
        let dirty: Vec<String> = store.dirty().unwrap().into_iter().map(|r| r.key).collect();
        assert_eq!(dirty, ["b"]);
    }

    #[test]
    fn profiles_list_in_creation_order() {
        let store = Store::in_memory().unwrap();
        store.create_profile("Ayush").unwrap();
        store.create_profile("Other").unwrap();
        let names: Vec<String> = store.profiles().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Ayush", "Other"]);
    }
}
