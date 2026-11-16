use std::time::Duration;

use freya::prelude::*;
use relix_proto::addon::{CatalogResponse, search_requests};
use crate::net::Tasks as JoinSet;

use crate::focus::{use_page_scroller, use_scroller};
use crate::home::{PosterRow, Row};
use crate::{Ctx, get};
use crate::theme::theme;

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Searches every catalog that supports it: one row per catalog, filled as each answers.
#[derive(PartialEq)]
pub struct Search;

impl Component for Search {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let page = use_scroller();
        use_page_scroller(page);
        let query = use_state(String::new);
        let mut rows = use_state(Vec::<Row>::new);
        let mut searched = use_state(|| false);
        let mut recent = use_state(|| recent_searches(&ctx));

        use_side_effect({
            let ctx = ctx.clone();
            move || {
                let q = query.read().clone();
                let ctx = ctx.clone();
                spawn(async move {
                    crate::net::sleep(DEBOUNCE).await;
                    // Typing continued; a newer run handles it.
                    if *query.peek() != q {
                        return;
                    }
                    let addons = ctx.all_addons();
                    let mut next = vec![];
                    let mut set = JoinSet::new();
                    // TMDB first, as on Browse: the fullest catalog.
                    if crate::tmdb::enabled() && !q.trim().is_empty() {
                        for (kind, name) in [("movie", "Movies"), ("series", "Series")] {
                            let id = format!("tmdb/{kind}");
                            next.push(Row { id: id.clone(), title: format!("TMDB · {name}"), metas: vec![], progress: vec![], loading: true, next: None });
                            let (http, q) = (ctx.http.clone(), q.clone());
                            set.spawn(async move { (id, crate::tmdb::search(&http, kind, &q).await.map(|metas| CatalogResponse { metas, cache: Default::default() })) });
                        }
                    }
                    for (i, c, req) in search_requests(&addons, &q) {
                        let id = format!("{i}/{}/{}", c.r#type, c.id);
                        let kind = if c.r#type == "movie" { "Movies".into() } else if c.r#type == "series" { "Series".into() } else { c.r#type.clone() };
                        let more = c.supports("skip").then(|| (addons[i].clone(), req.clone()));
                        next.push(Row { id: id.clone(), title: format!("{} · {kind}", addons[i].manifest.name), metas: vec![], progress: vec![], loading: true, next: more });
                        let (http, url) = (ctx.http.clone(), req.url(&addons[i]));
                        set.spawn(async move { (id, get::<CatalogResponse>(http, url).await.map_err(|e| e.to_string())) });
                    }
                    searched.set(!next.is_empty());
                    rows.set(next);
                    while let Some(Ok((id, res))) = set.join_next().await {
                        if *query.peek() != q {
                            return;
                        }
                        let Some(i) = rows.peek().iter().position(|r| r.id == id) else { continue };
                        let mut rows = rows.write();
                        rows[i].loading = false;
                        match res {
                            Ok(r) => rows[i].metas = r.metas,
                            Err(e) => crate::log!("search {id}: {e}"),
                        }
                    }
                    // Hide catalogs with no matches once they've answered.
                    rows.write().retain(|r| !r.metas.is_empty());
                    if !rows.peek().is_empty() {
                        remember_search(&ctx, &q);
                        recent.set(recent_searches(&ctx));
                    }
                });
            }
        });

        let results = rows.read().clone();
        page.wrap(
            ScrollView::new_controlled(page.controller)
                .scroll_with_arrows(false)
                .drag_scrolling(false)
                .child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new(
                            crate::theme::top(),
                            crate::theme::side(),
                            16.,
                            crate::theme::side(),
                        ))
                        .child(crate::focus::Field(
                            Input::new(query)
                                .width(Size::fill())
                                .placeholder("Titles, people, genres")
                                .auto_focus(true),
                        )),
                )
                // An empty box offers the last searches.
                .maybe(query.read().trim().is_empty() && !recent.read().is_empty(), |el| {
                    let mut query = query;
                    let chips: Vec<Element> = recent.read().iter().map(|q| {
                        let q = q.clone();
                        crate::focus::Focusable {
                            radius: 999.,
                            ..crate::focus::Focusable::new(
                                rect().padding(Gaps::new(8., 16., 8., 16.)).corner_radius(999.).background(theme().raised).child(label().text(q.clone()).font_size(14.)),
                                move |()| query.set(q.clone()),
                            )
                        }
                        .into_element()
                    }).collect();
                    let clear = {
                        let ctx = ctx.clone();
                        crate::focus::Focusable::new(label().text("Clear").font_size(14.).color(theme().muted), move |()| {
                            let _ = ctx.store.set_meta(RECENT, None);
                            recent.set(vec![]);
                        })
                    };
                    el.child(
                        rect()
                            .padding(Gaps::new(0., crate::theme::side(), 8., crate::theme::side()))
                            .spacing(10.)
                            .child(rect().horizontal().spacing(16.).cross_align(Alignment::Center).child(label().text("Recent searches").font_size(13.).color(theme().muted)).child(clear))
                            .child(rect().width(Size::fill()).horizontal().content(Content::wrap_spacing(8.)).spacing(8.).children(chips)),
                    )
                })
                .maybe(*searched.read() && results.is_empty(), |el| {
                    el.child(rect().padding(Gaps::new(24., crate::theme::side(), 0., crate::theme::side())).child(label().text("No matches yet.").color(theme().muted)))
                })
                .children(results.into_iter().map(|row| PosterRow { row: std::rc::Rc::new(row) }.into_element())),
        )
    }
}

/// Where recent searches are kept (the store's meta table, this device only).
const RECENT: &str = "recent_searches";

fn recent_searches(ctx: &Ctx) -> Vec<String> {
    ctx.store.meta(RECENT).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

/// Puts `q` first among the last 8 searches (once, ignoring case).
fn remember_search(ctx: &Ctx, q: &str) {
    let list = with_search(recent_searches(ctx), q);
    if let Err(e) = serde_json::to_string(&list).map_err(|e| e.to_string()).and_then(|j| ctx.store.set_meta(RECENT, Some(&j)).map_err(|e| e.to_string())) {
        crate::log!("saving recent searches: {e}");
    }
}

fn with_search(mut list: Vec<String>, q: &str) -> Vec<String> {
    let q = q.trim();
    list.retain(|s| !s.eq_ignore_ascii_case(q));
    list.insert(0, q.to_owned());
    list.truncate(8);
    list
}

#[cfg(test)]
mod tests {
    #[test]
    fn recent_searches_newest_first_once() {
        let list = super::with_search(vec!["Dune".into(), "Stree".into()], " stree ");
        assert_eq!(list, ["stree", "Dune"]);
        let full: Vec<String> = (0..8).map(|i| i.to_string()).collect();
        assert_eq!(super::with_search(full, "new").len(), 8);
    }
}
