use std::collections::{HashMap, HashSet};

use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::{
    Meta, MetaPreview, MetaResponse, ResourceRequest, Stream, StreamsResponse, SubtitlesResponse, Video, route, seasons, stream_groups,
    pick_binge_stream, subtitle_requests, without_torrents,
};
use tokio::task::JoinSet;

use crate::focus::{Focusable, use_page_scroller, use_scroller};
use crate::home::{backdrop, button, chip, layer, title_art};
use crate::library::{WatchState, is_finished, next_video, now, preview, record_playback};
use crate::player::Ended;
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
        let page = use_scroller();
        use_page_scroller(page);
        let mut meta = use_state(|| None::<Meta>);
        let mut season = use_state(|| None::<u32>);
        // The movie itself, or the episode the user picked.
        let mut video = use_state(|| None::<String>);
        let mut streams = use_state(Vec::<(usize, Vec<Stream>)>::new);
        // Until every addon has answered for the chosen video.
        let mut searching = use_state(|| false);
        let (store, profile) = (ctx.store.clone(), ctx.profile.clone());
        let mut watch = use_state(|| store.get::<WatchState>(&profile, "progress", &self.id).ok().flatten());
        let mut watched = use_state(|| {
            let prefix = format!("{}:", self.id);
            store.list::<bool>(&profile, "watched").unwrap_or_default().into_iter()
                .filter(|(k, v)| *v && (*k == self.id || k.starts_with(&prefix)))
                .map(|(k, _)| k)
                .collect::<HashSet<String>>()
        });
        let mut in_list = use_state(|| {
            store.get::<Option<MetaPreview>>(&profile, "library", &self.id).ok().flatten().flatten().is_some()
        });

        // Plays a stream of the chosen video, resuming where it stopped, and records where it stops.
        // When an episode ends (or "Next episode" is pressed) it moves on, auto-playing the next
        // episode's stream from the same release when an addon has one.
        let play = EventHandler::new({
            let ctx = ctx.clone();
            move |first: Play| {
                let (Some(m), Some(first_vid)) = (meta.peek().clone(), video.peek().clone()) else { return };
                let ctx = ctx.clone();
                spawn(async move {
                    let mut playing = ctx.player;
                    // Requests from the player's controls that Relix handles (fullscreen, Esc).
                    let (to_app, mut from_player) = tokio::sync::mpsc::unbounded_channel();
                    spawn({
                        let ctx = ctx.clone();
                        async move {
                            while let Some(msg) = from_player.recv().await {
                                match msg {
                                    player::Message::ToggleFullscreen => ctx.toggle_fullscreen(),
                                    player::Message::Escape => ctx.player_escape(),
                                    player::Message::Next => {}
                                }
                            }
                        }
                    });
                    let (mut req, mut vid) = (first, first_vid);
                    async {
                    loop {
                        let start = watch.peek().as_ref().map(|w| w.resume_at(&vid)).unwrap_or(0.);
                        let next = next_video(&m.videos, &vid);
                        let (to_player, inbox) = std::sync::mpsc::channel();
                        playing.set(Some(to_player.clone()));
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let overlay = player::Overlay {
                            title: m.name.clone(),
                            episode: m.videos.iter().find(|v| v.id == vid).map(episode_label).unwrap_or_default(),
                            has_next: next.is_some(),
                        };
                        let (url, headers, to_app) = (req.url.clone(), req.headers.clone(), to_app.clone());
                        // mpv blocks until playback stops, so it gets its own thread.
                        std::thread::spawn(move || {
                            let played = player::play(&url, &headers, start, &overlay, crate::native_window(), inbox, |msg| {
                                let _ = to_app.send(msg);
                            });
                            let _ = tx.send(played);
                        });
                        fetch_subtitles(&ctx, &m.r#type, &vid, req.filename.as_deref(), to_player);

                        let (mut p, ended) = match rx.await {
                            Ok(Ok(played)) => played,
                            Ok(Err(e)) => return eprintln!("mpv: {e}"),
                            Err(_) => return,
                        };
                        // Skipping to the next episode counts as having finished this one.
                        if ended == Some(Ended::NextRequested) {
                            p.position = p.duration;
                        }
                        let state = record_playback(preview(&m), &vid, p, next.clone(), now());
                        let saved = ctx.store.put(&ctx.profile, "progress", &m.id, &state).and_then(|()| {
                            if is_finished(p) {
                                watched.write().insert(vid.clone());
                                ctx.store.put(&ctx.profile, "watched", &vid, &true)?;
                            }
                            Ok(())
                        });
                        if let Err(e) = saved {
                            eprintln!("saving progress: {e}");
                        }
                        watch.set(Some(state));
                        ctx.refresh_local_rows();
                        ctx.sync_soon();

                        let Some(next_id) = next.filter(|_| is_finished(p)) else { return };
                        if let Some(v) = m.videos.iter().find(|v| v.id == next_id) {
                            season.set(v.season);
                        }
                        video.set(Some(next_id.clone()));
                        let Some(group) = req.binge_group.clone() else { return };
                        let Some(next_req) = binge_stream(&ctx, &m.r#type, &next_id, req.addon, &group).await else { return };
                        (req, vid) = (next_req, next_id);
                    }
                    }
                    .await;
                    playing.set(None);
                });
            }
        });

        let toggle_list = {
            let ctx = ctx.clone();
            move |()| {
                let Some(m) = meta.peek().clone() else { return };
                let add = !*in_list.peek();
                // Removal writes `null` rather than deleting, so it can sync.
                let value = add.then(|| preview(&m));
                match ctx.store.put(&ctx.profile, "library", &m.id, &value) {
                    Ok(()) => {
                        in_list.set(add);
                        ctx.refresh_local_rows();
                        ctx.sync_soon();
                    }
                    Err(e) => eprintln!("saving My List: {e}"),
                }
            }
        };

        // Reruns when the addon list changes, so it waits out startup instead of finding no addons.
        // ponytail: loads once per mount; navigating detail -> detail would need a keyed reload.
        use_side_effect({
            let (kind, id, ctx) = (self.kind.clone(), self.id.clone(), ctx.clone());
            move || {
                let addons = ctx.all_addons();
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
                            // Reopen where the user left off, else the first season.
                            let resume = watch.peek().as_ref().and_then(|w| r.meta.videos.iter().find(|v| v.id == w.video_id).cloned());
                            match resume {
                                Some(v) => {
                                    season.set(v.season);
                                    video.set(Some(v.id));
                                }
                                None => season.set(seasons(&r.meta.videos).first().map(|(s, _)| *s)),
                            }
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
                searching.set(true);
                let (kind, ctx) = (kind.clone(), ctx.clone());
                spawn(async move {
                    let addons = ctx.all_addons();
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
                        match res.map(|r| without_torrents(r.streams)) {
                            Ok(found) if !found.is_empty() => streams.write().push((i, found)),
                            Ok(_) => {}
                            Err(e) => eprintln!("streams from addon {i}: {e}"),
                        }
                    }
                    searching.set(false);
                });
            }
        });

        let Some(meta) = meta.read().clone() else {
            return rect().expanded().center().child(label().text("Loading…").color(MUTED)).into_element();
        };
        let addons = ctx.all_addons();
        let has_stream_addons = addons.iter().any(|a| a.serves(&ResourceRequest::new("stream", &self.kind, &self.id)));
        let groups: Vec<(usize, String, Vec<Stream>)> = stream_groups(streams.read().clone())
            .into_iter()
            .map(|(i, s)| (i, addons[i].manifest.name.clone(), s))
            .collect();
        let all_seasons: Vec<(u32, Vec<Video>)> =
            seasons(&meta.videos).into_iter().map(|(n, eps)| (n, eps.into_iter().cloned().collect())).collect();
        let current_season = *season.read();
        let episodes = all_seasons.iter().find(|(n, _)| Some(*n) == current_season).map(|(_, e)| e.clone()).unwrap_or_default();
        let picked = video.read().clone();
        let watched_now = watched.read().clone();
        let current = watch.read().clone();

        page.wrap(ScrollView::new_controlled(page.controller).scroll_with_arrows(false)
            .child(header(&meta, *in_list.read(), toggle_list))
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
                                    let text = if n == 0 { "Specials".into() } else { format!("Season {n}") };
                                    Focusable { radius: 16., ..Focusable::new(chip(text, active), move |()| season.set(Some(n))) }
                                        .into_element()
                                })),
                            )
                            .children(episodes.iter().map(|ep| {
                                let id = ep.id.clone();
                                let progress = current.as_ref().filter(|w| w.video_id == ep.id && !w.up_next).map(WatchState::fraction);
                                let row = episode_row(ep, picked.as_deref() == Some(&ep.id), watched_now.contains(&ep.id), progress);
                                Focusable::new(row, move |()| {
                                    video.set(Some(id.clone()))
                                })
                                .into_element()
                            }))
                    }))
                    .child(stream_panel(picked.is_some(), has_stream_addons, *searching.read(), groups, play)),
            ))
            .into_element()
    }
}

