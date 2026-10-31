//! Discord "Now watching": shows the title while video plays. Needs a Discord application id at
//! build time (RELIX_DISCORD_APP_ID); without one it stays off.

use std::sync::mpsc::{Sender, channel};

use discord_rich_presence::activity::{Activity, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

pub struct Watching {
    pub title: String,
    pub episode: String,
    pub image: Option<String>,
}

/// Starts the presence thread; `None` when the build has no Discord app id.
pub fn start() -> Option<Sender<Option<Watching>>> {
    let id = option_env!("RELIX_DISCORD_APP_ID").filter(|id| !id.is_empty())?;
    let (tx, rx) = channel::<Option<Watching>>();
    std::thread::spawn(move || {
        let mut client = DiscordIpcClient::new(id);
        let mut connected = false;
        for watching in rx {
            // Discord may start after Relix or close meanwhile: (re)connect on each change.
            if !connected {
                connected = client.connect().is_ok();
            }
            if !connected {
                continue;
            }
            let result = match &watching {
                Some(w) => {
                    let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                    let mut assets = Assets::new().large_text(&w.title);
                    if let Some(image) = &w.image {
                        assets = assets.large_image(image);
                    }
                    let mut activity = Activity::new().details(&w.title).assets(assets).timestamps(Timestamps::new().start(started));
                    // Discord rejects a state shorter than 2 characters.
                    if w.episode.len() >= 2 {
                        activity = activity.state(&w.episode);
                    }
                    client.set_activity(activity)
                }
                None => client.clear_activity(),
            };
            connected = result.is_ok();
        }
    });
    Some(tx)
}
