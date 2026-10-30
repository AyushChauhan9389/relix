use freya::prelude::*;

use crate::Ctx;
use crate::focus::{Focusable, use_page_scroller, use_scroller};
use crate::theme::{Look, palette, theme};

/// App settings; for now the look (Netflix or Stremio), synced with the account.
#[derive(PartialEq)]
pub struct Settings;

impl Component for Settings {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let page = use_scroller();
        use_page_scroller(page);
        let current = *ctx.look.read();
        let card = |look: Look, name: &'static str, blurb: &'static str| {
            let ctx = ctx.clone();
            look_card(look, name, blurb, look == current, move || ctx.set_look(look)).into_element()
        };

        page.wrap(
            ScrollView::new_controlled(page.controller).scroll_with_arrows(false).child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(theme().top, 48., 48., 48.))
                    .spacing(16.)
                    .child(label().text("Settings").font_size(32.).font_weight(FontWeight::BOLD))
                    .child(label().text("Appearance").font_size(22.).font_weight(FontWeight::BOLD))
                    .child(label().text("How Relix looks. Saved with your settings and synced to your account.").font_size(15.).color(theme().muted))
                    .child(
                        rect()
                            .horizontal()
                            .spacing(20.)
                            .child(card(Look::Netflix, "Netflix", "Top bar, big featured title, red accents."))
                            .child(card(Look::Stremio, "Stremio", "Icon sidebar, titled posters, purple accents.")),
                    ),
            ),
        )
    }
}

/// A theme choice with a miniature of its layout.
fn look_card(look: Look, name: &'static str, blurb: &'static str, selected: bool, pick: impl Fn() + 'static) -> Focusable {
    let p = palette(look);
    let poster = |i: usize| {
        rect()
            .width(Size::px(34.))
            .height(Size::px(50.))
            .corner_radius(p.radius / 2.)
            .background(if i == 0 { p.accent } else { p.placeholder })
            .into_element()
    };
    let posters = rect().horizontal().spacing(6.).children((0..4).map(poster));
    let mini = if p.sidebar {
        rect()
            .horizontal()
            .child(rect().width(Size::px(26.)).height(Size::fill()).padding(Gaps::new_all(6.)).background(p.border).child(
                rect().width(Size::px(14.)).height(Size::px(14.)).corner_radius(4.).background(p.accent),
            ))
            .child(
                rect()
                    .padding(Gaps::new_all(12.))
                    .spacing(10.)
                    .child(rect().width(Size::px(150.)).height(Size::px(12.)).corner_radius(6.).background(p.surface))
                    .child(posters),
            )
    } else {
        rect()
            .child(rect().width(Size::fill()).height(Size::px(18.)).padding(Gaps::new(5., 10., 5., 10.)).background(p.border).child(
                rect().width(Size::px(28.)).height(Size::px(8.)).background(p.accent),
            ))
            .child(rect().padding(Gaps::new(12., 12., 12., 12.)).spacing(10.).child(rect().width(Size::px(120.)).height(Size::px(10.)).background(p.raised)).child(posters))
    };
    let card = rect()
        .width(Size::px(300.))
        .padding(Gaps::new_all(14.))
        .spacing(12.)
        .corner_radius(12.)
        .background(theme().surface)
        .border(Border::new().fill(if selected { theme().accent } else { theme().border }).width(2.))
        .child(rect().width(Size::fill()).height(Size::px(130.)).corner_radius(8.).overflow(Overflow::Clip).background(p.bg).child(mini))
        .child(
            rect()
                .horizontal()
                .spacing(10.)
                .cross_align(Alignment::Center)
                .child(label().text(name).font_size(18.).font_weight(FontWeight::BOLD))
                .maybe(selected, |el| el.child(label().text("✓ In use").font_size(14.).color(theme().accent))),
        )
        .child(paragraph().font_size(14.).color(theme().muted).span(blurb));
    Focusable { radius: 12., ..Focusable::new(card, move |()| pick()) }
}
