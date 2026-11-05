//! Usernames, friends and Watch Together hosts (ADR 0010).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use relix_proto::social::{Friend, Friends, Me, ToClient, Username, valid_username};
use rusqlite::{OptionalExtension, params};

use crate::{ApiError, ApiResult, AppState, account_of, require_admin as admin_only};

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS friend_requests (from_id INTEGER NOT NULL, to_id INTEGER NOT NULL, PRIMARY KEY (from_id, to_id));
    CREATE TABLE IF NOT EXISTS friends (a INTEGER NOT NULL, b INTEGER NOT NULL, PRIMARY KEY (a, b));
    CREATE TABLE IF NOT EXISTS blocks (blocker INTEGER NOT NULL, blocked INTEGER NOT NULL, PRIMARY KEY (blocker, blocked));
    CREATE TABLE IF NOT EXISTS preview_hosts (account INTEGER PRIMARY KEY);";

/// Databases from before usernames get the column and its case-insensitive uniqueness.
pub fn migrate(db: &rusqlite::Connection) -> rusqlite::Result<()> {
    if !db.prepare("SELECT 1 FROM pragma_table_info('accounts') WHERE name = 'username'")?.exists([])? {
        db.execute_batch("ALTER TABLE accounts ADD COLUMN username TEXT;")?;
    }
    db.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS accounts_username ON accounts (username COLLATE NOCASE);")
}

/// Removes every trace of an account from the social tables (account deletion).
pub fn forget(db: &rusqlite::Connection, account: i64) -> rusqlite::Result<()> {
    for sql in [
        "DELETE FROM friend_requests WHERE from_id = ?1 OR to_id = ?1",
        "DELETE FROM friends WHERE a = ?1 OR b = ?1",
        "DELETE FROM blocks WHERE blocker = ?1 OR blocked = ?1",
        "DELETE FROM preview_hosts WHERE account = ?1",
    ] {
        db.execute(sql, params![account])?;
    }
    Ok(())
}

pub fn username_of(s: &AppState, account: i64) -> Result<Option<String>, ApiError> {
    Ok(s.db.lock().unwrap().query_row("SELECT username FROM accounts WHERE id = ?1", params![account], |r| r.get(0))?)
}

pub fn can_host(s: &AppState, account: i64) -> rusqlite::Result<bool> {
    s.db.lock().unwrap().query_row("SELECT EXISTS (SELECT 1 FROM preview_hosts WHERE account = ?1)", params![account], |r| r.get(0))
}

pub fn friend_ids(s: &AppState, account: i64) -> rusqlite::Result<Vec<i64>> {
    let db = s.db.lock().unwrap();
    let mut stmt = db.prepare("SELECT b FROM friends WHERE a = ?1")?;
    stmt.query_map(params![account], |r| r.get(0))?.collect()
}

/// A friend of `account` by username.
pub fn friend_by_name(s: &AppState, account: i64, name: &str) -> rusqlite::Result<Option<i64>> {
    s.db.lock().unwrap().query_row(
        "SELECT f.b FROM friends f JOIN accounts a ON a.id = f.b WHERE f.a = ?1 AND a.username = ?2 COLLATE NOCASE",
        params![account, name],
        |r| r.get(0),
    )
    .optional()
}

/// Another account by username, hidden from anyone it blocked.
fn other(s: &AppState, me: i64, name: &str) -> Result<i64, ApiError> {
    let db = s.db.lock().unwrap();
    let id: Option<i64> =
        db.query_row("SELECT id FROM accounts WHERE username = ?1 COLLATE NOCASE", params![name], |r| r.get(0)).optional()?;
    let Some(id) = id else { return Err(ApiError(StatusCode::NOT_FOUND, "no one has that username")) };
    if id == me {
        return Err(ApiError(StatusCode::BAD_REQUEST, "that's you"));
    }
    let blocked: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM blocks WHERE (blocker = ?1 AND blocked = ?2) OR (blocker = ?2 AND blocked = ?1))",
        params![id, me],
        |r| r.get(0),
    )?;
    if blocked {
        return Err(ApiError(StatusCode::NOT_FOUND, "no one has that username"));
    }
    Ok(id)
}

