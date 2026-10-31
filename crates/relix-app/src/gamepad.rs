//! Gamepads drive the same keys as the keyboard: D-pad or left stick move, A presses (or pauses
//! while video plays), B goes back.

use freya::prelude::{Key, NamedKey};
use gilrs::{Axis, Button, EventType, Gilrs};
use tokio::sync::mpsc::UnboundedSender;

pub enum Pad {
    Key(Key),
    /// A: press the focused item, or play/pause.
    Press,
}

/// A stick past this far counts as a press of that direction.
const STICK: f32 = 0.6;

/// Reads controllers until the app closes; runs on its own thread.
pub fn run(tx: UnboundedSender<Pad>) {
    let Ok(mut gilrs) = Gilrs::new() else { return };
    let (mut x, mut y) = (0f32, 0f32);
    loop {
        while let Some(ev) = gilrs.next_event_blocking(None) {
            let pad = match ev.event {
                EventType::ButtonPressed(b, _) => match b {
                    Button::DPadUp => Some(Pad::Key(Key::Named(NamedKey::ArrowUp))),
                    Button::DPadDown => Some(Pad::Key(Key::Named(NamedKey::ArrowDown))),
                    Button::DPadLeft => Some(Pad::Key(Key::Named(NamedKey::ArrowLeft))),
                    Button::DPadRight => Some(Pad::Key(Key::Named(NamedKey::ArrowRight))),
                    Button::South => Some(Pad::Press),
                    Button::East => Some(Pad::Key(Key::Named(NamedKey::Escape))),
                    _ => None,
                },
                EventType::AxisChanged(Axis::LeftStickX, v, _) => {
                    let step = stick_step(x, v);
                    x = v;
                    step.map(|right| Pad::Key(Key::Named(if right { NamedKey::ArrowRight } else { NamedKey::ArrowLeft })))
                }
                EventType::AxisChanged(Axis::LeftStickY, v, _) => {
                    let step = stick_step(y, v);
                    y = v;
                    // Up is positive on gilrs' Y axis.
                    step.map(|up| Pad::Key(Key::Named(if up { NamedKey::ArrowUp } else { NamedKey::ArrowDown })))
                }
                _ => None,
            };
            if let Some(pad) = pad
                && tx.send(pad).is_err()
            {
                return;
            }
        }
    }
}

/// One step when the stick crosses the threshold outward: `Some(true)` for positive.
fn stick_step(prev: f32, now: f32) -> Option<bool> {
    if now >= STICK && prev < STICK {
        Some(true)
    } else if now <= -STICK && prev > -STICK {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::stick_step;

    #[test]
    fn stick_steps_once_per_push() {
        assert_eq!(stick_step(0., 0.7), Some(true));
        assert_eq!(stick_step(0.7, 0.9), None, "holding it doesn't repeat");
        assert_eq!(stick_step(0.9, 0.2), None);
        assert_eq!(stick_step(0.2, -0.8), Some(false));
    }
}
