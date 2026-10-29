//! My List and Continue Watching (PLAN.md phase 2).

use relix_proto::addon::MetaPreview;
use serde::{Deserialize, Serialize};

pub use crate::player::Progress;

/// Watched this far and you're "started"; Netflix-style, a peek doesn't count.
const STARTED_SECS: f64 = 30.;
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

impl WatchState {
    pub fn finished(&self) -> bool {
        self.duration > 0. && self.position >= self.duration * FINISHED
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
    states.retain(|s| s.up_next || (s.position >= STARTED_SECS && !s.finished()));
    states.sort_by_key(|s| std::cmp::Reverse(s.updated));
    states
}

#[cfg(test)]
mod tests {
    use super::*;

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
            watched("barely", "barely", 10., 3000., 3),
            watched("done", "done", 2950., 3000., 4),
        ];
        let ids: Vec<_> = continue_watching(states).into_iter().map(|s| s.title.id).collect();
        assert_eq!(ids, ["new", "old"], "barely-started and finished titles stay out");
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
