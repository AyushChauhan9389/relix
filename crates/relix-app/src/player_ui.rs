//! The player screen: the video (video.rs) with Relix's controls over it. Icons: Lucide (ISC,
//! assets/icons).

use std::rc::Rc;
use std::time::{Duration, Instant};

use freya::animation::*;
use freya::prelude::*;

use crate::Ctx;
use crate::motion::{FadeIn, HOVER_MS, busy};
use crate::player::{Status, ToPlayer};
use crate::theme::theme;
use crate::video::{VideoRenderer, video};

/// The controls hide after this long without the mouse moving (never while paused or a menu is open).
const HIDE_AFTER: Duration = Duration::from_secs(3);
/// The Next Episode card shows this many seconds before the end.
const NEXT_CARD: f64 = 20.;
const SPEEDS: [f64; 6] = [0.5, 0.75, 1., 1.25, 1.5, 2.];

const ICON_BACK: &[u8] = include_bytes!("../assets/icons/arrow-left.svg");
const ICON_PLAY: &[u8] = include_bytes!("../assets/icons/play.svg");
const ICON_PAUSE: &[u8] = include_bytes!("../assets/icons/pause.svg");
const ICON_BACK10: &[u8] = include_bytes!("../assets/icons/rotate-ccw.svg");
const ICON_FWD10: &[u8] = include_bytes!("../assets/icons/rotate-cw.svg");
const ICON_VOLUME: &[u8] = include_bytes!("../assets/icons/volume-2.svg");
const ICON_VOLUME_LOW: &[u8] = include_bytes!("../assets/icons/volume-1.svg");
const ICON_MUTED: &[u8] = include_bytes!("../assets/icons/volume-x.svg");
const ICON_NEXT: &[u8] = include_bytes!("../assets/icons/skip-forward.svg");
const ICON_CC: &[u8] = include_bytes!("../assets/icons/captions.svg");
const ICON_SPEED: &[u8] = include_bytes!("../assets/icons/gauge.svg");
const ICON_FULLSCREEN: &[u8] = include_bytes!("../assets/icons/maximize.svg");
const ICON_EXIT_FULLSCREEN: &[u8] = include_bytes!("../assets/icons/minimize.svg");
const ICON_CHECK: &[u8] = include_bytes!("../assets/icons/check.svg");

#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Tracks,
    Speed,
}

/// Full-window video and controls while something plays.
#[derive(PartialEq)]
pub struct Player;

impl Component for Player {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let active = use_state(Instant::now);
        let mut menu = use_state(|| None::<Menu>);
        let mut visible = use_state(|| true);
        let mut last_click = use_state(|| None::<Instant>);
        // Checks a few times a second whether the controls should fade out.
        use_hook({
            let status = ctx.player_status;
            move || {
                spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        let keep = status.peek().paused || menu.peek().is_some() || active.peek().elapsed() < HIDE_AFTER;
                        if keep != *visible.peek() {
                            visible.set(keep);
                        }
                    }
                })
            }
        });
        let shown = use_animation_transition(*visible.read(), |from, to| {
            AnimNum::new(if from { 1. } else { 0. }, if to { 1. } else { 0. }).time(240).ease(Ease::Out).function(Function::Cubic)
        });
        let o = shown.get().value();

        let Some(np) = ctx.now_playing.read().clone() else { return rect().expanded().background((0, 0, 0)).into_element() };
        let status = ctx.player_status.read().clone();
        let r = np.video.clone();
        let wake = move || {
            let (mut active, mut visible) = (active, visible);
            active.set(Instant::now());
            visible.set(true);
        };

        rect()
            .expanded()
            .background((0, 0, 0))
            .on_global_pointer_move(move |_| wake())
            .child(
                video(r.clone(), *ctx.video_frame.read())
                    .width(Size::fill())
                    .height(Size::fill())
                    // A click pauses or plays, a double click toggles fullscreen (and undoes the pause).
                    .on_press({
                        let (r, ctx) = (r.clone(), ctx.clone());
                        move |_| {
                            let paused = ctx.player_status.peek().paused;
                            let _ = r.mpv().set_property("pause", !paused);
                            menu.set(None);
                            if last_click.peek().is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
                                ctx.toggle_fullscreen();
                                last_click.set(None);
                            } else {
                                last_click.set(Some(Instant::now()));
                            }
                        }
                    }),
            )
            .maybe(!status.loaded, |el| el.child(layer_center().child(busy("Loading…"))))
            .maybe(status.loaded && status.paused, |el| {
                el.child(layer_center().child(icon_button(ICON_PLAY, 64., true, None, {
                    let r = r.clone();
                    move |()| drop(r.mpv().set_property("pause", false))
                })))
            })
            .maybe(o > 0.01, |el| {
                el.child(top_bar(&ctx, &np.title, &np.episode).opacity(o))
                    .child(bottom_bar(&ctx, &r, &status, np.has_next, menu).opacity(o))
                    .maybe_child((*menu.read()).map(|m| menu_panel(m, &r, &status, menu).opacity(o)))
            })
            .maybe(np.has_next && status.duration > 0. && status.duration - status.position < NEXT_CARD, |el| {
                el.child(next_card(&ctx, status.duration - status.position))
            })
            .into_element()
    }
}

