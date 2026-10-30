//! Relix sync and account server (ADR 0003, 0004, 0005). Checks Argon2-hashed passwords, stores
//! each account's records, hands out sequence numbers and notifies devices over SSE.

pub mod cli;

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::Stream;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox};
use relix_proto::addon::Addon;
use relix_proto::sync::{
    AddedAdminAddon, AdminAddon, Login, PullResponse, Push, PushResponse, Session, Signup, SignupResponse, SyncedRecord, Verify,
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

const MAX_LOGIN_FAILURES: u32 = 10;
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_CODE_TRIES: i64 = 5;
const MIN_PASSWORD: usize = 8;

pub struct Config {
    /// Require an emailed code before an account can sign in (the official server does; self-hosters may not).
    pub verify_email: bool,
    pub mailer: Mailer,
    /// Bearer token for the `/admin` endpoints; `None` turns them off.
    pub admin_token: Option<String>,
}

pub enum Mailer {
    Smtp { transport: Box<AsyncSmtpTransport<Tokio1Executor>>, from: Mailbox },
    /// Collects mails instead of sending them (tests).
    Outbox(Arc<Mutex<Vec<(String, String)>>>),
    /// Prints mails to stdout (local development).
    Log,
}

impl Mailer {
    async fn send(&self, to: &str, text: String) -> Result<(), ApiError> {
        match self {
            Mailer::Smtp { transport, from } => {
                let to: Mailbox = to.parse().map_err(|_| ApiError(StatusCode::BAD_REQUEST, "invalid email address"))?;
                let msg = Message::builder()
                    .from(from.clone())
                    .to(to)
                    .subject("Your Relix code")
                    .body(text)
                    .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "couldn't build the email"))?;
                transport.send(msg).await.map_err(|e| {
                    eprintln!("smtp: {e}");
                    ApiError(StatusCode::BAD_GATEWAY, "couldn't send the email")
                })?;
            }
            Mailer::Outbox(outbox) => outbox.lock().unwrap().push((to.into(), text)),
            Mailer::Log => println!("mail to {to}: {text}"),
        }
        Ok(())
    }
}

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    config: Arc<Config>,
    /// (account, event data) after every change; `None` addresses every account.
    events: broadcast::Sender<(Option<i64>, String)>,
    // ponytail: in-memory throttle, resets on restart and isn't shared between instances; fine for one box.
    failures: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
}

#[derive(Debug)]
pub struct ApiError(StatusCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        eprintln!("db: {e}");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "database error")
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn normalize(email: &str) -> String {
    email.trim().to_lowercase()
}

pub fn app(db: Connection, config: Config) -> rusqlite::Result<Router> {
    db.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS accounts (
             id INTEGER PRIMARY KEY,
             email TEXT NOT NULL UNIQUE,
             password_hash TEXT NOT NULL,
             verified INTEGER NOT NULL,
             code TEXT,
             code_tries INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS sessions (token_hash TEXT PRIMARY KEY, account INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS records (
             account INTEGER NOT NULL,
             kind TEXT NOT NULL,
             key TEXT NOT NULL,
             value TEXT NOT NULL,
             seq INTEGER NOT NULL,
             PRIMARY KEY (account, kind, key)
         );
         CREATE INDEX IF NOT EXISTS records_by_seq ON records (account, seq);
         CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
         CREATE TABLE IF NOT EXISTS admin_addons (id INTEGER PRIMARY KEY, addon TEXT NOT NULL);",
    )?;
    let state = AppState {
        db: Arc::new(Mutex::new(db)),
        config: Arc::new(config),
        events: broadcast::channel(256).0,
        failures: Arc::default(),
    };
    Ok(Router::new()
        .route("/signup", post(signup))
        .route("/verify", post(verify))
        .route("/login", post(login))
        .route("/sync/push", post(push))
        .route("/sync/pull", get(pull))
        .route("/sync/events", get(events))
        .route("/addons/admin", get(admin_addons_for_users))
        .route("/admin/addons", get(admin_list).post(admin_add))
        .route("/admin/addons/{id}", delete(admin_remove))
        .with_state(state))
}

fn new_session(db: &Connection, account: i64) -> Result<Session, ApiError> {
    let mut raw = [0u8; 32];
    rand::fill(&mut raw);
    let token = URL_SAFE_NO_PAD.encode(raw);
    db.execute("INSERT INTO sessions (token_hash, account) VALUES (?1, ?2)", params![sha256_hex(token.as_bytes()), account])?;
    Ok(Session { token })
}

/// Argon2 is deliberately slow, so it runs off the async workers.
async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || Argon2::default().hash_password(password.as_bytes()).map(|h| h.to_string()))
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or(ApiError(StatusCode::INTERNAL_SERVER_ERROR, "couldn't store the password"))
}

