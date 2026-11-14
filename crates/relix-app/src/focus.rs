//! Spatial focus: arrow keys move to the nearest focusable in that direction (PLAN.md: navigation).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

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
    touch: State<TouchDrag>,
}

pub fn use_scroller() -> Scroller {
    Scroller { controller: use_scroll_controller(ScrollConfig::default), viewport: use_state(Area::default), touch: use_state(TouchDrag::default) }
}

/// A finger dragging a view. Freya's own touch scrolling measures speed between touch events,
/// which Android delivers in bursts microseconds apart, so its flings flew off: views here turn
/// it off (`drag_scrolling(false)`) and [`Scroller::wrap`] scrolls them instead.
#[derive(Clone, Copy, Default, PartialEq)]
struct TouchDrag {
    /// Where the finger went down, and where the view was then.
    from: Option<(CursorPoint, (f32, f32))>,
    /// Up/down or sideways, decided once the finger has moved: a row scrolls sideways while the
    /// page around it scrolls up and down, never both.
    vertical: Option<bool>,
    /// The latest speed sample: when, where the finger was, and its speed (points a second).
    sample: Option<(Instant, CursorPoint, (f64, f64))>,
    /// Counts touches, so a fling still running stops when the finger comes back.
    generation: u32,
}

/// How far a finger moves before it's a drag rather than a tap.
const SLOP: f64 = 8.;
/// Speed is measured over at least this long, never between two events of one burst.
const SAMPLE: Duration = Duration::from_millis(16);