fn header(meta: &Meta, in_list: bool, toggle_list: impl FnMut(()) + 'static) -> Rect {
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
                .child(Focusable::new(label().text("‹ Back").font_size(16.).color(MUTED), |()| RouterContext::get().go_back()))
                .child(title_art(meta.logo.as_deref(), &meta.name))
                .child(label().text(info.join("   ")).font_size(16.).color((70, 211, 105)))
                .maybe_child(meta.description.clone().map(|d| {
                    paragraph().width(Size::px(640.)).max_lines(4).text_overflow(TextOverflow::Ellipsis).font_size(17.).span(d)
                }))
                .child(Focusable::new(
                    button(if in_list { "✓ My List" } else { "+ My List" }, (109, 109, 110, 0.7), (255, 255, 255)),
                    toggle_list,
                )),
        )
}

fn episode_row(ep: &Video, picked: bool, watched: bool, progress: Option<f32>) -> Rect {
    let thumb: Element = match ep.thumbnail.as_deref().and_then(|u| Url::parse(u).ok()) {
        Some(url) => ImageViewer::new(url).width(Size::px(176.)).height(Size::px(99.)).error_renderer(|_| rect().into()).into(),
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
                        .text(format!("{}. {}{}", ep.episode.unwrap_or_default(), ep.title(), if watched { "  ✓" } else { "" }))
                        .font_size(17.)
                        .font_weight(FontWeight::BOLD),
                )
                .map(progress, |el, p| {
                    el.child(
                        rect()
                            .width(Size::px(240.))
                            .height(Size::px(4.))
                            .corner_radius(2.)
                            .background((90, 90, 90))
                            .child(rect().width(Size::percent(p.clamp(0., 1.) * 100.)).height(Size::fill()).corner_radius(2.).background((229, 9, 20))),
                    )
                })
                .maybe_child(ep.overview.clone().map(|o| {
                    paragraph().max_lines(3).text_overflow(TextOverflow::Ellipsis).font_size(14.).color(MUTED).span(o)
                })),
        )
}

