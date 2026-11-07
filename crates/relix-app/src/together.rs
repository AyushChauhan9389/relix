//! The screens for friends and Watch Together (ADR 0010): the Friends page, the invite toast,
//! the chat panel beside a party's video, and the "Watch together" dialog on a title's page.

use freya::prelude::*;
use freya::router::RouterContext;
use relix_proto::social::{PartyTitle, ToServer};

use crate::focus::{Focusable, use_page_scroller, use_scroller};
use crate::home::{button, button_owned};
use crate::social::{self, send_live};
use crate::sync::account;
use crate::theme::{ERROR, theme};
use crate::{Ctx, Route};

fn heading(text: &'static str) -> Label {
    label().text(text).font_size(12.).font_weight(FontWeight::BOLD).color(theme().muted).margin(Gaps::new(16., 0., 0., 0.))
}

fn card() -> Rect {
    rect().width(Size::fill()).max_width(Size::px(720.)).padding(Gaps::new_all(18.)).spacing(12.).corner_radius(theme().radius).background(theme().surface)
}

fn small(text: impl Into<String>, on_press: impl FnMut(()) + 'static) -> Focusable {
    Focusable::new(
        rect().padding(Gaps::new(6., 14., 6., 14.)).corner_radius(6.).background(theme().raised).child(label().text(text.into()).font_size(14.).font_weight(FontWeight::BOLD)),
        EventHandler::new(on_press),
    )
}

fn row() -> Rect {
    rect().width(Size::fill()).horizontal().content(Content::Flex).cross_align(Alignment::Center).spacing(10.)
}

fn grow(text: impl Into<String>, sub: Option<String>) -> Rect {
    rect()
        .width(Size::flex(1.))
        .spacing(2.)
        .child(label().text(text.into()).font_size(16.).font_weight(FontWeight::BOLD).max_lines(1).text_overflow(TextOverflow::Ellipsis))
        .maybe_child(sub.map(|s| label().text(s).font_size(13.).color(theme().muted).max_lines(1).text_overflow(TextOverflow::Ellipsis)))
}

/// Runs a request, showing its error on the page. The task belongs to the page (`scope`): a
/// plain `spawn` in a click belongs to the button, and dies when the button goes (a row that the
/// request itself removes, a button swapped for a spinner).
fn act(ctx: &Ctx, scope: ScopeId, mut error: State<Option<String>>, job: impl AsyncFnOnce(&Ctx) -> Result<(), String> + 'static) {
    let ctx = ctx.clone();
    error.set(None);
    spawn_in_scope(async move {
        if let Err(e) = job(&ctx).await {
            error.set(Some(e));
        }
    }, scope);
}

/// Friends, requests, invites, and joining a party by code.
#[derive(PartialEq)]
pub struct Friends;

impl Component for Friends {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let scope = use_hook(current_scope_id);
        let page = use_scroller();
        use_page_scroller(page);
        let mut name = use_state(String::new);
        let mut add = use_state(String::new);
        let mut code = use_state(String::new);
        let mut renaming = use_state(|| false);
        let error = use_state(|| None::<String>);
        // Fresh lists on every visit (the live connection keeps them current after that).
        use_hook({
            let ctx = ctx.clone();
            move || {
                spawn(async move { social::refresh(&ctx).await });
            }
        });
        let s = ctx.social;
        let me = s.me.read().clone();
        let friends = s.friends.read().clone();
        let invites = s.invites.read().clone();
        let connected = s.connected();
        let server_error = s.error.read().clone();

