//! The two looks Relix can wear, picked in Settings: Netflix (top bar, hero, red) and
//! Stremio (icon sidebar, titled posters, purple).

use std::cell::Cell;

use freya::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Look {
    Netflix,
    Stremio,
}

type Rgb = (u8, u8, u8);

pub struct Palette {
    pub bg: Rgb,
    /// Cards: addons, streams, account boxes.
    pub surface: Rgb,
    /// Secondary buttons and inactive chips.
    pub raised: Rgb,
    pub border: Rgb,
    /// Image placeholders.
    pub placeholder: Rgb,
    /// Progress bar track.
    pub track: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    /// The title page's episodes/streams panel.
    pub panel: (u8, u8, u8, f32),
    /// The main call to action (Play): background and text.
    pub cta: (Rgb, Rgb),
    /// Other main buttons (Sign in, Install): background and text.
    pub primary: (Rgb, Rgb),
    /// The button beside Play (My List, More Info).
    pub secondary: (u8, u8, u8, f32),
    /// Poster and card corners.
    pub radius: f32,
    /// Space above page content (the top bar floats over it).
    pub top: f32,
    /// Stremio layout: icon sidebar plus a search bar on top, no hero, titles under posters.
    pub sidebar: bool,
}

const NETFLIX: Palette = Palette {
    bg: (20, 20, 20),
    surface: (32, 32, 32),
    raised: (60, 60, 60),
    border: (40, 40, 40),
    placeholder: (38, 38, 38),
    track: (90, 90, 90),
    muted: (170, 170, 170),
    accent: (229, 9, 20),
    panel: (12, 12, 12, 0.82),
    cta: ((255, 255, 255), (0, 0, 0)),
    primary: ((229, 9, 20), (255, 255, 255)),
    secondary: (109, 109, 110, 0.7),
    radius: 4.,
    top: 100.,
    sidebar: false,
};

/// Black and greys; Stremio's purple only on Play and progress.
const STREMIO: Palette = Palette {
    bg: (0, 0, 0),
    surface: (22, 22, 22),
    raised: (40, 40, 40),
    border: (28, 28, 28),
    placeholder: (26, 26, 26),
    track: (64, 64, 64),
    muted: (150, 150, 150),
    accent: (123, 91, 245),
    panel: (16, 16, 16, 0.88),
    cta: ((123, 91, 245), (255, 255, 255)),
    primary: ((255, 255, 255), (0, 0, 0)),
    secondary: (40, 40, 40, 1.),
    radius: 10.,
    top: 88.,
    sidebar: true,
};

/// Errors and the 18+ badge, in every look.
pub const ERROR: Rgb = (229, 9, 20);

thread_local! {
    static LOOK: Cell<Look> = const { Cell::new(Look::Netflix) };
}

/// The current look's palette. Shell remounts every page when the look changes, so reading it
/// while rendering is always current.
pub fn theme() -> &'static Palette {
    palette(LOOK.with(Cell::get))
}

pub fn palette(look: Look) -> &'static Palette {
    match look {
        Look::Netflix => &NETFLIX,
        Look::Stremio => &STREMIO,
    }
}

pub fn set_look(look: Look) {
    LOOK.with(|l| l.set(look));
}

/// Freya's built-in widgets (inputs, switches) in the look's colours.
pub fn widgets(look: Look) -> Theme {
    let p = palette(look);
    let mut t = dark_theme();
    t.colors.primary = p.accent.into();
    t.colors.background = p.bg.into();
    t
}

/// The accent as mpv's ASS colour (BGR hex), for the player controls.
pub fn accent_bgr() -> String {
    let (r, g, b) = theme().accent;
    format!("{b:02X}{g:02X}{r:02X}")
}
