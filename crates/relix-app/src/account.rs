use freya::prelude::*;

use crate::Ctx;
use crate::focus::Focusable;
use crate::home::button;
use crate::sync::{self, account};
use crate::theme::theme;

/// Which form the signed-out screen shows.
#[derive(Clone, Copy, PartialEq)]
enum Form {
    SignIn,
    /// After creating an account when the server emailed a code.
    Verify,
    Forgot,
    /// The reset code was sent; enter it with a new password.
    Reset,
}

/// Sign in, create an account, reset a password; signed in: sync, sign out, delete the account.
#[derive(PartialEq)]
pub struct Account;

impl Component for Account {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let email = use_state(String::new);
        let mut password = use_state(String::new);
        let code = use_state(String::new);
        let mut busy = use_state(|| false);
        let mut error = use_state(|| None::<String>);
        let mut form = use_state(|| Form::SignIn);
        // Signed in: whether the delete confirmation is open.
        let mut deleting = use_state(|| false);

        // Runs one account action, showing progress and errors.
        let act = {
            let ctx = ctx.clone();
            move |kind: &'static str| {
                let ctx = ctx.clone();
                let (mail, pass, c) = (email.read().trim().to_owned(), password.read().clone(), code.read().trim().to_owned());
                let missing = match kind {
                    "sign_in" | "sign_up" => (mail.is_empty() || pass.is_empty()).then_some("Enter your email and password."),
                    "verify" => c.is_empty().then_some("Enter the code from your email."),
                    "forgot" => mail.is_empty().then_some("Enter your email."),
                    "reset" => (c.is_empty() || pass.is_empty()).then_some("Enter the code and a new password."),
                    "delete" => pass.is_empty().then_some("Enter your password to confirm."),
                    _ => None,
                };
                if let Some(msg) = missing {
                    return error.set(Some(msg.into()));
                }
                busy.set(true);
                error.set(None);
                spawn(async move {
                    let result = match kind {
                        "sign_in" => sync::sign_in(&ctx, &mail, &pass).await,
                        "sign_up" => sync::sign_up(&ctx, &mail, &pass).await.map(|needs| {
                            if needs {
                                form.set(Form::Verify)
                            }
                        }),
                        "verify" => sync::verify(&ctx, &mail, &c).await,
                        "forgot" => sync::forgot_password(&ctx, &mail).await.map(|()| form.set(Form::Reset)),
                        "reset" => sync::reset_password(&ctx, &mail, &c, &pass).await,
                        "delete" => sync::delete_account(&ctx, &pass).await,
                        _ => sync::sign_out_everywhere(&ctx).await,
                    };
                    busy.set(false);
                    match result {
                        Ok(()) => {
                            password.set(String::new());
                            // forgot and sign_up already moved to their next form.
                            if !matches!(kind, "forgot" | "sign_up") {
                                form.set(Form::SignIn);
                            }
                            deleting.set(false);
                        }
                        Err(e) => error.set(Some(e)),
                    }
                });
            }
        };
        let action = |text: &'static str, primary: bool, kind: &'static str| {
            let mut act = act.clone();
            let (bg, fg) = if primary { theme().primary } else { (theme().raised, (255, 255, 255)) };
            Focusable::new(button(text, bg, fg), move |()| act(kind))
        };
        let switch = |text: &'static str, to: Form| {
            Focusable::new(label().text(text).font_size(14.).color(theme().muted), move |()| {
                error.set(None);
                form.set(to)
            })
        };
        let field = |label_text: &'static str, el: Input| {
            rect().width(Size::fill()).spacing(6.).child(label().text(label_text).font_size(14.).color(theme().muted)).child(el)
        };
        let password_field = |label_text: &'static str| field(label_text, Input::new(password).width(Size::fill()).mode(InputMode::new_password()));
        let row = || rect().horizontal().spacing(12.);
        // Read for its subscription: sync status changes whenever the account does.
        let status = ctx.sync_status.read().clone();

        let body = match (account(&ctx.store), *form.read()) {
            (Some(acc), _) => {
                let ctx2 = ctx.clone();
                rect()
                    .spacing(12.)
                    .child(label().text(format!("Signed in as {}", acc.email)).font_size(20.))
                    .child(label().text(if status.is_empty() { "Syncing…".into() } else { status }).font_size(14.).color(theme().muted))
                    .child(
                        row()
                            .child(Focusable::new(button("Sync now", theme().raised, (255, 255, 255)), {
                                let ctx = ctx.clone();
                                move |()| ctx.sync_soon()
                            }))
                            .child(Focusable::new(button("Sign out", theme().raised, (255, 255, 255)), move |()| sync::sign_out(&ctx2)))
                            .child(action("Sign out everywhere", false, "sign_out_all")),
                    )
                    .child(if *deleting.read() {
                        rect()
                            .spacing(12.)
                            .padding(Gaps::new(16., 0., 0., 0.))
                            .child(paragraph().font_size(15.).color(theme().muted).span(
                                "This deletes your account and everything synced to it. What's saved on this device stays.",
                            ))
                            .child(password_field("Your password"))
                            .child(row().child(action("Delete permanently", true, "delete")).child(Focusable::new(
                                button("Cancel", theme().raised, (255, 255, 255)),
                                move |()| deleting.set(false),
                            )))
                    } else {
                        rect().padding(Gaps::new(16., 0., 0., 0.)).child(Focusable::new(
                            label().text("Delete account…").font_size(14.).color(crate::theme::ERROR),
                            move |()| deleting.set(true),
                        ))
                    })
            }
            (None, Form::Verify) => rect()
                .spacing(14.)
                .child(label().text("Check your email").font_size(22.).font_weight(FontWeight::BOLD))
                .child(field("Code from your email", Input::new(code).width(Size::fill()).placeholder("123456")))
                .child(action("Verify", true, "verify")),
            (None, Form::Forgot) => rect()
                .spacing(14.)
                .child(label().text("Reset your password").font_size(22.).font_weight(FontWeight::BOLD))
                .child(paragraph().font_size(15.).color(theme().muted).span("We'll email you a code to set a new password."))
                .child(field("Email", Input::new(email).width(Size::fill())))
                .child(row().child(action("Send code", true, "forgot")))
                .child(switch("‹ Back to sign in", Form::SignIn)),
            (None, Form::Reset) => rect()
                .spacing(14.)
                .child(label().text("Check your email").font_size(22.).font_weight(FontWeight::BOLD))
                .child(paragraph().font_size(15.).color(theme().muted).span(format!(
                    "If {} has an account, a code is on its way. It signs your other devices out.",
                    email.read().trim()
                )))
                .child(field("Code from your email", Input::new(code).width(Size::fill()).placeholder("123456")))
                .child(password_field("New password"))
                .child(row().child(action("Set new password", true, "reset")).child(action("Send again", false, "forgot")))
                .child(switch("‹ Back to sign in", Form::SignIn)),
            (None, Form::SignIn) => rect()
                .spacing(14.)
                .child(paragraph().font_size(15.).color(theme().muted).span(
                    "An account keeps your addons, My List, progress and settings on Relix's servers, so they follow you to every device.",
                ))
                .child(field("Email", Input::new(email).width(Size::fill())))
                .child(password_field("Password"))
                .child(row().child(action("Sign in", true, "sign_in")).child(action("Create account", false, "sign_up")))
                .child(switch("Forgot password?", Form::Forgot)),
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
