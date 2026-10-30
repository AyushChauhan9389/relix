//! Shared motion: fade-and-rise entrances and the timings every animation uses.

use freya::animation::*;
use freya::prelude::*;

/// Entrances (pages, panels, images).
pub const ENTER_MS: u64 = 320;
/// Hover/focus feedback.
pub const HOVER_MS: u64 = 180;

/// Fades its child in (and up by `rise` px) when it mounts. Give it a `.key(...)` that changes
/// with the content (a route, a panel mode) to replay the entrance on every change.
#[derive(PartialEq)]
pub struct FadeIn {
    pub child: Element,
    /// Fill the parent (pages, panels) instead of hugging the child (images).
    pub fill: bool,
    pub rise: f32,
    key: DiffKey,
}

impl FadeIn {
    pub fn page(child: impl Into<Element>) -> Self {
        Self { child: child.into(), fill: true, rise: 14., key: DiffKey::None }
    }

    pub fn image(child: impl Into<Element>) -> Self {
        Self { child: child.into(), fill: false, rise: 0., key: DiffKey::None }
    }
}

impl KeyExt for FadeIn {
    fn key_mut(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for FadeIn {
    fn render(&self) -> impl IntoElement {
        let t = use_animation(|conf| {
            conf.on_creation(OnCreation::Run);
            AnimNum::new(0., 1.).time(ENTER_MS).ease(Ease::Out).function(Function::Cubic)
        });
        let v = t.get().value();
        rect()
            .maybe(self.fill, |el| el.expanded())
            .opacity(v)
            .offset_y(self.rise * (1. - v))
            .child(self.child.clone())
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// Eases between two scroll positions (smooth scrolling for keyboard focus).
pub fn glide(mut controller: ScrollController, vertical: bool, from: f32, to: f32) {
    if (to - from).abs() < 1. {
        return;
    }
    spawn(async move {
        const STEPS: u32 = 12;
        for i in 1..=STEPS {
            let t = i as f32 / STEPS as f32;
            let eased = 1. - (1. - t).powi(3);
            let at = (from + (to - from) * eased) as i32;
            if vertical {
                controller.scroll_to_y(at);
            } else {
                controller.scroll_to_x(at);
            }
            tokio::time::sleep(std::time::Duration::from_millis(14)).await;
        }
    });
}
