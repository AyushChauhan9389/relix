//! The player screen: the video (video.rs) with Relix's controls over it. Icons: Lucide (ISC,
//! assets/icons).

use std::rc::Rc;
use std::time::{Duration, Instant};

use freya::animation::*;
use freya::prelude::*;

use crate::Ctx;
use crate::home::{backdrop, title_art};
use crate::motion::{FadeIn, HOVER_MS};
use crate::player::{Status, ToPlayer, Track};
use crate::theme::theme;
use crate::video::{VideoRenderer, video};

/// The controls hide after this long without the mouse moving (never while paused or a menu is open).
const HIDE_AFTER: Duration = Duration::from_secs(3);
/// The Next Episode card shows this many seconds before the end.
const NEXT_CARD: f64 = 20.;
/// Seconds of credits before skipping automatically moves on to the next episode.
const CREDITS_WAIT: f64 = 10.;
const SPEEDS: [f64; 6] = [0.5, 0.75, 1., 1.25, 1.5, 2.];
/// The volume goes up to 150% (set_volume).
pub const MAX_VOLUME: f64 = 150.;
/// Two presses this close are a double click (desktop: fullscreen) or double tap (phones: skip 10 s).
const DOUBLE_TAP: Duration = Duration::from_millis(300);

/// Sets the volume, 0-150%. mpv's own volume covers 0-100; above that a gain filter boosts it with a
/// limiter right after, so loud scenes don't crackle (mpv applies its volume after the filters, so
/// a limiter alone wouldn't see the boost).
pub fn set_volume(ctx: &Ctx, mpv: &crate::mpv::Mpv, v: f64) {
    let v = v.clamp(0., MAX_VOLUME);
    let _ = mpv.set_property("volume", v.min(100.));
    let gain = (v / 100.).max(1.);
    let mut boost = ctx.volume_boost;
    if (gain - *boost.peek()).abs() > 0.001 {
        let af = if gain > 1. { format!("lavfi=[volume={gain:.2},alimiter=limit=0.95]") } else { String::new() };
        let _ = mpv.set_property("af", af);
        boost.set(gain);
    }
}

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
pub enum Menu {
    Tracks,
    Speed,
}

/// Full-window video and controls while something plays.
/// The picture alone: the only part of the player that redraws with every video frame, so the
/// controls re-render only when the player's state changes (a few times a second).
struct VideoView {
    renderer: Rc<VideoRenderer>,
    /// With the press's x in the window, when a pointer or finger made it.
    on_press: EventHandler<Option<f64>>,
}

impl PartialEq for VideoView {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.renderer, &other.renderer) && self.on_press == other.on_press
    }
}

impl Component for VideoView {
    fn render(&self) -> impl IntoElement {
        let frame = *use_consume::<Ctx>().video_frame.read();
        let on_press = self.on_press.clone();
        video(self.renderer.clone(), frame).width(Size::fill()).height(Size::fill()).on_press(move |e: Event<PressEventData>| {
            on_press.call(match &*e {
                PressEventData::Mouse(m) => Some(m.global_location.x),
                PressEventData::Touch(t) => Some(t.global_location.x),
                _ => None,
            })
        })
    }
}

#[derive(PartialEq)]
pub struct Player;

