use freya::prelude::*;

use crate::Ctx;
use crate::focus::Focusable;
use crate::home::button;
use crate::sync::{self, account};
use crate::theme::theme;


/// Sign in, create an account (with an emailed code if the server asks), or sign out.
#[derive(PartialEq)]
pub struct Account;

impl Component for Account {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let email = use_state(String::new);
        let password = use_state(String::new);
        let code = use_state(String::new);
        let mut busy = use_state(|| false);
        let mut error = use_state(|| None::<String>);
        // Set after creating an account when the server emailed a code.
        let mut needs_code = use_state(|| false);
        let mut signed_in = use_state(|| account(&ctx.store));

        // Runs one account action, showing progress and errors.
        let act = {
            let ctx = ctx.clone();
            move |kind: &'static str| {
                let ctx = ctx.clone();
                let (mail, pass, c) = (email.read().trim().to_owned(), password.read().clone(), code.read().trim().to_owned());
                if mail.is_empty() || (kind != "verify" && pass.is_empty()) {
                    return error.set(Some("Enter your email and password.".into()));
                }
                busy.set(true);
                error.set(None);
                spawn(async move {
                    let result = match kind {
                        "sign_in" => sync::sign_in(&ctx, &mail, &pass).await,
                        "sign_up" => sync::sign_up(&ctx, &mail, &pass).await.map(|code| needs_code.set(code)),
                        _ => sync::verify(&ctx, &mail, &c).await.map(|()| needs_code.set(false)),
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

        let body = match (signed_in.read().clone(), *needs_code.read()) {
            (_, true) => {
                let mut act = act.clone();
                rect()
                    .spacing(14.)
                    .child(label().text("Check your email").font_size(22.).font_weight(FontWeight::BOLD))
                    .child(field("Code from your email", Input::new(code).width(Size::fill()).placeholder("123456")))
                    .child(Focusable::new(button("Verify", theme().primary.0, theme().primary.1), move |()| act("verify")))
            }
            (Some(acc), false) => {
                let ctx2 = ctx.clone();
                rect()
                    .spacing(12.)
                    .child(label().text(format!("Signed in as {}", acc.email)).font_size(20.))
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
            (None, false) => {
                let (mut a1, mut a2) = (act.clone(), act.clone());
                rect()
                    .spacing(14.)
                    .child(paragraph().font_size(15.).color(theme().muted).span(
                        "An account keeps your addons, My List, progress and settings on Relix's servers, so they follow you to every device.",
                    ))
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
                    .maybe(*busy.read(), |el| el.child(crate::motion::busy("Working…")))
                    .maybe_child(error.read().clone().map(|e| label().text(e).font_size(15.).color(crate::theme::ERROR))),
            )
    }
}
