use freya::prelude::*;

use crate::focus::{Focusable, use_page_scroller, use_scroller};
use crate::home::button;
use crate::motion::busy;
use crate::{Ctx, debug, update};
use crate::theme::{Look, theme};

/// App settings: the look (Netflix or Stremio, synced with the account) and updates.
#[derive(PartialEq)]
pub struct Settings;

impl Component for Settings {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let page = use_scroller();
        use_page_scroller(page);
        let current = *ctx.look.read();
        let phone = crate::theme::phone();
        let mut checking = use_state(|| false);
        let mut found = use_state(|| None::<String>);
        let mut debug_on = use_state(debug::enabled);
        let ready = ctx.update.read().clone();
        let check = {
            let ctx = ctx.clone();
            move |()| {
                if *checking.peek() {
                    return;
                }
                checking.set(true);
                let ctx = ctx.clone();
                spawn(async move {
                    found.set(Some(update::check_now(ctx).await));
                    checking.set(false);
                });
            }
        };
        let option = |look: Look, name: &'static str| {
            let (ctx, on) = (ctx.clone(), look == current);
            Focusable {
                radius: 6.,
                ..Focusable::new(
                    rect()
                        .padding(Gaps::new(8., 18., 8., 18.))
                        .corner_radius(6.)
                        .maybe(on, |el| el.background((255, 255, 255)))
                        .child(label().text(name).font_size(14.).font_weight(FontWeight::BOLD).color(if on { (0, 0, 0) } else { theme().muted })),
                    move |()| ctx.set_look(look),
                )
            }
            .into_element()
        };

        page.wrap(
            ScrollView::new_controlled(page.controller).scroll_with_arrows(false).drag_scrolling(false).child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(crate::theme::top(), crate::theme::side(), 48., crate::theme::side()))
                    .spacing(12.)
                    .child(label().text("Settings").font_size(32.).font_weight(FontWeight::BOLD))
                    .child(label().text("APPEARANCE").font_size(12.).font_weight(FontWeight::BOLD).color(theme().muted).margin(Gaps::new(16., 0., 0., 0.)))
                    .child(
                        rect()
                            .width(Size::fill())
                            .max_width(Size::px(720.))
                            .padding(Gaps::new_all(18.))
                            .maybe(!phone, |el| el.horizontal())
                            .content(Content::Flex)
                            .cross_align(if phone { Alignment::Start } else { Alignment::Center })
                            .spacing(12.)
                            .corner_radius(theme().radius)
                            .background(theme().surface)
                            .child(
                                rect()
                                    .width(if phone { Size::fill() } else { Size::flex(1.) })
                                    .spacing(4.)
                                    .child(label().text("Theme").font_size(16.).font_weight(FontWeight::BOLD))
                                    .child(label().text("Netflix: top bar and featured titles. Stremio: sidebar and titled posters.").font_size(13.).color(theme().muted)),
                            )
                            .child(
                                rect()
                                    .horizontal()
                                    .padding(Gaps::new_all(3.))
                                    .spacing(3.)
                                    .corner_radius(8.)
                                    .background(theme().raised)
                                    .child(option(Look::Netflix, "Netflix"))
                                    .child(option(Look::Stremio, "Stremio")),
                            ),
                    )
                    .child(label().text("UPDATES").font_size(12.).font_weight(FontWeight::BOLD).color(theme().muted).margin(Gaps::new(16., 0., 0., 0.)))
                    .child(
                        rect()
                            .width(Size::fill())
                            .max_width(Size::px(720.))
                            .padding(Gaps::new_all(18.))
                            .maybe(!phone, |el| el.horizontal())
                            .content(Content::Flex)
                            .cross_align(if phone { Alignment::Start } else { Alignment::Center })
                            .spacing(12.)
                            .corner_radius(theme().radius)
                            .background(theme().surface)
                            .child(
                                rect()
                                    .width(if phone { Size::fill() } else { Size::flex(1.) })
                                    .spacing(4.)
                                    .child(label().text(format!("Relix {}", env!("CARGO_PKG_VERSION"))).font_size(16.).font_weight(FontWeight::BOLD))
                                    .child(if *checking.read() {
                                        busy("Checking…").into_element()
                                    } else {
                                        label()
                                            .text(found.read().clone().unwrap_or_else(|| {
                                                if update::enabled() { "Relix checks for updates when it starts." } else { "Updates are off in this build." }.into()
                                            }))
                                            .font_size(13.)
                                            .color(theme().muted)
                                            .into_element()
                                    }),
                            )
                            .child(match ready {
                                // Same as the banner's Restart: swap in the new files and reopen.
                                Some(ready) => Focusable::new(button("Restart to update", theme().primary.0, theme().primary.1), move |()| {
                                    let ready = ready.clone();
                                    // Copying the files happens off the UI thread; on success Relix restarts.
                                    spawn(async move {
                                        let applied = tokio::task::spawn_blocking(move || update::apply(&ready).map_err(|e| e.to_string())).await;
                                        if let Ok(Err(e)) = applied {
                                            found.set(Some(format!("Couldn't install the update: {e}")));
                                        }
                                    });
                                })
                                .into_element(),
                                None if update::enabled() => Focusable::new(button("Check for updates", theme().raised, (255, 255, 255)), check).into_element(),
                                None => rect().into_element(),
                            }),
                    )
                    .child(label().text("FRIENDS").font_size(12.).font_weight(FontWeight::BOLD).color(theme().muted).margin(Gaps::new(16., 0., 0., 0.)))
                    .child(privacy("share_watching", true, "Show friends what I'm watching", "Friends see the title next to your name while you watch."))
                    .child(privacy("appear_offline", false, "Appear offline", "Friends see you as offline. You can still join parties."))
                    .child(label().text("TROUBLESHOOTING").font_size(12.).font_weight(FontWeight::BOLD).color(theme().muted).margin(Gaps::new(16., 0., 0., 0.)))
                    .child(
                        rect()
                            .width(Size::fill())
                            .max_width(Size::px(720.))
                            .padding(Gaps::new_all(18.))
                            .maybe(!phone, |el| el.horizontal())
                            .content(Content::Flex)
                            .cross_align(if phone { Alignment::Start } else { Alignment::Center })
                            .spacing(12.)
                            .corner_radius(theme().radius)
                            .background(theme().surface)
                            .child(
                                rect()
                                    .width(if phone { Size::fill() } else { Size::flex(1.) })
                                    .spacing(4.)
                                    .child(label().text("Debug mode").font_size(16.).font_weight(FontWeight::BOLD))
                                    .child(
                                        label()
                                            .text(format!(
                                                "Logs what Relix and the player do to debug.log and mpv.log in {}. The performance overlay (Ctrl+Shift+P) appears after a restart.",
                                                crate::data_dir().display()
                                            ))
                                            .font_size(13.)
                                            .color(theme().muted),
                                    ),
                            )
                            // Android keeps app data private: no file manager can open it.
                            .maybe(*debug_on.read() && !cfg!(target_os = "android"), |el| {
                                el.child(Focusable::new(button("Open folder", theme().raised, (255, 255, 255)), |()| {
                                    crate::addons::open_in_browser(&crate::data_dir().to_string_lossy())
                                }))
                            })
                            .child(Switch::new(WritableSetter::new(debug_on.into_readable(), move |on| {
                                debug::set(on);
                                debug_on.set(on);
                            }))),
                    ),
            ),
        )
    }
}

