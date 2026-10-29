use std::time::Duration;

use freya::animation::*;
use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::MetaPreview;

use crate::focus::{Focusable, Scroller, use_page_scroller, use_scroller};
use crate::profiles::avatar_color;
use crate::store::Profile;
use crate::{Ctx, Route};

pub const BG: (u8, u8, u8) = (20, 20, 20);
const POSTER_W: f32 = 150.;
const POSTER_H: f32 = 225.;
const ROW_H: f32 = POSTER_H + 60.;
const HERO_H: f32 = 600.;
const NAV_H: f32 = 68.;
const FEATURED: usize = 5;
const ROTATE_EVERY: Duration = Duration::from_secs(8);

#[derive(Clone, PartialEq)]
pub struct Row {
    pub title: String,
    pub metas: Vec<MetaPreview>,
}

/// A vertically virtualized page: item 0 is the hero, the rest are rows.
#[derive(PartialEq)]
pub struct Home;

impl Component for Home {
    fn render(&self) -> impl IntoElement {
        let page = use_scroller();
        use_page_scroller(page);
        let rows = use_consume::<Ctx>().rows.read().clone();
        let featured: Vec<MetaPreview> = rows
            .first()
            .map(|r| r.metas.iter().filter(|m| m.background.is_some()).take(FEATURED).cloned().collect())
            .unwrap_or_default();
        let len = rows.len() + 1;

        page.wrap(
            VirtualScrollView::new_with_data_controlled(
                (featured, rows),
                |item, (featured, rows)| {
                    if item.index == 0 {
                        Hero { items: featured.clone() }.into()
                    } else {
                        PosterRow { row: rows[item.index - 1].clone() }.into()
                    }
                },
                page.controller,
            )
            .scroll_with_arrows(false)
            .length(len)
            .item_size(ItemSize::Dynamic(Callback::new(|i| if i == 0 { HERO_H } else { ROW_H }))),
        )
    }
}

pub fn nav(profile: &Profile) -> Rect {
    let link = |text: &'static str, active: bool| {
        label()
            .text(text)
            .font_size(15.)
            .maybe(!active, |el| el.color((200, 200, 200)))
            .maybe(active, |el| el.font_weight(FontWeight::BOLD))
    };
    rect()
        .position(Position::new_absolute().top(0.).left(0.))
        .width(Size::fill())
        .height(Size::px(NAV_H))
        .padding(Gaps::new(0., 48., 0., 48.))
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(28.)
        .background(LinearGradient::new().angle(180.).stop(((0, 0, 0, 0.8), 0.)).stop(((0, 0, 0, 0.), 100.)))
        .child(
            Link::new(Route::Home)
                .child(label().text("RELIX").font_size(30.).font_weight(FontWeight::BLACK).color((229, 9, 20))),
        )
        .child(Link::new(Route::Home).child(link("Home", true)))
        .child(link("Series", false))
        .child(link("Movies", false))
        .child(link("My List", false))
        .child(
            rect()
                .position(Position::new_absolute().right(48.).top(18.))
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(24.)
                .child(Link::new(Route::Addons).child(link("Addons", false)))
                .child(link("Search", false))
                .child(
                    Link::new(Route::Profiles).child(
                        rect()
                            .width(Size::px(32.))
                            .height(Size::px(32.))
                            .corner_radius(4.)
                            .background(avatar_color(profile))
                            .center()
                            .child(label().text(profile.name.chars().next().unwrap_or('?').to_uppercase().to_string()).font_weight(FontWeight::BOLD)),
                    ),
                ),
        )
}

/// Featured titles cross-fading every few seconds, Netflix billboard style.
#[derive(PartialEq)]
struct Hero {
    items: Vec<MetaPreview>,
}

impl Component for Hero {
    fn render(&self) -> impl IntoElement {
        let mut index = use_state(|| 0usize);
        let mut prev = use_state(|| 0usize);
        let len = self.items.len();
        // The ticker outlives this render, and rows arrive after mount, so it reads the count from state.
        let mut count = use_state(|| len);
        count.set_if_modified(len);

        use_hook(move || {
            spawn(async move {
                loop {
                    tokio::time::sleep(ROTATE_EVERY).await;
                    let (i, len) = (*index.read(), *count.read());
                    if len > 1 {
                        prev.set(i);
                        index.set((i + 1) % len);
                    }
                }
            });
        });

        let current = *index.read();
        let fade = use_animation_with_dependencies(&current, |conf, _| {
            conf.on_creation(OnCreation::Finish);
            conf.on_change(OnChange::Rerun);
            AnimNum::new(0., 1.).time(900).ease(Ease::InOut)
        });

        let Some(meta) = self.items.get(current) else {
            return rect().width(Size::fill()).height(Size::px(HERO_H)).into_element();
        };
        let behind = self.items.get(*prev.read()).filter(|_| *prev.read() != current);

        rect()
            .width(Size::fill())
            .height(Size::px(HERO_H))
            .overflow(Overflow::Clip)
            // Freya doesn't order siblings on the same layer, so every stacked piece gets its own.
            .maybe_child(behind.map(|m| backdrop(m.background.as_deref()).layer(1)))
            .child(backdrop(meta.background.as_deref()).layer(2).opacity(fade.get().value()))
            .child(details(meta).layer(5))
            .child(dots(len, current).layer(5))
            .into_element()
    }
}

