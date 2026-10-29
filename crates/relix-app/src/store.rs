//! Local data: profiles and per-profile records, shaped like the sync server's (ADR 0006).

use relix_proto::addon::Addon;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct Store {
    db: Connection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kids: bool,
    // ponytail: stored as typed; a 4-digit PIN is a child lock, hashing it wouldn't stop anyone with disk access.
    pub pin: Option<String>,
    /// Manifest URLs a kids profile may use.
    pub allowed_addons: Vec<String>,
}

/// One synced piece of profile data, see GLOSSARY.md.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub profile: String,
    pub kind: String,
    pub key: String,
    pub value: String,
}

impl Profile {
    /// Kids profiles get only allowlisted addons; nobody in a kids profile gets adult addons.
    pub fn visible_addons<'a>(&self, addons: &'a [Addon]) -> Vec<&'a Addon> {
        addons
            .iter()
            .filter(|a| {
                !self.kids
                    || (!a.manifest.behavior_hints.adult && self.allowed_addons.contains(&a.manifest_url.to_string()))
            })
            .collect()
    }
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
                 name TEXT NOT NULL,
                 kids INTEGER NOT NULL DEFAULT 0,
                 pin TEXT,
                 allowed_addons TEXT NOT NULL DEFAULT '[]'
             );
             CREATE TABLE IF NOT EXISTS records (
                 profile TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 key TEXT NOT NULL,
                 value TEXT NOT NULL,
                 seq INTEGER,
                 dirty INTEGER NOT NULL DEFAULT 1,
                 PRIMARY KEY (profile, kind, key)
             );",
        )?;
        Ok(Self { db })
    }

    pub fn create_profile(&self, name: &str, kids: bool) -> Result<Profile> {
        let id: String = self.db.query_row(
            "INSERT INTO profiles (name, kids) VALUES (?1, ?2) RETURNING id",
            params![name, kids],
            |r| r.get(0),
        )?;
        Ok(Profile { id, name: name.into(), kids, pin: None, allowed_addons: vec![] })
    }

    pub fn save_profile(&self, p: &Profile) -> Result<()> {
        self.db.execute(
            "UPDATE profiles SET name = ?2, kids = ?3, pin = ?4, allowed_addons = ?5 WHERE id = ?1",
            params![p.id, p.name, p.kids, p.pin, serde_json::to_string(&p.allowed_addons)?],
        )?;
        Ok(())
    }

    pub fn profiles(&self) -> Result<Vec<Profile>> {
        let mut stmt = self.db.prepare("SELECT id, name, kids, pin, allowed_addons FROM profiles ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, String>(4)?))
        })?;
        rows.map(|row| {
            let (id, name, kids, pin, allowed) = row?;
            Ok(Profile { id, name, kids, pin, allowed_addons: serde_json::from_str(&allowed)? })
        })
        .collect()
    }

    /// Writes a record and marks it for the next sync.
    pub fn put<T: Serialize>(&self, profile: &str, kind: &str, key: &str, value: &T) -> Result<()> {
        self.db.execute(
            "INSERT INTO records (profile, kind, key, value, dirty) VALUES (?1, ?2, ?3, ?4, 1)
             ON CONFLICT (profile, kind, key) DO UPDATE SET value = excluded.value, dirty = 1",
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

    /// Records changed locally and not yet accepted by the sync server.
    pub fn dirty(&self) -> Result<Vec<Record>> {
        let mut stmt = self.db.prepare("SELECT profile, kind, key, value FROM records WHERE dirty = 1 ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| {
            Ok(Record { profile: r.get(0)?, kind: r.get(1)?, key: r.get(2)?, value: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relix_proto::addon::{Addon, manifest_url};

    fn addon(id: &str, adult: bool) -> Addon {
        let manifest = serde_json::from_value(serde_json::json!({
            "id": id, "version": "1.0.0", "name": id,
            "behaviorHints": { "adult": adult },
        }))
        .unwrap();
        Addon { manifest_url: manifest_url(&format!("https://{id}.example/manifest.json")).unwrap(), manifest }
    }

    #[test]
    fn records_round_trip_per_profile() {
        let store = Store::in_memory().unwrap();
        let a = store.create_profile("Ayush", false).unwrap();
        let b = store.create_profile("Kid", true).unwrap();

        store.put(&a.id, "progress", "tt1:1:1", &42.5).unwrap();
        assert_eq!(store.get::<f64>(&a.id, "progress", "tt1:1:1").unwrap(), Some(42.5));
        assert_eq!(store.get::<f64>(&b.id, "progress", "tt1:1:1").unwrap(), None, "profiles must not see each other's data");

        store.put(&a.id, "progress", "tt1:1:1", &60.0).unwrap();
        assert_eq!(store.get::<f64>(&a.id, "progress", "tt1:1:1").unwrap(), Some(60.0));
    }

    #[test]
    fn local_writes_are_dirty_until_synced() {
        let store = Store::in_memory().unwrap();
        let p = store.create_profile("Ayush", false).unwrap();
        store.put(&p.id, "library", "tt1", &true).unwrap();
        store.put(&p.id, "library", "tt2", &true).unwrap();

        let dirty: Vec<String> = store.dirty().unwrap().into_iter().map(|r| r.key).collect();
        assert_eq!(dirty, ["tt1", "tt2"]);
    }

    #[test]
    fn profiles_list_in_creation_order() {
        let store = Store::in_memory().unwrap();
        store.create_profile("Ayush", false).unwrap();
        store.create_profile("Kid", true).unwrap();
        let names: Vec<(String, bool)> = store.profiles().unwrap().into_iter().map(|p| (p.name, p.kids)).collect();
        assert_eq!(names, [("Ayush".to_owned(), false), ("Kid".to_owned(), true)]);
    }

    #[test]
    fn kids_see_only_allowed_non_adult_addons() {
        let addons = [addon("cinemeta", false), addon("debrid", false), addon("spicy", true)];
        let mut kid = Profile { id: "k".into(), name: "Kid".into(), kids: true, pin: None, allowed_addons: vec![] };
        kid.allowed_addons = vec![addons[0].manifest_url.to_string(), addons[2].manifest_url.to_string()];
        let ids = |p: &Profile| p.visible_addons(&addons).iter().map(|a| a.manifest.id.clone()).collect::<Vec<_>>();

        assert_eq!(ids(&kid), ["cinemeta"], "adult addons stay hidden even when allowlisted");

        let adult = Profile { kids: false, ..kid };
        assert_eq!(ids(&adult), ["cinemeta", "debrid", "spicy"]);
    }
}
