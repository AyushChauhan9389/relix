//! Spatial focus: arrow keys move to the nearest focusable in that direction (PLAN.md: navigation).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use freya::animation::*;
use freya::prelude::*;

use crate::motion::{HOVER_MS, glide};

/// Space the floating nav covers at the top of every page.
const TOP_INSET: f32 = 90.;
const MARGIN: f32 = 24.;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// The candidate to focus when moving `dir` from `from`, or `None` at an edge.
///
/// Scores each candidate ahead of `from` by the gap along `dir` plus twice the
/// sideways offset of the centres, so the same row/column beats a closer diagonal.
pub fn next_focus<K>(from: Area, candidates: &[(K, Area)], dir: Dir) -> Option<&K> {
    let c = from.center();
    candidates
        .iter()
        .filter_map(|(key, a)| {
            let ac = a.center();
            let (gap, side) = match dir {
                Dir::Right if ac.x > c.x && a.min_x() >= from.min_x() + 1. => (a.min_x() - from.max_x(), ac.y - c.y),
                Dir::Left if ac.x < c.x && a.max_x() <= from.max_x() - 1. => (from.min_x() - a.max_x(), ac.y - c.y),
                Dir::Down if ac.y > c.y && a.min_y() >= from.min_y() + 1. => (a.min_y() - from.max_y(), ac.x - c.x),
                Dir::Up if ac.y < c.y && a.max_y() <= from.max_y() - 1. => (from.min_y() - a.max_y(), ac.x - c.x),
                _ => return None,
            };
            Some((gap.max(0.) + 2. * side.abs(), key))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, key)| key)
}

/// A scroll view plus its on-screen viewport, so focus can scroll targets into view.
#[derive(Clone, Copy, PartialEq)]
pub struct Scroller {
    pub controller: ScrollController,
    viewport: State<Area>,
}

pub fn use_scroller() -> Scroller {
    Scroller { controller: use_scroll_controller(ScrollConfig::default), viewport: use_state(Area::default) }
}

impl Scroller {
    /// Wraps the scroll view in a rect that measures the viewport.
    pub fn wrap(self, view: impl IntoElement) -> Rect {
        let mut viewport = self.viewport;
        // Size events also come on relayouts that change nothing; only real changes notify readers.
        rect()
            .expanded()
            .on_sized(move |e: Event<SizedEventData>| {
                if *viewport.peek() != e.area {
                    viewport.set(e.area)
                }
            })
            .child(view)
    }

    /// The viewport's width; reading it re-renders the reader when the window is resized.
    pub fn width(&self) -> f32 {
        self.viewport.read().width()
    }

    /// Glides the view so `area` is inside it (with `top_inset` clear of the nav).
    fn reveal(self, area: Area, dir: Direction, top_inset: f32) {
        let vp = *self.viewport.peek();
        let pos = self.controller.position();
        match dir {
            Direction::Vertical => {
                let (top, bottom) = (vp.min_y() + top_inset, vp.max_y() - MARGIN);
                if area.min_y() < top {
                    glide(self.controller, true, pos.y, pos.y + top - area.min_y());
                } else if area.max_y() > bottom {
                    glide(self.controller, true, pos.y, pos.y - (area.max_y() - bottom));
                }
            }
            Direction::Horizontal => {
                let (left, right) = (vp.min_x() + MARGIN, vp.max_x() - MARGIN);
                if area.min_x() < left {
                    glide(self.controller, false, pos.x, pos.x + left - area.min_x());
                } else if area.max_x() > right {
                    glide(self.controller, false, pos.x, pos.x - (area.max_x() - right));
                }
            }
        }
    }
}

#[derive(Clone)]
struct Entry {
    area: Area,
    /// The horizontal row this focusable scrolls within, if any.
    row: Option<Scroller>,
    /// What Enter does, so a gamepad's A button can press it too.
    on_press: EventHandler<()>,
}

/// Every mounted focusable and the current page's vertical scroller.
#[derive(Clone, Default)]
pub struct Registry {
    entries: Rc<RefCell<HashMap<AccessibilityId, Entry>>>,
    page: Rc<RefCell<Option<Scroller>>>,
}

impl Registry {
    /// Handles arrow keys; returns whether focus moved.
    pub fn navigate(&self, dir: Dir) -> bool {
        let focused = *Platform::get().current_window().focused_accessibility_id.peek();
        let entries = self.entries.borrow();
        let target = match entries.get(&focused) {
            Some(from) => {
                let others: Vec<(AccessibilityId, Area)> =
                    entries.iter().filter(|(id, _)| **id != focused).map(|(id, e)| (*id, e.area)).collect();
                next_focus(from.area, &others, dir).copied()
            }
            // Nothing focused yet (id 0 is the window root, so not a text input): start at the top-left.
            None if focused == AccessibilityId(0) => entries
                .iter()
                .min_by(|a, b| (a.1.area.min_y(), a.1.area.min_x()).partial_cmp(&(b.1.area.min_y(), b.1.area.min_x())).unwrap())
                .map(|(id, _)| *id),
            None => None,
        };
        let Some(target) = target else { return false };
        let entry = entries[&target].clone();
        drop(entries);
        target.request_focus();
        if let Some(row) = entry.row {
            row.reveal(entry.area, Direction::Horizontal, 0.);
        }
        if let Some(page) = *self.page.borrow() {
            page.reveal(entry.area, Direction::Vertical, TOP_INSET);
        }
        true
    }