/// What playing a stream needs, and what auto-playing the next episode matches on.
#[derive(Clone, PartialEq)]
struct Play {
    url: String,
    headers: HashMap<String, String>,
    filename: Option<String>,
    binge_group: Option<String>,
    /// Index of the addon the stream came from.
    addon: usize,
}

impl Play {
    fn of(stream: &Stream, addon: usize) -> Option<Self> {
        Some(Self {
            url: stream.playable_url()?.to_owned(),
            headers: stream.behavior_hints.proxy_headers.clone().map(|h| h.request).unwrap_or_default(),
            filename: stream.behavior_hints.filename.clone(),
            binge_group: stream.behavior_hints.binge_group.clone(),
            addon,
        })
    }
}

/// "S1:E2 Cat's in the Bag...", as the player shows it.
fn episode_label(v: &Video) -> String {
    match (v.season, v.episode) {
        (Some(s), Some(e)) => format!("S{s}:E{e} {}", v.title()),
        _ => v.title().to_owned(),
    }
}

/// Asks every subtitle addon for `video_id`; each addon's tracks reach mpv as they arrive.
fn fetch_subtitles(ctx: &Ctx, kind: &str, video_id: &str, filename: Option<&str>, to_mpv: std::sync::mpsc::Sender<player::ToPlayer>) {
    let addons = ctx.all_addons();
    for (i, req) in subtitle_requests(&addons, kind, video_id, filename) {
        let (http, url, addon, to_mpv) = (ctx.http.clone(), req.url(&addons[i]), addons[i].manifest.name.clone(), to_mpv.clone());
        spawn(async move {
            match get::<SubtitlesResponse>(http, url).await {
                Ok(r) => {
                    let _ = to_mpv.send(player::ToPlayer::Subtitles(player::sub_add_args(&r.subtitles, &addon)));
                }
                Err(e) => eprintln!("subtitles from {addon}: {e}"),
            }
        });
    }
}

