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

/// A grey block that gently pulses where content is still loading.
#[derive(PartialEq)]
pub struct Placeholder {
    pub width: Size,
    pub height: f32,
    pub radius: f32,
}

impl Placeholder {
    pub fn new(width: impl Into<Size>, height: f32) -> Self {
        Self { width: width.into(), height, radius: crate::theme::theme().radius }
    }
}

impl Component for Placeholder {
    fn render(&self) -> impl IntoElement {
        let pulse = use_animation(|conf| {
            conf.on_creation(OnCreation::Run);
            // Reversing on every finish makes it breathe back and forth.
            conf.on_finish(OnFinish::Reverse { delay: std::time::Duration::ZERO });
            AnimNum::new(0.45, 1.).time(850).ease(Ease::InOut).function(Function::Sine)
        });
        rect()
            .width(self.width.clone())
            .height(Size::px(self.height))
            .corner_radius(self.radius)
            .background(crate::theme::theme().placeholder)
            .opacity(pulse.get().value())
    }
}

/// Relix's loader: a bright arc sweeping round a faint ring, readable on any background
/// (Freya's own is a thin arc in a muted colour that vanishes over video).
#[derive(PartialEq)]
pub struct Spinner {
    pub size: f32,
}

impl Component for Spinner {
    fn render(&self) -> impl IntoElement {
        let spin = use_animation(|conf| {
            conf.on_creation(OnCreation::Run);
            conf.on_finish(OnFinish::restart());
            AnimNum::new(0., 360.).time(900)
        });
        SvgViewer::new(
            br##"<svg viewBox="0 0 100 100" xmlns="http://www.w3.org/2000/svg">
                <circle cx="50" cy="50" r="40" fill="none" stroke="#ffffff" stroke-opacity="0.22" stroke-width="9"/>
                <circle cx="50" cy="50" r="40" fill="none" stroke="#ffffff" stroke-width="9"
                    stroke-dasharray="75 252" stroke-linecap="round"/>
            </svg>"##
                .as_slice(),
        )
        .show_loader(false)
        .a11y_role(AccessibilityRole::ProgressIndicator)
        .width(Size::px(self.size))
        .height(Size::px(self.size))
        .rotation(spin.get().value())
    }
}

/// A spinner with a short note, for actions in progress.
pub fn busy(text: &str) -> Rect {
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(10.)
        .child(Spinner { size: 22. })
        .child(label().text(text.to_owned()).font_size(15.).color(crate::theme::theme().muted))
}
