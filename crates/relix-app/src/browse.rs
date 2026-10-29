use freya::prelude::*;
use relix_proto::addon::{Catalog, CatalogResponse, MetaPreview, discover_catalogs};

use crate::focus::{Focusable, use_page_scroller, use_scroller};
use crate::home::{button, chip, poster};
use crate::{Ctx, get};

const MUTED: (u8, u8, u8) = (170, 170, 170);

/// Discover: browse one catalog of a type at a time, filtered by genre, a page at a time.
#[derive(PartialEq)]
pub struct Browse {
    pub kind: String,
}

impl Component for Browse {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let page = use_scroller();
        use_page_scroller(page);
        // Movies <-> Series reuses this component with a new `kind`.
        let kind = use_reactive(&self.kind);
        let mut last_kind = use_state(|| self.kind.clone());
        let mut picked = use_state(|| 0usize);
        let mut genre = use_state(|| None::<String>);
        let mut items = use_state(Vec::<MetaPreview>::new);
        let mut more = use_state(|| false);

        let catalogs = move |ctx: &Ctx| -> Vec<(usize, Catalog)> {
            let addons = ctx.addons.read().clone();
            discover_catalogs(&addons, &kind.read()).map(|(i, c)| (i, c.clone())).collect()
        };

        // Fetches a page of the chosen catalog: page 0 replaces the grid, later pages append.
        let load = {
            let ctx = ctx.clone();
            move |skip: usize| {
                let ctx = ctx.clone();
                let Some((i, c)) = catalogs(&ctx).get(*picked.peek()).cloned() else { return };
                let g = genre.peek().clone();
                let addons = ctx.addons.peek().clone();
                let url = c.discover_request(g.as_deref(), skip).url(&addons[i]);
                spawn(async move {
                    match get::<CatalogResponse>(ctx.http.clone(), url).await {
                        Ok(r) => {
                            more.set(!r.metas.is_empty());
                            if skip == 0 {
                                items.set(r.metas);
                            } else {
                                items.write().extend(r.metas);
                            }
                        }
                        Err(e) => eprintln!("browse: {e}"),
                    }
                });
            }
        };

        use_side_effect({
            let (ctx, load) = (ctx.clone(), load.clone());
            move || {
                let k = kind.read().clone();
                let _ = (picked.read(), genre.read(), ctx.addons.read());
                if *last_kind.peek() != k {
                    // A new type starts from its first catalog, unfiltered; those writes rerun this.
                    last_kind.set(k);
                    picked.set(0);
                    genre.set(None);
                    return;
                }
                items.set(vec![]);
                load(0);
            }
        });

        let list = catalogs(&ctx);
        let current = list.get(*picked.read()).map(|(_, c)| c.clone());
        let addons = ctx.addons.read().clone();
        let several_addons = list.iter().any(|(i, _)| *i != list.first().map(|f| f.0).unwrap_or_default());
        let chosen_genre = genre.read().clone();
        let grid = items.read().clone();
        let title = match self.kind.as_str() {
            "movie" => "Movies".to_owned(),
            "series" => "Series".to_owned(),
            other => other.to_owned(),
        };

        page.wrap(
            ScrollView::new_controlled(page.controller).scroll_with_arrows(false).child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(100., 48., 48., 48.))
                    .spacing(16.)
                    .child(label().text(title).font_size(32.).font_weight(FontWeight::BOLD))
                    .child(rect().horizontal().content(Content::wrap_spacing(8.)).spacing(8.).children(list.iter().enumerate().map(
                        |(n, (i, c))| {
                            let name = c.name.clone().unwrap_or_else(|| c.id.clone());
                            let text = if several_addons { format!("{name} · {}", addons[*i].manifest.name) } else { name };
                            Focusable { radius: 16., ..Focusable::new(chip(text, n == *picked.read()), move |()| {
                                genre.set(None);
                                picked.set(n);
                            }) }
                            .into_element()
                        },
                    )))
                    .map(current.filter(|c| !c.genres().is_empty()), |el, c| {
                        // "All" only when the catalog works without a genre.
                        let all = c.discover_request(None, 0).extra.is_empty();
                        let default = c.discover_request(None, 0).extra.first().map(|(_, g)| g.clone());
                        let active = chosen_genre.clone().or(default);
                        el.child(
                            rect()
                                .horizontal()
                                .content(Content::wrap_spacing(8.))
                                .spacing(8.)
                                .maybe(all, |el| {
                                    el.child(Focusable { radius: 16., ..Focusable::new(chip("All".into(), active.is_none()), move |()| genre.set(None)) })
                                })
                                .children(c.genres().iter().map(|g| {
                                    let g2 = g.clone();
                                    Focusable { radius: 16., ..Focusable::new(chip(g.clone(), active.as_deref() == Some(g)), move |()| genre.set(Some(g2.clone()))) }
                                        .into_element()
                                })),
                        )
                    })
                    .maybe(list.is_empty(), |el| el.child(label().text("No addon has a browsable catalog for this type.").color(MUTED)))
                    .child(
                        rect()
                            .horizontal()
                            .content(Content::wrap_spacing(16.))
                            .spacing(8.)
                            .children(grid.iter().map(|m| poster(m, None, None))),
                    )
                    .maybe(*more.read(), |el| {
                        let skip = grid.len();
                        el.child(Focusable::new(button("Load more", (60, 60, 60), (255, 255, 255)), move |()| load(skip)))
                    }),
            ),
        )
    }
}