/// The next episode's stream from the same release as the one just watched, if any addon has it.
async fn binge_stream(ctx: &Ctx, kind: &str, video_id: &str, prev_addon: usize, group: &str) -> Option<Play> {
    let addons = ctx.all_addons();
    let req = ResourceRequest::new("stream", kind, video_id);
    let mut set = JoinSet::new();
    for (i, addon) in addons.iter().enumerate().filter(|(_, a)| a.serves(&req)) {
        let (http, url) = (ctx.http.clone(), req.url(addon));
        set.spawn(async move { (i, get::<StreamsResponse>(http, url).await) });
    }
    let mut groups = vec![];
    while let Some(Ok((i, res))) = set.join_next().await {
        if let Ok(r) = res {
            groups.push((i, without_torrents(r.streams)));
        }
    }
    let groups = stream_groups(groups);
    let (addon, stream) = pick_binge_stream(&groups, prev_addon, group)?;
    Play::of(stream, addon)
}

fn stream_panel(picked: bool, has_stream_addons: bool, searching: bool, groups: Vec<(usize, String, Vec<Stream>)>, play: EventHandler<Play>) -> Rect {
    let panel = rect().width(Size::px(460.)).spacing(10.).child(label().text("Streams").font_size(22.).font_weight(FontWeight::BOLD));
    let note = |text: &'static str| label().text(text).font_size(15.).color(MUTED);
    if !has_stream_addons {
        return panel.child(note("No stream addons installed. Add one in Addons."));
    }
    if !picked {
        return panel.child(note("Pick an episode to see its streams."));
    }
    if groups.is_empty() {
        return panel.child(note(if searching {
            "Looking for streams…"
        } else {
            "No playable streams found. Torrent addons need a debrid service (e.g. Torrentio with Real-Debrid)."
        }));
    }
    panel.children(groups.into_iter().map(|(i, addon, streams)| {
        rect()
            .spacing(6.)
            .child(label().text(addon).font_size(14.).color(MUTED))
            .children(streams.into_iter().map(|s| stream_card(s, i, play.clone())))
            .into_element()
    }))
}

fn stream_card(stream: Stream, addon: usize, play: EventHandler<Play>) -> Element {
    let request = Play::of(&stream, addon);
    let name = stream.name.clone().unwrap_or_default().replace('\n', " ");
    let title = stream.title.clone().or(stream.description.clone()).unwrap_or_default();
    let card = rect()
        .width(Size::fill())
        .padding(Gaps::new_all(12.))
        .spacing(4.)
        .corner_radius(6.)
        .background((32, 32, 32))
        .maybe(request.is_none(), |el| el.opacity(0.45))
        .child(label().text(name).font_size(15.).font_weight(FontWeight::BOLD))
        .child(paragraph().font_size(13.).color(MUTED).span(title));
    match request {
        Some(request) => Focusable::new(card, move |()| play.call(request.clone())).into(),
        None => card.into(),
    }
}
