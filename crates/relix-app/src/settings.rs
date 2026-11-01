use freya::prelude::*;

use crate::focus::{Focusable, use_page_scroller, use_scroller};
use crate::home::button;
use crate::motion::busy;
use crate::{Ctx, update};
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
        let mut checking = use_state(|| false);
        let mut found = use_state(|| None::<String>);
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
            ScrollView::new_controlled(page.controller).scroll_with_arrows(false).child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(theme().top, 48., 48., 48.))
                    .spacing(12.)
                    .child(label().text("Settings").font_size(32.).font_weight(FontWeight::BOLD))
                    .child(label().text("APPEARANCE").font_size(12.).font_weight(FontWeight::BOLD).color(theme().muted).margin(Gaps::new(16., 0., 0., 0.)))
                    .child(
                        rect()
                            .width(Size::px(720.))
                            .padding(Gaps::new_all(18.))
                            .horizontal()
                            .content(Content::Flex)
                            .cross_align(Alignment::Center)
                            .corner_radius(theme().radius)
                            .background(theme().surface)
                            .child(
                                rect()
                                    .width(Size::flex(1.))
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
                            .width(Size::px(720.))
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
                                    .child(label().text(format!("Relix {}", env!("CARGO_PKG_VERSION"))).font_size(16.).font_weight(FontWeight::BOLD))
                                    .child(if *checking.read() {
                                        busy("Checking…").into_element()
                                    } else {
                                        label()
                                            .text(found.read().clone().unwrap_or_else(|| "Relix checks for updates when it starts.".into()))
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
                                None => Focusable::new(button("Check for updates", theme().raised, (255, 255, 255)), check).into_element(),
                            }),
                    ),
            ),
        )
    }
}
