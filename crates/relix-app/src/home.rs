use std::time::Duration;

use freya::animation::*;
use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::{Addon, CatalogResponse, MetaPreview, ResourceRequest};

use crate::focus::{Focusable, Scroller, use_page_scroller, use_scroller};
use crate::motion::{FadeIn, HOVER_MS, Placeholder};
use crate::{Ctx, Route};
use crate::theme::theme;

const POSTER_W: f32 = 150.;
const POSTER_H: f32 = 225.;
const HERO_H: f32 = 600.;
const NAV_H: f32 = 68.;
const FEATURED: usize = 5;
const ROTATE_EVERY: Duration = Duration::from_secs(8);

/// What each poster in a row's virtual list needs.
type RowData = (Vec<MetaPreview>, Vec<f32>, Scroller, bool, std::collections::HashSet<String>, EventHandler<()>);

#[derive(Clone, PartialEq)]
pub struct Row {
    /// Stable identity, so late catalog answers land in the right row.
    pub id: String,
    pub title: String,
    pub metas: Vec<MetaPreview>,
    /// Watched fraction per poster (Continue Watching only).
    pub progress: Vec<f32>,
    /// Its catalog hasn't answered yet: shows placeholder posters.
    pub loading: bool,
    /// The catalog request to page with `skip` as the row scrolls, if the catalog supports it.
    pub next: Option<(Addon, ResourceRequest)>,
}

/// A vertically virtualized page: item 0 is the hero, the rest are rows.
#[derive(PartialEq)]
pub struct Home;

