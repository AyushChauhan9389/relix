use std::collections::HashSet;

use freya::prelude::*;
use relix_proto::addon::{Catalog, CatalogResponse, MetaPreview, discover_catalogs};
use tokio::task::JoinSet;

use crate::filter::{Filter, Sort, apply, year_bounds};
use crate::focus::{Focusable, Scroller, use_page_scroller, use_scroller};
use crate::home::{chip, poster};
use crate::motion::{Placeholder, busy};
use crate::theme::theme;
use crate::{Ctx, get};

const POSTER_W: f32 = 150.;
const POSTER_H: f32 = 225.;

/// Discover: one catalog of a type at a time, narrowed by genres (several, all or any), years,
/// rating and watched state, and sorted; loads a page at a time.
#[derive(PartialEq)]
pub struct Browse {
    pub kind: String,
}

impl Component for Browse {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let page = use_scroller();
        let (catalog_row, genre_row) = (use_scroller(), use_scroller());
        use_page_scroller(page);
        // Movies <-> Series reuses this component with a new `kind`.
        let kind = use_reactive(&self.kind);
        let mut last_kind = use_state(|| self.kind.clone());
        let mut picked = use_state(|| 0usize);
        let mut filter = use_state(Filter::default);
        let mut items = use_state(Vec::<MetaPreview>::new);
        // Items received per request so far: the next page's `skip` for each.
        let mut skips = use_state(Vec::<usize>::new);
        let mut more = use_state(|| false);
        let mut loading = use_state(|| true);
        // Bumped on every fresh load, so answers for an older filter are dropped.
        let mut generation = use_state(|| 0u64);

        let catalogs = move |ctx: &Ctx| -> Vec<(usize, Catalog)> {
            let addons = ctx.all_addons();
            discover_catalogs(&addons, &kind.read()).map(|(i, c)| (i, c.clone())).collect()
        };

        // Fetches the next page for every genre request; a fresh load (`reset`) replaces the grid.
        let mut load = {
            let ctx = ctx.clone();
            move |reset: bool| {
                let ctx = ctx.clone();
                let Some((i, c)) = catalogs(&ctx).get(*picked.peek()).cloned() else {
                    loading.set(false);
                    return;
                };
                if reset {
                    let next = *generation.peek() + 1;
                    generation.set(next);
                    items.set(vec![]);
                }
                let gen_now = *generation.peek();
                let genres = requested_genres(&c, &filter.peek());
                let page_skips: Vec<usize> = if reset { vec![0; genres.len()] } else { skips.peek().clone() };
                let addons = ctx.all_addons();
                loading.set(true);
                spawn(async move {
                    let mut set = JoinSet::new();
                    for (n, g) in genres.iter().enumerate() {
                        let url = c.discover_request(g.as_deref(), page_skips[n]).url(&addons[i]);
                        let http = ctx.http.clone();
                        set.spawn(async move { (n, get::<CatalogResponse>(http, url).await) });
                    }
                    let mut pages = vec![vec![]; genres.len()];
                    while let Some(Ok((n, res))) = set.join_next().await {
                        match res {
                            Ok(r) => pages[n] = r.metas,
                            Err(e) => eprintln!("browse: {e}"),
                        }
                    }
                    if *generation.peek() != gen_now {
                        return;
                    }
                    more.set(pages.iter().any(|p| !p.is_empty()));
                    skips.set(page_skips.iter().zip(&pages).map(|(s, p)| s + p.len()).collect());
                    // Several genres (match any): merge their pages, each title once.
                    let mut seen: HashSet<String> = items.peek().iter().map(|m| m.id.clone()).collect();
                    let fresh: Vec<MetaPreview> = pages.into_iter().flatten().filter(|m| seen.insert(m.id.clone())).collect();
                    items.write().extend(fresh);
                    loading.set(false);
                });
            }
        };

        use_side_effect({
            let (ctx, mut load) = (ctx.clone(), load.clone());
            move || {
                let k = kind.read().clone();
                let _ = (picked.read(), ctx.all_addons());
                // Only genre changes need new requests; other filters work on what's loaded.
                let _ = (filter.read().genres.clone(), filter.read().match_any);
                if *last_kind.peek() != k {
                    // A new type starts from its first catalog, unfiltered; those writes rerun this.
                    last_kind.set(k);
                    picked.set(0);
                    filter.set(Filter::default());
                    return;
                }
                load(true);
            }
        });

        let list = catalogs(&ctx);
        let current = list.get(*picked.read()).map(|(_, c)| c.clone());
        let addons = ctx.all_addons();
        let several_addons = list.iter().any(|(i, _)| *i != list.first().map(|f| f.0).unwrap_or_default());
        let f = filter.read().clone();
        let loaded = items.read().clone();
        let watched: HashSet<String> =
            ctx.store.list::<bool>(&ctx.profile, "watched").unwrap_or_default().into_iter().map(|(k, _)| k).collect();
        let shown = apply(&loaded, &f, &watched);
        let title = match self.kind.as_str() {
            "movie" => "Movies".to_owned(),
            "series" => "Series".to_owned(),
            other => other.to_owned(),
        };
        let first_load = *loading.read() && loaded.is_empty();