    /// Presses the focused item (a gamepad's A button); returns whether something was focused.
    pub fn press(&self) -> bool {
        let focused = *Platform::get().current_window().focused_accessibility_id.peek();
        let on_press = self.entries.borrow().get(&focused).map(|e| e.on_press.clone());
        on_press.map(|f| f.call(())).is_some()
    }
}

/// Makes `scroller` the page arrow-key focus scrolls vertically, while this component lives.
pub fn use_page_scroller(scroller: Scroller) {
    let registry = use_consume::<Registry>();
    use_hook({
        let registry = registry.clone();
        move || *registry.page.borrow_mut() = Some(scroller)
    });
    use_drop(move || *registry.page.borrow_mut() = None);
}

/// Something arrow keys can move to and Enter can press, with a focus ring.
#[derive(PartialEq)]
pub struct Focusable {
    pub child: Element,
    pub on_press: EventHandler<()>,
    pub row: Option<Scroller>,
    pub radius: f32,
    /// How much it grows when hovered or focused (posters grow more).
    pub grow: f32,
}

impl Focusable {
    pub fn new(child: impl Into<Element>, on_press: impl Into<EventHandler<()>>) -> Self {
        Self { child: child.into(), on_press: on_press.into(), row: None, radius: 6., grow: 1.04 }
    }
}

impl Component for Focusable {
    fn render(&self) -> impl IntoElement {
        let id = use_a11y();
        let focus = use_focus(id);
        let registry = use_consume::<Registry>();
        use_drop({
            let registry = registry.clone();
            move || {
                registry.entries.borrow_mut().remove(&id);
            }
        });
        let (on_press, row) = (self.on_press.clone(), self.row);
        let pressed = self.on_press.clone();
        let mut hovered = use_state(|| false);
        // Hover or focus lifts it a little, eased both ways.
        let lifted = *hovered.read() || focus().is_focused();
        let grow = self.grow;
        let lift = use_animation_transition(lifted, move |from, to| {
            AnimNum::new(if from { 1. } else { 0. }, if to { 1. } else { 0. }).time(HOVER_MS).ease(Ease::Out).function(Function::Cubic)
        });
        let l = lift.get().value();

        rect()
            .a11y_id(id)
            .a11y_focusable(true)
            .corner_radius(self.radius)
            .scale((1. + (grow - 1.) * l, 1. + (grow - 1.) * l))
            .on_pointer_enter(move |_| hovered.set(true))
            .on_pointer_leave(move |_| hovered.set(false))
            .on_sized(move |e: Event<SizedEventData>| {
                let moved = registry.entries.borrow_mut().insert(id, Entry { area: e.area, row, on_press: pressed.clone() }).is_some_and(|old| old.area != e.area);
                // A resize or scroll moves it without a pointer-leave; the next pointer move re-hovers it.
                if moved && *hovered.peek() {
                    hovered.set(false);
                }
            })
            .on_press(move |_| on_press.call(()))
            .border(
                Border::new()
                    // Freya only reports keyboard focus after Tab, so ring any focus; spatial focus is keyboard-driven.
                    .fill(if focus().is_focused() { Color::WHITE } else { Color::TRANSPARENT })
                    .width(3.)
                    .alignment(BorderAlignment::Outer),
            )
            .child(self.child.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: f32, y: f32, w: f32, h: f32) -> Area {
        Area::new((x, y).into(), (w, h).into())
    }

    /// Two rows of three 100x150 posters, 10px apart.
    fn grid() -> Vec<(&'static str, Area)> {
        vec![
            ("a0", area(0., 0., 100., 150.)),
            ("a1", area(110., 0., 100., 150.)),
            ("a2", area(220., 0., 100., 150.)),
            ("b0", area(0., 200., 100., 150.)),
            ("b1", area(110., 200., 100., 150.)),
            ("b2", area(220., 200., 100., 150.)),
        ]
    }

    fn from(name: &str) -> Area {
        grid().into_iter().find(|(n, _)| *n == name).unwrap().1
    }

    #[test]
    fn moves_within_a_row_and_between_rows() {
        let g = grid();
        assert_eq!(next_focus(from("a0"), &g, Dir::Right), Some(&"a1"));
        assert_eq!(next_focus(from("a1"), &g, Dir::Left), Some(&"a0"));
        assert_eq!(next_focus(from("a1"), &g, Dir::Down), Some(&"b1"));
        assert_eq!(next_focus(from("b2"), &g, Dir::Up), Some(&"a2"));
    }

    #[test]
    fn stops_at_the_edges() {
        let g = grid();
        assert_eq!(next_focus(from("a0"), &g, Dir::Left), None);
        assert_eq!(next_focus(from("a0"), &g, Dir::Up), None);
        assert_eq!(next_focus(from("b2"), &g, Dir::Right), None);
    }

    #[test]
    fn prefers_the_same_row_over_a_closer_diagonal() {
        let cands = [("same_row", area(300., 0., 100., 100.)), ("diagonal", area(120., 160., 100., 100.))];
        assert_eq!(next_focus(area(0., 0., 100., 100.), &cands, Dir::Right), Some(&"same_row"));
    }

    #[test]
    fn down_from_a_button_lands_on_the_aligned_poster() {
        // Hero "Play" button above a row of posters.
        let cands = [
            ("p0", area(48., 700., 150., 225.)),
            ("p1", area(206., 700., 150., 225.)),
            ("p2", area(364., 700., 150., 225.)),
        ];
        assert_eq!(next_focus(area(220., 470., 120., 44.), &cands, Dir::Down), Some(&"p1"));
    }
}