fn changed(s: &AppState, accounts: &[i64]) {
    for a in accounts {
        s.hub.send(*a, &ToClient::FriendsChanged);
    }
}

pub async fn me(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<Me> {
    let account = account_of(&s, &headers)?;
    let (email, username) =
        s.db.lock().unwrap().query_row("SELECT email, username FROM accounts WHERE id = ?1", params![account], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(Json(Me { email, username, can_host: can_host(&s, account)? }))
}

pub async fn set_username(State(s): State<AppState>, headers: HeaderMap, Json(req): Json<Username>) -> Result<StatusCode, ApiError> {
    let account = account_of(&s, &headers)?;
    let name = req.username.trim();
    if !valid_username(name) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "use 3-20 letters, digits or _"));
    }
    let db = s.db.lock().unwrap();
    let taken: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM accounts WHERE username = ?1 COLLATE NOCASE AND id != ?2)",
        params![name, account],
        |r| r.get(0),
    )?;
    if taken {
        return Err(ApiError(StatusCode::CONFLICT, "that username is taken"));
    }
    db.execute("UPDATE accounts SET username = ?1 WHERE id = ?2", params![name, account])?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<Friends> {
    let account = account_of(&s, &headers)?;
    let (friends, incoming, outgoing) = {
        let db = s.db.lock().unwrap();
        let names = |sql: &str| -> rusqlite::Result<Vec<(i64, String)>> {
            let mut stmt = db.prepare(sql)?;
            stmt.query_map(params![account], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default())))?.collect()
        };
        (
            names("SELECT a.id, a.username FROM friends f JOIN accounts a ON a.id = f.b WHERE f.a = ?1 ORDER BY a.username COLLATE NOCASE")?,
            names("SELECT a.id, a.username FROM friend_requests r JOIN accounts a ON a.id = r.from_id WHERE r.to_id = ?1 ORDER BY a.username")?,
            names("SELECT a.id, a.username FROM friend_requests r JOIN accounts a ON a.id = r.to_id WHERE r.from_id = ?1 ORDER BY a.username")?,
        )
    };
    let friends = friends
        .into_iter()
        .map(|(id, username)| {
            let (online, watching) = s.hub.presence(id);
            Friend { username, online, watching }
        })
        .collect();
    let names = |v: Vec<(i64, String)>| v.into_iter().map(|(_, n)| n).collect();
    Ok(Json(Friends { friends, incoming: names(incoming), outgoing: names(outgoing) }))
}

pub async fn request(State(s): State<AppState>, headers: HeaderMap, Json(req): Json<Username>) -> Result<StatusCode, ApiError> {
    let account = account_of(&s, &headers)?;
    let other = other(&s, account, &req.username)?;
    {
        let db = s.db.lock().unwrap();
        let crossed: bool =
            db.query_row("SELECT EXISTS (SELECT 1 FROM friend_requests WHERE from_id = ?1 AND to_id = ?2)", params![other, account], |r| r.get(0))?;
        if crossed {
            befriend(&db, account, other)?;
        } else {
            let already: bool = db.query_row("SELECT EXISTS (SELECT 1 FROM friends WHERE a = ?1 AND b = ?2)", params![account, other], |r| r.get(0))?;
            if !already {
                db.execute("INSERT OR IGNORE INTO friend_requests (from_id, to_id) VALUES (?1, ?2)", params![account, other])?;
            }
        }
    }
    changed(&s, &[account, other]);
    Ok(StatusCode::NO_CONTENT)
}

fn befriend(db: &rusqlite::Connection, a: i64, b: i64) -> rusqlite::Result<()> {
    db.execute("DELETE FROM friend_requests WHERE (from_id = ?1 AND to_id = ?2) OR (from_id = ?2 AND to_id = ?1)", params![a, b])?;
    db.execute("INSERT OR IGNORE INTO friends (a, b) VALUES (?1, ?2), (?2, ?1)", params![a, b])?;
    Ok(())
}