impl Component for Player {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        // Its focusables are the player's controls (focus.rs), the only ones arrows reach meanwhile.
        use_provide_context(|| crate::focus::InPlayer);
        // Whatever the page had focused (the Play that started this) would take Enter meanwhile;
        // it gets focus back after.
        let (page_focus, window) = use_hook(|| {
            let window = Platform::get().current_window();
            let focused = *window.focused_accessibility_id.peek();
            focused.request_unfocus();
            (focused, window.id)
        });
        // Closing the window drops this after the window is gone, and asking a gone window panics.
        use_drop(move || {
            if Platform::get().try_window(window).is_some() {
                page_focus.request_focus();
            }
        });
        let active = ctx.player_active;
        let mut menu = ctx.player_menu;
        let mut visible = use_state(|| true);
        let mut last_click = use_state(|| None::<Instant>);
        // Phones: the lock button hides every control but Unlock, so a hand on the screen does nothing.
        let mut locked = use_state(|| false);
        // The mouse pointer goes with the controls and comes back with them (and when the player closes).
        #[cfg(not(any(target_os = "emscripten", target_os = "android")))]
        {
            use_side_effect(move || {
                let show = *visible.read();
                Platform::get().with_window(Platform::window_id(), move |w| w.set_cursor_visible(show));
            });
            use_drop(move || {
                if Platform::get().try_window(window).is_some() {
                    Platform::get().with_window(window.into(), |w| w.set_cursor_visible(true));
                }
            });
        }
        // Checks a few times a second whether the controls should fade out.
        use_hook({
            let status = ctx.player_status;
            move || {
                spawn(async move {
                    loop {
                        crate::net::sleep(Duration::from_millis(250)).await;
                        let keep = status.peek().paused || menu.peek().is_some() || active.peek().elapsed() < HIDE_AFTER;
                        if keep != *visible.peek() {
                            visible.set(keep);
                        }
                    }
                })
            }
        });
        // Settings > Skip intros automatically: each intro is skipped once as it starts; seeking back
        // into one plays it.
        let auto = use_hook(|| ctx.store.get::<bool>(&ctx.profile, "settings", "auto_skip_intro").ok().flatten().unwrap_or(false));
        let offer_smaller = use_hook(|| ctx.store.get::<bool>(&ctx.profile, "settings", "offer_smaller").ok().flatten().unwrap_or(true));
        use_side_effect({
            let ctx = ctx.clone();
            let mut skipped = use_state(Vec::<f64>::new);
            let mut nexted = use_state(|| false);
            let mut shared = use_state(|| false);
            move || {
                let s = ctx.player_status.read();
                let intros = ctx.now_playing.peek().as_ref().map(|np| intros_of(&s, np)).unwrap_or_default();
                // A file with intro chapters teaches every other copy of the episode: shared once.
                let already = *shared.peek();
                if let (Some(&(start, end)), false) = (s.intros.first(), already) {
                    shared.set(true);
                    share_intro(&ctx, start, end);
                }
                // And the next episode CREDITS_WAIT seconds into the credits, as the card counts down.
                if auto && !*nexted.peek() && s.credits.is_some_and(|c| s.position >= c + CREDITS_WAIT) && ctx.now_playing.peek().as_ref().is_some_and(|np| np.has_next) {
                    nexted.set(true);
                    if let Some(p) = ctx.player.peek().as_ref() {
                        let _ = p.send(ToPlayer::Next);
                    }
                }
                let Some((start, end)) = intros.iter().copied().find(|(a, b)| s.position >= *a && s.position < b - 1.) else { return };
                if !auto || s.seeking || skipped.peek().contains(&start) {
                    return;
                }
                skipped.write().push(start);
                if let Some(np) = ctx.now_playing.peek().as_ref() {
                    crate::social::seek_to(&ctx, np.video.mpv(), end);
                }
            }
        });
        let shown = use_animation_transition(*visible.read(), |from, to| {
            AnimNum::new(if from { 1. } else { 0. }, if to { 1. } else { 0. }).time(240).ease(Ease::Out).function(Function::Cubic)
        });
        let o = shown.get().value();

        let Some(np) = ctx.now_playing.read().clone() else { return rect().expanded().background((0, 0, 0)).into_element() };
        let status = ctx.player_status.read().clone();
        // Phones: touch-sized controls kept clear of the system bars; the video itself fills the screen.
        let phone = crate::theme::phone();
        let safe = *ctx.safe.read();
        let r = np.video.clone();
        let (party, chat_open) = (*ctx.social.party_player.read(), *ctx.social.chat_open.read());
        let wake = move || {
            let (mut active, mut visible) = (active, visible);
            active.set(Instant::now());
            visible.set(true);
        };

