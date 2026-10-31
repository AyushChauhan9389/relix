//! My List and Continue Watching (PLAN.md phase 2).

use relix_proto::addon::{Meta, MetaPreview, Video, seasons};
use serde::{Deserialize, Serialize};

pub use crate::player::Progress;
use crate::home::Row;
use crate::store::Store;

/// Watched this far and you're "started"; Netflix-style, a peek doesn't count.
/// Past this share of the runtime counts as finished (credits).
const FINISHED: f64 = 0.95;

/// Where a profile is in a title: the progress record, one per title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchState {
    /// Enough of the title to draw its poster and open it, without the network.
    pub title: MetaPreview,
    /// The movie itself or the episode to continue.
    pub video_id: String,
    pub position: f64,
    pub duration: f64,
    /// The previous episode was finished and this one hasn't started.
    pub up_next: bool,
    /// Seconds since the Unix epoch.
    pub updated: u64,
}

/// Whether playback stopped in the credits or later.
pub fn is_finished(p: Progress) -> bool {
    p.duration > 0. && p.position >= p.duration * FINISHED
}

impl WatchState {
    pub fn finished(&self) -> bool {
        is_finished(Progress { position: self.position, duration: self.duration })
    }

    /// Where to start `video_id`: the saved position if it's the video we stopped in.
    pub fn resume_at(&self, video_id: &str) -> f64 {
        if self.video_id == video_id && !self.finished() { self.position } else { 0. }
    }

    pub fn fraction(&self) -> f32 {
        if self.duration > 0. { (self.position / self.duration) as f32 } else { 0. }
    }
}

/// The progress record after mpv stops playing `video_id`.
/// A finished episode with a `next_video` becomes "up next" for that episode.
pub fn record_playback(title: MetaPreview, video_id: &str, p: Progress, next_video: Option<String>, now: u64) -> WatchState {
    let state = WatchState { title, video_id: video_id.into(), position: p.position, duration: p.duration, up_next: false, updated: now };
    match next_video {
        Some(next) if state.finished() => WatchState { video_id: next, position: 0., duration: 0., up_next: true, ..state },
        _ => state,
    }
}

/// The Continue Watching row: up-next episodes and started, unfinished titles, newest first.
pub fn continue_watching(mut states: Vec<WatchState>) -> Vec<WatchState> {
    // Anything pressed Play on counts, even before a second is watched.
    states.retain(|s| s.up_next || !s.finished());
    states.sort_by_key(|s| std::cmp::Reverse(s.updated));
    states
}

/// The episode after `current` in airing order, skipping specials.
pub fn next_video(videos: &[Video], current: &str) -> Option<String> {
    let order: Vec<&Video> =
        seasons(videos).into_iter().filter(|(s, _)| *s > 0).flat_map(|(_, eps)| eps).collect();
    let i = order.iter().position(|v| v.id == current)?;
    order.get(i + 1).map(|v| v.id.clone())
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

/// The poster-sized part of a title, as stored in progress and My List records.
pub fn preview(m: &Meta) -> MetaPreview {
    MetaPreview {
        id: m.id.clone(),
        r#type: m.r#type.clone(),
        name: m.name.clone(),
        poster: m.poster.clone(),
        background: m.background.clone(),
        logo: m.logo.clone(),
        description: m.description.clone(),
        release_info: m.release_info.clone(),
        genres: m.genres.clone(),
        imdb_rating: m.imdb_rating.clone(),
    }
}

/// Home's Continue Watching and My List rows, straight from the local store.
pub fn local_rows(store: &Store, profile: &str) -> Vec<Row> {
    let states: Vec<WatchState> =
        store.list::<WatchState>(profile, "progress").unwrap_or_default().into_iter().map(|(_, s)| s).collect();
    let watching = continue_watching(states);
    // Removing from My List writes `null`, so the removal can sync.
    let list: Vec<MetaPreview> = store
        .list::<Option<MetaPreview>>(profile, "library")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(_, m)| m)
        .collect();

    let mut rows = vec![];
    if !watching.is_empty() {
        rows.push(Row {
            id: "local/continue".into(),
            title: "Continue Watching".into(),
            progress: watching.iter().map(WatchState::fraction).collect(),
            metas: watching.into_iter().map(|s| s.title).collect(),
            loading: false,
        });
    }
    if !list.is_empty() {
        rows.push(Row { id: "local/my-list".into(), title: "My List".into(), metas: list, progress: vec![], loading: false });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_episode_crosses_seasons_and_skips_specials() {
        let v = |id: &str, s: u32, e: u32| -> Video {
            serde_json::from_value(serde_json::json!({ "id": id, "season": s, "episode": e })).unwrap()
        };
        let videos = [v("s1e2", 1, 2), v("s0e1", 0, 1), v("s2e1", 2, 1), v("s1e1", 1, 1)];
        assert_eq!(next_video(&videos, "s1e1").as_deref(), Some("s1e2"));
        assert_eq!(next_video(&videos, "s1e2").as_deref(), Some("s2e1"));
        assert_eq!(next_video(&videos, "s2e1"), None);
    }

    fn title(id: &str) -> MetaPreview {
        serde_json::from_value(serde_json::json!({ "id": id, "type": "series", "name": id })).unwrap()
    }

    fn watched(id: &str, video: &str, position: f64, duration: f64, updated: u64) -> WatchState {
        record_playback(title(id), video, Progress { position, duration }, None, updated)
    }

    #[test]
    fn shows_started_unfinished_titles_newest_first() {
        let states = vec![
            watched("old", "old", 600., 3000., 1),
            watched("new", "new", 900., 3000., 2),
            watched("just-pressed", "just-pressed", 0., 0., 3),
            watched("done", "done", 2950., 3000., 4),
        ];
        let ids: Vec<_> = continue_watching(states).into_iter().map(|s| s.title.id).collect();
        assert_eq!(ids, ["just-pressed", "new", "old"], "finished titles stay out, just-started ones show");
    }

    #[test]
    fn finishing_an_episode_queues_the_next_one() {
        let s = record_playback(title("tt1"), "tt1:1:1", Progress { position: 2900., duration: 3000. }, Some("tt1:1:2".into()), 5);
        assert_eq!(s.video_id, "tt1:1:2");
        assert!(s.up_next);
        assert_eq!(s.position, 0.);
        let ids: Vec<_> = continue_watching(vec![s]).into_iter().map(|s| s.video_id).collect();
        assert_eq!(ids, ["tt1:1:2"], "up-next shows even though it hasn't started");
    }

    #[test]
    fn finishing_the_last_episode_leaves_the_row() {
        let s = record_playback(title("tt1"), "tt1:5:16", Progress { position: 2990., duration: 3000. }, None, 5);
        assert!(continue_watching(vec![s]).is_empty());
    }

    #[test]
    fn resume_position_only_for_the_same_video() {
        let s = watched("tt1", "tt1:1:3", 1200., 3000., 1);
        assert_eq!(s.resume_at("tt1:1:3"), 1200.);
        assert_eq!(s.resume_at("tt1:1:4"), 0.);
    }
}
