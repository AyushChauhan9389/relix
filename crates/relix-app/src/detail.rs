use std::collections::{HashMap, HashSet};

use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::{
    Meta, MetaPreview, MetaResponse, ResourceRequest, Stream, StreamsResponse, SubtitlesResponse, Video, route, seasons, stream_groups,
    pick_binge_stream, pick_preferred, subtitle_requests, without_torrents,
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::focus::{Focusable, use_page_scroller, use_scroller};
use crate::motion::{FadeIn, Placeholder, busy};
use crate::home::{backdrop, button, layer, play_button, title_art};
use crate::library::{Progress, WatchState, is_finished, next_video, now, preview, record_playback};
use crate::player::Ended;
use crate::{Ctx, get, player};
use crate::theme::theme;


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
        // Why the last stream didn't play, shown above the streams.
        let mut play_error = use_state(|| None::<String>);
        // One-click Play: the video to play as soon as its streams are in, and which video the
        // loaded streams belong to (so it never plays the previous episode's).
        let mut autoplay = use_state(|| None::<String>);
        let mut streams_for = use_state(|| None::<String>);
        // Set from the moment Play is pressed until playback ends, so a second press can't start a second player.
        let mut starting = use_state(|| false);
        // Home's Play opened this page.
        let from_hero = use_hook({
            let (title, id) = (ctx.autoplay_title, self.id.clone());
            move || title.peek().as_deref() == Some(id.as_str())
        });
        // The title's details couldn't be loaded from any addon.
        let mut load_failed = use_state(|| false);
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
                if *starting.peek() {
                    return;
                }
                starting.set(true);
                let ctx = ctx.clone();
                play_error.set(None);
                spawn(async move {
                    let mut playing = ctx.player;
                    // Requests from the player's controls that Relix handles (fullscreen, Esc).
                    let (to_app, mut from_player) = tokio::sync::mpsc::unbounded_channel();
                    spawn({
                        let (ctx, title) = (ctx.clone(), preview(&m));
                        async move {
                            let mut status = ctx.player_status;
                            // Saved every 5s while playing (sent to the server every 30s), so closing
                            // the window or a crash doesn't lose what was watched.
                            let (mut saved_at, mut synced_at) = (std::time::Instant::now(), std::time::Instant::now());
                            while let Some::<player::Status>(s) = from_player.recv().await {
                                let p = Progress { position: s.position, duration: s.duration };
                                if s.loaded && p.position > 0. && !is_finished(p) && saved_at.elapsed().as_secs() >= 5 {
                                    saved_at = std::time::Instant::now();
                                    if let Some(vid) = video.peek().clone() {
                                        let state = record_playback(title.clone(), &vid, p, None, now());
                                        if let Err(e) = ctx.store.put(&ctx.profile, "progress", &title.id, &state) {
                                            eprintln!("saving progress: {e}");
                                        } else if synced_at.elapsed().as_secs() >= 30 {
                                            synced_at = std::time::Instant::now();
                                            ctx.sync_soon();
                                        }
                                    }
                                }
                                status.set(s);
                            }
                        }
                    });
                    let (mut req, mut vid) = (first, first_vid);
                    async {
                    loop {
                        if let Some(addon) = ctx.all_addons().get(req.addon) {
                            let pref = PreferredStream { addon: addon.manifest.id.clone(), name: req.name.clone().unwrap_or_default() };
                            if let Err(e) = ctx.store.put(&ctx.profile, "settings", "preferred_stream", &pref) {
                                eprintln!("saving preferred stream: {e}");
                            }
                        }
                        let start = watch.peek().as_ref().map(|w| w.resume_at(&vid)).unwrap_or(0.);
                        // Marked as watching the moment Play is pressed, before the stream even opens.
                        let duration = watch.peek().as_ref().filter(|w| w.video_id == vid).map_or(0., |w| w.duration);
                        let state = record_playback(preview(&m), &vid, Progress { position: start, duration }, None, now());
                        match ctx.store.put(&ctx.profile, "progress", &m.id, &state) {
                            Ok(()) => {
                                watch.set(Some(state));
                                ctx.refresh_local_rows();
                                ctx.sync_soon();
                            }
                            Err(e) => eprintln!("saving progress: {e}"),
                        }
                        let next = next_video(&m.videos, &vid);
                        let (to_player, inbox) = std::sync::mpsc::channel();
                        playing.set(Some(to_player.clone()));
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let overlay = player::Overlay {
                            title: m.name.clone(),
                            episode: m.videos.iter().find(|v| v.id == vid).map(episode_label).unwrap_or_default(),
                            has_next: next.is_some(),
                            logo: m.logo.clone(),
                            background: m.background.clone(),
                        };
                        let (url, headers, to_app) = (req.url.clone(), req.headers.clone(), to_app.clone());
                        // The player hands its mpv over from its thread; the video is drawn here.
                        let (to_ui, mut mpv_rx) = tokio::sync::mpsc::unbounded_channel();
                        spawn({
                            let (ctx, overlay) = (ctx.clone(), overlay.clone());
                            async move {
                                if let Some(mpv) = mpv_rx.recv().await {
                                    ctx.show_video(mpv, overlay);
                                }
                            }
                        });
                        // mpv blocks until playback stops, so it gets its own thread.
                        std::thread::spawn(move || {
                            let video = Box::new(move |mpv| drop(to_ui.send(mpv)));
                            let played = player::play(&url, &headers, start, &overlay.title, Some(video), inbox, |msg| {
                                let _ = to_app.send(msg);
                            });
                            let _ = tx.send(played);
                        });
                        fetch_subtitles(&ctx, &m.r#type, &vid, req.filename.as_deref(), to_player);

                        let (mut p, ended) = match rx.await {
                            // mpv reports a dead link either as a failed load (-13, MPV_ERROR_LOADING_FAILED) or as an errored end.
                            Ok(Ok((_, Some(Ended::Failed))) | Err(libmpv2::Error::Raw(-13))) => {
                                return play_error.set(Some("This stream didn't play: the link is dead or the format isn't supported. Try another one.".into()));
                            }
                            Ok(Ok(played)) => played,
                            Ok(Err(e)) => return play_error.set(Some(format!("The player failed to start: {e}"))),
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
                    let mut now = ctx.now_playing;
                    now.set(None);
                    starting.set(false);
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
                            // Open on the season the user left off in (Resume opens the episode itself).
                            let resume = watch.peek().as_ref().and_then(|w| r.meta.videos.iter().find(|v| v.id == w.video_id).cloned());
                            season.set(resume.clone().and_then(|v| v.season).or_else(|| seasons(&r.meta.videos).first().map(|(s, _)| *s)));
                            let first_episode = || seasons(&r.meta.videos).into_iter().find(|(n, _)| *n > 0).and_then(|(_, eps)| eps.first().map(|v| (*v).clone()));
                            if seasons(&r.meta.videos).is_empty() {
                                video.set(Some(id.clone()));
                                if from_hero {
                                    autoplay.set(Some(id.clone()));
                                }
                            } else if from_hero && let Some(v) = resume.or_else(first_episode) {
                                season.set(v.season);
                                video.set(Some(v.id.clone()));
                                autoplay.set(Some(v.id));
                            }
                            if from_hero {
                                let mut title = ctx.autoplay_title;
                                title.set(None);
                            }
                            meta.set(Some(r.meta));
                            return;
                        }
                    }
                    eprintln!("no addon answered meta for {kind}/{id}");
                    load_failed.set(true);
                });
            }
        });

        use_side_effect({
            let (kind, ctx) = (self.kind.clone(), ctx.clone());
            move || {
                let Some(id) = video.read().clone() else { return };
                streams.set(vec![]);
                streams_for.set(Some(id.clone()));
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

        // Plays the queued video once its streams are in: the remembered addon's same-quality stream when
        // it has one. Waits for that addon's answer, or for every addon when nothing is remembered.
        use_side_effect({
            let (ctx, play) = (ctx.clone(), play.clone());
            move || {
                let Some(target) = autoplay.read().clone() else { return };
                let (groups, done) = (streams.read().clone(), !*searching.read());
                if streams_for.read().as_deref() != Some(target.as_str()) {
                    return;
                }
                let addons = ctx.all_addons();
                let pref = ctx.store.get::<PreferredStream>(&ctx.profile, "settings", "preferred_stream").ok().flatten();
                let pref_addon = pref.as_ref().and_then(|p| addons.iter().position(|a| a.manifest.id == p.addon));
                if !done && !pref_addon.is_some_and(|i| groups.iter().any(|(a, _)| *a == i)) {
                    return;
                }
                autoplay.set(None);
                let groups = stream_groups(groups);
                let name = pref.as_ref().map_or("", |p| p.name.as_str());
                if let Some(req) = pick_preferred(&groups, pref_addon, name).and_then(|(i, s)| Play::of(s, i)) {
                    play.call(req);
                }
            }
        });

        let Some(meta) = meta.read().clone() else {
            let no_meta_addon = route(&ctx.all_addons(), &ResourceRequest::new("meta", &self.kind, &self.id)).next().is_none();
            if *load_failed.read() || no_meta_addon && !ctx.addons.read().is_empty() {
                return rect()
                    .expanded()
                    .center()
                    .spacing(14.)
                    .child(label().text("Couldn't load this title").font_size(26.).font_weight(FontWeight::BOLD))
                    .child(paragraph().width(Size::px(460.)).text_align(TextAlign::Center).font_size(16.).color(theme().muted).span(
                        if no_meta_addon {
                            "None of your addons can show details for it."
                        } else {
                            "Your addons didn't return its details. Check your internet connection and try again."
                        },
                    ))
                    .child(Focusable::new(button("‹ Back", theme().raised, (255, 255, 255)), |()| RouterContext::get().go_back()))
                    .into_element();
            }
            return loading_page().into_element();
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

        let is_series = !all_seasons.is_empty();
        // Series: the panel shows episodes until one is picked, then its streams.
        let showing_streams = picked.is_some();
        let picked_video = picked.as_ref().and_then(|id| meta.videos.iter().find(|v| &v.id == id)).cloned();

        // The big button: resume where you were, else start from the beginning.
        let (play_label, on_play): (String, Box<dyn FnMut(())>) = if is_series {
            let target = current
                .as_ref()
                .and_then(|w| meta.videos.iter().find(|v| v.id == w.video_id))
                .or_else(|| all_seasons.iter().find(|(n, _)| *n > 0).and_then(|(_, eps)| eps.first()))
                .cloned();
            let started = current.as_ref().is_some_and(|w| w.position > 0. || w.up_next);
            let label = match &target {
                Some(v) => format!("{} {}", if started { "Resume" } else { "Play" }, short_label(v)),
                None => "Play".into(),
            };
            (label, Box::new(move |()| {
                if let Some(v) = &target {
                    season.set(v.season);
                    video.set(Some(v.id.clone()));
                    autoplay.set(Some(v.id.clone()));
                }
            }))
        } else {
            let resumes = current.as_ref().is_some_and(|w| w.position > 0. && !w.finished());
            let id = meta.id.clone();
            (if resumes { "Resume".into() } else { "Play".into() }, Box::new(move |()| autoplay.set(Some(id.clone()))))
        };

        let panel: Element = if showing_streams {
            rect()
                .expanded()
                .spacing(12.)
                .maybe(is_series, |el| {
                    el.child(
                        rect()
                            .horizontal()
                            .spacing(12.)
                            .cross_align(Alignment::Center)
                            .child(Focusable::new(label().text("‹ Episodes").font_size(15.).color(theme().muted), move |()| video.set(None)))
                            .maybe_child(picked_video.as_ref().map(|v| {
                                label().text(short_label(v)).font_size(16.).font_weight(FontWeight::BOLD).max_lines(1).text_overflow(TextOverflow::Ellipsis)
                            })),
                    )
                })
                .child(page.wrap(
                    ScrollView::new_controlled(page.controller)
                        .scroll_with_arrows(false)
                        .maybe_child(play_error.read().clone().map(|e| {
                            paragraph().width(Size::fill()).font_size(15.).color(crate::theme::ERROR).span(e)
                        }))
                        .child(stream_panel(has_stream_addons, *searching.read(), groups, play)),
                ))
                .into()
        } else {
            rect()
                .expanded()
                .spacing(12.)
                .child(season_switcher(&all_seasons, current_season, season))
                .child(page.wrap(
                    ScrollView::new_controlled(page.controller).scroll_with_arrows(false).child(
                        rect().width(Size::fill()).spacing(6.).children(episodes.iter().map(|ep| {
                            let id = ep.id.clone();
                            let progress = current.as_ref().filter(|w| w.video_id == ep.id && !w.up_next).map(WatchState::fraction);
                            let row = episode_row(ep, watched_now.contains(&ep.id), progress);
                            Focusable::new(row, move |()| video.set(Some(id.clone()))).into_element()
                        })),
                    ),
                ))
                .into()
        };

        rect()
            .expanded()
            .child(backdrop(meta.background.as_deref()).layer(1))
            // Darken the whole picture so text and the panel read on any backdrop.
            .child(layer().layer(2).background((0, 0, 0, 0.45)))
            .child(
                layer()
                    .layer(5)
                    .horizontal()
                    .content(Content::Flex)
                    .padding(Gaps::new(theme().top - 8., 40., 32., 48.))
                    .spacing(40.)
                    .child(info_column(&meta, *in_list.read(), toggle_list, play_label, on_play))
                    .child(
                        rect()
                            .width(Size::px(460.))
                            .height(Size::fill())
                            .padding(Gaps::new_all(16.))
                            .corner_radius(12.)
                            .background(theme().panel)
                            // Switching episodes ↔ streams (or seasons) slides the new content in.
                            .child(FadeIn::page(panel).key(format!("{showing_streams}-{current_season:?}-{picked:?}"))),
                    ),
            )
            .into_element()
    }
}

/// "‹ Season 2 ›": steps through the seasons (specials last), like Stremio's picker.
fn season_switcher(all: &[(u32, Vec<Video>)], current: Option<u32>, mut season: State<Option<u32>>) -> Rect {
    let i = all.iter().position(|(n, _)| Some(*n) == current).unwrap_or(0);
    let name = |n: u32| if n == 0 { "Specials".to_owned() } else { format!("Season {n}") };
    let step = |to: Option<u32>, glyph: &'static str| {
        let enabled = to.is_some();
        Focusable {
            radius: 18.,
            ..Focusable::new(
                rect()
                    .width(Size::px(36.))
                    .height(Size::px(36.))
                    .corner_radius(18.)
                    .center()
                    .background(theme().raised)
                    .opacity(if enabled { 1. } else { 0.35 })
                    .child(label().text(glyph).font_size(20.)),
                move |()| {
                    if to.is_some() {
                        season.set(to);
                    }
                },
            )
        }
    };
    rect()
        .horizontal()
        .content(Content::Flex)
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(10.)
        .child(step(i.checked_sub(1).map(|p| all[p].0), "‹"))
        .child(
            label()
                .width(Size::flex(1.))
                .text_align(TextAlign::Center)
                .text(all.get(i).map(|(n, _)| name(*n)).unwrap_or_default())
                .font_size(18.)
                .font_weight(FontWeight::BOLD),
        )
        .child(step(all.get(i + 1).map(|(n, _)| *n), "›"))
}

/// "S1:E2 Cat's in the Bag..." for an episode, its title otherwise.
fn short_label(v: &Video) -> String {
    episode_label(v)
}

/// The title's details on the left of the page.
fn info_column(meta: &Meta, in_list: bool, toggle_list: impl FnMut(()) + 'static, play_label: String, on_play: Box<dyn FnMut(())>) -> Rect {
    let facts: Vec<String> = [meta.release_info.clone(), meta.runtime.clone()].into_iter().flatten().collect();
    let people = |title: &str, names: Vec<String>| {
        (!names.is_empty()).then(|| {
            paragraph()
                .max_lines(2)
                .text_overflow(TextOverflow::Ellipsis)
                .font_size(15.)
                .span(Span::new(format!("{title}  ")).color(theme().muted))
                .span(names.into_iter().take(5).collect::<Vec<_>>().join(", "))
        })
    };
    let directors = meta.people("director", "Directors");
    let (makers_title, makers) = if directors.is_empty() { ("Created by", meta.people("writer", "Writers")) } else { ("Directed by", directors) };
    let mut on_play = on_play;

    rect()
        .width(Size::flex(1.))
        .height(Size::fill())
        .spacing(16.)
        .main_align(Alignment::End)
        // Pinned top-left; the rest of the column sits at the bottom.
        .child(rect().position(Position::new_absolute().top(0.).left(0.)).child(Focusable::new(
            label().text("‹ Back").font_size(16.).color(theme().muted),
            |()| RouterContext::get().go_back(),
        )))
        .child(title_art(meta.logo.as_deref(), &meta.name))
        .child(
            rect()
                .horizontal()
                .spacing(14.)
                .cross_align(Alignment::Center)
                .child(label().text(facts.join("  ·  ")).font_size(16.).color((220, 220, 220)))
                .maybe_child(meta.rating().map(|r| label().text(format!("★ {r}")).font_size(16.).font_weight(FontWeight::BOLD).color((245, 197, 24)))),
        )
        .maybe(!meta.genres.is_empty(), |el| {
            el.child(rect().horizontal().spacing(8.).children(meta.genres.iter().take(4).map(|g| {
                rect()
                    .padding(Gaps::new(4., 12., 4., 12.))
                    .corner_radius(14.)
                    .border(Border::new().fill((255, 255, 255, 0.35)).width(1.))
                    .child(label().text(g.clone()).font_size(13.))
                    .into_element()
            })))
        })
        .maybe_child(meta.description.clone().map(|d| {
            paragraph().width(Size::px(640.)).max_lines(5).text_overflow(TextOverflow::Ellipsis).font_size(17.).span(d)
        }))
        .maybe_child(people("Cast", meta.people("cast", "Cast")).map(|p| p.width(Size::px(640.))))
        .maybe_child(people(makers_title, makers).map(|p| p.width(Size::px(640.))))
        .child(
            rect()
                .horizontal()
                .spacing(12.)
                .padding(Gaps::new(8., 0., 0., 0.))
                .child(Focusable::new(play_button(play_label.clone()), move |()| on_play(())))
                .child(Focusable::new(
                    button(if in_list { "✓ My List" } else { "+ My List" }, theme().secondary, (255, 255, 255)),
                    toggle_list,
                )),
        )
}

fn episode_row(ep: &Video, watched: bool, progress: Option<f32>) -> Rect {
    let (w, h) = (128., 72.);
    let thumb: Element = match ep.thumbnail.as_deref().and_then(|u| Url::parse(u).ok()) {
        Some(url) => ImageViewer::new(url)
            .width(Size::px(w))
            .height(Size::px(h))
            .loading_placeholder(Placeholder::new(Size::px(w), h))
            .error_renderer(|_| rect().into())
            .into(),
        None => rect().width(Size::px(w)).height(Size::px(h)).background(theme().placeholder).into(),
    };
    rect()
        .horizontal()
        .content(Content::Flex)
        .width(Size::fill())
        .padding(Gaps::new_all(8.))
        .spacing(12.)
        .corner_radius(8.)
        .child(
            rect()
                .spacing(3.)
                .child(rect().corner_radius(4.).overflow(Overflow::Clip).child(thumb))
                .map(progress, |el, p| {
                    el.child(
                        rect()
                            .width(Size::px(w))
                            .height(Size::px(3.))
                            .corner_radius(2.)
                            .background(theme().track)
                            .child(rect().width(Size::px(w * p.clamp(0., 1.))).height(Size::fill()).corner_radius(2.).background(theme().accent)),
                    )
                }),
        )
        .child(
            rect()
                .width(Size::flex(1.))
                .spacing(4.)
                .child(
                    rect()
                        .horizontal()
                        .content(Content::Flex)
                        .width(Size::fill())
                        .child(
                            label()
                                .width(Size::flex(1.))
                                .text(format!("{}. {}", ep.episode.unwrap_or_default(), ep.title()))
                                .font_size(15.)
                                .font_weight(FontWeight::BOLD)
                                .max_lines(1)
                                .text_overflow(TextOverflow::Ellipsis),
                        )
                        .maybe(watched, |el| el.child(label().text("✓").font_size(15.).color((70, 211, 105)))),
                )
                .maybe_child(ep.released.as_deref().and_then(|d| d.get(..10)).map(|d| label().text(d.to_owned()).font_size(12.).color(theme().muted)))
                .maybe_child(ep.overview.clone().map(|o| {
                    paragraph().max_lines(2).text_overflow(TextOverflow::Ellipsis).font_size(13.).color(theme().muted).span(o)
                })),
        )
}

/// The title page's layout in pulsing blocks, while its details load.
fn loading_page() -> Rect {
    let episode = || {
        rect()
            .horizontal()
            .spacing(12.)
            .padding(Gaps::new_all(8.))
            .child(Placeholder::new(Size::px(128.), 72.))
            .child(rect().spacing(8.).child(Placeholder::new(Size::px(180.), 16.)).child(Placeholder::new(Size::px(220.), 12.)))
            .into_element()
    };
    rect()
        .expanded()
        .horizontal()
        .content(Content::Flex)
        .padding(Gaps::new(theme().top - 8., 40., 32., 48.))
        .spacing(40.)
        .child(
            rect()
                .width(Size::flex(1.))
                .height(Size::fill())
                .main_align(Alignment::End)
                .spacing(16.)
                .child(Placeholder::new(Size::px(380.), 110.))
                .child(Placeholder::new(Size::px(240.), 18.))
                .child(Placeholder::new(Size::px(560.), 16.))
                .child(Placeholder::new(Size::px(520.), 16.))
                .child(Placeholder::new(Size::px(300.), 16.))
                .child(rect().horizontal().spacing(12.).child(Placeholder::new(Size::px(130.), 44.)).child(Placeholder::new(Size::px(140.), 44.))),
        )
        .child(
            rect()
                .width(Size::px(460.))
                .height(Size::fill())
                .padding(Gaps::new_all(16.))
                .spacing(8.)
                .corner_radius(12.)
                .background(theme().panel)
                .children((0..6).map(|_| episode())),
        )
}

/// What playing a stream needs, and what auto-playing the next episode matches on.
#[derive(Clone, PartialEq)]
struct Play {
    url: String,
    headers: HashMap<String, String>,
    filename: Option<String>,
    binge_group: Option<String>,
    /// The stream's name, usually its quality label ("Torrentio\n4k").
    name: Option<String>,
    /// Index of the addon the stream came from.
    addon: usize,
}

/// The addon and quality label of the last stream played, for one-click Play (a synced setting).
#[derive(Serialize, Deserialize)]
struct PreferredStream {
    addon: String,
    name: String,
}

impl Play {
    fn of(stream: &Stream, addon: usize) -> Option<Self> {
        Some(Self {
            url: stream.playable_url()?.to_owned(),
            headers: stream.behavior_hints.proxy_headers.clone().map(|h| h.request).unwrap_or_default(),
            filename: stream.behavior_hints.filename.clone(),
            binge_group: stream.behavior_hints.binge_group.clone(),
            name: stream.name.clone(),
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

fn stream_panel(has_stream_addons: bool, searching: bool, groups: Vec<(usize, String, Vec<Stream>)>, play: EventHandler<Play>) -> Rect {
    let panel = rect().width(Size::fill()).spacing(10.);
    let note = |text: &'static str| paragraph().width(Size::fill()).font_size(15.).color(theme().muted).span(text);
    if !has_stream_addons {
        return panel.child(note("No stream addons installed. Add one in Addons."));
    }
    if groups.is_empty() && searching {
        return panel
            .spacing(12.)
            .child(busy("Looking for streams…"))
            .children((0..4).map(|_| Placeholder::new(Size::fill(), 64.).into_element()));
    }
    if groups.is_empty() {
        return panel.child(note("No playable streams found. Torrent addons need a debrid service (e.g. Torrentio with Real-Debrid)."));
    }
    panel
        .children(groups.into_iter().map(|(i, addon, streams)| {
            rect()
                .spacing(6.)
                .child(label().text(addon).font_size(14.).color(theme().muted))
                .children(streams.into_iter().map(|s| stream_card(s, i, play.clone())))
                .into_element()
        }))
        // Slower addons may still answer below what's shown.
        .maybe(searching, |el| el.child(busy("Asking more addons…")))
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
        .background(theme().surface)
        .maybe(request.is_none(), |el| el.opacity(0.45))
        .child(label().text(name).font_size(15.).font_weight(FontWeight::BOLD))
        .child(paragraph().font_size(13.).color(theme().muted).span(title));
    match request {
        Some(request) => Focusable::new(card, move |()| play.call(request.clone())).into(),
        None => card.into(),
    }
}