        let with_panel = party && chat_open;
        let is_locked = *locked.read();
        let player = rect()
            .width(if with_panel { Size::flex(1.) } else { Size::fill() })
            .height(Size::fill())
            .background((0, 0, 0))
            // A touch also moves the pointer, which would show the controls a tap is meant to hide.
            .maybe(!phone, |el| el.on_global_pointer_move(move |_| wake()))
            .child(VideoView {
                renderer: r.clone(),
                // A click pauses or plays, a double click toggles fullscreen (and undoes the pause).
                // On phones a tap shows or hides the controls instead, as phone players do.
                on_press: EventHandler::new({
                    let (r, ctx) = (r.clone(), ctx.clone());
                    move |x: Option<f64>| {
                        if phone {
                            menu.set(None);
                            // A second tap soon after the first skips 10 seconds, back on the left half
                            // and ahead on the right; more taps keep skipping. Not while locked.
                            let again = last_click.peek().is_some_and(|t| t.elapsed() < DOUBLE_TAP);
                            last_click.set(Some(Instant::now()));
                            if let (true, false, Some(x)) = (again, *locked.peek(), x) {
                                let width = Platform::get().current_window().root_size.peek().width as f64;
                                crate::social::seek_by(&ctx, r.mpv(), if x < width / 2. { -10. } else { 10. });
                                wake();
                                return;
                            }
                            let mut active = active;
                            if *visible.peek() {
                                active.set(Instant::now() - HIDE_AFTER);
                                visible.set(false);
                            } else {
                                wake();
                            }
                            return;
                        }
                        let paused = ctx.player_status.peek().paused;
                        crate::social::set_paused(&ctx, r.mpv(), !paused);
                        menu.set(None);
                        if last_click.peek().is_some_and(|t| t.elapsed() < DOUBLE_TAP) {
                            ctx.toggle_fullscreen();
                            last_click.set(None);
                        } else {
                            last_click.set(Some(Instant::now()));
                        }
                    }
                }),
            })
            .maybe(!status.loaded, |el| el.child(Loading { logo: np.logo.clone(), background: np.background.clone(), title: np.title.clone() }.into_element()))
            // Stalled on the network or seeking past what's downloaded: never just a frozen frame.
            .maybe(status.loaded && (status.buffering || status.seeking), |el| el.child(layer_center().layer(1).child(spinner())))
            // Phones: while the controls show, Pause in the middle too, as phone players have it (a tap
            // on the video shows and hides the controls; it doesn't pause).
            .maybe(phone && o > 0.01 && status.loaded && !status.paused && !status.buffering && !status.seeking && !is_locked, |el| {
                el.child(layer_center().layer(1).opacity(o).child(icon_button(ICON_PAUSE, 64., true, None, {
                    let (r, ctx) = (r.clone(), ctx.clone());
                    move |()| crate::social::set_paused(&ctx, r.mpv(), true)
                })))
            })
            .maybe(status.loaded && status.paused && !is_locked, |el| {
                el.child(layer_center().layer(1).child(icon_button(ICON_PLAY, 64., true, None, {
                    let (r, ctx) = (r.clone(), ctx.clone());
                    move |()| crate::social::set_paused(&ctx, r.mpv(), false)
                })))
            })
            .maybe(is_locked && o > 0.01, |el| {
                el.child(layer_center().layer(1).opacity(o).child(crate::focus::Focusable::new(
                    rect().padding(Gaps::new(12., 24., 12., 24.)).corner_radius(999.).background((255, 255, 255)).child(label().text("Unlock").font_size(16.).font_weight(FontWeight::BOLD).color((0, 0, 0))),
                    move |()| locked.set(false),
                )))
            })
            .maybe(o > 0.01 && !is_locked, |el| {
                // Own layers above the video: Freya doesn't order siblings on one layer.
                el.child(top_bar(&ctx, &np.title, &np.episode, phone, &safe).opacity(o).layer(1))
                    .child(bottom_bar(&ctx, &r, &status, np.has_next, menu, phone, &safe).opacity(o).layer(1))
                    .maybe_child((*menu.read()).map(|m| menu_panel(&ctx, m, &r, &status, menu, phone, &safe).opacity(o).layer(2)))
            })
            // Watch Together plays one stream for everyone, so it never offers another.
            // On phones, top left under the back arrow: no touch does anything until Unlock.
            .maybe(phone && o > 0.01 && !is_locked, |el| {
                el.child(
                    rect().position(Position::new_absolute().top(60. + safe.top()).left(16. + safe.left())).opacity(o).layer(2).child(crate::focus::Focusable::new(
                        rect().padding(Gaps::new(8., 16., 8., 16.)).corner_radius(999.).background((0, 0, 0, 0.6)).child(label().text("Lock").font_size(15.).font_weight(FontWeight::BOLD)),
                        move |()| {
                            menu.set(None);
                            locked.set(true);
                        },
                    )),
                )
            })
            .maybe(status.slow && offer_smaller && !party && !is_locked && (status.buffering || o > 0.01), |el| el.child(smaller_card(&ctx, phone, &safe)))
            .maybe_child(current_intro(&status, &np).filter(|_| !is_locked).map(|end| skip_intro(&ctx, &r, end, phone, &safe)))
            .maybe_child(next_in(&status, auto).filter(|_| np.has_next && !is_locked).map(|left| next_card(&ctx, left, phone, &safe)))
            // Watch Together: a button that brings the chat panel back.
            .maybe(party && !chat_open && o > 0.01 && !is_locked, |el| {
                let mut open = ctx.social.chat_open;
                el.child(
                    rect()
                        .position(Position::new_absolute().top(if phone { 10. + safe.top() } else { 28. }).right(if phone { 16. + safe.right() } else { 40. }))
                        .opacity(o)
                        .layer(2)
                        .child(crate::focus::Focusable::new(
                            rect().padding(Gaps::new(8., 16., 8., 16.)).corner_radius(999.).background((0, 0, 0, 0.6)).child(label().text("Chat").font_size(15.).font_weight(FontWeight::BOLD)),
                            move |()| open.set(true),
                        )),
                )
            });
        // Watch Together's panel sits beside the video, so it never covers the controls.
        if with_panel {
            rect().expanded().horizontal().content(Content::Flex).background((0, 0, 0)).child(player).child(crate::together::ChatPanel).into_element()
        } else {
            player.into_element()
        }
    }
}

/// While the stream opens: the title's backdrop with its logo breathing in the middle, like Stremio.
#[derive(PartialEq)]
struct Loading {
    logo: Option<String>,
    background: Option<String>,
    title: String,
}

impl Component for Loading {
    fn render(&self) -> impl IntoElement {
        let pulse = use_animation(|conf| {
            conf.on_creation(OnCreation::Run);
            conf.on_finish(OnFinish::Reverse { delay: Duration::ZERO });
            AnimNum::new(0.35, 1.).time(1100).ease(Ease::InOut).function(Function::Sine)
        });
        rect()
            .position(Position::new_absolute().top(0.).left(0.))
            .width(Size::percent(100.))
            .height(Size::percent(100.))
            .background((0, 0, 0))
            .layer(1)
            .child(backdrop(self.background.as_deref()).opacity(0.35))
            .child(
                layer_center()
                    .spacing(36.)
                    .child(rect().opacity(pulse.get().value()).child(title_art(self.logo.as_deref(), &self.title)))
                    .child(spinner()),
            )
    }
}