impl Component for Home {
    fn render(&self) -> impl IntoElement {
        let page = use_scroller();
        use_page_scroller(page);
        let ctx = use_consume::<Ctx>();
        let rows = ctx.rows.read().clone();
        if rows.is_empty() && !*ctx.rows_loading.read() {
            return nothing_to_show(&ctx).into_element();
        }
        // Featured titles come from the first catalog, not Continue Watching or My List.
        let featured: std::rc::Rc<[MetaPreview]> = rows
            .iter()
            .find(|r| !r.id.starts_with("local/"))
            .map(|r| r.metas.iter().filter(|m| m.background.is_some()).take(FEATURED).cloned().collect())
            .unwrap_or_else(|| std::rc::Rc::from([]));
        let loading = *ctx.rows_loading.read();
        // Nothing yet (first start, addons loading): placeholder rows.
        let rows = if rows.is_empty() {
            (0..3).map(|n| Row { id: format!("skeleton/{n}"), title: String::new(), metas: vec![], progress: vec![], loading: true, next: None }).collect()
        } else {
            rows
        };
        let len = rows.len() + 1;
        // Shared, so the per-scroll-frame item builder copies pointers, not whole rows of titles.
        let rows: Vec<std::rc::Rc<Row>> = rows.into_iter().map(std::rc::Rc::new).collect();

        page.wrap(
            VirtualScrollView::new_with_data_controlled(
                (featured, rows, loading),
                |item, (featured, rows, loading)| {
                    if item.index == 0 && theme().sidebar {
                        // The Stremio look has no hero: just room under its top bar.
                        rect().height(Size::px(theme().top - 24.)).into()
                    } else if item.index == 0 {
                        Hero { items: featured.clone(), loading: *loading }.into()
                    } else {
                        PosterRow { row: rows[item.index - 1].clone() }.into()
                    }
                },
                page.controller,
            )
            .scroll_with_arrows(false)
            .length(len)
            .item_size(ItemSize::Dynamic(Callback::new(|i| if i > 0 { row_h() } else if theme().sidebar { theme().top - 24. } else { HERO_H }))),
        )
        .into_element()
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
        Route::Browse { kind } | Route::BrowseCatalog { kind, .. } if kind == "series" => Some(NavLink::Series),
        Route::Browse { kind } | Route::BrowseCatalog { kind, .. } if kind == "movie" => Some(NavLink::Movies),
        Route::MyList => Some(NavLink::MyList),
        Route::Addons => Some(NavLink::Addons),
        Route::Search => Some(NavLink::Search),
        Route::Settings => Some(NavLink::Settings),
        Route::Account => Some(NavLink::Account),
        Route::Browse { .. } | Route::BrowseCatalog { .. } | Route::Detail { .. } => None,
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

/// `compact` (a narrow window) tightens the margins and gaps so every link still fits on one line.
pub fn nav(active: Option<NavLink>, ctx: &Ctx, compact: bool) -> Rect {
    let link = move |which: NavLink, text: &'static str, to: Route| NavItem { text, to, active: active == Some(which) };
    let side = if compact { 20. } else { 48. };
    rect()
        .position(Position::new_absolute().top(0.).left(0.))
        .width(Size::fill())
        .height(Size::px(NAV_H))
        .padding(Gaps::new(0., side, 0., side))
        .horizontal()
        .content(Content::Flex)
        .cross_align(Alignment::Center)
        .spacing(if compact { 14. } else { 28. })
        .background(theme().bg)
        .border(Border::new().fill(theme().border).width(BorderWidth { top: 0., right: 0., bottom: 1., left: 0. }))
        .child(
            Link::new(Route::Home).child(logo(52.)),
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
                .spacing(if compact { 12. } else { 24. })
                .child(link(NavLink::Addons, "Addons", Route::Addons))
                .child(link(NavLink::Search, "Search", Route::Search))
                .child(link(NavLink::Settings, "Settings", Route::Settings))
                .child(link(NavLink::Account, "Account", Route::Account))
                .child(fullscreen_button(ctx)),
        )
}

/// Featured titles cross-fading every few seconds, Netflix billboard style.
#[derive(PartialEq)]
struct Hero {
    items: std::rc::Rc<[MetaPreview]>,
    /// Catalogs are still answering: the empty hero pulses rather than sitting blank.
    loading: bool,
}

impl Component for Hero {
    fn render(&self) -> impl IntoElement {
        let autoplay = use_consume::<Ctx>().autoplay_title;
        let mut index = use_state(|| 0usize);
        let mut prev = use_state(|| 0usize);
        let len = self.items.len();
        // The ticker outlives this render, and rows arrive after mount, so it reads the count from state.
        let mut count = use_state(|| len);
        count.set_if_modified(len);

        use_hook(move || {
            let focused = Platform::get().current_window().is_app_focused;
            spawn(async move {
                loop {
                    tokio::time::sleep(ROTATE_EVERY).await;
                    let (i, len) = (*index.peek(), *count.peek());
                    // Behind other windows or minimized, Relix shouldn't spend anything redrawing.
                    if len > 1 && *focused.peek() {
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
            AnimNum::new(0., 1.).time(500).ease(Ease::InOut)
        });

        let Some(meta) = self.items.get(current) else {
            // Nothing to feature and nothing coming (offline, or catalogs without art): stay still,
            // since a pulse that never ends keeps the window redrawing.
            if !self.loading {
                return rect().width(Size::fill()).height(Size::px(theme().top + 40.)).into_element();
            }
            // Featured titles still loading: the hero's shape, pulsing.
            return rect()
                .width(Size::fill())
                .height(Size::px(HERO_H))
                .padding(Gaps::new(theme().top + 40., 48., 110., 48.))
                .main_align(Alignment::End)
                .spacing(16.)
                .child(Placeholder::new(Size::px(380.), 90.))
                .child(Placeholder::new(Size::px(520.), 18.))
                .child(Placeholder::new(Size::px(460.), 18.))
                .child(rect().horizontal().spacing(12.).child(Placeholder::new(Size::px(110.), 44.)).child(Placeholder::new(Size::px(150.), 44.)))
                .into_element();
        };
        let behind = self.items.get(*prev.read()).filter(|_| *prev.read() != current);

        rect()
            .width(Size::fill())
            .height(Size::px(HERO_H))
            .overflow(Overflow::Clip)
            // Freya doesn't order siblings on the same layer, so every stacked piece gets its own.
            .maybe_child(behind.map(|m| backdrop(m.background.as_deref()).layer(1)))
            .child(backdrop(meta.background.as_deref()).layer(2).opacity(fade.get().value()))
            .child(details(meta, autoplay).layer(5))
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
                    // Fill-sized images decode at their full size otherwise: 33 MB each for 4K art.
                    .decode_mode(DecodeMode::Custom(Size2D::new(1920., 1080.)))
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

fn details(meta: &MetaPreview, autoplay: State<Option<String>>) -> Rect {
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
                .child(Focusable::new(play_button("Play".into()), play_title(meta, autoplay)))
                .child(Focusable::new(button("More Info", theme().secondary, (255, 255, 255)), open_title(meta))),
        )
}

/// Navigates to a title's detail page.
pub fn open_title(meta: &MetaPreview) -> EventHandler<()> {
    let route = Route::Detail { kind: meta.r#type.clone(), id: meta.id.clone() };
    EventHandler::new(move |()| {
        let _ = RouterContext::get().push(route.clone());
    })
}

/// Opens a title's page and plays it right away (its remembered stream, or the first playable one).
fn play_title(meta: &MetaPreview, mut autoplay: State<Option<String>>) -> EventHandler<()> {
    let (open, id) = (open_title(meta), meta.id.clone());
    EventHandler::new(move |()| {
        autoplay.set(Some(id.clone()));
        open.call(());
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
                .loading_placeholder(Placeholder::new(Size::px(320.), 100.))
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
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(10.)
        .padding(Gaps::new(10., 24., 10., 24.))
        .corner_radius(theme().radius.min(8.))
        .background(bg.into())
        .child(label().text(text).font_size(18.).font_weight(FontWeight::BOLD).color(fg.into()))
}

/// The Play button: a drawn triangle (font glyphs render lopsided) and the look's Play colours.
pub fn play_button(text: String) -> Rect {
    let (bg, fg) = theme().cta;
    const TRIANGLE: &str = r#"<svg viewBox="0 0 24 24"><path d="M7 4.5v15a1 1 0 0 0 1.5.86l12.5-7.5a1 1 0 0 0 0-1.72L8.5 3.64A1 1 0 0 0 7 4.5z"/></svg>"#;
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(10.)
        .padding(Gaps::new(10., 26., 10., 20.))
        .corner_radius(theme().radius.min(8.))
        .background(bg)
        .child(SvgViewer::new(TRIANGLE.as_bytes()).width(Size::px(20.)).height(Size::px(20.)).fill(fg))
        .child(label().text(text).font_size(18.).font_weight(FontWeight::BOLD).color(fg))
}

pub fn chip(text: String, active: bool) -> Rect {
    // shortcut: width estimated from the text (~8px a character at 14px) because Freya measures a
    // chip at the end of a row against the space left and wraps its text; measure text if that misfits.
    // (A min width came out ~1.5x on a 150% display, so this sets the width itself.)
    let w = text.chars().count() as f32 * 8. + 28.;
    rect()
        .width(Size::px(w))
        .main_align(Alignment::Center)
        .horizontal()
        .padding(Gaps::new(6., 0., 6., 0.))
        .corner_radius(16.)
        .background(if active { (255, 255, 255) } else { theme().raised })
        .child(label().text(text).font_size(14.).color(if active { (0, 0, 0) } else { (230, 230, 230) }))
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

pub struct PosterRow {
    pub row: std::rc::Rc<Row>,
}

/// Same row object, same row: comparing whole rows of titles on every scroll frame was the cost.
impl PartialEq for PosterRow {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.row, &other.row)
    }
}

impl Component for PosterRow {
    fn render(&self) -> impl IntoElement {
        let row = use_scroller();
        let ctx = use_consume::<Ctx>();
        let new = ctx.new_episodes.read().clone();
        // Later pages, fetched as the row nears its end: (row id, items, no more pages).
        let mut paged = use_state(|| (String::new(), Vec::<MetaPreview>::new(), false));
        let busy = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
        let mut metas = self.row.metas.clone();
        let (more, done) = {
            let p = paged.read();
            if p.0 == self.row.id { (p.1.clone(), p.2) } else { (vec![], false) }
        };
        metas.extend(more);
        let load_more = EventHandler::new({
            let (id, next, received, base, http) = (self.row.id.clone(), self.row.next.clone(), metas.len(), metas.clone(), ctx.http.clone());
            move |()| {
                let Some((addon, req)) = next.clone() else { return };
                if done || busy.get() {
                    return;
                }
                busy.set(true);
                let (id, http, busy, base) = (id.clone(), http.clone(), busy.clone(), base.clone());
                spawn(async move {
                    let url = req.extra("skip", &received.to_string()).url(&addon);
                    let page = crate::get::<CatalogResponse>(http, url).await.map(|r| r.metas).unwrap_or_default();
                    // Addons that ignore `skip` repeat page one: no new ids means the end.
                    let fresh: Vec<_> = page.into_iter().filter(|m| !base.iter().any(|b| b.id == m.id)).collect();
                    let mut p = paged.write();
                    if p.0 != id {
                        *p = (id, vec![], false);
                    }
                    p.2 = fresh.is_empty();
                    p.1.extend(fresh);
                    busy.set(false);
                });
            }
        });
        let progress = self.row.progress.clone();
        let len = if self.row.loading { 8 } else { metas.len() };
        let loading = self.row.loading;
        rect()
            .height(Size::px(row_h()))
            .padding(Gaps::new(12., 48., 12., 48.))
            .spacing(10.)
            .child(if self.row.title.is_empty() {
                Placeholder::new(Size::px(200.), 22.).into_element()
            } else {
                rect()
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(16.)
                    .child(label().text(self.row.title.clone()).font_size(20.).font_weight(FontWeight::BOLD))
                    .maybe_child(view_all(&ctx, self.row.next.as_ref()))
                    .into_element()
            })
            .child(
                row.wrap(
                    VirtualScrollView::new_with_data_controlled(
                        (metas, progress, row, loading, new, load_more),
                        |item, (metas, progress, row, loading, new, load_more): &RowData| {
                            if !*loading && item.index + 6 >= metas.len() {
                                load_more.call(());
                            }
                            let card: Element = if *loading {
                                Placeholder::new(Size::px(POSTER_W), POSTER_H).into()
                            } else {
                                let m = &metas[item.index];
                                poster(m, progress.get(item.index).copied(), new.contains(&m.id), Some(*row))
                            };
                            rect().key(item.index).padding(Gaps::new(0., 8., 0., 0.)).child(card).into()
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

/// "View all" beside a catalog row's title: its catalog in the Movies/Series grid. Search rows
/// (their request carries the query) and catalogs the grid can't browse get none.
fn view_all(ctx: &Ctx, next: Option<&(Addon, ResourceRequest)>) -> Option<Element> {
    let (addon, req) = next.filter(|(_, r)| r.extra.is_empty())?;
    let addons = ctx.all_addons();
    let catalog = relix_proto::addon::discover_catalogs(&addons, &req.r#type)
        .position(|(i, c)| addons[i].manifest.id == addon.manifest.id && c.id == req.id)?;
    let route = Route::BrowseCatalog { kind: req.r#type.clone(), catalog };
    Some(
        label()
            .text("View all ›")
            .font_size(14.)
            .color(theme().muted)
            .on_press(move |_| drop(RouterContext::get().push(route.clone())))
            .into_element(),
    )
}

/// A poster that opens its title; `row` is the horizontal row it scrolls within, if any.
pub fn poster(meta: &MetaPreview, progress: Option<f32>, new_episode: bool, row: Option<Scroller>) -> Element {
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
                    .loading_placeholder(Placeholder::new(Size::px(POSTER_W), POSTER_H))
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
    let art = art.maybe(new_episode, |el| {
        el.child(
            rect()
                .position(Position::new_absolute().left(8.).top(8.))
                .layer(Layer::OverlayLevel(1))
                .padding(Gaps::new(3., 7., 3., 7.))
                .corner_radius(4.)
                .background(theme().accent)
                .child(label().text("NEW EPISODE").font_size(11.).font_weight(FontWeight::BOLD)),
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

/// Under a poster grid: "Loading more…" while a page is coming, "Load more" when there are more.
#[derive(Clone, PartialEq)]
pub struct GridFooter {
    pub loading: bool,
    pub more: bool,
    pub load_more: EventHandler<()>,
}

/// A grid's header, always re-rendered with its screen (it holds filters and other live state).
struct GridHeader(Element);

impl PartialEq for GridHeader {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

/// Posters in rows of as many as fit, virtualized: only the rows on screen exist, so a long grid
/// doesn't start every image load and placeholder animation at once. `header` scrolls above the
/// first row; `placeholders` shows two pulsing rows (first load). It fills its parent and scrolls
/// with `page`. Call it unconditionally (it measures the header with a hook).
pub fn poster_grid(
    page: Scroller,
    header: Element,
    metas: std::rc::Rc<Vec<MetaPreview>>,
    new: std::rc::Rc<std::collections::HashSet<String>>,
    placeholders: bool,
    footer: Option<GridFooter>,
) -> Element {
    const GAP: f32 = 8.;
    let header_h = use_state(|| 200f32);
    let cols = ((page.width() + GAP) / (POSTER_W + GAP)).floor().max(1.) as usize;
    let titled = theme().sidebar;
    // A fixed row height (the Stremio look adds a title line), so the list knows where rows sit.
    let row_h = POSTER_H + if titled { 30. } else { 0. } + 16.;
    let rows = if placeholders { 2 } else { metas.len().div_ceil(cols) };
    let len = 1 + rows + usize::from(footer.is_some());
    page.wrap(
        VirtualScrollView::new_with_data_controlled(
            (GridHeader(header), metas, new, placeholders, footer, cols, rows),
            move |item, (header, metas, new, placeholders, footer, cols, rows)| {
                if item.index == 0 {
                    let mut measured = header_h;
                    return rect()
                        .width(Size::fill())
                        .on_sized(move |e: Event<SizedEventData>| {
                            if (*measured.peek() - e.area.height()).abs() > 0.5 {
                                measured.set(e.area.height());
                            }
                        })
                        .child(header.0.clone())
                        .into();
                }
                let item_index = item.index - 1;
                if item_index >= *rows {
                    let footer = footer.as_ref();
                    return rect()
                        .padding(Gaps::new(8., 0., 32., 0.))
                        .maybe(footer.is_some_and(|f| f.loading), |el| el.child(crate::motion::busy("Loading more…")))
                        .maybe_child(footer.filter(|f| f.more && !f.loading).map(|f| {
                            let load = f.load_more.clone();
                            Focusable::new(button("Load more", theme().raised, (255, 255, 255)), move |()| load.call(()))
                        }))
                        .into();
                }
                let cells: Vec<Element> = if *placeholders {
                    (0..*cols).map(|_| Placeholder::new(Size::px(POSTER_W), POSTER_H).into_element()).collect()
                } else {
                    metas.iter().skip(item_index * cols).take(*cols).map(|m| poster(m, None, new.contains(&m.id), None)).collect()
                };
                rect().key(item.index).horizontal().spacing(GAP).height(Size::px(row_h)).children(cells).into()
            },
            page.controller,
        )
        .scroll_with_arrows(false)
        .length(len)
        .item_size(ItemSize::Dynamic(Callback::new(move |i| match i {
            0 => *header_h.peek(),
            i if i <= rows => row_h,
            _ => 96.,
        }))),
    )
    .into()
}

/// Home with no rows once loading is done: no addon offers Home catalogs, or every catalog failed (usually offline).
fn nothing_to_show(ctx: &Ctx) -> Rect {
    let no_addons = ctx.all_addons().iter().all(|a| a.manifest.home_catalogs().next().is_none());
    let (title, text) = if no_addons {
        ("Nothing on Home yet", "Home shows the catalogs your addons offer. Install one with catalogs, like Cinemeta, to fill it.")
    } else {
        ("Couldn't load anything", "None of your addons answered. Check your internet connection and try again.")
    };
    let action = if no_addons {
        Focusable::new(button("Go to Addons", theme().primary.0, theme().primary.1), |()| {
            let _ = RouterContext::get().push(Route::Addons);
        })
    } else {
        let ctx = ctx.clone();
        Focusable::new(button("Try again", theme().primary.0, theme().primary.1), move |()| {
            spawn(crate::load_rows(ctx.clone()));
        })
    };
    rect()
        .expanded()
        .center()
        .spacing(14.)
        .child(label().text(title).font_size(26.).font_weight(FontWeight::BOLD))
        .child(paragraph().width(Size::px(460.)).text_align(TextAlign::Center).font_size(16.).color(theme().muted).span(text))
        .child(action)
}

/// The Relix logo, bundled with the app.
fn logo(size: f32) -> ImageViewer {
    const LOGO: &[u8] = include_bytes!("../assets/logo.png");
    ImageViewer::new(LOGO).width(Size::px(size)).height(Size::px(size))
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

/// The Stremio look's top bar: a search field (opening Search) above every page.
pub fn top_bar(ctx: &Ctx) -> Rect {
    rect()
        .position(Position::new_absolute().top(0.).left(0.))
        .width(Size::percent(100.))
        .height(Size::px(64.))
        .padding(Gaps::new(0., 48., 0., 48.))
        .horizontal()
        .content(Content::Flex)
        .cross_align(Alignment::Center)
        .background(theme().bg)
        .child(
            Link::new(Route::Search).child(
                rect()
                    .width(Size::px(440.))
                    .height(Size::px(40.))
                    .padding(Gaps::new(0., 14., 0., 14.))
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(10.)
                    .corner_radius(8.)
                    .background(theme().surface)
                    .child(SvgViewer::new(ICON_SEARCH.as_bytes()).width(Size::px(18.)).height(Size::px(18.)).stroke(theme().muted))
                    .child(label().text("Search movies & series").font_size(14.).color(theme().muted)),
            ),
        )
        .child(rect().width(Size::flex(1.)))
        .child(fullscreen_button(ctx))
}

/// Enters or leaves fullscreen for the whole app, like Stremio's (F11 does the same).
fn fullscreen_button(ctx: &Ctx) -> Focusable {
    let icon = if *ctx.fullscreen.read() { ICON_EXIT_FULLSCREEN } else { ICON_FULLSCREEN };
    let ctx = ctx.clone();
    Focusable::new(
        rect().padding(Gaps::new_all(6.)).child(SvgViewer::new(icon.as_bytes()).width(Size::px(22.)).height(Size::px(22.)).stroke((230, 230, 230))),
        move |()| ctx.toggle_fullscreen(),
    )
}

pub const SIDEBAR_W: f32 = 80.;

/// 24px stroke icons for the Stremio sidebar; SvgViewer tints the root stroke.
const ICON_HOME: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 10.5 12 3l9 7.5V21h-6v-6H9v6H3z"/></svg>"#;
const ICON_MOVIES: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="m15.5 8.5-2 5-5 2 2-5z"/></svg>"#;
const ICON_SERIES: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="7" width="18" height="13" rx="2"/><path d="m8 3 4 4 4-4"/></svg>"#;
const ICON_LIBRARY: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3h12v18l-6-4-6 4z"/></svg>"#;
const ICON_SEARCH: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>"#;
const ICON_ADDONS: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg>"#;
const ICON_SETTINGS: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1"/><circle cx="15" cy="6" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="17" cy="18" r="2"/></svg>"#;
const ICON_FULLSCREEN: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3H5a2 2 0 0 0-2 2v3M21 8V5a2 2 0 0 0-2-2h-3M3 16v3a2 2 0 0 0 2 2h3M16 21h3a2 2 0 0 0 2-2v-3"/></svg>"#;
const ICON_EXIT_FULLSCREEN: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3v3a2 2 0 0 1-2 2H3M21 8h-3a2 2 0 0 1-2-2V3M3 16h3a2 2 0 0 1 2 2v3M16 21v-3a2 2 0 0 1 2-2h3"/></svg>"#;
const ICON_ACCOUNT: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 4-6 8-6s8 2 8 6"/></svg>"#;

/// Stremio's left rail: wordmark, then icon links; the current one sits on a grey pill.
pub fn sidebar(active: Option<NavLink>) -> Rect {
    let item = move |which: NavLink, icon: &'static str, text: &'static str, to: Route| {
        SideItem { icon, text, to, active: active == Some(which) }
    };
    rect()
        .position(Position::new_absolute().top(0.).left(0.))
        .width(Size::px(SIDEBAR_W))
        .height(Size::percent(100.))
        .spacing(4.)
        .cross_align(Alignment::Center)
        .background(theme().bg)
        .child(
            rect().height(Size::px(64.)).main_align(Alignment::Center).child(
                Link::new(Route::Home).child(logo(46.)),
            ),
        )
        .child(item(NavLink::Home, ICON_HOME, "Board", Route::Home))
        .child(item(NavLink::Movies, ICON_MOVIES, "Movies", Route::Browse { kind: "movie".into() }))
        .child(item(NavLink::Series, ICON_SERIES, "Series", Route::Browse { kind: "series".into() }))
        .child(item(NavLink::MyList, ICON_LIBRARY, "Library", Route::MyList))
        .child(item(NavLink::Addons, ICON_ADDONS, "Addons", Route::Addons))
        .child(item(NavLink::Settings, ICON_SETTINGS, "Settings", Route::Settings))
        .child(item(NavLink::Account, ICON_ACCOUNT, "Account", Route::Account))
}

/// A sidebar link: grey pill and white when current, lighter on hover.
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
        let color = use_animation_transition(self.active || *hovered.read(), |from, to| {
            let c = |bright: bool| if bright { (255, 255, 255) } else { theme().muted };
            AnimColor::new(c(from), c(to)).time(HOVER_MS)
        });
        let c: Color = (&*color.read()).into();
        Link::new(self.to.clone()).child(
            rect()
                .width(Size::px(64.))
                .padding(Gaps::new(9., 0., 7., 0.))
                .spacing(5.)
                .corner_radius(12.)
                .cross_align(Alignment::Center)
                .maybe(self.active, |el| el.background(theme().raised))
                .on_pointer_enter(move |_| hovered.set(true))
                .on_pointer_leave(move |_| hovered.set(false))
                .child(SvgViewer::new(self.icon.as_bytes()).width(Size::px(22.)).height(Size::px(22.)).stroke(c))
                .child(label().text(self.text).font_size(11.).color(c)),
        )
    }
}
