use freya::prelude::*;
use relix_proto::addon::MetaPreview;

use crate::Ctx;
use crate::focus::{use_page_scroller, use_scroller};
use crate::home::poster;
use crate::theme::theme;

/// Everything saved with "+ My List", newest first.
#[derive(PartialEq)]
pub struct MyList;

impl Component for MyList {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let page = use_scroller();
        use_page_scroller(page);
        // Re-read whenever Home's local rows change (adds, removals, sync).
        let _ = ctx.rows.read();
        let new = ctx.new_episodes.read().clone();
        let titles: Vec<MetaPreview> = ctx
            .store
            .list::<Option<MetaPreview>>(&ctx.profile, "library")
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, m)| m)
            .collect();

        page.wrap(
            ScrollView::new_controlled(page.controller).scroll_with_arrows(false).child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(theme().top, 48., 48., 48.))
                    .spacing(16.)
                    .child(label().text("My List").font_size(32.).font_weight(FontWeight::BOLD))
                    .maybe(titles.is_empty(), |el| {
                        el.child(label().text("Titles you add with \"+ My List\" show up here.").color(theme().muted))
                    })
                    .child(
                        rect()
                            .horizontal()
                            .content(Content::wrap_spacing(16.))
                            .spacing(8.)
                            .children(titles.iter().map(|m| poster(m, None, new.contains(&m.id), None))),
                    ),
            ),
        )
    }
}