/// Unknown emails (`hash: None`) are checked against a throwaway hash, so they take as long as real ones.
async fn password_matches(password: String, hash: Option<String>) -> bool {
    static DUMMY: OnceLock<String> = OnceLock::new();
    tokio::task::spawn_blocking(move || {
        let dummy = || DUMMY.get_or_init(|| Argon2::default().hash_password(b"relix").map(|h| h.to_string()).unwrap_or_default()).clone();
        let checked = hash.clone().unwrap_or_else(dummy);
        Argon2::default().verify_password(password.as_bytes(), checked.as_str()).is_ok() && hash.is_some()
    })
    .await
    .unwrap_or(false)
}

fn six_digit_code() -> String {
    let mut b = [0u8; 4];
    rand::fill(&mut b);
    format!("{:06}", u32::from_le_bytes(b) % 1_000_000)
}

async fn signup(State(s): State<AppState>, Json(req): Json<Signup>) -> ApiResult<SignupResponse> {
    let email = normalize(&req.email);
    if !email.contains('@') {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid email address"));
    }
    if req.password.chars().count() < MIN_PASSWORD {
        return Err(ApiError(StatusCode::BAD_REQUEST, "use a password of at least 8 characters"));
    }
    let password_hash = hash_password(req.password).await?;
    let code = s.config.verify_email.then(six_digit_code);
    let session = {
        let db = s.db.lock().unwrap();
        let taken: bool = db.query_row("SELECT EXISTS (SELECT 1 FROM accounts WHERE email = ?1)", params![email], |r| r.get(0))?;
        if taken {
            return Err(ApiError(StatusCode::CONFLICT, "an account with this email already exists"));
        }
        db.execute(
            "INSERT INTO accounts (email, password_hash, verified, code) VALUES (?1, ?2, ?3, ?4)",
            params![email, password_hash, code.is_none(), code],
        )?;
        match code {
            None => Some(new_session(&db, db.last_insert_rowid())?),
            Some(_) => None,
        }
    };
    if let Some(code) = code {
        s.config.mailer.send(&email, format!("Your Relix verification code is {code}")).await?;
    }
    Ok(Json(SignupResponse { session }))
}

async fn verify(State(s): State<AppState>, Json(req): Json<Verify>) -> ApiResult<Session> {
    let db = s.db.lock().unwrap();
    let row: Option<(i64, Option<String>, i64)> = db
        .query_row("SELECT id, code, code_tries FROM accounts WHERE email = ?1", params![normalize(&req.email)], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((id, Some(code), tries)) = row else {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "wrong or expired code"));
    };
    if tries >= MAX_CODE_TRIES {
        return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "too many attempts; sign up again"));
    }
    if req.code.trim() != code {
        db.execute("UPDATE accounts SET code_tries = code_tries + 1 WHERE id = ?1", params![id])?;
        return Err(ApiError(StatusCode::UNAUTHORIZED, "wrong or expired code"));
    }
    db.execute("UPDATE accounts SET verified = 1, code = NULL WHERE id = ?1", params![id])?;
    Ok(Json(new_session(&db, id)?))
}

async fn login(State(s): State<AppState>, Json(req): Json<Login>) -> ApiResult<Session> {
    let email = normalize(&req.email);
    {
        let mut failures = s.failures.lock().unwrap();
        if let Some((count, since)) = failures.get(&email).copied() {
            if since.elapsed() > FAILURE_WINDOW {
                failures.remove(&email);
            } else if count >= MAX_LOGIN_FAILURES {
                return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "too many attempts; try again later"));
            }
        }
    }
    let row: Option<(i64, String, bool)> = s
        .db
        .lock()
        .unwrap()
        .query_row("SELECT id, password_hash, verified FROM accounts WHERE email = ?1", params![email], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let matches = password_matches(req.password, row.as_ref().map(|r| r.1.clone())).await;
    match row {
        Some((id, _, verified)) if matches => {
            s.failures.lock().unwrap().remove(&email);
            if !verified {
                return Err(ApiError(StatusCode::FORBIDDEN, "verify your email first"));
            }
            Ok(Json(new_session(&s.db.lock().unwrap(), id)?))
        }
        _ => {
            let mut failures = s.failures.lock().unwrap();
            let entry = failures.entry(email).or_insert((0, Instant::now()));
            entry.0 += 1;
            Err(ApiError(StatusCode::UNAUTHORIZED, "wrong email or password"))
        }
    }
}

/// The account behind the request's bearer token.
fn account_of(s: &AppState, headers: &HeaderMap) -> Result<i64, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(ApiError(StatusCode::UNAUTHORIZED, "sign in first"))?;
    s.db.lock()
        .unwrap()
        .query_row("SELECT account FROM sessions WHERE token_hash = ?1", params![sha256_hex(token.as_bytes())], |r| r.get(0))
        .optional()?
        .ok_or(ApiError(StatusCode::UNAUTHORIZED, "sign in first"))
}