/// A friends privacy switch (a synced setting); friends see the change right away.
#[derive(PartialEq)]
struct Privacy {
    key: &'static str,
    default: bool,
    title: &'static str,
    about: &'static str,
}

fn privacy(key: &'static str, default: bool, title: &'static str, about: &'static str) -> Privacy {
    Privacy { key, default, title, about }
}

impl Component for Privacy {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let (key, default) = (self.key, self.default);
        let mut on = use_state(|| ctx.store.get::<bool>(&ctx.profile, "settings", key).ok().flatten().unwrap_or(default));
        rect()
            .width(Size::fill())
            .max_width(Size::px(720.))
            .padding(Gaps::new_all(18.))
            .horizontal()
            .content(Content::Flex)
            .cross_align(Alignment::Center)
            .spacing(12.)
            .corner_radius(theme().radius)
            .background(theme().surface)
            .child(
                rect()
                    .width(Size::flex(1.))
                    .spacing(4.)
                    .child(label().text(self.title).font_size(16.).font_weight(FontWeight::BOLD))
                    .child(label().text(self.about).font_size(13.).color(theme().muted)),
            )
            .child(Switch::new(WritableSetter::new(on.into_readable(), move |value| {
                if let Err(e) = ctx.store.put(&ctx.profile, "settings", key, &value) {
                    crate::log!("saving setting: {e}");
                }
                on.set(value);
                crate::social::send_presence(&ctx);
                ctx.sync_soon();
            })))
    }
}
