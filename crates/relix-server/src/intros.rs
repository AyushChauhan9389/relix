//! Shared intro times: each account submits an episode's intro once (its latest replaces it);
//! everyone reads the median of the submissions, so one wrong one can't move it.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use relix_proto::intro::{Intro, valid_video_id};
use rusqlite::params;

use crate::{ApiError, ApiResult, AppState, account_of};

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS intros (video TEXT NOT NULL, account INTEGER NOT NULL, start REAL NOT NULL, end REAL NOT NULL,
        PRIMARY KEY (video, account));";

pub async fn get(State(s): State<AppState>, Path(video): Path<String>) -> ApiResult<Intro> {
    if !valid_video_id(&video) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "bad video id"));
    }
    let rows: Vec<(f64, f64)> = {
        let db = s.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT start, end FROM intros WHERE video = ?1")?;
        stmt.query_map(params![video], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
    };
    median(rows).map(Json).ok_or(ApiError(StatusCode::NOT_FOUND, "no intro known"))
}

pub async fn put(State(s): State<AppState>, headers: HeaderMap, Path(video): Path<String>, Json(intro): Json<Intro>) -> Result<StatusCode, ApiError> {
    let account = account_of(&s, &headers)?;
    if !valid_video_id(&video) || !intro.valid() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "not a plausible intro"));
    }
    s.db.lock().unwrap().execute(
        "INSERT INTO intros (video, account, start, end) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (video, account) DO UPDATE SET start = excluded.start, end = excluded.end",
        params![video, account, intro.start, intro.end],
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes an account's submissions (account deletion).
pub fn forget(db: &rusqlite::Connection, account: i64) -> rusqlite::Result<()> {
    db.execute("DELETE FROM intros WHERE account = ?1", params![account]).map(drop)
}

/// The middle start and middle end (the lower middle of an even count).
fn median(rows: Vec<(f64, f64)>) -> Option<Intro> {
    let mid = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[(v.len() - 1) / 2]
    };
    if rows.is_empty() {
        return None;
    }
    let (starts, ends) = rows.into_iter().unzip();
    Some(Intro { start: mid(starts), end: mid(ends) })
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_wrong_submission_doesnt_move_the_median() {
        let m = super::median(vec![(0., 90.), (2., 91.), (0., 600.)]).unwrap();
        assert_eq!((m.start, m.end), (0., 91.));
        assert!(super::median(vec![]).is_none());
    }
}