/// The player's loader: a large ring on a soft dark disc, readable over any picture.
fn spinner() -> Rect {
    rect()
        .width(Size::px(88.))
        .height(Size::px(88.))
        .corner_radius(44.)
        .background((0, 0, 0, 0.45))
        .center()
        .child(crate::motion::Spinner { size: 56. })
}

fn layer_center() -> Rect {
    rect().position(Position::new_absolute().top(0.).left(0.)).width(Size::percent(100.)).height(Size::percent(100.)).center()
}

fn top_bar(ctx: &Ctx, title: &str, episode: &str, phone: bool, safe: &Gaps) -> Rect {
    let ctx = ctx.clone();
    rect()
        .position(Position::new_absolute().top(0.).left(0.))
        .width(Size::percent(100.))
        // A fixed height: Freya draws gradients on sized rects.
        .height(Size::px(140.))
        .padding(if phone { Gaps::new(10. + safe.top(), 16. + safe.right(), 0., 8. + safe.left()) } else { Gaps::new(24., 40., 0., 32.) })
        .background(LinearGradient::new().angle(0.).stop(((0, 0, 0, 0.8), 0.)).stop(((0, 0, 0, 0.), 100.)))
        // Its own row, centred: the bar is tall for the fade, and the title sits level with the arrow.
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::Flex)
                .cross_align(Alignment::Center)
                .spacing(if phone { 8. } else { 16. })
                .child(icon_button(ICON_BACK, 30., false, None, move |()| {
                    if let Some(p) = ctx.player.peek().as_ref() {
                        let _ = p.send(ToPlayer::Stop);
                    }
                }))
                .child(
                    rect()
                        .spacing(2.)
                        .width(Size::flex(1.))
                        .child(label().text(title.to_owned()).font_size(if phone { 17. } else { 22. }).font_weight(FontWeight::BOLD).max_lines(1).text_overflow(TextOverflow::Ellipsis))
                        .maybe(!episode.is_empty(), |el| el.child(label().text(episode.to_owned()).font_size(15.).color((200, 200, 200)))),
                ),
        )
}