pub async fn accept(State(s): State<AppState>, headers: HeaderMap, Json(req): Json<Username>) -> Result<StatusCode, ApiError> {
    let account = account_of(&s, &headers)?;
    let other = other(&s, account, &req.username)?;
    {
        let db = s.db.lock().unwrap();
        let asked: bool =
            db.query_row("SELECT EXISTS (SELECT 1 FROM friend_requests WHERE from_id = ?1 AND to_id = ?2)", params![other, account], |r| r.get(0))?;
        if !asked {
            return Err(ApiError(StatusCode::NOT_FOUND, "no request from them"));
        }
        befriend(&db, account, other)?;
    }
    changed(&s, &[account, other]);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn decline(State(s): State<AppState>, headers: HeaderMap, Json(req): Json<Username>) -> Result<StatusCode, ApiError> {
    let account = account_of(&s, &headers)?;
    let other = other(&s, account, &req.username)?;
    s.db.lock().unwrap().execute("DELETE FROM friend_requests WHERE from_id = ?1 AND to_id = ?2", params![other, account])?;
    changed(&s, &[account, other]);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(State(s): State<AppState>, headers: HeaderMap, Path(name): Path<String>) -> Result<StatusCode, ApiError> {
    let account = account_of(&s, &headers)?;
    let other = other(&s, account, &name)?;
    unfriend(&s.db.lock().unwrap(), account, other)?;
    changed(&s, &[account, other]);
    Ok(StatusCode::NO_CONTENT)
}

fn unfriend(db: &rusqlite::Connection, a: i64, b: i64) -> rusqlite::Result<()> {
    db.execute("DELETE FROM friends WHERE (a = ?1 AND b = ?2) OR (a = ?2 AND b = ?1)", params![a, b])?;
    db.execute("DELETE FROM friend_requests WHERE (from_id = ?1 AND to_id = ?2) OR (from_id = ?2 AND to_id = ?1)", params![a, b])?;
    Ok(())
}

pub async fn block(State(s): State<AppState>, headers: HeaderMap, Json(req): Json<Username>) -> Result<StatusCode, ApiError> {
    let account = account_of(&s, &headers)?;
    let other = other(&s, account, &req.username)?;
    {
        let db = s.db.lock().unwrap();
        unfriend(&db, account, other)?;
        db.execute("INSERT OR IGNORE INTO blocks (blocker, blocked) VALUES (?1, ?2)", params![account, other])?;
    }
    changed(&s, &[account, other]);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn admin_hosts(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<Vec<String>> {
    admin_only(&s, &headers)?;
    let db = s.db.lock().unwrap();
    let mut stmt = db.prepare("SELECT a.username FROM preview_hosts p JOIN accounts a ON a.id = p.account ORDER BY a.username")?;
    let names = stmt.query_map([], |r| Ok(r.get::<_, Option<String>>(0)?.unwrap_or_default()))?.collect::<rusqlite::Result<_>>()?;
    Ok(Json(names))
}

fn account_by_name(s: &AppState, name: &str) -> Result<i64, ApiError> {
    s.db.lock()
        .unwrap()
        .query_row("SELECT id FROM accounts WHERE username = ?1 COLLATE NOCASE", params![name], |r| r.get(0))
        .optional()?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "no one has that username"))
}

pub async fn admin_allow(State(s): State<AppState>, headers: HeaderMap, Path(name): Path<String>) -> Result<StatusCode, ApiError> {
    admin_only(&s, &headers)?;
    let id = account_by_name(&s, &name)?;
    s.db.lock().unwrap().execute("INSERT OR IGNORE INTO preview_hosts (account) VALUES (?1)", params![id])?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn admin_disallow(State(s): State<AppState>, headers: HeaderMap, Path(name): Path<String>) -> Result<StatusCode, ApiError> {
    admin_only(&s, &headers)?;
    let id = account_by_name(&s, &name)?;
    s.db.lock().unwrap().execute("DELETE FROM preview_hosts WHERE account = ?1", params![id])?;
    Ok(StatusCode::NO_CONTENT)
}