impl Scroller {
    /// Wraps the scroll view in a rect that measures the viewport and scrolls it by touch.
    pub fn wrap(self, view: impl IntoElement) -> Rect {
        let mut viewport = self.viewport;
        let (mut touch, controller) = (self.touch, self.controller);
        // Size events also come on relayouts that change nothing; only real changes notify readers.
        rect()
            .expanded()
            .on_sized(move |e: Event<SizedEventData>| {
                if *viewport.peek() != e.area {
                    viewport.set(e.area)
                }
            })
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if !matches!(e.data(), PointerEventData::Touch(_)) {
                    return;
                }
                let (at, pos) = (e.global_location(), controller.position());
                let generation = touch.peek().generation + 1;
                touch.set(TouchDrag { from: Some((at, (pos.x, pos.y))), vertical: None, sample: Some((Instant::now(), at, (0., 0.))), generation });
            })
            .on_global_pointer_move(move |e: Event<PointerEventData>| {
                let drag = *touch.peek();
                let Some((start, (x0, y0))) = drag.from else { return };
                let at = e.global_location();
                let (dx, dy) = (at.x - start.x, at.y - start.y);
                let vertical = match drag.vertical {
                    Some(v) => v,
                    None if dx.abs().max(dy.abs()) > SLOP => dy.abs() > dx.abs(),
                    None => return,
                };
                let mut controller = controller;
                if vertical {
                    controller.scroll_to_y((y0 as f64 + dy) as i32);
                } else {
                    controller.scroll_to_x((x0 as f64 + dx) as i32);
                }
                let now = Instant::now();
                let sample = match drag.sample {
                    Some((t, p, _)) if now - t >= SAMPLE => {
                        let secs = (now - t).as_secs_f64();
                        Some((now, at, ((at.x - p.x) / secs, (at.y - p.y) / secs)))
                    }
                    other => other,
                };
                touch.set(TouchDrag { vertical: Some(vertical), sample, ..drag });
            })
            .on_global_pointer_up(move |_: Event<PointerEventData>| {
                let drag = *touch.peek();
                if drag.from.is_none() {
                    return;
                }
                touch.set(TouchDrag { from: None, ..drag });
                // A finger that stopped before lifting doesn't throw the view.
                let (Some(vertical), Some((t, _, (vx, vy)))) = (drag.vertical, drag.sample) else { return };
                if t.elapsed() < Duration::from_millis(100) {
                    let pos = controller.position();
                    let (from, speed) = if vertical { (pos.y, vy) } else { (pos.x, vx) };
                    fling(controller, touch, drag.generation, vertical, from, speed as f32);
                }
            })
            .child(view)
    }

    /// The viewport's width; reading it re-renders the reader when the window is resized.
    pub fn width(&self) -> f32 {
        self.viewport.read().width()
    }

    /// Glides the view most of a screen up or down; returns whether it can go further that way.
    fn step(self, down: bool) -> bool {
        let (vp, pos) = (*self.viewport.peek(), self.controller.position());
        let by = vp.height() * 0.6;
        let to = if down { pos.y - by } else { pos.y + by };
        // Already at the top (positions run negative going down).
        if !down && pos.y >= 0. {
            return false;
        }
        glide(self.controller, true, pos.y, to.min(0.));
        true
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

/// Lets a thrown view coast to a stop: it starts at the finger's speed and travels about a
/// third of a second's worth, easing out.
fn fling(mut controller: ScrollController, touch: State<TouchDrag>, generation: u32, vertical: bool, from: f32, speed: f32) {
    const MAX_SPEED: f32 = 8000.;
    let speed = speed.clamp(-MAX_SPEED, MAX_SPEED);
    if speed.abs() < 300. {
        return;
    }
    let distance = speed * 0.3;
    // An ease-out cubic starts at 3 × distance / time, so this time starts it at the finger's speed.
    let time = 3. * distance / speed;
    // One step per frame the display shows (Freya's ticker), not on a timer: a timer drifts
    // against a 90/120 Hz screen and the coast judders.
    let (ticker, window) = (RenderingTicker::get(), Platform::get().current_window());
    spawn(async move {
        let started = Instant::now();
        loop {
            window.send(UserEvent::RequestRedraw);
            ticker.tick().await;
            if touch.peek().generation != generation {
                return;
            }
            let t = (started.elapsed().as_secs_f32() / time).min(1.);
            let at = (from + distance * (1. - (1. - t).powi(3))) as i32;
            let moved = if vertical { controller.scroll_to_y(at) } else { controller.scroll_to_x(at) };
            // Done, or stopped at an end.
            if t >= 1. || !moved {
                return;
            }
        }
    });
}

#[derive(Clone)]
struct Entry {
    area: Area,
    /// The horizontal row this focusable scrolls within, if any.
    row: Option<Scroller>,
    /// What Enter does, so a gamepad's A button can press it too.
    on_press: EventHandler<()>,
    /// One of the player's controls: while video plays only these are reachable, since the page
    /// under the player stays mounted.
    in_player: bool,
    /// A text field: Left and Right move its cursor, only Up and Down leave it.
    text: bool,
    /// The control a remote focuses first in the player (play/pause).
    primary: bool,
    /// In the Stremio look's sidebar: Up and Down stay in it, and on the page they stay on the
    /// page (rows not built yet would otherwise lose to a sidebar link below); Left and Right cross.
    rail: bool,
}

/// Every mounted focusable and the current page's vertical scroller.
#[derive(Clone, Default)]
pub struct Registry {
    entries: Rc<RefCell<HashMap<AccessibilityId, Entry>>>,
    page: Rc<RefCell<Option<Scroller>>>,
    /// The player's primary control takes focus as soon as it mounts (the controls were hidden).
    pending_primary: Rc<std::cell::Cell<bool>>,
}

/// Provided by the player, so the focusables inside it know they're its controls.
#[derive(Clone)]
pub struct InPlayer;

impl Registry {
    /// Handles arrow keys among the page's focusables, or the player's; returns whether focus moved.
    pub fn navigate(&self, dir: Dir, in_player: bool) -> bool {
        let focused = *Platform::get()
            .current_window()
            .focused_accessibility_id
            .peek();
        let entries = self.entries.borrow();
        let in_scope = |e: &Entry| e.in_player == in_player;
        let target = match entries.get(&focused).filter(|e| in_scope(e)) {
            Some(from) if from.text && matches!(dir, Dir::Left | Dir::Right) => return false,
            Some(from) => {
                let vertical = matches!(dir, Dir::Up | Dir::Down);
                let others: Vec<(AccessibilityId, Area)> = entries
                    .iter()
                    .filter(|(id, e)| {
                        **id != focused && in_scope(e) && !(vertical && e.rail != from.rail)
                    })
                    .map(|(id, e)| (*id, e.area))
                    .collect();
                next_focus(from.area, &others, dir).copied()
            }
            // Nothing focused here yet (id 0 is the window root; anything else unknown is a text
            // input that keeps its keys): start at the top-left.
            None if focused == AccessibilityId(0) || entries.contains_key(&focused) => entries
                .iter()
                .filter(|(_, e)| in_scope(e))
                .min_by(|a, b| {
                    (a.1.area.min_y(), a.1.area.min_x())
                        .partial_cmp(&(b.1.area.min_y(), b.1.area.min_x()))
                        .unwrap()
                })
                .map(|(id, _)| *id),
            None => None,
        };
        let Some(target) = target else {
            drop(entries);
            // Rows off-screen aren't built yet (virtual lists): scroll that way, so the next press
            // finds them.
            let page = *self.page.borrow();
            return match (page, dir) {
                (Some(page), Dir::Up | Dir::Down) if !in_player => page.step(dir == Dir::Down),
                _ => false,
            };
        };
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

    /// Whether one of the player's controls has focus.
    /// A text field has focus: keys are for typing.
    pub fn typing(&self) -> bool {
        let focused = *Platform::get()
            .current_window()
            .focused_accessibility_id
            .peek();
        self.entries.borrow().get(&focused).is_some_and(|e| e.text)
    }

    pub fn player_focused(&self) -> bool {
        let focused = *Platform::get()
            .current_window()
            .focused_accessibility_id
            .peek();
        self.entries
            .borrow()
            .get(&focused)
            .is_some_and(|e| e.in_player)
    }

    /// Focuses the player's play/pause button, now or as soon as the controls show.
    pub fn focus_player(&self) {
        let primary = self
            .entries
            .borrow()
            .iter()
            .find(|(_, e)| e.in_player && e.primary)
            .map(|(id, _)| *id);
        match primary {
            Some(id) => id.request_focus(),
            None => self.pending_primary.set(true),
        }
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
    // Pages scroll up and down only: Freya also scrolls a view sideways when its content is a
    // little wider than the window, and a touchpad swipe then slid the whole page left.
    use_side_effect(move || {
        let mut c = scroller.controller;
        let (x, _): (i32, i32) = c.into();
        if x != 0 {
            c.scroll_to_x(0);
        }
    });
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
    /// Its width in its parent, when it should stretch (its child then fills it).
    pub width: Option<Size>,
    /// The player's play/pause button ([`Registry::focus_player`]).
    pub primary: bool,
    /// A link in the Stremio look's sidebar ([`Entry::rail`]).
    pub rail: bool,
}

impl Focusable {
    pub fn new(child: impl Into<Element>, on_press: impl Into<EventHandler<()>>) -> Self {
        Self {
            child: child.into(),
            on_press: on_press.into(),
            row: None,
            radius: 6.,
            grow: 1.04,
            width: None,
            primary: false,
            rail: false,
        }
    }
}

impl Component for Focusable {
    fn render(&self) -> impl IntoElement {
        let id = use_a11y();
        let focus = use_focus(id);
        let registry = use_consume::<Registry>();
        let in_player = use_try_consume::<InPlayer>().is_some();
        let (primary, rail) = (self.primary, self.rail);
        use_drop({
            let registry = registry.clone();
            move || {
                registry.entries.borrow_mut().remove(&id);
            }
        });
        let (on_press, row) = (self.on_press.clone(), self.row);
        let pressed = self.on_press.clone();
        let mut hovered = use_state(|| false);
        // Where a touch went down: lifting the finger after dragging (scrolling a row or the page,
        // swiping the hero) is a scroll, not a tap.
        let down = use_hook(|| Rc::new(std::cell::Cell::new(None::<CursorPoint>)));
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
            .map(self.width.clone(), |el, w| el.width(w))
            .corner_radius(self.radius)
            .scale((1. + (grow - 1.) * l, 1. + (grow - 1.) * l))
            .on_pointer_enter(move |_| hovered.set(true))
            .on_pointer_leave(move |_| hovered.set(false))
            .on_sized(move |e: Event<SizedEventData>| {
                let entry = Entry {
                    area: e.area,
                    row,
                    on_press: pressed.clone(),
                    in_player,
                    text: false,
                    primary,
                    rail,
                };
                let moved = registry
                    .entries
                    .borrow_mut()
                    .insert(id, entry)
                    .is_some_and(|old| old.area != e.area);
                if primary && registry.pending_primary.replace(false) {
                    id.request_focus();
                }
                // A resize or scroll moves it without a pointer-leave; the next pointer move re-hovers it.
                if moved && *hovered.peek() {
                    hovered.set(false);
                }
            })
            .on_pointer_down({
                let down = down.clone();
                move |e: Event<PointerEventData>| down.set(Some(e.global_location()))
            })
            .on_press(move |e: Event<PressEventData>| {
                if let (PressEventData::Touch(t), Some(from)) = (e.data(), down.get())
                    && is_drag(from, t.global_location)
                {
                    return;
                }
                on_press.call(())
            })
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

/// A text field the arrow keys can reach, so a TV remote can get to it (its soft keyboard opens
/// on focus). Up and Down leave it; Left and Right stay with its cursor.
#[derive(PartialEq)]
pub struct Field(pub Input);

impl Component for Field {
    fn render(&self) -> impl IntoElement {
        let id = use_a11y();
        let registry = use_consume::<Registry>();
        let in_player = use_try_consume::<InPlayer>().is_some();
        use_drop({
            let registry = registry.clone();
            move || {
                registry.entries.borrow_mut().remove(&id);
            }
        });
        rect()
            .width(Size::fill())
            .on_sized(move |e: Event<SizedEventData>| {
                let on_press = EventHandler::new(move |()| id.request_focus());
                registry.entries.borrow_mut().insert(
                    id,
                    Entry {
                        area: e.area,
                        row: None,
                        on_press,
                        in_player,
                        text: true,
                        primary: false,
                        rail: false,
                    },
                );
            })
            .child(
                self.0
                    .clone()
                    .a11y_id(id)
                    .on_pre_key_down(|e: Event<KeyboardEventData>| match &e.key {
                        // Up and Down pass through to the arrow-key focus (lib.rs) instead of the field
                        // keeping them; otherwise Freya's default: the field takes the key.
                        Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) => false,
                        Key::Named(NamedKey::Enter | NamedKey::Escape | NamedKey::Shift) => true,
                        Key::Named(NamedKey::Tab) => false,
                        _ => {
                            e.stop_propagation();
                            e.prevent_default();
                            true
                        }
                    }),
            )
    }
}

/// A touch that travelled this far (logical pixels) was a drag, not a tap.
fn is_drag(from: CursorPoint, to: CursorPoint) -> bool {
    (to - from).length() > 12.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_short_touch_is_a_tap() {
        assert!(!is_drag((100., 100.).into(), (105., 103.).into()));
        assert!(is_drag((100., 100.).into(), (60., 102.).into()), "a swipe across a row");
        assert!(is_drag((100., 100.).into(), (100., 140.).into()), "a scroll down the page");
    }

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

#[cfg(test)]
mod touch {
    use std::cell::RefCell;

    use freya_testing::prelude::*;

    use super::*;

    thread_local! {
        static PAGE: RefCell<Option<ScrollController>> = const { RefCell::new(None) };
    }

    fn page() -> impl IntoElement {
        let page = use_scroller();
        PAGE.with(|c| *c.borrow_mut() = Some(page.controller));
        page.wrap(
            ScrollView::new_controlled(page.controller)
                .drag_scrolling(false)
                .children((0..60).map(|i| rect().height(Size::px(80.)).width(Size::fill()).child(label().text(format!("row {i}"))).into_element())),
        )
    }

    /// Drags up 500 pixels on a 2.75× phone (~182 points) in ten moves, `gap` apart, holds still
    /// for `rest`, then lets go; returns where the page was at release and once it settled.
    fn drag(gap: Duration, rest: Duration) -> (f32, f32) {
        // The app runs inside Tokio (main.rs); the fling's timer needs it.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _rt = rt.enter();
        let (mut t, _) = TestingRunner::new(page, Size2D::new(1080., 2340.), |_| {}, 2.75);
        t.sync_and_update();
        let y = || PAGE.with(|c| c.borrow().unwrap()).position().y;
        t.press_touch((500., 1500.));
        for step in 1..=10 {
            std::thread::sleep(gap);
            t.move_touch((500., 1500. - 50. * step as f64));
        }
        std::thread::sleep(rest);
        let released = t.run_in(y);
        t.release_touch((500., 1000.));
        t.poll(Duration::from_millis(16), Duration::from_millis(1200));
        (released, t.run_in(y))
    }

    #[test]
    fn the_page_follows_the_finger() {
        // A burst of moves (as Android sends them) lands exactly under the finger, and a finger
        // that stopped before lifting doesn't throw the page.
        let (released, settled) = drag(Duration::ZERO, Duration::from_millis(150));
        assert!((released + 181.8).abs() < 2., "follows the finger: {released}");
        assert!((settled - released).abs() < 2., "no throw after resting: {settled}");
    }

    #[test]
    fn a_flick_coasts_a_little_further() {
        // ~18 points every 20ms is about 900 points a second: it should coast ~270 more.
        let (released, settled) = drag(Duration::from_millis(20), Duration::ZERO);
        let coast = released - settled;
        assert!((150. ..450.).contains(&coast), "coasted {coast} past {released}");
    }
}