/// Last write wins per record: every write gets the account's next sequence number (ADR 0004).
async fn push(State(s): State<AppState>, headers: HeaderMap, Json(req): Json<Push>) -> ApiResult<PushResponse> {
    let account = account_of(&s, &headers)?;
    let seqs = {
        let mut db = s.db.lock().unwrap();
        let tx = db.transaction()?;
        let mut seq: i64 =
            tx.query_row("SELECT coalesce(max(seq), 0) FROM records WHERE account = ?1", params![account], |r| r.get(0))?;
        let mut seqs = vec![];
        for r in &req.records {
            seq += 1;
            tx.execute(
                "INSERT INTO records (account, kind, key, value, seq) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (account, kind, key) DO UPDATE SET value = excluded.value, seq = excluded.seq",
                params![account, r.kind, r.key, r.value, seq],
            )?;
            seqs.push(seq as u64);
        }
        tx.commit()?;
        seqs
    };
    if let Some(last) = seqs.last() {
        // No listeners is fine.
        let _ = s.events.send((Some(account), last.to_string()));
    }
    Ok(Json(PushResponse { seqs }))
}

#[derive(serde::Deserialize)]
struct Since {
    since: u64,
}

async fn pull(State(s): State<AppState>, headers: HeaderMap, Query(q): Query<Since>) -> ApiResult<PullResponse> {
    let account = account_of(&s, &headers)?;
    let db = s.db.lock().unwrap();
    let mut stmt = db.prepare("SELECT kind, key, value, seq FROM records WHERE account = ?1 AND seq > ?2 ORDER BY seq")?;
    let records = stmt
        .query_map(params![account, q.since as i64], |r| {
            Ok(SyncedRecord { kind: r.get(0)?, key: r.get(1)?, value: r.get(2)?, seq: r.get::<_, i64>(3)? as u64 })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Json(PullResponse { records }))
}

/// One `data: <seq>` event whenever this account's records change, and `data: admin` when the
/// admin's addons change; devices then pull.
async fn events(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let account = account_of(&s, &headers)?;
    let stream = BroadcastStream::new(s.events.subscribe())
        .filter_map(move |e| match e {
            Ok((to, data)) if to.is_none_or(|a| a == account) => Some(Ok(Event::default().data(data))),
            // Lagged receivers just miss a notice; the next one (or a reconnect) pulls everything anyway.
            _ => None,
        });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn require_admin(s: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let given = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    // Comparing digests keeps the check's timing independent of how much of the token matched.
    match (&s.config.admin_token, given) {
        (Some(want), Some(got)) if sha256_hex(want.as_bytes()) == sha256_hex(got.as_bytes()) => Ok(()),
        _ => Err(ApiError(StatusCode::UNAUTHORIZED, "admin token required")),
    }
}

fn load_admin_addons(db: &Connection) -> Result<Vec<AdminAddon>, ApiError> {
    let mut stmt = db.prepare("SELECT id, addon FROM admin_addons ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = vec![];
    for row in rows {
        let (id, json) = row?;
        match serde_json::from_str::<Addon>(&json) {
            Ok(addon) => out.push(AdminAddon { id: id as u64, addon }),
            Err(e) => eprintln!("admin addon {id} unreadable: {e}"),
        }
    }
    Ok(out)
}

async fn admin_list(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<Vec<AdminAddon>> {
    require_admin(&s, &headers)?;
    Ok(Json(load_admin_addons(&s.db.lock().unwrap())?))
}

async fn admin_add(State(s): State<AppState>, headers: HeaderMap, Json(addon): Json<Addon>) -> ApiResult<AddedAdminAddon> {
    require_admin(&s, &headers)?;
    let id = {
        let db = s.db.lock().unwrap();
        db.execute("INSERT INTO admin_addons (addon) VALUES (?1)", params![serde_json::to_string(&addon).unwrap()])?;
        db.last_insert_rowid() as u64
    };
    let _ = s.events.send((None, "admin".into()));
    Ok(Json(AddedAdminAddon { id }))
}

async fn admin_remove(State(s): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    require_admin(&s, &headers)?;
    let removed = s.db.lock().unwrap().execute("DELETE FROM admin_addons WHERE id = ?1", params![id])?;
    if removed == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "no admin addon with that id"));
    }
    let _ = s.events.send((None, "admin".into()));
    Ok(StatusCode::OK)
}

/// What signed-in users receive: the admin's addons in the order they were added.
async fn admin_addons_for_users(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<Vec<Addon>> {
    account_of(&s, &headers)?;
    Ok(Json(load_admin_addons(&s.db.lock().unwrap())?.into_iter().map(|a| a.addon).collect()))
}