fn bottom_bar(ctx: &Ctx, r: &Rc<VideoRenderer>, s: &Status, has_next: bool, menu: State<Option<Menu>>, phone: bool, safe: &Gaps) -> Rect {
    let pct = if s.duration > 0. { s.position / s.duration * 100. } else { 0. };
    let (seek, seek_ctx) = (r.clone(), ctx.clone());
    let timeline = rect()
        .width(Size::fill())
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(16.)
        .child(
            rect().width(Size::flex(1.)).child(
                Slider::new(move |v: f64| {
                    let duration = seek_ctx.player_status.peek().duration;
                    crate::social::seek_to(&seek_ctx, seek.mpv(), v / 100. * duration);
                })
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
    let (toggle_ctx, back_ctx, fwd_ctx) = (ctx.clone(), ctx.clone(), ctx.clone());
    let left = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(10.)
        .child(IconButton {
            primary: true,
            ..icon_button(
                if paused { ICON_PLAY } else { ICON_PAUSE },
                30.,
                true,
                None,
                move |()| crate::social::set_paused(&toggle_ctx, toggle.mpv(), !paused),
            )
        })
        .child(icon_button(
            ICON_BACK10,
            30.,
            false,
            Some("10"),
            move |()| crate::social::seek_by(&back_ctx, back.mpv(), -10.),
        ))
        .child(icon_button(ICON_FWD10, 30., false, Some("10"), move |()| {
            crate::social::seek_by(&fwd_ctx, fwd.mpv(), 10.)
        }))
        .maybe(!phone, |el| el.child(icon_button(volume_icon, 28., false, None, move |()| drop(mute.mpv().set_property("mute", !muted)))))
        .maybe(!phone, |el| el.child(
            // Freya's slider runs 0-100; it maps onto 0-150% volume.
            rect().width(Size::px(160.)).child(
                Slider::new({
                    let ctx = ctx.clone();
                    move |v: f64| {
                        set_volume(&ctx, vol.mpv(), v / 100. * MAX_VOLUME);
                        let _ = vol.mpv().set_property("mute", false);
                    }
                })
                .value(if muted { 0. } else { (s.volume * *ctx.volume_boost.read()).min(MAX_VOLUME) / MAX_VOLUME * 100. })
                .size(Size::fill()),
            ),
        ));
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
        .maybe(!phone, |el| {
            el.child(icon_button(
                if *ctx.fullscreen.read() { ICON_EXIT_FULLSCREEN } else { ICON_FULLSCREEN },
                28.,
                false,
                None,
                move |()| fs_ctx.toggle_fullscreen(),
            ))
        });

    rect()
        .position(Position::new_absolute().bottom(0.).left(0.))
        .width(Size::percent(100.))
        .height(Size::px(220.))
        .main_align(Alignment::End)
        .spacing(10.)
        .padding(if phone { Gaps::new(0., 12. + safe.right(), 10. + safe.bottom(), 12. + safe.left()) } else { Gaps::new(0., 40., 24., 40.) })
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
fn menu_panel(ctx: &Ctx, m: Menu, r: &Rc<VideoRenderer>, s: &Status, menu: State<Option<Menu>>, phone: bool, safe: &Gaps) -> Rect {
    // A fixed height (Freya paints backgrounds on sized boxes); long lists scroll inside it.
    let rows = match m {
        // The subtitle style column has 3 rows.
        Menu::Tracks => (s.subs.len() + 1).max(s.audio.len()).max(3),
        Menu::Speed => SPEEDS.len(),
    };
    let list_h = (rows as f32 * 38.).min(380.);
    let columns = match m {
        Menu::Tracks if s.audio.len() > 1 => 3.,
        Menu::Tracks => 2.,
        Menu::Speed => 1.,
    };
    // Phones: a sheet across the bottom with the columns one under another, scrolling together.
    let panel = if phone {
        rect()
            .position(Position::new_absolute().bottom(0.).left(0.))
            .width(Size::percent(100.))
            .height(Size::percent(65.))
            .padding(Gaps::new(16., 16. + safe.right(), 8. + safe.bottom(), 16. + safe.left()))
    } else {
        rect()
            .position(Position::new_absolute().bottom(110.).right(40.))
            .width(Size::px(columns * 300. + (columns - 1.) * 24. + 32.))
            .height(Size::px(list_h + 64.))
            .padding(Gaps::new_all(16.))
            .horizontal()
            .spacing(24.)
    }
    .corner_radius(12.)
    .background((20, 20, 20, 0.96))
    .border(Border::new().fill((255, 255, 255, 0.08)).width(1.));
    let column = |title: &'static str, items: Vec<Element>| {
        let head = label().text(title).font_size(13.).color(theme().muted);
        if phone {
            rect().width(Size::fill()).spacing(8.).child(head).children(items)
        } else {
            rect().width(Size::px(300.)).spacing(8.).child(head).child(ScrollView::new().height(Size::px(list_h)).children(items))
        }
    };
    let panel_with = move |panel: Rect, columns: Vec<Rect>| -> Rect {
        if phone {
            panel.child(ScrollView::new().spacing(20.).children(columns.into_iter().map(Rect::into_element)))
        } else {
            panel.children(columns.into_iter().map(Rect::into_element))
        }
    };
    let option = |text: String, selected: bool, on: Box<dyn Fn()>| {
        MenuOption { text, selected, on_press: EventHandler::new(move |()| on()) }.into_element()
    };
    match m {
        Menu::Tracks => {
            // Picking a track also remembers its language for the next stream ("Off" remembers no subtitles).
            let pick = |prop: &'static str, track: Option<&Track>| {
                let (r, ctx) = (r.clone(), ctx.clone());
                let (id, lang) = (track.map(|t| t.id), track.and_then(|t| t.lang.clone()));
                Box::new(move || {
                    let _ = match id {
                        Some(id) => r.mpv().set_property(prop, id),
                        None => r.mpv().set_property(prop, "no"),
                    };
                    let mut prefs = ctx.player_prefs();
                    match (prop, id) {
                        ("sid", None) => prefs.sub_lang = Some("off".into()),
                        ("sid", Some(_)) if lang.is_some() => prefs.sub_lang = lang.clone(),
                        ("aid", Some(_)) if lang.is_some() => prefs.audio_lang = lang.clone(),
                        _ => {}
                    }
                    ctx.save_player_prefs(&prefs);
                    let mut menu = menu;
                    menu.set(None);
                }) as Box<dyn Fn()>
            };
            let subs = std::iter::once(option("Off".into(), s.sid.is_none(), pick("sid", None)))
                .chain(s.subs.iter().map(|t| option(t.title.clone(), s.sid == Some(t.id), pick("sid", Some(t)))))
                .collect();
            let audio = s.audio.iter().map(|t| option(t.title.clone(), s.aid == Some(t.id), pick("aid", Some(t)))).collect();
            let mut columns = vec![column("Subtitles", subs)];
            if s.audio.len() > 1 {
                columns.push(column("Audio", audio));
            }
            columns.push(column("Subtitle style", subtitle_style(ctx, r)));
            panel_with(panel, columns)
        }
        Menu::Speed => panel_with(panel, vec![column("Speed", SPEEDS.iter().map(|&v| {
            let r = r.clone();
            option(if v == 1. { "Normal".into() } else { format!("{v}x") }, s.speed == v, Box::new(move || {
                let _ = r.mpv().set_property("speed", v);
                let mut menu = menu;
                menu.set(None);
            }))
        }).collect())]),
    }
}

/// Size and background (remembered) and delay (this stream only), changed live.
fn subtitle_style(ctx: &Ctx, r: &Rc<VideoRenderer>) -> Vec<Element> {
    let mpv = r.mpv();
    let prefs = ctx.player_prefs();
    let delay = mpv.get_property::<f64>("sub-delay").unwrap_or(0.);
    let scale = |step: f64| {
        let (ctx, r) = (ctx.clone(), r.clone());
        move || {
            let mut prefs = ctx.player_prefs();
            prefs.sub_scale = ((prefs.sub_scale + step) * 10.).round().clamp(5., 25.) / 10.;
            let _ = r.mpv().set_property("sub-scale", prefs.sub_scale);
            ctx.save_player_prefs(&prefs);
        }
    };
    let shift = |step: f64| {
        let (ctx, r) = (ctx.clone(), r.clone());
        move || shift_subs(&ctx, r.mpv(), step)
    };
    let background = {
        let (ctx, r) = (ctx.clone(), r.clone());
        move || {
            let mut prefs = ctx.player_prefs();
            prefs.sub_background = !prefs.sub_background;
            let _ = r.mpv().set_property("sub-border-style", prefs.border_style());
            ctx.save_player_prefs(&prefs);
        }
    };
    vec![
        stepper("Size", format!("{:.0}%", prefs.sub_scale * 100.), scale(-0.1), scale(0.1)),
        stepper("Delay (Z / X)", format!("{delay:+.1}s"), shift(-0.1), shift(0.1)),
        MenuOption {
            text: "Background box".into(),
            selected: prefs.sub_background,
            on_press: EventHandler::new(move |()| background()),
        }
        .into_element(),
    ]
}

/// Moves the subtitles `step` seconds later (negative: earlier), remembered for this stream.
fn shift_subs(ctx: &Ctx, mpv: &crate::mpv::Mpv, step: f64) {
    let _ = mpv.command("add", &["sub-delay", &step.to_string()]);
    let delay = (mpv.get_property::<f64>("sub-delay").unwrap_or(0.) * 10.).round() / 10.;
    let Some(stream) = ctx.now_playing.peek().as_ref().map(|np| np.stream.clone()).filter(|s| !s.is_empty()) else { return };
    let value = (delay != 0.).then(|| delay.to_string());
    if let Err(e) = ctx.store.set_meta(&sub_delay_key(&stream), value.as_deref()) {
        crate::log!("saving subtitle delay: {e}");
    }
}

/// Where a stream's subtitle delay is kept (the store's meta table, this device only).
pub fn sub_delay_key(stream: &str) -> String {
    format!("sub_delay {stream}")
}

/// "Size  − 100% +": a value with buttons either side.
fn stepper(name: &str, value: String, minus: impl Fn() + 'static, plus: impl Fn() + 'static) -> Element {
    let button = |glyph: &'static str, f: Box<dyn Fn()>| {
        let face = rect()
            .width(Size::px(30.))
            .height(Size::px(30.))
            .corner_radius(15.)
            .center()
            .background((255, 255, 255, 0.10))
            .child(label().text(glyph).font_size(17.));
        crate::focus::Focusable {
            radius: 15.,
            grow: 1.,
            ..crate::focus::Focusable::new(face, move |()| f())
        }
    };
    rect()
        .width(Size::fill())
        .horizontal()
        .content(Content::Flex)
        .cross_align(Alignment::Center)
        .padding(Gaps::new(4., 10., 4., 38.))
        .child(label().width(Size::flex(1.)).text(name.to_owned()).font_size(15.))
        .child(button("−", Box::new(minus)))
        .child(label().width(Size::px(64.)).text_align(TextAlign::Center).text(value).font_size(14.))
        .child(button("+", Box::new(plus)))
        .into_element()
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
        let row = rect()
            .width(Size::fill())
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(10.)
            .padding(Gaps::new(8., 10., 8., 10.))
            .corner_radius(8.)
            .background(if *hovered.read() { (255, 255, 255, 0.10) } else { (255, 255, 255, 0.) })
            .on_pointer_enter(move |_| hovered.set(true))
            .on_pointer_leave(move |_| hovered.set(false))
            .child(
                rect()
                    .width(Size::px(18.))
                    .maybe_child(self.selected.then(|| svg(ICON_CHECK, 18.))),
            )
            .child(
                label()
                    .text(self.text.clone())
                    .font_size(15.)
                    .max_lines(1)
                    .text_overflow(TextOverflow::Ellipsis),
            );
        crate::focus::Focusable {
            radius: 8.,
            grow: 1.,
            width: Some(Size::fill()),
            ..crate::focus::Focusable::new(row, self.on_press.clone())
        }
    }
}