        let grid = if first_load {
            rect().horizontal().content(Content::wrap_spacing(16.)).spacing(8.).children((0..16).map(|_| {
                rect().spacing(8.).child(Placeholder::new(Size::px(POSTER_W), POSTER_H)).into_element()
            }))
        } else {
            rect().horizontal().content(Content::wrap_spacing(16.)).spacing(8.).children(shown.iter().map(|m| poster(m, None, false, None)))
        };

        page.wrap(
            ScrollView::new_controlled(page.controller).scroll_with_arrows(false).child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(theme().top, 48., 48., 48.))
                    .spacing(14.)
                    .child(label().text(title).font_size(32.).font_weight(FontWeight::BOLD))
                    .child(chip_row(catalog_row, list.iter().enumerate().map(
                        |(n, (i, c))| {
                            let name = c.name.clone().unwrap_or_else(|| c.id.clone());
                            let text = if several_addons { format!("{name} · {}", addons[*i].manifest.name) } else { name };
                            Focusable { radius: 16., row: Some(catalog_row), ..Focusable::new(chip(text, n == *picked.read()), move |()| {
                                filter.set(Filter::default());
                                picked.set(n);
                            }) }
                            .into_element()
                        },
                    ).collect()))
                    .map(current.clone().filter(|c| !c.genres().is_empty()), |el, c| el.child(genre_chips(&c, &f, filter, genre_row)))
                    .maybe(!list.is_empty(), |el| el.child(filter_bar(&loaded, &f, filter, current.as_ref())))
                    .maybe(list.is_empty(), |el| el.child(label().text("No addon has a browsable catalog for this type.").color(theme().muted)))
                    .maybe(!first_load && !loaded.is_empty(), |el| {
                        el.child(label().text(format!("{} titles", shown.len())).font_size(13.).color(theme().muted))
                    })
                    .maybe(!first_load && shown.is_empty() && !*loading.read() && !list.is_empty(), |el| {
                        el.child(
                            label()
                                .text(if loaded.is_empty() { "Nothing in this catalog." } else { "No titles match these filters. Load more or loosen them." })
                                .color(theme().muted),
                        )
                    })
                    .child(grid)
                    .maybe(*loading.read() && !loaded.is_empty(), |el| el.child(busy("Loading more…")))
                    .maybe(*more.read() && !*loading.read(), |el| {
                        el.child(Focusable::new(crate::home::button("Load more", theme().raised, (255, 255, 255)), move |()| load(false)))
                    }),
            ),
        )
    }
}

/// The genre values to request: one per request. Catalogs that require a genre use one
/// (the chosen or default); otherwise none (whole catalog), the first (match all, the rest
/// filtered here), or each chosen genre (match any, merged).
fn requested_genres(c: &Catalog, f: &Filter) -> Vec<Option<String>> {
    let required = !c.discover_request(None, 0).extra.is_empty();
    match (&f.genres[..], required, f.match_any) {
        ([], true, _) => vec![None],
        ([], false, _) => vec![None],
        ([first, ..], true, _) | ([first, ..], false, false) => vec![Some(first.clone())],
        (several, false, true) => several.iter().cloned().map(Some).collect(),
    }
}

/// Genre chips: toggles (several at once) with an All/Any switch, or a single choice when the
/// catalog requires exactly one genre (e.g. Cinemeta's "New", where genres are years).
fn genre_chips(c: &Catalog, f: &Filter, mut filter: State<Filter>, row: Scroller) -> Element {
    let required = !c.discover_request(None, 0).extra.is_empty();
    let default = c.discover_request(None, 0).extra.first().map(|(_, g)| g.clone());
    let chip_for = |text: String, on: bool, pick: Box<dyn Fn(&mut Filter)>| {
        Focusable { radius: 16., row: Some(row), ..Focusable::new(chip(text, on), move |()| pick(&mut filter.write())) }.into_element()
    };
    let mut chips = vec![];
    if required {
        let active = f.genres.first().cloned().or(default);
        for g in c.genres() {
            let g2 = g.clone();
            chips.push(chip_for(g.clone(), active.as_deref() == Some(g), Box::new(move |f| f.genres = vec![g2.clone()])));
        }
    } else {
        chips.push(chip_for("All".into(), f.genres.is_empty(), Box::new(|f| f.genres.clear())));
        for g in c.genres() {
            let (g2, on) = (g.clone(), f.genres.contains(g));
            chips.push(chip_for(g.clone(), on, Box::new(move |f| {
                if let Some(at) = f.genres.iter().position(|x| *x == g2) {
                    f.genres.remove(at);
                } else {
                    f.genres.push(g2.clone());
                }
            })));
        }
    }
    let match_switch = (!required && f.genres.len() > 1).then(|| {
        segmented(&[("Match all", !f.match_any), ("Match any", f.match_any)], move |n| filter.write().match_any = n == 1)
    });
    rect()
        .width(Size::fill())
        .spacing(10.)
        .child(chip_row(row, chips))
        .maybe_child(match_switch)
        .into()
}