        let body = rect().width(Size::fill()).spacing(12.);
        let body = if account(&ctx.store).is_none() {
            body.child(paragraph().font_size(15.).color(theme().muted).span("Sign in to add friends and watch together."))
                .child(Focusable::new(button("Sign in", theme().primary.0, theme().primary.1), |()| drop(RouterContext::get().push(Route::Account))))
        } else if me.as_ref().is_none_or(|m| m.username.is_none() || *renaming.read()) {
            let save = {
                let ctx = ctx.clone();
                move |()| {
                    let n = name.peek().clone();
                    act(&ctx, scope, error, async move |ctx| {
                        social::set_username(ctx, &n).await?;
                        renaming.set(false);
                        Ok(())
                    })
                }
            };
            let submit = save.clone();
            body.child(heading("YOUR USERNAME")).child(
                card()
                    .child(paragraph().font_size(14.).color(theme().muted).span("Friends find you by it. 3 to 20 letters, digits or _."))
                    .child(crate::focus::Field(Input::new(name).width(Size::fill()).placeholder("username").on_submit(move |_| submit.clone()(()))))
                    .child(rect().horizontal().spacing(10.).child(Focusable::new(button("Save", theme().primary.0, theme().primary.1), EventHandler::new(save))).maybe(*renaming.read(), |el| {
                        el.child(Focusable::new(button("Cancel", theme().raised, (255, 255, 255)), move |()| renaming.set(false)))
                    })),
            )
        } else {
            let me = me.unwrap_or_else(|| unreachable!());
            let username = me.username.clone().unwrap_or_default();
            let send_request = {
                let ctx = ctx.clone();
                move |()| {
                    let n = add.peek().trim().to_owned();
                    if n.is_empty() {
                        return;
                    }
                    act(&ctx, scope, error, async move |ctx| {
                        social::friend(ctx, "request", &n).await?;
                        add.set(String::new());
                        Ok(())
                    })
                }
            };
            let join = {
                let ctx = ctx.clone();
                move |()| {
                    let c = code.peek().trim().to_owned();
                    if c.is_empty() {
                        return;
                    }
                    let mut e = ctx.social.error;
                    e.set(None);
                    send_live(&ctx, ToServer::Join { code: c });
                    code.set(String::new());
                }
            };
            let action = |label_text: &'static str, verb: &'static str, who: String| {
                let ctx = ctx.clone();
                small(label_text, move |()| {
                    let who = who.clone();
                    act(&ctx, scope, error, async move |ctx| social::friend(ctx, verb, &who).await)
                })
            };
            let (submit_add, submit_join) = (send_request.clone(), join.clone());
            body.child(row().child(grow(format!("@{username}"), Some(if connected { "Online".into() } else { "Connecting…".into() }))).child(small("Change username", move |()| {
                name.set(username.clone());
                renaming.set(true);
            })))
            .child(heading("WATCH TOGETHER"))
            .child(
                card()
                    .child(paragraph().font_size(14.).color(theme().muted).span(if me.can_host {
                        "Start a party from a movie or episode page (Watch together), or join one with its code."
                    } else {
                        "Join a party with the code its host gives you. Hosting is in a closed preview."
                    }))
                    .child(
                        row()
                            .child(rect().width(Size::flex(1.)).child(crate::focus::Field(Input::new(code).width(Size::fill()).placeholder("Party code").on_submit(move |_| submit_join.clone()(())))))
                            .child(Focusable::new(button("Join", theme().primary.0, theme().primary.1), EventHandler::new(join))),
                    )
                    .children(invites.into_iter().map(|i| {
                        let what = if i.title.episode.is_empty() { i.title.name.clone() } else { format!("{} · {}", i.title.name, i.title.episode) };
                        let (ctx, ctx2, c, c2) = (ctx.clone(), ctx.clone(), i.code.clone(), i.code.clone());
                        row()
                            .child(grow(format!("{} invited you", i.from), Some(what)))
                            .child(small("Join", move |()| send_live(&ctx, ToServer::Join { code: c.clone() })))
                            .child(small("Dismiss", move |()| {
                                let mut invites = ctx2.social.invites;
                                invites.write().retain(|i| i.code != c2);
                            }))
                            .into_element()
                    })),
            )
            .child(heading("ADD A FRIEND"))
            .child(
                card().child(
                    row()
                        .child(rect().width(Size::flex(1.)).child(crate::focus::Field(Input::new(add).width(Size::fill()).placeholder("Their username").on_submit(move |_| submit_add.clone()(())))))
                        .child(Focusable::new(button("Send request", theme().primary.0, theme().primary.1), EventHandler::new(send_request))),
                ),
            )
            .maybe(!friends.incoming.is_empty() || !friends.outgoing.is_empty(), |el| {
                el.child(heading("REQUESTS")).child(
                    card()
                        .children(friends.incoming.iter().map(|n| {
                            row()
                                .child(grow(n.clone(), Some("Wants to be friends".into())))
                                .child(action("Accept", "accept", n.clone()))
                                .child(action("Decline", "decline", n.clone()))
                                .child(action("Block", "block", n.clone()))
                                .into_element()
                        }))
                        .children(friends.outgoing.iter().map(|n| row().child(grow(n.clone(), Some("Request sent".into()))).child(action("Cancel", "remove", n.clone())).into_element())),
                )
            })
            .child(heading("FRIENDS"))
            .child(if friends.friends.is_empty() {
                card().child(label().text("No friends yet: send a request to someone's username.").font_size(14.).color(theme().muted))
            } else {
                card().children(friends.friends.iter().map(|f| {
                    let status = match (&f.online, &f.watching) {
                        (true, Some(w)) => format!("● Watching {w}"),
                        (true, None) => "● Online".into(),
                        (false, _) => "Offline".into(),
                    };
                    row().child(grow(f.username.clone(), Some(status))).child(action("Remove", "remove", f.username.clone())).child(action("Block", "block", f.username.clone())).into_element()
                }))
            })
        };
        let shown_error = error.read().clone().or(server_error);

