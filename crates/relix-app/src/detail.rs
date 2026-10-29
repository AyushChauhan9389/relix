use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::{Meta, MetaResponse, ResourceRequest, Stream, StreamsResponse, Video, route, seasons, stream_groups};
use tokio::task::JoinSet;

use crate::home::{backdrop, layer, title_art};
use crate::{Ctx, get, player};

const HEADER_H: f32 = 520.;
const MUTED: (u8, u8, u8) = (170, 170, 170);

/// A title's page: header, episodes (series only) and the stream list for the chosen video.
#[derive(PartialEq)]
pub struct Detail {
    pub kind: String,
    pub id: String,
}

impl Component for Detail {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let mut meta = use_state(|| None::<Meta>);
        let mut season = use_state(|| None::<u32>);
        // The movie itself, or the episode the user picked.
        let mut video = use_state(|| None::<String>);
        let mut streams = use_state(Vec::<(usize, Vec<Stream>)>::new);

        // Reruns when the addon list changes, so it waits out startup instead of finding no addons.
        // ponytail: loads once per mount; navigating detail -> detail would need a keyed reload.
        use_side_effect({
            let (kind, id, ctx) = (self.kind.clone(), self.id.clone(), ctx.clone());
            move || {
                let addons = ctx.addons.read().clone();
                let req = ResourceRequest::new("meta", &kind, &id);
                if meta.peek().is_some() || route(&addons, &req).next().is_none() {
                    return;
                }
                let (kind, id, ctx) = (kind.clone(), id.clone(), ctx.clone());
                spawn(async move {
                    let req = ResourceRequest::new("meta", &kind, &id);
                    // The first addon in list order that answers wins, like Stremio.
                    for (_, url) in route(&addons, &req) {
                        if let Ok(r) = get::<MetaResponse>(ctx.http.clone(), url).await {
                            season.set(seasons(&r.meta.videos).first().map(|(s, _)| *s));
                            if seasons(&r.meta.videos).is_empty() {
                                video.set(Some(id.clone()));
                            }
                            meta.set(Some(r.meta));
                            return;
                        }
                    }
                    eprintln!("no addon answered meta for {kind}/{id}");
                });
            }
        });

        use_side_effect({
            let (kind, ctx) = (self.kind.clone(), ctx.clone());
            move || {
                let Some(id) = video.read().clone() else { return };
                streams.set(vec![]);
                let (kind, ctx) = (kind.clone(), ctx.clone());
                spawn(async move {
                    let addons = ctx.addons.peek().clone();
                    let req = ResourceRequest::new("stream", &kind, &id);
                    let mut set = JoinSet::new();
                    for (i, addon) in addons.iter().enumerate().filter(|(_, a)| a.serves(&req)) {
                        let (http, url) = (ctx.http.clone(), req.url(addon));
                        set.spawn(async move { (i, get::<StreamsResponse>(http, url).await) });
                    }
                    while let Some(Ok((i, res))) = set.join_next().await {
                        // Drop answers for an episode the user already moved away from.
                        if video.peek().as_deref() != Some(id.as_str()) {
                            return;
                        }
                        match res {
                            Ok(r) if !r.streams.is_empty() => streams.write().push((i, r.streams)),
                            Ok(_) => {}
                            Err(e) => eprintln!("streams from addon {i}: {e}"),
                        }
                    }
                });
            }
        });

        let Some(meta) = meta.read().clone() else {
            return rect().expanded().center().child(label().text("Loading…").color(MUTED)).into_element();
        };
        let addons = ctx.addons.read().clone();
        let has_stream_addons = addons.iter().any(|a| a.serves(&ResourceRequest::new("stream", &self.kind, &self.id)));
        let groups: Vec<(String, Vec<Stream>)> = stream_groups(streams.read().clone())
            .into_iter()
            .map(|(i, s)| (addons[i].manifest.name.clone(), s))
            .collect();
        let all_seasons: Vec<(u32, Vec<Video>)> =
            seasons(&meta.videos).into_iter().map(|(n, eps)| (n, eps.into_iter().cloned().collect())).collect();
        let current_season = *season.read();
        let episodes = all_seasons.iter().find(|(n, _)| Some(*n) == current_season).map(|(_, e)| e.clone()).unwrap_or_default();
        let picked = video.read().clone();

        ScrollView::new()
            .child(header(&meta))
            .child(
                rect()
                    .horizontal()
                    .content(Content::Flex)
                    .padding(Gaps::new(0., 48., 48., 48.))
                    .spacing(32.)
                    .maybe_child((!all_seasons.is_empty()).then(|| {
                        rect()
                            .width(Size::flex(1.))
                            .spacing(12.)
                            .child(
                                rect().horizontal().spacing(8.).children(all_seasons.iter().map(|(n, _)| {
                                    let n = *n;
                                    let active = Some(n) == current_season;
                                    chip(if n == 0 { "Specials".into() } else { format!("Season {n}") }, active)
                                        .on_press(move |_| season.set(Some(n)))
                                        .into_element()
                                })),
                            )
                            .children(episodes.iter().map(|ep| {
                                let id = ep.id.clone();
                                episode_row(ep, picked.as_deref() == Some(&ep.id))
                                    .on_press(move |_| video.set(Some(id.clone())))
                                    .into_element()
                            }))
                    }))
                    .child(stream_panel(picked.is_some(), has_stream_addons, groups)),
            )
            .into_element()
    }
}