fn layer_center() -> Rect {
    rect().position(Position::new_absolute().top(0.).left(0.)).width(Size::percent(100.)).height(Size::percent(100.)).center()
}

fn top_bar(ctx: &Ctx, title: &str, episode: &str) -> Rect {
    let ctx = ctx.clone();
    rect()
        .position(Position::new_absolute().top(0.).left(0.))
        .width(Size::percent(100.))
        // A fixed height: Freya draws gradients on sized rects.
        .height(Size::px(140.))
        .horizontal()
        .spacing(16.)
        .padding(Gaps::new(24., 40., 0., 32.))
        .background(LinearGradient::new().angle(0.).stop(((0, 0, 0, 0.8), 0.)).stop(((0, 0, 0, 0.), 100.)))
        .child(icon_button(ICON_BACK, 30., false, None, move |()| {
            if let Some(p) = ctx.player.peek().as_ref() {
                let _ = p.send(ToPlayer::Stop);
            }
        }))
        .child(
            rect()
                .spacing(2.)
                .child(label().text(title.to_owned()).font_size(22.).font_weight(FontWeight::BOLD))
                .maybe(!episode.is_empty(), |el| el.child(label().text(episode.to_owned()).font_size(15.).color((200, 200, 200)))),
        )
}

fn bottom_bar(ctx: &Ctx, r: &Rc<VideoRenderer>, s: &Status, has_next: bool, menu: State<Option<Menu>>) -> Rect {
    let pct = if s.duration > 0. { s.position / s.duration * 100. } else { 0. };
    let seek = r.clone();
    let timeline = rect()
        .width(Size::fill())
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(16.)
        .child(
            rect().width(Size::flex(1.)).child(
                Slider::new(move |v: f64| drop(seek.mpv().command("seek", &[&format!("{v:.3}"), "absolute-percent+exact"])))
                    .value(pct)
                    .size(Size::fill()),
            ),
        )
        .child(label().text(format!("-{}", clock(s.duration - s.position))).font_size(15.).color((220, 220, 220)));

    let mpv = |r: &Rc<VideoRenderer>| r.clone();
    let (toggle, back, fwd, mute, vol) = (mpv(r), mpv(r), mpv(r), mpv(r), mpv(r));
    let paused = s.paused;
    let muted = s.muted;
    let volume_icon = if muted || s.volume == 0. { ICON_MUTED } else if s.volume < 50. { ICON_VOLUME_LOW } else { ICON_VOLUME };
    let next_ctx = ctx.clone();
    let fs_ctx = ctx.clone();
    let left = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(10.)
        .child(icon_button(if paused { ICON_PLAY } else { ICON_PAUSE }, 30., true, None, move |()| {
            drop(toggle.mpv().set_property("pause", !paused))
        }))
        .child(icon_button(ICON_BACK10, 30., false, Some("10"), move |()| drop(back.mpv().command("seek", &["-10", "relative"]))))
        .child(icon_button(ICON_FWD10, 30., false, Some("10"), move |()| drop(fwd.mpv().command("seek", &["10", "relative"]))))
        .child(icon_button(volume_icon, 28., false, None, move |()| drop(mute.mpv().set_property("mute", !muted))))
        .child(
            rect().width(Size::px(110.)).child(
                Slider::new(move |v: f64| {
                    let _ = vol.mpv().set_property("volume", v);
                    let _ = vol.mpv().set_property("mute", false);
                })
                .value(if muted { 0. } else { s.volume.min(100.) })
                .size(Size::fill()),
            ),
        );
    let toggle_menu = move |m: Menu| {
        let mut menu = menu;
        let open = *menu.peek() == Some(m);
        menu.set(if open { None } else { Some(m) });
    };
    let right = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(10.)
        .maybe(has_next, |el| {
            el.child(icon_button(ICON_NEXT, 28., false, None, move |()| {
                if let Some(p) = next_ctx.player.peek().as_ref() {
                    let _ = p.send(ToPlayer::Next);
                }
            }))
        })
        .child(icon_button(ICON_CC, 28., false, None, move |()| toggle_menu(Menu::Tracks)))
        .child(icon_button(ICON_SPEED, 28., false, None, move |()| toggle_menu(Menu::Speed)))
        .child(icon_button(
            if *ctx.fullscreen.read() { ICON_EXIT_FULLSCREEN } else { ICON_FULLSCREEN },
            28.,
            false,
            None,
            move |()| fs_ctx.toggle_fullscreen(),
        ));

    rect()
        .position(Position::new_absolute().bottom(0.).left(0.))
        .width(Size::percent(100.))
        .height(Size::px(220.))
        .main_align(Alignment::End)
        .spacing(10.)
        .padding(Gaps::new(0., 40., 24., 40.))
        .background(LinearGradient::new().angle(0.).stop(((0, 0, 0, 0.), 0.)).stop(((0, 0, 0, 0.9), 100.)))
        .child(timeline)
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .cross_align(Alignment::Center)
                .content(Content::Flex)
                .child(left)
                .child(rect().width(Size::flex(1.)))
                .maybe(s.speed != 1., |el| el.child(label().text(format!("{}x", s.speed)).font_size(15.).color((220, 220, 220))))
                .child(right),
        )
}

