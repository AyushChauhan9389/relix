//! Shared intro times: where an episode's intro starts and ends, by its video id ("tt0903747:1:2").

/// An intro in seconds from the start of the episode.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Intro {
    pub start: f64,
    pub end: f64,
}

impl Intro {
    /// Plausible for an intro: in the first 20 minutes, 5 seconds to 5 minutes long.
    pub fn valid(&self) -> bool {
        self.start >= 0. && self.end <= 1200. && (5. ..=300.).contains(&(self.end - self.start))
    }
}

/// Video ids as Stremio addons make them: short, letters, digits and `:._-`.
pub fn valid_video_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || ":._-".contains(c))
}