fn header(meta: &Meta) -> Rect {
    let info: Vec<String> = [meta.release_info.clone(), meta.runtime.clone(), (!meta.genres.is_empty()).then(|| meta.genres.join(" · "))]
        .into_iter()
        .flatten()
        .collect();
    rect()
        .width(Size::fill())
        .height(Size::px(HEADER_H))
        .overflow(Overflow::Clip)
        .child(backdrop(meta.background.as_deref()).layer(1))
        .child(
            layer()
                .layer(5)
                .padding(Gaps::new(96., 48., 32., 48.))
                .spacing(16.)
                .main_align(Alignment::End)
                .child(label().text("‹ Back").font_size(16.).color(MUTED).on_press(|_| RouterContext::get().go_back()))
                .child(title_art(meta.logo.as_deref(), &meta.name))
                .child(label().text(info.join("   ")).font_size(16.).color((70, 211, 105)))
                .maybe_child(meta.description.clone().map(|d| {
                    paragraph().width(Size::px(640.)).max_lines(4).text_overflow(TextOverflow::Ellipsis).font_size(17.).span(d)
                })),
        )
}

fn chip(text: String, active: bool) -> Rect {
    rect()
        .padding(Gaps::new(6., 14., 6., 14.))
        .corner_radius(16.)
        .background(if active { (255, 255, 255, 1.) } else { (60, 60, 60, 1.) })
        .child(label().text(text).font_size(14.).color(if active { (0, 0, 0) } else { (230, 230, 230) }))
}

fn episode_row(ep: &Video, picked: bool) -> Rect {
    let thumb: Element = match ep.thumbnail.as_deref().and_then(|u| Url::parse(u).ok()) {
        Some(url) => ImageViewer::new(url).width(Size::px(176.)).height(Size::px(99.)).into(),
        None => rect().width(Size::px(176.)).height(Size::px(99.)).background((50, 50, 50)).into(),
    };
    rect()
        .horizontal()
        .content(Content::Flex)
        .width(Size::fill())
        .padding(Gaps::new_all(12.))
        .spacing(16.)
        .corner_radius(6.)
        .background(if picked { (51, 51, 51) } else { (24, 24, 24) })
        .child(rect().corner_radius(4.).overflow(Overflow::Clip).child(thumb))
        .child(
            rect()
                .width(Size::flex(1.))
                .spacing(6.)
                .child(
                    label()
                        .text(format!("{}. {}", ep.episode.unwrap_or_default(), ep.title()))
                        .font_size(17.)
                        .font_weight(FontWeight::BOLD),
                )
                .maybe_child(ep.overview.clone().map(|o| {
                    paragraph().max_lines(3).text_overflow(TextOverflow::Ellipsis).font_size(14.).color(MUTED).span(o)
                })),
        )
}

fn stream_panel(picked: bool, has_stream_addons: bool, groups: Vec<(String, Vec<Stream>)>) -> Rect {
    let panel = rect().width(Size::px(460.)).spacing(10.).child(label().text("Streams").font_size(22.).font_weight(FontWeight::BOLD));
    let note = |text: &'static str| label().text(text).font_size(15.).color(MUTED);
    if !has_stream_addons {
        return panel.child(note("No stream addons installed. Add one in Addons."));
    }
    if !picked {
        return panel.child(note("Pick an episode to see its streams."));
    }
    if groups.is_empty() {
        return panel.child(note("Looking for streams…"));
    }
    panel.children(groups.into_iter().map(|(addon, streams)| {
        rect()
            .spacing(6.)
            .child(label().text(addon).font_size(14.).color(MUTED))
            .children(streams.into_iter().map(stream_card))
            .into_element()
    }))
}

fn stream_card(stream: Stream) -> Element {
    let url = stream.playable_url().map(str::to_owned);
    let headers = stream.behavior_hints.proxy_headers.clone().map(|h| h.request).unwrap_or_default();
    let name = stream.name.clone().unwrap_or_default().replace('\n', " ");
    let title = stream.title.clone().or(stream.description.clone()).unwrap_or_default();
    rect()
        .width(Size::fill())
        .padding(Gaps::new_all(12.))
        .spacing(4.)
        .corner_radius(6.)
        .background((32, 32, 32))
        .maybe(url.is_none(), |el| el.opacity(0.45))
        .map(url, |el, url| {
            el.on_press(move |_| {
                let (url, headers) = (url.clone(), headers.clone());
                // mpv blocks until its window closes, so it gets its own thread.
                std::thread::spawn(move || {
                    if let Err(e) = player::play(&url, &headers) {
                        eprintln!("mpv: {e}");
                    }
                });
            })
        })
        .child(label().text(name).font_size(15.).font_weight(FontWeight::BOLD))
        .child(paragraph().font_size(13.).color(MUTED).span(title))
        .maybe_child(
            matches!(stream.source, relix_proto::addon::StreamSource::Torrent { .. })
                .then(|| label().text("Torrent · coming soon").font_size(12.).color((229, 9, 20))),
        )
        .into()
}
