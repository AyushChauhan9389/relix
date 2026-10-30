use std::time::Duration;

use freya::prelude::*;
use relix_proto::addon::{CatalogResponse, search_requests};
use tokio::task::JoinSet;

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

        use_side_effect({
            let ctx = ctx.clone();
            move || {
                let q = query.read().clone();
                let ctx = ctx.clone();
                spawn(async move {
                    tokio::time::sleep(DEBOUNCE).await;
                    // Typing continued; a newer run handles it.
                    if *query.peek() != q {
                        return;
                    }
                    let addons = ctx.all_addons();
                    let mut next = vec![];
                    let mut set = JoinSet::new();
                    for (i, c, req) in search_requests(&addons, &q) {
                        let id = format!("{i}/{}/{}", c.r#type, c.id);
                        let kind = if c.r#type == "movie" { "Movies".into() } else if c.r#type == "series" { "Series".into() } else { c.r#type.clone() };
                        next.push(Row { id: id.clone(), title: format!("{} · {kind}", addons[i].manifest.name), metas: vec![], progress: vec![], loading: true });
                        let (http, url) = (ctx.http.clone(), req.url(&addons[i]));
                        set.spawn(async move { (id, get::<CatalogResponse>(http, url).await) });
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
                            Err(e) => eprintln!("search {id}: {e}"),
                        }
                    }
                    // Hide catalogs with no matches once they've answered.
                    rows.write().retain(|r| !r.metas.is_empty());
                });
            }
        });

        let results = rows.read().clone();
        page.wrap(
            ScrollView::new_controlled(page.controller)
                .scroll_with_arrows(false)
                .child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new(theme().top, 48., 16., 48.))
                        .child(Input::new(query).width(Size::fill()).placeholder("Titles, people, genres").auto_focus(true)),
                )
                .maybe(*searched.read() && results.is_empty(), |el| {
                    el.child(rect().padding(Gaps::new(24., 48., 0., 48.)).child(label().text("No matches yet.").color(theme().muted)))
                })
                .children(results.into_iter().map(|row| PosterRow { row }.into_element())),
        )
    }
}