/// Seconds until the next episode starts, while the Next Episode card should show: from the credits
/// (counting down CREDITS_WAIT when skipping automatically) or the last NEXT_CARD seconds.
fn next_in(s: &Status, auto: bool) -> Option<f64> {
    let to_end = s.duration - s.position;
    match s.credits.filter(|c| s.position >= *c) {
        Some(c) if auto => Some(c + CREDITS_WAIT - s.position),
        Some(_) => Some(to_end),
        None => (s.duration > 0. && to_end < NEXT_CARD).then_some(to_end),
    }
}

/// The file's intro chapters, or else the times other viewers shared for the episode.
fn intros_of(s: &Status, np: &crate::NowPlaying) -> Vec<(f64, f64)> {
    if s.intros.is_empty() { np.shared_intro.get().into_iter().collect() } else { s.intros.clone() }
}

/// The end of the intro playing now, if one is (not in its last second, where skipping is moot).
fn current_intro(s: &Status, np: &crate::NowPlaying) -> Option<f64> {
    intros_of(s, np).into_iter().find(|(start, end)| s.position >= *start && s.position < end - 1.).map(|(_, end)| end)
}

/// Sends an intro found in the file's chapters to the shared intro times (signed in only).
fn share_intro(ctx: &Ctx, start: f64, end: f64) {
    let intro = relix_proto::intro::Intro { start, end };
    let (Some(np), Some(acc)) = (ctx.now_playing.peek().clone(), crate::sync::account(&ctx.store)) else { return };
    if np.video_id.is_empty() || !intro.valid() {
        return;
    }
    let (http, url) = (ctx.http.clone(), crate::sync::server_url(&format!("/intros/{}", np.video_id)));
    spawn(async move {
        if let Err(e) = crate::sync::send(&http, "PUT", url, Some(&acc.token), Some(&intro)).await {
            crate::log!("sharing intro: {e}");
        }
    });
}