/// Year range, minimum rating, sort and hide-watched; choices come from what's loaded.
fn filter_bar(loaded: &[MetaPreview], f: &Filter, mut filter: State<Filter>, current: Option<&Catalog>) -> Rect {
    let bounds = year_bounds(loaded);
    let years: Vec<u32> = bounds.map(|(lo, hi)| (lo..=hi).rev().collect()).unwrap_or_default();
    // A catalog whose genres are years (e.g. "New") makes a year range pointless.
    let genres_are_years = current.is_some_and(|c| c.genres().first().is_some_and(|g| g.parse::<u32>().is_ok()));
    let (from, to) = f.years.or(bounds).unwrap_or_default();
    let year_select = |value: u32, set: fn(&mut Filter, u32, Option<(u32, u32)>)| {
        let years = years.clone();
        Select::new().selected_item(value.to_string()).children(years.into_iter().map(move |y| {
            MenuItem::new().selected(y == value).on_press(move |_| set(&mut filter.write(), y, bounds)).child(y.to_string()).into_element()
        }))
    };
    let ratings = [(None, "Any"), (Some(6.), "6+"), (Some(7.), "7+"), (Some(8.), "8+")];

    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(24.)
        .maybe(!years.is_empty() && !genres_are_years, |el| {
            el.child(field(
                "Years",
                rect()
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(8.)
                    .child(year_select(from, |f, y, b| {
                        let to = f.years.or(b).map(|(_, t)| t).unwrap_or(y);
                        f.years = Some((y.min(to), to.max(y)));
                    }))
                    .child(label().text("–").color(theme().muted))
                    .child(year_select(to, |f, y, b| {
                        let from = f.years.or(b).map(|(fr, _)| fr).unwrap_or(y);
                        f.years = Some((from.min(y), y.max(from)));
                    }))
                    .into(),
            ))
        })
        .child(field(
            "Rating",
            segmented(&ratings.map(|(r, t)| (t, f.min_rating == r)), move |n| filter.write().min_rating = ratings[n].0).into(),
        ))
        .child(field(
            "Sort",
            Select::new()
                .selected_item(Sort::ALL.iter().find(|(s, _)| *s == f.sort).map(|(_, t)| *t).unwrap_or_default())
                .children(Sort::ALL.map(|(s, t)| {
                    MenuItem::new().selected(s == f.sort).on_press(move |_| filter.write().sort = s).child(t).into_element()
                }))
                .into(),
        ))
        .child(field(
            "Watched",
            Focusable {
                radius: 16.,
                ..Focusable::new(chip("Hide watched".into(), f.hide_watched), move |()| {
                    let on = filter.peek().hide_watched;
                    filter.write().hide_watched = !on;
                })
            }
            .into(),
        ))
        .maybe(f.is_active(), |el| {
            el.child(field("", Focusable::new(label().text("Clear").font_size(14.).color(theme().muted), move |()| filter.set(Filter::default())).into()))
        })
}

/// A small label above a control.
fn field(name: &'static str, control: Element) -> Rect {
    rect().spacing(6.).child(label().text(name).font_size(12.).color(theme().muted)).child(control)
}

/// Joined toggle buttons; `on_pick` gets the chosen index.
fn segmented(options: &[(&'static str, bool)], on_pick: impl FnMut(usize) + Clone + 'static) -> Rect {
    rect()
        .horizontal()
        .padding(Gaps::new_all(3.))
        .spacing(3.)
        .corner_radius(8.)
        .background(theme().raised)
        .children(options.iter().enumerate().map(|(n, (text, on))| {
            let mut on_pick = on_pick.clone();
            Focusable {
                radius: 6.,
                ..Focusable::new(
                    rect()
                        .padding(Gaps::new(6., 12., 6., 12.))
                        .corner_radius(6.)
                        .maybe(*on, |el| el.background((255, 255, 255)))
                        .child(label().text(*text).font_size(13.).font_weight(FontWeight::BOLD).color(if *on { (0, 0, 0) } else { theme().muted })),
                    move |()| on_pick(n),
                )
            }
            .into_element()
        }))
}

/// One row of chips that scrolls sideways (wrapping squeezes the last chip on a line).
fn chip_row(row: Scroller, chips: Vec<Element>) -> Element {
    // `wrap` fills its parent, so the parent fixes the row's height.
    rect()
        .width(Size::fill())
        .height(Size::px(42.))
        .child(row.wrap(
            ScrollView::new_controlled(row.controller)
                .direction(Direction::Horizontal)
                .scroll_with_arrows(false)
                .show_scrollbar(false)
                .child(rect().horizontal().padding(Gaps::new_all(4.)).spacing(8.).children(chips)),
        ))
        .into()
}
