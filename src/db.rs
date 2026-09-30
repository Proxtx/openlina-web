//! SQLite storage: users (upload tokens), mods and their versions, votes, exported packs.
//!
//! Package files live next to the database (see `store`); the database holds their metadata.
//! No raw IP addresses are stored: votes are keyed by a salted hash of the voter's address.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    token_hash TEXT NOT NULL UNIQUE,
    admin INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS mods (
    id TEXT PRIMARY KEY,
    owner INTEGER NOT NULL REFERENCES users(id),
    created INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS versions (
    mod_id TEXT NOT NULL REFERENCES mods(id),
    version TEXT NOT NULL,
    manifest TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size INTEGER NOT NULL,
    media TEXT NOT NULL,
    status TEXT NOT NULL,
    uploaded_by INTEGER NOT NULL REFERENCES users(id),
    created INTEGER NOT NULL,
    PRIMARY KEY (mod_id, version)
);
CREATE TABLE IF NOT EXISTS votes (
    mod_id TEXT NOT NULL,
    voter TEXT NOT NULL,
    value INTEGER NOT NULL,
    created INTEGER NOT NULL,
    PRIMARY KEY (mod_id, voter)
);
CREATE TABLE IF NOT EXISTS packs (id TEXT PRIMARY KEY, body TEXT NOT NULL, created INTEGER NOT NULL);
";

pub const STATUSES: &[&str] = &["unreviewed", "reviewed", "rejected"];

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub admin: bool,
}

