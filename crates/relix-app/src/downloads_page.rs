use freya::prelude::*;
use freya::router::RouterContext;

use crate::downloads::{self, Download, Status};
use crate::focus::{Focusable, use_page_scroller, use_scroller};
use crate::home::button;
use crate::theme::theme;
use crate::{Ctx, Route};

/// This device's downloads: progress, pause, resume, delete, and Play for the finished ones.
#[derive(PartialEq)]
pub struct Downloads;

impl Component for Downloads {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let page = use_scroller();
        use_page_scroller(page);
        let list = ctx.downloads.read().clone();
        let used: u64 = list.iter().map(|d| if d.status == Status::Done { d.size.unwrap_or(d.done) } else { d.done }).sum();
        let folder = downloads::folder(&ctx.store);

        page.wrap(
            ScrollView::new_controlled(page.controller).scroll_with_arrows(false).drag_scrolling(false).child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(crate::theme::top(), crate::theme::side(), 48., crate::theme::side()))
                    .spacing(12.)
                    .child(label().text("Downloads").font_size(32.).font_weight(FontWeight::BOLD))
                    .child(
                        label()
                            .text(if list.is_empty() {
                                "Nothing downloaded yet. On a title's page, the arrow beside a stream saves it for watching offline.".to_owned()
                            } else {
                                format!("{} in {}", downloads::size_text(used), folder.display())
                            })
                            .font_size(14.)
                            .color(theme().muted),
                    )
                    .children(list.into_iter().map(|d| row(&ctx, d).into_element())),
            ),
        )
    }
}

fn row(ctx: &Ctx, d: Download) -> Rect {
    let fraction = d.size.filter(|s| *s > 0).map(|s| (d.done as f32 / s as f32).clamp(0., 1.));
    let status = match &d.status {
        Status::Done => format!("Downloaded · {}", downloads::size_text(d.size.unwrap_or(d.done))),
        Status::Running => match d.size {
            Some(size) => format!("Downloading · {} of {} · {:.0}%", downloads::size_text(d.done), downloads::size_text(size), fraction.unwrap_or(0.) * 100.),
            None => "Starting…".to_owned(),
        },
        Status::Queued => "Waiting for the downloads before it".to_owned(),
        Status::Paused => match d.size {
            Some(size) => format!("Paused · {} of {}", downloads::size_text(d.done), downloads::size_text(size)),
            None => "Paused".to_owned(),
        },
        Status::Failed(why) => format!("Failed: {why}"),
    };
    let action = |text: &'static str, on: Box<dyn Fn()>| Focusable::new(button(text, theme().raised, (255, 255, 255)), move |()| on()).into_element();
    let (c, v) = (ctx.clone(), d.video.clone());
    let mut actions: Vec<Element> = match d.status {
        Status::Done => {
            let (id, kind, video) = (d.meta.id.clone(), d.meta.r#type.clone(), d.video.clone());
            vec![Focusable::new(button("Play", theme().primary.0, theme().primary.1), {
                let ctx = ctx.clone();
                move |()| {
                    // Its page plays this video once it opens, offline too (detail.rs).
                    let (mut title, mut wanted) = (ctx.autoplay_title, ctx.autoplay_video);
                    title.set(Some(id.clone()));
                    wanted.set(Some(video.clone()));
                    drop(RouterContext::get().push(Route::Detail { kind: kind.clone(), id: id.clone() }));
                }
            })
            .into_element()]
        }
        Status::Running | Status::Queued => vec![action("Pause", Box::new(move || downloads::pause(&c, &v)))],
        Status::Paused | Status::Failed(_) => vec![action("Resume", Box::new(move || downloads::resume(&c, &v)))],
    };
    let (c, v) = (ctx.clone(), d.video.clone());
    actions.push(action("Delete", Box::new(move || downloads::remove(&c, &v))));
    let poster = d.meta.poster.as_deref().and_then(crate::image_url);
    rect()
        .width(Size::fill())
        .max_width(Size::px(900.))
        .padding(Gaps::new_all(12.))
        .horizontal()
        .content(Content::Flex)
        .cross_align(Alignment::Center)
        .spacing(16.)
        .corner_radius(theme().radius)
        .background(theme().surface)
        .child(
            rect().width(Size::px(60.)).height(Size::px(90.)).corner_radius(6.).overflow(Overflow::Clip).background(theme().placeholder).maybe_child(
                poster.map(|url| ImageViewer::new(url).width(Size::px(60.)).height(Size::px(90.))),
            ),
        )
        .child(
            rect()
                .width(Size::flex(1.))
                .spacing(6.)
                .child(label().text(d.meta.name.clone()).font_size(17.).font_weight(FontWeight::BOLD).max_lines(1).text_overflow(TextOverflow::Ellipsis))
                .maybe(!d.episode.is_empty(), |el| el.child(label().text(d.episode.clone()).font_size(14.).max_lines(1).text_overflow(TextOverflow::Ellipsis)))
                .child(paragraph().width(Size::fill()).font_size(13.).color(if matches!(d.status, Status::Failed(_)) { crate::theme::ERROR } else { theme().muted }).span(status))
                .maybe(d.status != Status::Done && fraction.is_some(), |el| {
                    el.child(
                        rect()
                            .width(Size::fill())
                            .height(Size::px(4.))
                            .corner_radius(2.)
                            .background(theme().track)
                            .child(rect().width(Size::percent(fraction.unwrap_or(0.) * 100.)).height(Size::fill()).corner_radius(2.).background(theme().accent)),
                    )
                }),
        )
        .child(rect().horizontal().spacing(8.).children(actions))
}
