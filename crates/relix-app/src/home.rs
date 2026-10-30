use std::time::Duration;

use freya::animation::*;
use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::MetaPreview;

use crate::focus::{Focusable, Scroller, use_page_scroller, use_scroller};
use crate::motion::{FadeIn, HOVER_MS};
use crate::{Ctx, Route};
use crate::theme::theme;

const POSTER_W: f32 = 150.;
const POSTER_H: f32 = 225.;
const HERO_H: f32 = 600.;
/// Stremio's Home starts with a search bar instead of the hero.
const SEARCH_H: f32 = 96.;
const NAV_H: f32 = 68.;
const FEATURED: usize = 5;
const ROTATE_EVERY: Duration = Duration::from_secs(8);

#[derive(Clone, PartialEq)]
pub struct Row {
    /// Stable identity, so late catalog answers land in the right row.
    pub id: String,
    pub title: String,
    pub metas: Vec<MetaPreview>,
    /// Watched fraction per poster (Continue Watching only).
    pub progress: Vec<f32>,
}

/// A vertically virtualized page: item 0 is the hero, the rest are rows.
#[derive(PartialEq)]
pub struct Home;

impl Component for Home {
    fn render(&self) -> impl IntoElement {
        let page = use_scroller();
        use_page_scroller(page);
        let rows = use_consume::<Ctx>().rows.read().clone();
        // Featured titles come from the first catalog, not Continue Watching or My List.
        let featured: Vec<MetaPreview> = rows
            .iter()
            .find(|r| !r.id.starts_with("local/"))
            .map(|r| r.metas.iter().filter(|m| m.background.is_some()).take(FEATURED).cloned().collect())
            .unwrap_or_default();
        let len = rows.len() + 1;

        page.wrap(
            VirtualScrollView::new_with_data_controlled(
                (featured, rows),
                |item, (featured, rows)| {
                    if item.index == 0 && theme().sidebar {
                        search_bar().into()
                    } else if item.index == 0 {
                        Hero { items: featured.clone() }.into()
                    } else {
                        PosterRow { row: rows[item.index - 1].clone() }.into()
                    }
                },
                page.controller,
            )
            .scroll_with_arrows(false)
            .length(len)
            .item_size(ItemSize::Dynamic(Callback::new(|i| if i > 0 { row_h() } else if theme().sidebar { SEARCH_H } else { HERO_H }))),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NavLink {
    Home,
    Series,
    Movies,
    MyList,
    Addons,
    Search,
    Settings,
    Account,
}

/// The nav link to highlight on `route`, if it has one.
pub fn active_link(route: &Route) -> Option<NavLink> {
    match route {
        Route::Home => Some(NavLink::Home),
        Route::Browse { kind } if kind == "series" => Some(NavLink::Series),
        Route::Browse { kind } if kind == "movie" => Some(NavLink::Movies),
        Route::MyList => Some(NavLink::MyList),
        Route::Addons => Some(NavLink::Addons),
        Route::Search => Some(NavLink::Search),
        Route::Settings => Some(NavLink::Settings),
        Route::Account => Some(NavLink::Account),
        Route::Browse { .. } | Route::Detail { .. } => None,
    }
}

/// A nav link whose colour eases on hover.
#[derive(PartialEq)]
struct NavItem {
    text: &'static str,
    to: Route,
    active: bool,
}

impl Component for NavItem {
    fn render(&self) -> impl IntoElement {
        let mut hovered = use_state(|| false);
        let bright = self.active || *hovered.read();
        let color = use_animation_transition(bright, |from, to| {
            let c = |b: bool| if b { (255, 255, 255) } else { (190, 190, 190) };
            AnimColor::new(c(from), c(to)).time(HOVER_MS)
        });
        Link::new(self.to.clone()).child(
            label()
                .text(self.text)
                .font_size(15.)
                .color(&*color.read())
                .maybe(self.active, |el| el.font_weight(FontWeight::BOLD))
                .on_pointer_enter(move |_| hovered.set(true))
                .on_pointer_leave(move |_| hovered.set(false)),
        )
    }
}

pub fn nav(active: Option<NavLink>) -> Rect {
    let link = move |which: NavLink, text: &'static str, to: Route| NavItem { text, to, active: active == Some(which) };
    rect()
        .position(Position::new_absolute().top(0.).left(0.))
        .width(Size::fill())
        .height(Size::px(NAV_H))
        .padding(Gaps::new(0., 48., 0., 48.))
        .horizontal()
        .content(Content::Flex)
        .cross_align(Alignment::Center)
        .spacing(28.)
        .background(theme().bg)
        .border(Border::new().fill(theme().border).width(BorderWidth { top: 0., right: 0., bottom: 1., left: 0. }))
        .child(
            Link::new(Route::Home)
                .child(label().text("RELIX").font_size(30.).font_weight(FontWeight::BLACK).color(theme().accent)),
        )
        .child(link(NavLink::Home, "Home", Route::Home))
        .child(link(NavLink::Series, "Series", Route::Browse { kind: "series".into() }))
        .child(link(NavLink::Movies, "Movies", Route::Browse { kind: "movie".into() }))
        .child(link(NavLink::MyList, "My List", Route::MyList))
        .child(rect().width(Size::flex(1.)))
        .child(
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(24.)
                .child(link(NavLink::Addons, "Addons", Route::Addons))
                .child(link(NavLink::Search, "Search", Route::Search))
                .child(link(NavLink::Settings, "Settings", Route::Settings))
                .child(link(NavLink::Account, "Account", Route::Account)),
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
            .child(FadeIn::page(
                ImageViewer::new(url)
                    .width(Size::fill())
                    .height(Size::fill())
                    .aspect_ratio(AspectRatio::Max)
                    .image_cover(ImageCover::Center)
                    .loading_placeholder(rect())
                    .error_renderer(|_| rect().into()),
            ))
            .child(shade()),
        None => layer().background(theme().border).child(shade()),
    }
}

/// Darkens the left side for text and fades the bottom into the page.
fn shade() -> Rect {
    layer()
        .layer(1)
        .background(LinearGradient::new().angle(270.).stop(((0, 0, 0, 0.85), 0.)).stop(((0, 0, 0, 0.), 60.)))
        .child(layer().layer(1).background(
            LinearGradient::new().angle(0.).stop(((theme().bg.0, theme().bg.1, theme().bg.2, 0.), 65.)).stop((theme().bg, 100.)),
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
                .child(Focusable::new(button("Play", theme().cta.0, theme().cta.1), open_title(meta)))
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
    let text = |name: String| label().text(name).font_size(56.).font_weight(FontWeight::BOLD).into();
    match logo.and_then(|u| Url::parse(u).ok()) {
        Some(url) => {
            let name = name.to_owned();
            ImageViewer::new(url)
                .width(Size::px(420.))
                .height(Size::px(150.))
                .aspect_ratio(AspectRatio::Min)
                .error_renderer(move |_| text(name.clone()))
                .into()
        }
        None => text(name.to_owned()),
    }
}

pub fn button(text: &'static str, bg: impl Into<Color>, fg: impl Into<Color>) -> Rect {
    button_owned(text.to_owned(), bg, fg)
}

pub fn button_owned(text: String, bg: impl Into<Color>, fg: impl Into<Color>) -> Rect {
    rect()
        .padding(Gaps::new(10., 28., 10., 24.))
        .corner_radius(4.)
        .background(bg.into())
        .child(label().text(text).font_size(18.).font_weight(FontWeight::BOLD).color(fg.into()))
}

pub fn chip(text: String, active: bool) -> Rect {
    rect()
        .padding(Gaps::new(6., 14., 6., 14.))
        .corner_radius(16.)
        .background(if active { theme().cta.0 } else { theme().raised })
        .child(label().text(text).font_size(14.).color(if active { theme().cta.1 } else { (230, 230, 230) }))
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
pub struct PosterRow {
    pub row: Row,
}

impl Component for PosterRow {
    fn render(&self) -> impl IntoElement {
        let row = use_scroller();
        let metas = self.row.metas.clone();
        let progress = self.row.progress.clone();
        let len = metas.len();
        rect()
            .height(Size::px(row_h()))
            .padding(Gaps::new(12., 48., 12., 48.))
            .spacing(10.)
            .child(label().text(self.row.title.clone()).font_size(20.).font_weight(FontWeight::BOLD))
            .child(
                row.wrap(
                    VirtualScrollView::new_with_data_controlled(
                        (metas, progress, row),
                        |item, (metas, progress, row): &(Vec<MetaPreview>, Vec<f32>, Scroller)| {
                            rect()
                                .key(item.index)
                                .padding(Gaps::new(0., 8., 0., 0.))
                                .child(poster(&metas[item.index], progress.get(item.index).copied(), Some(*row)))
                                .into()
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

/// A poster that opens its title; `row` is the horizontal row it scrolls within, if any.
pub fn poster(meta: &MetaPreview, progress: Option<f32>, row: Option<Scroller>) -> Element {
    // Shown when there's no poster or it fails to load.
    let blank = |name: String| -> Element {
        rect()
            .width(Size::px(POSTER_W))
            .height(Size::px(POSTER_H))
            .padding(Gaps::new_all(8.))
            .background(theme().placeholder)
            .center()
            .child(paragraph().text_align(TextAlign::Center).font_size(15.).span(name))
            .into()
    };
    let name = meta.name.clone();
    let art: Element = rect()
        .corner_radius(theme().radius)
        .overflow(Overflow::Clip)
        .child(match meta.poster.as_deref().and_then(|p| Url::parse(p).ok()) {
            Some(url) => FadeIn::image(
                ImageViewer::new(url)
                    .width(Size::px(POSTER_W))
                    .height(Size::px(POSTER_H))
                    // A soft grey card until the poster arrives.
                    .loading_placeholder(rect().width(Size::px(POSTER_W)).height(Size::px(POSTER_H)).background(theme().placeholder))
                    .error_renderer(move |_| blank(name.clone())),
            )
            .into(),
            None => blank(name),
        })
        .into();
    // Sized to the poster so the progress bar's position is exact in both looks.
    let art = rect().width(Size::px(POSTER_W)).height(Size::px(POSTER_H)).child(art).map(progress, |el, p| {
        el.child(
            rect()
                .position(Position::new_absolute().left(8.).bottom(8.))
                .layer(Layer::OverlayLevel(1)) // above the poster image, below the nav
                .width(Size::px(POSTER_W - 16.))
                .height(Size::px(4.))
                .corner_radius(2.)
                .background(theme().track)
                .child(rect().width(Size::percent(p.clamp(0., 1.) * 100.)).height(Size::fill()).corner_radius(2.).background(theme().accent)),
        )
    });
    let card = Focusable { row, radius: theme().radius, grow: 1.08, ..Focusable::new(art, open_title(meta)) };
    if !theme().sidebar {
        return card.into();
    }
    rect()
        .width(Size::px(POSTER_W))
        .spacing(8.)
        .child(card)
        .child(label().text(meta.name.clone()).font_size(14.).max_lines(1).text_overflow(TextOverflow::Ellipsis))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_page_highlights_its_nav_link() {
        assert_eq!(active_link(&Route::Home), Some(NavLink::Home));
        assert_eq!(active_link(&Route::Browse { kind: "series".into() }), Some(NavLink::Series));
        assert_eq!(active_link(&Route::Browse { kind: "movie".into() }), Some(NavLink::Movies));
        assert_eq!(active_link(&Route::MyList), Some(NavLink::MyList));
        assert_eq!(active_link(&Route::Addons), Some(NavLink::Addons));
        assert_eq!(active_link(&Route::Search), Some(NavLink::Search));
        assert_eq!(active_link(&Route::Account), Some(NavLink::Account));
        assert_eq!(active_link(&Route::Settings), Some(NavLink::Settings));
        assert_eq!(active_link(&Route::Browse { kind: "anime".into() }), None, "no nav link for other types");
        assert_eq!(active_link(&Route::Detail { kind: "movie".into(), id: "tt1".into() }), None);
    }
}

/// A row's height: posters, plus the title under each one in the Stremio look.
fn row_h() -> f32 {
    POSTER_H + if theme().sidebar { 88. } else { 60. }
}

/// Stremio Home's top bar: a search field that opens Search.
fn search_bar() -> Rect {
    rect().width(Size::fill()).padding(Gaps::new(theme().top, 48., 0., 48.)).child(Focusable {
        radius: 24.,
        ..Focusable::new(
            rect()
                .width(Size::px(560.))
                .height(Size::px(46.))
                .padding(Gaps::new(0., 20., 0., 20.))
                .corner_radius(24.)
                .background(theme().surface)
                .main_align(Alignment::Center)
                .child(label().text("Search movies & series").font_size(15.).color(theme().muted)),
            |()| {
                let _ = RouterContext::get().push(Route::Search);
            },
        )
    })
}

pub const SIDEBAR_W: f32 = 96.;

/// 24px stroke icons for the Stremio sidebar; SvgViewer tints the root stroke.
const ICON_HOME: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 10.5 12 3l9 7.5V21h-6v-6H9v6H3z"/></svg>"#;
const ICON_MOVIES: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="m15.5 8.5-2 5-5 2 2-5z"/></svg>"#;
const ICON_SERIES: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="7" width="18" height="13" rx="2"/><path d="m8 3 4 4 4-4"/></svg>"#;
const ICON_LIBRARY: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3h12v18l-6-4-6 4z"/></svg>"#;
const ICON_SEARCH: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>"#;
const ICON_ADDONS: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg>"#;
const ICON_SETTINGS: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1"/><circle cx="15" cy="6" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="17" cy="18" r="2"/></svg>"#;
const ICON_ACCOUNT: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 4-6 8-6s8 2 8 6"/></svg>"#;

/// Stremio's left rail: logo, then icon + label links.
pub fn sidebar(active: Option<NavLink>) -> Rect {
    let item = move |which: NavLink, icon: &'static str, text: &'static str, to: Route| {
        SideItem { icon, text, to, active: active == Some(which) }
    };
    rect()
        .position(Position::new_absolute().top(0.).left(0.))
        .width(Size::px(SIDEBAR_W))
        .height(Size::percent(100.))
        .padding(Gaps::new(20., 0., 20., 0.))
        .spacing(4.)
        .cross_align(Alignment::Center)
        .background(theme().bg)
        .border(Border::new().fill(theme().border).width(BorderWidth { top: 0., right: 1., bottom: 0., left: 0. }))
        .child(
            Link::new(Route::Home).child(
                rect()
                    .width(Size::px(44.))
                    .height(Size::px(44.))
                    .margin(Gaps::new(0., 0., 18., 0.))
                    .corner_radius(12.)
                    .background(theme().accent)
                    .center()
                    .child(label().text("R").font_size(24.).font_weight(FontWeight::BLACK)),
            ),
        )
        .child(item(NavLink::Home, ICON_HOME, "Board", Route::Home))
        .child(item(NavLink::Movies, ICON_MOVIES, "Movies", Route::Browse { kind: "movie".into() }))
        .child(item(NavLink::Series, ICON_SERIES, "Series", Route::Browse { kind: "series".into() }))
        .child(item(NavLink::MyList, ICON_LIBRARY, "Library", Route::MyList))
        .child(item(NavLink::Search, ICON_SEARCH, "Search", Route::Search))
        .child(item(NavLink::Addons, ICON_ADDONS, "Addons", Route::Addons))
        .child(item(NavLink::Settings, ICON_SETTINGS, "Settings", Route::Settings))
        .child(item(NavLink::Account, ICON_ACCOUNT, "Account", Route::Account))
}

/// A sidebar link: accent when current, brightening on hover, eased.
#[derive(PartialEq)]
struct SideItem {
    icon: &'static str,
    text: &'static str,
    to: Route,
    active: bool,
}

impl Component for SideItem {
    fn render(&self) -> impl IntoElement {
        let mut hovered = use_state(|| false);
        let color = use_animation_transition((self.active, *hovered.read()), |from, to| {
            let c = |(active, hovered): (bool, bool)| if active { theme().accent } else if hovered { (255, 255, 255) } else { theme().muted };
            AnimColor::new(c(from), c(to)).time(HOVER_MS)
        });
        let c: Color = (&*color.read()).into();
        Link::new(self.to.clone()).child(
            rect()
                .width(Size::px(SIDEBAR_W))
                .padding(Gaps::new(10., 0., 10., 0.))
                .spacing(6.)
                .cross_align(Alignment::Center)
                .on_pointer_enter(move |_| hovered.set(true))
                .on_pointer_leave(move |_| hovered.set(false))
                .child(SvgViewer::new(self.icon.as_bytes()).width(Size::px(26.)).height(Size::px(26.)).stroke(c))
                .child(label().text(self.text).font_size(12.).color(c)),
        )
    }
}
