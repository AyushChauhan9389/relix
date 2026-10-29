//! Relix sync and account server (ADR 0003, 0004, 0005). Stores only login hashes,
//! wrapped keys and opaque record values; hands out sequence numbers; notifies devices over SSE.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD};
use futures_util::Stream;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox};
use relix_proto::sync::{
    Login, Prelogin, PreloginResponse, PullResponse, Push, PushResponse, Session, Signup, SignupResponse, SyncedRecord,
    Verify,
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

const MAX_LOGIN_FAILURES: u32 = 10;
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_CODE_TRIES: i64 = 5;

pub struct Config {
    /// Require an emailed code before an account can sign in (the official server does; self-hosters may not).
    pub verify_email: bool,
    pub mailer: Mailer,
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
    /// (account, newest seq) after every push.
    events: broadcast::Sender<(i64, u64)>,
    /// Seeds fake salts for unknown emails.
    secret: Arc<Vec<u8>>,
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
             salt TEXT NOT NULL,
             login_hash TEXT NOT NULL,
             wrapped_key TEXT NOT NULL,
             recovery_wrapped_key TEXT NOT NULL,
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
         CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);",
    )?;
    let secret: Option<Vec<u8>> = db.query_row("SELECT value FROM meta WHERE key = 'secret'", [], |r| r.get(0)).optional()?;
    let secret = match secret {
        Some(s) => s,
        None => {
            let mut s = vec![0u8; 32];
            rand::fill(&mut s[..]);
            db.execute("INSERT INTO meta (key, value) VALUES ('secret', ?1)", params![s])?;
            s
        }
    };

    let state = AppState {
        db: Arc::new(Mutex::new(db)),
        config: Arc::new(config),
        events: broadcast::channel(256).0,
        secret: Arc::new(secret),
        failures: Arc::default(),
    };
    Ok(Router::new()
        .route("/signup", post(signup))
        .route("/verify", post(verify))
        .route("/prelogin", post(prelogin))
        .route("/login", post(login))
        .route("/sync/push", post(push))
        .route("/sync/pull", get(pull))
        .route("/sync/events", get(events))
        .with_state(state))
}

/// Creates a session for `account` and returns it with the account's wrapped keys.
fn new_session(db: &Connection, account: i64) -> Result<Session, ApiError> {
    let mut raw = [0u8; 32];
    rand::fill(&mut raw);
    let token = URL_SAFE_NO_PAD.encode(raw);
    db.execute("INSERT INTO sessions (token_hash, account) VALUES (?1, ?2)", params![sha256_hex(token.as_bytes()), account])?;
    let (wrapped_key, recovery_wrapped_key) = db.query_row(
        "SELECT wrapped_key, recovery_wrapped_key FROM accounts WHERE id = ?1",
        params![account],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(Session { token, wrapped_key, recovery_wrapped_key })
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
    let code = s.config.verify_email.then(six_digit_code);
    let session = {
        let db = s.db.lock().unwrap();
        let taken: bool = db.query_row("SELECT EXISTS (SELECT 1 FROM accounts WHERE email = ?1)", params![email], |r| r.get(0))?;
        if taken {
            return Err(ApiError(StatusCode::CONFLICT, "an account with this email already exists"));
        }
        db.execute(
            "INSERT INTO accounts (email, salt, login_hash, wrapped_key, recovery_wrapped_key, verified, code)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![email, req.salt, sha256_hex(req.login_hash.as_bytes()), req.wrapped_key, req.recovery_wrapped_key, code.is_none(), code],
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

async fn prelogin(State(s): State<AppState>, Json(req): Json<Prelogin>) -> ApiResult<PreloginResponse> {
    let email = normalize(&req.email);
    let salt: Option<String> =
        s.db.lock().unwrap().query_row("SELECT salt FROM accounts WHERE email = ?1", params![email], |r| r.get(0)).optional()?;
    // Unknown emails get a stable made-up salt, so the answer doesn't reveal who has an account.
    let salt = salt.unwrap_or_else(|| B64.encode(&Sha256::digest([s.secret.as_slice(), email.as_bytes()].concat())[..16]));
    Ok(Json(PreloginResponse { salt }))
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
    let db = s.db.lock().unwrap();
    let row: Option<(i64, String, bool)> = db
        .query_row("SELECT id, login_hash, verified FROM accounts WHERE email = ?1", params![email], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    match row {
        Some((id, hash, verified)) if hash == sha256_hex(req.login_hash.as_bytes()) => {
            s.failures.lock().unwrap().remove(&email);
            if !verified {
                return Err(ApiError(StatusCode::FORBIDDEN, "verify your email first"));
            }
            Ok(Json(new_session(&db, id)?))
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
        let _ = s.events.send((account, *last));
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

/// One `data: <seq>` event whenever this account's records change; devices then pull.
async fn events(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let account = account_of(&s, &headers)?;
    let stream = BroadcastStream::new(s.events.subscribe())
        .filter_map(move |e| match e {
            Ok((a, seq)) if a == account => Some(Ok(Event::default().data(seq.to_string()))),
            // Lagged receivers just miss a notice; the next one (or a reconnect) pulls everything anyway.
            _ => None,
        });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
