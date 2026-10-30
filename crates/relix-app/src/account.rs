use freya::prelude::*;

use crate::Ctx;
use crate::focus::Focusable;
use crate::home::button;
use crate::sync::{self, DEFAULT_SERVER, account};
use crate::theme::theme;


/// Sign in, create an account (recovery key shown once, emailed code if the server asks), or sign out.
#[derive(PartialEq)]
pub struct Account;

impl Component for Account {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let server = use_state(|| DEFAULT_SERVER.to_owned());
        let email = use_state(String::new);
        let password = use_state(String::new);
        let code = use_state(String::new);
        let mut busy = use_state(|| false);
        let mut error = use_state(|| None::<String>);
        // Set after creating an account: the recovery key to show, and whether a code is still needed.
        let mut recovery = use_state(|| None::<(String, bool)>);
        let mut signed_in = use_state(|| account(&ctx.store));

        // Runs one account action, showing progress and errors.
        let act = {
            let ctx = ctx.clone();
            move |kind: &'static str| {
                let ctx = ctx.clone();
                let (srv, mail, pass, c) = (server.read().trim().to_owned(), email.read().trim().to_owned(), password.read().clone(), code.read().trim().to_owned());
                if mail.is_empty() || (kind != "verify" && pass.is_empty()) {
                    return error.set(Some("Enter your email and password.".into()));
                }
                busy.set(true);
                error.set(None);
                spawn(async move {
                    let result = match kind {
                        "sign_in" => sync::sign_in(&ctx, &srv, &mail, &pass).await,
                        "sign_up" if pass.chars().count() < 10 => Err("Use at least 10 characters; this password also protects your addon keys.".into()),
                        "sign_up" => sync::sign_up(&ctx, &srv, &mail, &pass).await.map(|r| recovery.set(Some(r))),
                        _ => sync::verify(&ctx, &srv, &mail, &c).await.map(|()| {
                            let shown = recovery.peek().clone();
                            if let Some((key, _)) = shown {
                                recovery.set(Some((key, false)));
                            }
                        }),
                    };
                    busy.set(false);
                    match result {
                        Ok(()) => signed_in.set(account(&ctx.store)),
                        Err(e) => error.set(Some(e)),
                    }
                });
            }
        };

        let field = |label_text: &'static str, el: Input| {
            rect().width(Size::fill()).spacing(6.).child(label().text(label_text).font_size(14.).color(theme().muted)).child(el)
        };
        let status = ctx.sync_status.read().clone();

        let body = match (signed_in.read().clone(), recovery.read().clone()) {
            // Just created: the recovery key comes first, whatever else is pending.
            (_, Some((key, needs_code))) => rect()
                .spacing(14.)
                .child(label().text("Save your recovery key").font_size(22.).font_weight(FontWeight::BOLD))
                .child(paragraph().font_size(15.).color(theme().muted).span(
                    "It's the only way back into your addon list if you forget your password. Relix can't recover it for you. Write it down or keep it in a password manager.",
                ))
                .child(
                    rect()
                        .padding(Gaps::new_all(16.))
                        .corner_radius(6.)
                        .background(theme().surface)
                        .child(paragraph().font_size(18.).font_family("Consolas").span(key)),
                )
                .map(needs_code.then_some(()), |el, ()| {
                    let mut act = act.clone();
                    el.child(field("Code from your email", Input::new(code).width(Size::fill()).placeholder("123456")))
                        .child(Focusable::new(button("Verify", theme().primary.0, theme().primary.1), move |()| act("verify")))
                })
                .map((!needs_code).then_some(()), |el, ()| {
                    el.child(Focusable::new(button("I've saved it", theme().primary.0, theme().primary.1), move |()| recovery.set(None)))
                }),
            (Some(acc), None) => {
                let ctx2 = ctx.clone();
                rect()
                    .spacing(12.)
                    .child(label().text(format!("Signed in as {}", acc.email)).font_size(20.))
                    .child(label().text(format!("Server: {}", acc.server)).font_size(14.).color(theme().muted))
                    .child(label().text(if status.is_empty() { "Syncing…".into() } else { status }).font_size(14.).color(theme().muted))
                    .child(
                        rect()
                            .horizontal()
                            .spacing(12.)
                            .child(Focusable::new(button("Sync now", theme().raised, (255, 255, 255)), {
                                let ctx = ctx.clone();
                                move |()| ctx.sync_soon()
                            }))
                            .child(Focusable::new(button("Sign out", theme().raised, (255, 255, 255)), move |()| {
                                sync::sign_out(&ctx2);
                                signed_in.set(None);
                            })),
                    )
            }
            (None, None) => {
                let (mut a1, mut a2) = (act.clone(), act.clone());
                rect()
                    .spacing(14.)
                    .child(paragraph().font_size(15.).color(theme().muted).span(
                        "An account syncs your addons, My List and progress across devices. Your addon list is encrypted on this device first, so the server never sees your debrid keys.",
                    ))
                    .child(field("Server", Input::new(server).width(Size::fill())))
                    .child(field("Email", Input::new(email).width(Size::fill())))
                    .child(field("Password", Input::new(password).width(Size::fill()).mode(InputMode::new_password())))
                    .child(
                        rect()
                            .horizontal()
                            .spacing(12.)
                            .child(Focusable::new(button("Sign in", theme().primary.0, theme().primary.1), move |()| a1("sign_in")))
                            .child(Focusable::new(button("Create account", theme().raised, (255, 255, 255)), move |()| a2("sign_up"))),
                    )
            }
        };

        rect()
            .expanded()
            .padding(Gaps::new(theme().top, 48., 48., 48.))
            .child(
                rect()
                    .width(Size::px(560.))
                    .spacing(16.)
                    .child(label().text("Account").font_size(32.).font_weight(FontWeight::BOLD))
                    .child(body)
                    .maybe(*busy.read(), |el| el.child(label().text("Working…").color(theme().muted)))
                    .maybe_child(error.read().clone().map(|e| label().text(e).font_size(15.).color(crate::theme::ERROR))),
            )
    }
}