#[derive(Debug, Clone)]
pub struct Version {
    pub mod_id: String,
    pub version: String,
    pub manifest: String,
    pub sha256: String,
    pub size: i64,
    /// Media file names (`icon.png`, `*.gif`), served under `/media/<id>/<version>/`.
    pub media: Vec<String>,
    pub status: String,
    pub uploaded_by: String,
    pub created: i64,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("system randomness");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path).with_context(|| format!("opening {}", path.display()))?)
    }

    pub fn open_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The secret salt for voter hashes, created on first use.
    pub fn salt(&self) -> Result<Vec<u8>> {
        let c = self.conn();
        if let Some(s) = c.query_row("SELECT value FROM meta WHERE key = 'salt'", [], |r| r.get(0)).optional()? {
            return Ok(s);
        }
        let mut salt = vec![0u8; 32];
        getrandom::fill(&mut salt).expect("system randomness");
        c.execute("INSERT INTO meta (key, value) VALUES ('salt', ?1)", params![salt])?;
        Ok(salt)
    }

    // ------------------------------------------------------------------ users

    /// Create a user; returns the upload token (only its hash is stored).
    pub fn add_user(&self, name: &str, admin: bool) -> Result<String> {
        let token = format!("olt_{}", random_hex(20));
        self.conn()
            .execute(
                "INSERT INTO users (name, token_hash, admin, created) VALUES (?1, ?2, ?3, ?4)",
                params![name, sha256_hex(token.as_bytes()), admin, now()],
            )
            .with_context(|| format!("adding user `{name}` (does it exist already?)"))?;
        Ok(token)
    }

    pub fn user_by_token(&self, token: &str) -> Result<Option<User>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, admin FROM users WHERE token_hash = ?1",
                params![sha256_hex(token.as_bytes())],
                |r| Ok(User { id: r.get(0)?, name: r.get(1)?, admin: r.get(2)? }),
            )
            .optional()?)
    }

    /// Replace a user's token; the old one stops working. Returns the new token.
    pub fn rotate_token(&self, name: &str) -> Result<Option<String>> {
        let token = format!("olt_{}", random_hex(20));
        let n = self
            .conn()
            .execute("UPDATE users SET token_hash = ?1 WHERE name = ?2", params![sha256_hex(token.as_bytes()), name])?;
        Ok((n > 0).then_some(token))
    }

    pub fn users(&self) -> Result<Vec<User>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT id, name, admin FROM users ORDER BY id")?;
        let rows = st.query_map([], |r| Ok(User { id: r.get(0)?, name: r.get(1)?, admin: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ------------------------------------------------------------------ mods

    pub fn mod_owner(&self, id: &str) -> Result<Option<i64>> {
        Ok(self.conn().query_row("SELECT owner FROM mods WHERE id = ?1", params![id], |r| r.get(0)).optional()?)
    }

    pub fn insert_version(&self, v: &Version, uploader: i64) -> Result<()> {
        let mut c = self.conn();
        let tx = c.transaction()?;
        tx.execute("INSERT OR IGNORE INTO mods (id, owner, created) VALUES (?1, ?2, ?3)", params![v.mod_id, uploader, v.created])?;
        tx.execute(
            "INSERT INTO versions (mod_id, version, manifest, sha256, size, media, status, uploaded_by, created)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                v.mod_id,
                v.version,
                v.manifest,
                v.sha256,
                v.size,
                serde_json::to_string(&v.media)?,
                v.status,
                uploader,
                v.created
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn versions_where(&self, clause: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<Version>> {
        let c = self.conn();
        let sql = format!(
            "SELECT v.mod_id, v.version, v.manifest, v.sha256, v.size, v.media, v.status, u.name, v.created
             FROM versions v JOIN users u ON u.id = v.uploaded_by {clause} ORDER BY v.mod_id, v.created"
        );
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(args, |r| {
            let media: String = r.get(5)?;
            Ok(Version {
                mod_id: r.get(0)?,
                version: r.get(1)?,
                manifest: r.get(2)?,
                sha256: r.get(3)?,
                size: r.get(4)?,
                media: serde_json::from_str(&media).unwrap_or_default(),
                status: r.get(6)?,
                uploaded_by: r.get(7)?,
                created: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn all_versions(&self) -> Result<Vec<Version>> {
        self.versions_where("", &[])
    }

    pub fn versions_of(&self, id: &str) -> Result<Vec<Version>> {
        self.versions_where("WHERE v.mod_id = ?1", &[&id])
    }

    pub fn versions_by_status(&self, status: &str) -> Result<Vec<Version>> {
        self.versions_where("WHERE v.status = ?1", &[&status])
    }

    pub fn versions_by_uploader(&self, user: i64) -> Result<Vec<Version>> {
        self.versions_where("WHERE v.uploaded_by = ?1", &[&user])
    }

    /// Set a version's review status. Returns false if there is no such version.
    pub fn set_status(&self, id: &str, version: &str, status: &str) -> Result<bool> {
        let n = self
            .conn()
            .execute("UPDATE versions SET status = ?1 WHERE mod_id = ?2 AND version = ?3", params![status, id, version])?;
        Ok(n > 0)
    }

    // ------------------------------------------------------------------ votes

    /// Record a vote (1 or -1), or remove it (0).
    pub fn vote(&self, id: &str, voter: &str, value: i64) -> Result<()> {
        let c = self.conn();
        if value == 0 {
            c.execute("DELETE FROM votes WHERE mod_id = ?1 AND voter = ?2", params![id, voter])?;
        } else {
            c.execute(
                "INSERT INTO votes (mod_id, voter, value, created) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (mod_id, voter) DO UPDATE SET value = excluded.value, created = excluded.created",
                params![id, voter, value, now()],
            )?;
        }
        Ok(())
    }

    pub fn scores(&self) -> Result<HashMap<String, i64>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT mod_id, SUM(value) FROM votes GROUP BY mod_id")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn votes_of(&self, voter: &str) -> Result<HashMap<String, i64>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT mod_id, value FROM votes WHERE voter = ?1")?;
        let rows = st.query_map(params![voter], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ------------------------------------------------------------------ packs

    pub fn insert_pack(&self, id: &str, body: &str) -> Result<()> {
        self.conn().execute("INSERT INTO packs (id, body, created) VALUES (?1, ?2, ?3)", params![id, body, now()])?;
        Ok(())
    }

    pub fn pack(&self, id: &str) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT body FROM packs WHERE id = ?1", params![id], |r| r.get(0)).optional()?)
    }
}