        page.wrap(
            ScrollView::new_controlled(page.controller).scroll_with_arrows(false).drag_scrolling(false).child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(crate::theme::top(), crate::theme::side(), 48., crate::theme::side()))
                    .spacing(12.)
                    .child(label().text("Friends").font_size(32.).font_weight(FontWeight::BOLD))
                    .maybe_child(shown_error.map(|e| paragraph().width(Size::fill()).max_width(Size::px(720.)).font_size(15.).color(ERROR).span(e)))
                    .child(body),
            ),
        )
    }
}

/// "X invited you to watch Y · Join", bottom-right, while not playing.
pub fn invite_toast(ctx: &Ctx) -> Option<Rect> {
    let invite = ctx.social.invites.read().last().cloned()?;
    let (join, dismiss) = (ctx.clone(), ctx.clone());
    let code = invite.code.clone();
    Some(
        rect()
            .position(Position::new_absolute().right(24.).bottom(24.))
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(12.)
            .padding(Gaps::new(12., 12., 12., 20.))
            .corner_radius(8.)
            .background(theme().surface)
            .child(label().text(format!("{} invited you to watch {}", invite.from, invite.title.name)).font_size(15.).max_lines(1).text_overflow(TextOverflow::Ellipsis))
            .child(Focusable::new(button("Join", theme().primary.0, theme().primary.1), move |()| send_live(&join, ToServer::Join { code: invite.code.clone() })))
            .child(small("✕", move |()| {
                let mut invites = dismiss.social.invites;
                invites.write().retain(|i| i.code != code);
            })),
    )
}

const PANEL_W: f32 = 340.;

/// Beside a party's video: who's there, inviting friends, host tools, and the chat.
#[derive(PartialEq)]
pub struct ChatPanel;

impl Component for ChatPanel {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let mut text = use_state(String::new);
        let s = ctx.social;
        let Some(party) = s.party.read().clone() else { return rect().into_element() };
        let lines = s.lines.read().clone();
        let me = s.me.read().as_ref().and_then(|m| m.username.clone()).unwrap_or_default();
        let host = party.host == me;
        let friends = s.friends.read().clone();
        let members: Vec<String> = party.members.iter().map(|m| m.username.clone()).collect();
        let invitable: Vec<String> = friends.friends.iter().filter(|f| f.online && !members.contains(&f.username)).map(|f| f.username.clone()).collect();
        let mut send = {
            let ctx = ctx.clone();
            move || {
                let t = text.peek().trim().to_owned();
                if !t.is_empty() {
                    send_live(&ctx, ToServer::Chat { text: t });
                    text.set(String::new());
                }
            }
        };
        let send2 = send.clone();
        let open = party.open_control;
        let ctx_open = ctx.clone();