/// Subtitles and audio, or playback speed, above the bottom-right buttons.
fn menu_panel(m: Menu, r: &Rc<VideoRenderer>, s: &Status, menu: State<Option<Menu>>) -> Rect {
    let panel = rect()
        .position(Position::new_absolute().bottom(110.).right(40.))
        .padding(Gaps::new_all(16.))
        .corner_radius(12.)
        .background((20, 20, 20, 0.96))
        .border(Border::new().fill((255, 255, 255, 0.08)).width(1.))
        .horizontal()
        .spacing(32.);
    let column = |title: &'static str| rect().width(Size::px(240.)).spacing(4.).child(label().text(title).font_size(13.).color(theme().muted));
    let option = |text: String, selected: bool, on: Box<dyn Fn()>| {
        MenuOption { text, selected, on_press: EventHandler::new(move |()| on()) }.into_element()
    };
    match m {
        Menu::Tracks => {
            let pick = |prop: &'static str, id: Option<i64>| {
                let r = r.clone();
                Box::new(move || {
                    let _ = match id {
                        Some(id) => r.mpv().set_property(prop, id),
                        None => r.mpv().set_property(prop, "no"),
                    };
                    let mut menu = menu;
                    menu.set(None);
                }) as Box<dyn Fn()>
            };
            let subs = column("Subtitles")
                .child(option("Off".into(), s.sid.is_none(), pick("sid", None)))
                .children(s.subs.iter().map(|t| option(t.title.clone(), s.sid == Some(t.id), pick("sid", Some(t.id)))));
            let audio = column("Audio").children(s.audio.iter().map(|t| option(t.title.clone(), s.aid == Some(t.id), pick("aid", Some(t.id)))));
            panel.child(subs).maybe(s.audio.len() > 1, |el| el.child(audio))
        }
        Menu::Speed => panel.child(column("Speed").children(SPEEDS.iter().map(|&v| {
            let r = r.clone();
            option(if v == 1. { "Normal".into() } else { format!("{v}x") }, s.speed == v, Box::new(move || {
                let _ = r.mpv().set_property("speed", v);
                let mut menu = menu;
                menu.set(None);
            }))
        }))),
    }
}

/// A menu row with a hover highlight and a check when selected.
#[derive(PartialEq)]
struct MenuOption {
    text: String,
    selected: bool,
    on_press: EventHandler<()>,
}

impl Component for MenuOption {
    fn render(&self) -> impl IntoElement {
        let mut hovered = use_state(|| false);
        let on_press = self.on_press.clone();
        rect()
            .width(Size::fill())
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(10.)
            .padding(Gaps::new(8., 10., 8., 10.))
            .corner_radius(8.)
            .background(if *hovered.read() { (255, 255, 255, 0.10) } else { (255, 255, 255, 0.) })
            .on_pointer_enter(move |_| hovered.set(true))
            .on_pointer_leave(move |_| hovered.set(false))
            .on_press(move |_| on_press.call(()))
            .child(rect().width(Size::px(18.)).maybe_child(self.selected.then(|| svg(ICON_CHECK, 18.))))
            .child(label().text(self.text.clone()).font_size(15.).max_lines(1).text_overflow(TextOverflow::Ellipsis))
    }
}

