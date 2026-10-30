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
    /// Poster and card corners.
    pub radius: f32,
    /// Space above page content (Netflix's top bar floats over it).
    pub top: f32,
    /// Stremio layout: icon sidebar instead of the top bar, no hero, titles under posters.
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
    radius: 4.,
    top: 100.,
    sidebar: false,
};

const STREMIO: Palette = Palette {
    bg: (17, 15, 30),
    surface: (30, 27, 54),
    raised: (48, 44, 84),
    border: (40, 36, 70),
    placeholder: (34, 31, 60),
    track: (70, 64, 110),
    muted: (165, 160, 195),
    accent: (123, 91, 245),
    panel: (24, 21, 44, 0.86),
    cta: ((123, 91, 245), (255, 255, 255)),
    radius: 12.,
    top: 36.,
    sidebar: true,
};

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