/// "Skip Intro", bottom right while an intro plays (also the S key).
fn skip_intro(ctx: &Ctx, r: &Rc<VideoRenderer>, end: f64, phone: bool, safe: &Gaps) -> Rect {
    let (ctx, r) = (ctx.clone(), r.clone());
    let (bottom, right) = if phone { (100. + safe.bottom(), 16. + safe.right()) } else { (140., 40.) };
    // FadeIn::image hugs the button; ::page would stretch it over the box and lose the anchor.
    rect().position(Position::new_absolute().bottom(bottom).right(right)).layer(2).child(FadeIn::image(crate::focus::Focusable {
        radius: 10.,
        ..crate::focus::Focusable::new(
            rect()
                .padding(Gaps::new(14., 22., 14., 22.))
                .corner_radius(10.)
                .background((255, 255, 255))
                .child(label().text("Skip Intro").font_size(17.).font_weight(FontWeight::BOLD).color((0, 0, 0))),
            move |()| crate::social::seek_to(&ctx, r.mpv(), end),
        )
    }))
}

/// After repeated stalls: "Switch to a smaller version", top right under the title.
fn smaller_card(ctx: &Ctx, phone: bool, safe: &Gaps) -> Rect {
    let ctx = ctx.clone();
    let (top, right) = if phone { (80. + safe.top(), 16. + safe.right()) } else { (100., 40.) };
    rect().position(Position::new_absolute().top(top).right(right)).layer(2).child(FadeIn::image(crate::focus::Focusable {
        radius: 10.,
        ..crate::focus::Focusable::new(
            rect()
                .spacing(2.)
                .padding(Gaps::new(12., 18., 12., 18.))
                .corner_radius(10.)
                .background((255, 255, 255))
                .child(label().text("Switch to a smaller version").font_size(16.).font_weight(FontWeight::BOLD).color((0, 0, 0)))
                .child(label().text("This stream is faster than your connection").font_size(13.).color((70, 70, 70))),
            move |()| {
                if let Some(p) = ctx.player.peek().as_ref() {
                    let _ = p.send(ToPlayer::Smaller);
                }
            },
        )
    }))
}

fn next_card(ctx: &Ctx, left: f64, phone: bool, safe: &Gaps) -> Rect {
    let ctx = ctx.clone();
    let (bottom, right) = if phone {
        (100. + safe.bottom(), 16. + safe.right())
    } else {
        (140., 40.)
    };
    let next = move |()| {
        if let Some(p) = ctx.player.peek().as_ref() {
            let _ = p.send(ToPlayer::Next);
        }
    };
    rect()
        .position(Position::new_absolute().bottom(bottom).right(right))
        .layer(2)
        .child(FadeIn::image(crate::focus::Focusable {
            radius: 10.,
            ..crate::focus::Focusable::new(
                rect()
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(12.)
                    .padding(Gaps::new(14., 22., 14., 18.))
                    .corner_radius(10.)
                    .background((255, 255, 255))
                    .color((0, 0, 0))
                    .child(
                        SvgViewer::new(ICON_PLAY)
                            .width(Size::px(26.))
                            .height(Size::px(26.))
                            .fill((0, 0, 0))
                            .stroke((0, 0, 0)),
                    )
                    .child(
                        rect()
                            .child(
                                label()
                                    .text("Next Episode")
                                    .font_size(17.)
                                    .font_weight(FontWeight::BOLD),
                            )
                            .child(
                                label()
                                    .text(format!("Starts in {}s", left.ceil().max(0.)))
                                    .font_size(13.)
                                    .color((90, 90, 90)),
                            ),
                    ),
                next,
            )
        }))
}

fn svg(icon: &'static [u8], size: f32) -> SvgViewer {
    SvgViewer::new(icon).width(Size::px(size)).height(Size::px(size)).stroke((255, 255, 255))
}

