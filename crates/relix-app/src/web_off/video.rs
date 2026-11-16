//! The web build plays video in the page's own player (web.rs), over the canvas: the player
//! screen's video area is empty there, and its mpv is the stand-in that controls that player.

use std::rc::Rc;
use std::sync::Arc;

use freya::prelude::*;

use crate::mpv::Mpv;

pub struct VideoRenderer {
    mpv: Arc<Mpv>,
}

impl VideoRenderer {
    pub fn new(mpv: Arc<Mpv>, _ready: std::sync::mpsc::Sender<()>, _on_frame: impl Fn() + 'static) -> Self {
        Self { mpv }
    }

    pub fn mpv(&self) -> &Mpv {
        &self.mpv
    }
}

pub fn video(_: Rc<VideoRenderer>, _: u64) -> Rect {
    rect()
}