fn next_card(ctx: &Ctx, left: f64) -> Rect {
    let ctx = ctx.clone();
    rect()
        .position(Position::new_absolute().bottom(140.).right(40.))
        .child(FadeIn::page(
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(12.)
                .padding(Gaps::new(14., 22., 14., 18.))
                .corner_radius(10.)
                .background((255, 255, 255))
                .color((0, 0, 0))
                .on_press(move |_| {
                    if let Some(p) = ctx.player.peek().as_ref() {
                        let _ = p.send(ToPlayer::Next);
                    }
                })
                .child(SvgViewer::new(ICON_PLAY).width(Size::px(26.)).height(Size::px(26.)).fill((0, 0, 0)).stroke((0, 0, 0)))
                .child(
                    rect()
                        .child(label().text("Next Episode").font_size(17.).font_weight(FontWeight::BOLD))
                        .child(label().text(format!("Starts in {}s", left.ceil().max(0.))).font_size(13.).color((90, 90, 90))),
                ),
        ))
}

fn svg(icon: &'static [u8], size: f32) -> SvgViewer {
    SvgViewer::new(icon).width(Size::px(size)).height(Size::px(size)).stroke((255, 255, 255))
}

fn icon_button(icon: &'static [u8], size: f32, filled: bool, badge: Option<&'static str>, on_press: impl Fn(()) + 'static) -> IconButton {
    IconButton { icon, size, filled, badge, on_press: EventHandler::new(on_press) }
}

/// A round icon button that lightens and grows a little on hover.
#[derive(PartialEq)]
struct IconButton {
    icon: &'static [u8],
    size: f32,
    /// Solid icons (play, pause) instead of outlines.
    filled: bool,
    /// A small number inside the icon ("10" on the skip buttons).
    badge: Option<&'static str>,
    on_press: EventHandler<()>,
}

impl Component for IconButton {
    fn render(&self) -> impl IntoElement {
        let mut hovered = use_state(|| false);
        let t = use_animation_transition(*hovered.read(), |from, to| {
            AnimNum::new(if from { 1. } else { 0. }, if to { 1. } else { 0. }).time(HOVER_MS).ease(Ease::Out).function(Function::Cubic)
        });
        let h = t.get().value();
        let on_press = self.on_press.clone();
        let icon = svg(self.icon, self.size);
        let icon = if self.filled { icon.fill((255, 255, 255)) } else { icon };
        rect()
            .padding(Gaps::new_all(8.))
            .corner_radius(999.)
            .background((255, 255, 255, 0.14 * h))
            .scale((1. + 0.1 * h, 1. + 0.1 * h))
            .on_pointer_enter(move |_| hovered.set(true))
            .on_pointer_leave(move |_| hovered.set(false))
            .on_press(move |_| on_press.call(()))
            .child(
                rect()
                    .width(Size::px(self.size))
                    .height(Size::px(self.size))
                    .center()
                    .child(rect().position(Position::new_absolute().top(0.).left(0.)).child(icon))
                    .maybe_child(self.badge.map(|b| label().text(b).font_size(self.size * 0.3).font_weight(FontWeight::BOLD))),
            )
    }
}

/// "1:02:03" or "4:05".
fn clock(secs: f64) -> String {
    let s = secs.max(0.) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Keys while video plays: Space pauses, arrows seek and change volume, F fullscreen, M mute,
/// N next episode. Returns false for keys it doesn't use.
pub fn key(ctx: &Ctx, key: &Key) -> bool {
    let Some(np) = ctx.now_playing.peek().clone() else { return false };
    let (mpv, s) = (np.video.mpv(), ctx.player_status.peek().clone());
    match key {
        Key::Character(c) if c == " " => drop(mpv.set_property("pause", !s.paused)),
        Key::Named(NamedKey::ArrowLeft) => drop(mpv.command("seek", &["-10", "relative"])),
        Key::Named(NamedKey::ArrowRight) => drop(mpv.command("seek", &["10", "relative"])),
        Key::Named(NamedKey::ArrowUp) => drop(mpv.set_property("volume", (s.volume + 5.).min(100.))),
        Key::Named(NamedKey::ArrowDown) => drop(mpv.set_property("volume", (s.volume - 5.).max(0.))),
        Key::Character(c) if c.eq_ignore_ascii_case("f") => ctx.toggle_fullscreen(),
        Key::Character(c) if c.eq_ignore_ascii_case("m") => drop(mpv.set_property("mute", !s.muted)),
        Key::Character(c) if c.eq_ignore_ascii_case("n") && np.has_next => {
            if let Some(p) = ctx.player.peek().as_ref() {
                let _ = p.send(ToPlayer::Next);
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn clock_shows_hours_only_when_needed() {
        assert_eq!(super::clock(65.), "1:05");
        assert_eq!(super::clock(3723.), "1:02:03");
        assert_eq!(super::clock(-3.), "0:00");
    }
}