fn icon_button(
    icon: &'static [u8],
    size: f32,
    filled: bool,
    badge: Option<&'static str>,
    on_press: impl Fn(()) + 'static,
) -> IconButton {
    IconButton {
        icon,
        size,
        filled,
        badge,
        on_press: EventHandler::new(on_press),
        primary: false,
    }
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
    /// The control a remote lands on first (focus.rs).
    primary: bool,
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
        let icon = if self.filled {
            icon.fill((255, 255, 255))
        } else {
            icon
        };
        let face = rect()
            .padding(Gaps::new_all(8.))
            .corner_radius(999.)
            .background((255, 255, 255, 0.14 * h))
            .scale((1. + 0.1 * h, 1. + 0.1 * h))
            .on_pointer_enter(move |_| hovered.set(true))
            .on_pointer_leave(move |_| hovered.set(false))
            .child(
                rect()
                    .width(Size::px(self.size))
                    .height(Size::px(self.size))
                    .center()
                    .child(
                        rect()
                            .position(Position::new_absolute().top(0.).left(0.))
                            .child(icon),
                    )
                    .maybe_child(self.badge.map(|b| {
                        label()
                            .text(b)
                            .font_size(self.size * 0.3)
                            .font_weight(FontWeight::BOLD)
                    })),
            );
        // Reachable by a remote's arrows (focus.rs); the ring shows which one Enter presses.
        crate::focus::Focusable {
            radius: 999.,
            grow: 1.,
            primary: self.primary,
            ..crate::focus::Focusable::new(face, on_press)
        }
    }
}

/// "1:02:03" or "4:05".
fn clock(secs: f64) -> String {
    let s = secs.max(0.) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Keys while video plays: Space or Enter pauses, arrows seek and change volume, F fullscreen,
/// M mute, N next episode, and a remote's media keys. Returns false for keys it doesn't use.
///
/// A TV remote has only arrows, OK and Back: there any key shows the controls, Up or Down moves
/// onto them, arrows then move between them and OK presses one, and Back leaves them.
pub fn key(ctx: &Ctx, registry: &crate::focus::Registry, key: &Key) -> bool {
    let Some(np) = ctx.now_playing.peek().clone() else {
        return false;
    };
    let (mpv, s) = (np.video.mpv(), ctx.player_status.peek().clone());
    let remote = cfg!(target_os = "android");
    let on_controls = registry.player_focused();
    if remote {
        let mut active = ctx.player_active;
        active.set(Instant::now());
        if on_controls {
            let dir = match key {
                Key::Named(NamedKey::ArrowUp) => Some(crate::focus::Dir::Up),
                Key::Named(NamedKey::ArrowDown) => Some(crate::focus::Dir::Down),
                Key::Named(NamedKey::ArrowLeft) => Some(crate::focus::Dir::Left),
                Key::Named(NamedKey::ArrowRight) => Some(crate::focus::Dir::Right),
                Key::Named(NamedKey::Escape) => {
                    let mut menu = ctx.player_menu;
                    menu.set(None);
                    Platform::get()
                        .current_window()
                        .focused_accessibility_id
                        .peek()
                        .request_unfocus();
                    active.set(Instant::now() - HIDE_AFTER);
                    return true;
                }
                // OK presses the control (below); media keys work as ever.
                _ => None,
            };
            if let Some(dir) = dir {
                registry.navigate(dir, true);
                return true;
            }
        }
    }
    match key {
        Key::Character(c) if c == " " => crate::social::set_paused(ctx, mpv, !s.paused),
        // Enter on a focused control presses it (Freya) instead.
        Key::Named(NamedKey::Enter | NamedKey::MediaPlayPause)
            if !on_controls || *key != Key::Named(NamedKey::Enter) =>
        {
            crate::social::set_paused(ctx, mpv, !s.paused)
        }
        Key::Named(NamedKey::MediaPlay) => crate::social::set_paused(ctx, mpv, false),
        Key::Named(NamedKey::MediaPause) => crate::social::set_paused(ctx, mpv, true),
        Key::Named(NamedKey::ArrowLeft | NamedKey::MediaRewind) => {
            crate::social::seek_by(ctx, mpv, -10.)
        }
        Key::Named(NamedKey::ArrowRight | NamedKey::MediaFastForward) => {
            crate::social::seek_by(ctx, mpv, 10.)
        }
        Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) if remote => registry.focus_player(),
        // Read the boost into a value first: a borrow taken inside the call's arguments lives until the
        // statement ends, and set_volume writes the same State (that panicked above 100%).
        Key::Named(NamedKey::ArrowUp) | Key::Named(NamedKey::ArrowDown) => {
            let current = s.volume * *ctx.volume_boost.peek();
            let step = if matches!(key, Key::Named(NamedKey::ArrowUp)) { 5. } else { -5. };
            set_volume(ctx, mpv, current + step)
        }
        Key::Character(c) if c.eq_ignore_ascii_case("f") => ctx.toggle_fullscreen(),
        Key::Character(c) if c.eq_ignore_ascii_case("m") => drop(mpv.set_property("mute", !s.muted)),
        // mpv's own keys for it: subtitles 0.1 s earlier (Z) or later (X).
        Key::Character(c) if c.eq_ignore_ascii_case("z") => shift_subs(ctx, mpv, -0.1),
        Key::Character(c) if c.eq_ignore_ascii_case("x") => shift_subs(ctx, mpv, 0.1),
        Key::Character(c) if c.eq_ignore_ascii_case("s") && current_intro(&s, &np).is_some() => {
            crate::social::seek_to(ctx, mpv, current_intro(&s, &np).unwrap_or_default())
        }
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