        rect()
            .width(Size::px(PANEL_W))
            .height(Size::fill())
            .padding(Gaps::new_all(16.))
            .spacing(10.)
            .content(Content::Flex)
            .background((20, 20, 20))
            .child(row().child(grow("Watch together", Some(format!("Code {}", party.code)))).child(small("✕", {
                let mut open = s.chat_open;
                move |()| open.set(false)
            })))
            .child(rect().width(Size::fill()).spacing(4.).children(party.members.iter().map(|m| {
                let tag = match (m.username == party.host, m.buffering) {
                    (true, true) => " · host · loading",
                    (true, false) => " · host",
                    (false, true) => " · loading",
                    (false, false) => "",
                };
                let who = m.username.clone();
                let (kick_ctx, host_ctx, who2) = (ctx.clone(), ctx.clone(), who.clone());
                row()
                    .child(label().width(Size::flex(1.)).text(format!("{}{tag}", m.username)).font_size(14.).max_lines(1).text_overflow(TextOverflow::Ellipsis))
                    .maybe(host && m.username != me, |el| {
                        el.child(small("Host", move |()| send_live(&host_ctx, ToServer::MakeHost { username: who.clone() })))
                            .child(small("Remove", move |()| send_live(&kick_ctx, ToServer::Kick { username: who2.clone() })))
                    })
                    .into_element()
            })))
            .maybe(host, |el| {
                el.child(row().child(label().width(Size::flex(1.)).text("Everyone can pause and seek").font_size(14.).color(theme().muted)).child(Switch::new(WritableSetter::new(
                    Readable::from_value(open),
                    move |on| send_live(&ctx_open, ToServer::OpenControl { open: on }),
                ))))
            })
            .maybe(!invitable.is_empty(), |el| {
                el.child(rect().width(Size::fill()).spacing(4.).child(label().text("INVITE").font_size(12.).font_weight(FontWeight::BOLD).color(theme().muted)).children(invitable.into_iter().map(|f| {
                    let ctx = ctx.clone();
                    let who = f.clone();
                    row().child(label().width(Size::flex(1.)).text(f).font_size(14.)).child(small("Invite", move |()| send_live(&ctx, ToServer::Invite { username: who.clone() }))).into_element()
                })))
            })
            // The newest lines, from the bottom up: older ones go off the top.
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.))
                    .overflow(Overflow::Clip)
                    .main_align(Alignment::End)
                    .child(rect().width(Size::fill()).spacing(6.).children(lines.into_iter().rev().take(40).collect::<Vec<_>>().into_iter().rev().map(|l| {
                        if l.from.is_empty() {
                            paragraph().width(Size::fill()).font_size(13.).color(theme().muted).span(l.text).into_element()
                        } else {
                            paragraph().width(Size::fill()).font_size(14.).span(Span::new(format!("{}  ", l.from)).font_weight(FontWeight::BOLD)).span(l.text).into_element()
                        }
                    }))),
            )
            .child(
                row()
                    .child(rect().width(Size::flex(1.)).child(crate::focus::Field(Input::new(text).width(Size::fill()).placeholder("Say something").on_submit(move |_| send2.clone()()))))
                    .child(small("Send", move |()| send())),
            )
            .into_element()
    }
}

/// The dialog behind "Watch together" on a title's page: paste a link, start the party.
#[derive(PartialEq)]
pub struct StartParty {
    pub title: PartyTitle,
    pub open: State<bool>,
}

impl Component for StartParty {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let scope = use_hook(current_scope_id);
        let link = use_state(String::new);
        let mut busy = use_state(|| false);
        let mut error = use_state(|| None::<String>);
        let mut open = self.open;
        let start = {
            let (ctx, title) = (ctx.clone(), self.title.clone());
            move |()| {
                if *busy.peek() {
                    return;
                }
                busy.set(true);
                error.set(None);
                let (ctx, title, url) = (ctx.clone(), title.clone(), link.peek().trim().to_owned());
                // The dialog's task, not the button's: the button gives way to a spinner (see `act`).
                spawn_in_scope(
                    async move {
                        let result = social::create_party(&ctx, title, &url).await;
                        busy.set(false);
                        match result {
                            Ok(()) => open.set(false),
                            Err(e) => error.set(Some(e)),
                        }
                    },
                    scope,
                );
            }
        };
        let submit = start.clone();
        let what = if self.title.episode.is_empty() { self.title.name.clone() } else { format!("{} · {}", self.title.name, self.title.episode) };
        rect()
            .position(Position::new_absolute().top(0.).left(0.))
            .width(Size::percent(100.))
            .height(Size::percent(100.))
            .center()
            .background((0, 0, 0, 0.6))
            .child(
                rect()
                    .width(Size::px(560.))
                    .max_width(Size::percent(92.))
                    .padding(Gaps::new_all(24.))
                    .spacing(14.)
                    .corner_radius(12.)
                    .background(theme().surface)
                    .child(label().text("Watch together").font_size(24.).font_weight(FontWeight::BOLD))
                    .child(label().text(what).font_size(16.).color(theme().muted))
                    .child(paragraph().width(Size::fill()).font_size(14.).color(theme().muted).span(
                        "Paste a Google Drive share link (shared as \"Anyone with the link\") or a direct video link. Everyone in the party streams it from there.",
                    ))
                    .child(crate::focus::Field(Input::new(link).width(Size::fill()).placeholder("https://drive.google.com/file/d/…").on_submit(move |_| submit.clone()(()))))
                    .maybe_child(error.read().clone().map(|e| paragraph().width(Size::fill()).font_size(14.).color(ERROR).span(e)))
                    .child(
                        rect()
                            .horizontal()
                            .spacing(12.)
                            .child(if *busy.read() {
                                crate::motion::busy("Checking the link…").into_element()
                            } else {
                                Focusable::new(button_owned("Start party".into(), theme().primary.0, theme().primary.1), EventHandler::new(start)).into_element()
                            })
                            .child(Focusable::new(button("Cancel", theme().raised, (255, 255, 255)), move |()| open.set(false))),
                    ),
            )
    }
}
