//! The web build plays video in the page's own player (web.rs), not inside the canvas: the
//! player screen's video area is empty there.

use std::rc::Rc;

use freya::prelude::*;

use crate::mpv::Mpv;

pub struct VideoRenderer {
    mpv: Mpv,
}

impl VideoRenderer {
    pub fn mpv(&self) -> &Mpv {
        &self.mpv
    }
}

pub fn video(_: Rc<VideoRenderer>, _: u64) -> Rect {
    rect()
}
