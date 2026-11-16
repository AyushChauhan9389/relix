//! Gamepads on the web: not yet (the Gamepad API would do it).

use freya::prelude::Key;
use tokio::sync::mpsc::UnboundedSender;

pub enum Pad {
    Key(Key),
    Press,
}

pub fn run(_: UnboundedSender<Pad>) {}