pub fn layer() -> Rect {
    rect().position(Position::new_absolute().top(0.).left(0.)).width(Size::percent(100.)).height(Size::percent(100.))
}

pub fn backdrop(background: Option<&str>) -> Rect {
    match background.and_then(|u| Url::parse(u).ok()) {
        Some(url) => layer()
            .child(
                ImageViewer::new(url)
                    .width(Size::fill())
                    .height(Size::fill())
                    .aspect_ratio(AspectRatio::Max)
                    .image_cover(ImageCover::Center),
            )
            .child(shade()),
        None => layer().background((40, 40, 40)).child(shade()),
    }
}

/// Darkens the left side for text and fades the bottom into the page.
fn shade() -> Rect {
    layer()
        .layer(1)
        .background(LinearGradient::new().angle(270.).stop(((0, 0, 0, 0.85), 0.)).stop(((0, 0, 0, 0.), 60.)))
        .child(layer().layer(1).background(
            LinearGradient::new().angle(0.).stop(((BG.0, BG.1, BG.2, 0.), 65.)).stop((BG, 100.)),
        ))
}

fn details(meta: &MetaPreview) -> Rect {
    rect()
        .position(Position::new_absolute().left(48.).bottom(110.))
        .width(Size::px(560.))
        .spacing(16.)
        .child(title_art(meta.logo.as_deref(), &meta.name))
        .maybe_child(meta.release_info.clone().map(|r| label().text(r).font_size(16.).color((70, 211, 105))))
        .maybe_child(meta.description.clone().map(|d| {
            paragraph().max_lines(3).text_overflow(TextOverflow::Ellipsis).font_size(17.).span(d)
        }))
        .child(
            rect()
                .horizontal()
                .spacing(12.)
                .child(Focusable::new(button("Play", (255, 255, 255), (0, 0, 0)), open_title(meta)))
                .child(Focusable::new(button("More Info", (109, 109, 110, 0.7), (255, 255, 255)), open_title(meta))),
        )
}

/// Navigates to a title's detail page.
pub fn open_title(meta: &MetaPreview) -> EventHandler<()> {
    let route = Route::Detail { kind: meta.r#type.clone(), id: meta.id.clone() };
    EventHandler::new(move |()| {
        let _ = RouterContext::get().push(route.clone());
    })
}

/// The title's logo, or its name when the addon has no logo.
pub fn title_art(logo: Option<&str>, name: &str) -> Element {
    match logo.and_then(|u| Url::parse(u).ok()) {
        Some(url) => ImageViewer::new(url)
            .width(Size::px(420.))
            .height(Size::px(150.))
            .aspect_ratio(AspectRatio::Min)
            .into(),
        None => label().text(name.to_owned()).font_size(56.).font_weight(FontWeight::BOLD).into(),
    }
}

pub fn button(text: &'static str, bg: impl Into<Color>, fg: impl Into<Color>) -> Rect {
    rect()
        .padding(Gaps::new(10., 28., 10., 24.))
        .corner_radius(4.)
        .background(bg.into())
        .child(label().text(text).font_size(18.).font_weight(FontWeight::BOLD).color(fg.into()))
}

fn dots(len: usize, current: usize) -> Rect {
    rect()
        .position(Position::new_absolute().right(48.).bottom(110.))
        .horizontal()
        .spacing(6.)
        .children((0..len).map(|i| {
            rect()
                .width(Size::px(if i == current { 22. } else { 8. }))
                .height(Size::px(4.))
                .corner_radius(2.)
                .background(if i == current { (255, 255, 255, 1.) } else { (255, 255, 255, 0.4) })
                .into_element()
        }))
}

#[derive(PartialEq)]
struct PosterRow {
    row: Row,
}

impl Component for PosterRow {
    fn render(&self) -> impl IntoElement {
        let row = use_scroller();
        let metas = self.row.metas.clone();
        let len = metas.len();
        rect()
            .height(Size::px(ROW_H))
            .padding(Gaps::new(12., 48., 12., 48.))
            .spacing(10.)
            .child(label().text(self.row.title.clone()).font_size(20.).font_weight(FontWeight::BOLD))
            .child(
                row.wrap(
                    VirtualScrollView::new_with_data_controlled(
                        (metas, row),
                        |item, (metas, row): &(Vec<MetaPreview>, Scroller)| {
                            rect().key(item.index).padding(Gaps::new(0., 8., 0., 0.)).child(poster(&metas[item.index], *row)).into()
                        },
                        row.controller,
                    )
                    .direction(Direction::Horizontal)
                    .scroll_with_arrows(false)
                    .length(len)
                    .item_size(POSTER_W + 8.),
                ),
            )
    }
}

fn poster(meta: &MetaPreview, row: Scroller) -> Element {
    let art: Element = match meta.poster.as_deref().and_then(|p| Url::parse(p).ok()) {
        Some(url) => rect()
            .corner_radius(4.)
            .overflow(Overflow::Clip)
            .child(ImageViewer::new(url).width(Size::px(POSTER_W)).height(Size::px(POSTER_H)))
            .into(),
        None => rect()
            .width(Size::px(POSTER_W))
            .height(Size::px(POSTER_H))
            .corner_radius(4.)
            .background((50, 50, 50))
            .center()
            .child(meta.name.clone())
            .into(),
    };
    Focusable::new(art, open_title(meta)).row(row).into()
}
