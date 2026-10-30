use freya::prelude::*;
use relix_proto::addon::{Addon, Manifest, base_url, manifest_url};

use crate::focus::Focusable;
use crate::home::button;
use crate::{Ctx, get, stremio};
use crate::theme::theme;


/// Installed addons: install by URL or stremio:// link, reorder, configure, remove.
#[derive(PartialEq)]
pub struct Addons;

impl Component for Addons {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let mut url = use_state(String::new);
        let mut status = use_state(|| None::<String>);
        let addons = ctx.addons.read().clone();
        let admin = ctx.admin_addons.read().clone();
        let admin_on = *ctx.admin_enabled.read();

        let install = {
            let ctx = ctx.clone();
            move |input: String| {
                let ctx = ctx.clone();
                spawn(async move {
                    let Ok(manifest_url) = manifest_url(&input) else {
                        status.set(Some("That isn't an addon link. It should end in /manifest.json.".into()));
                        return;
                    };
                    status.set(Some("Installing…".into()));
                    match get::<Manifest>(ctx.http.clone(), manifest_url.to_string()).await {
                        Ok(manifest) => {
                            let name = manifest.name.clone();
                            let mut list = ctx.addons.peek().clone();
                            // Reinstalling an addon (e.g. after reconfiguring) replaces it in place.
                            let new = Addon { manifest_url, manifest };
                            match list.iter().position(|a| a.manifest.id == new.manifest.id) {
                                Some(i) => list[i] = new,
                                None => list.push(new),
                            }
                            ctx.save_addons(list);
                            url.set(String::new());
                            status.set(Some(format!("Installed {name}.")));
                        }
                        Err(e) => status.set(Some(format!("Couldn't load that addon: {e}"))),
                    }
                });
            }
        };

        ScrollView::new()
            .child(
            rect()
                .width(Size::fill())
                .padding(Gaps::new(theme().top, 48., 48., 48.))
                .spacing(16.)
                .child(label().text("Addons").font_size(32.).font_weight(FontWeight::BOLD))
                .child(
                    rect()
                        .horizontal()
                        .content(Content::Flex)
                        .width(Size::fill())
                        .spacing(12.)
                        .cross_align(Alignment::Center)
                        .child(
                            Input::new(url)
                                .width(Size::flex(1.))
                                .placeholder("Paste an addon link (https://…/manifest.json or stremio://…)")
                                .on_submit({
                                    let install = install.clone();
                                    move |text: String| install(text)
                                }),
                        )
                        .child(button("Install", theme().primary.0, theme().primary.1).on_press(move |_| install(url.read().clone()))),
                )
                .maybe_child(status.read().clone().map(|s| label().text(s).font_size(15.).color(theme().muted)))
                .children(addons.iter().enumerate().map(|(i, addon)| card(&ctx, &addons, i, addon).into_element()))
                .maybe(!admin.is_empty(), |el| {
                    let ctx = ctx.clone();
                    el.child(
                        rect()
                            .width(Size::fill())
                            .padding(Gaps::new(16., 0., 0., 0.))
                            .spacing(12.)
                            .child(
                                rect()
                                    .horizontal()
                                    .spacing(12.)
                                    .cross_align(Alignment::Center)
                                    .child(label().text("From your server").font_size(22.).font_weight(FontWeight::BOLD))
                                    .child(Switch::new(WritableSetter::new(ctx.admin_enabled.into_readable(), move |on| ctx.set_admin_enabled(on))))
                                    .child(label().text(if admin_on { "On" } else { "Off" }).font_size(14.).color(theme().muted)),
                            )
                            .child(
                                label()
                                    .text("Your server's admin provides these. Turn them off to use only your own addons.")
                                    .font_size(14.)
                                    .color(theme().muted),
                            )
                            .children(admin.iter().map(|a| admin_card(a, admin_on).into_element())),
                    )
                })
                .child(StremioPanel),
            )
            .into_element()
    }
}

fn card(ctx: &Ctx, addons: &[Addon], i: usize, addon: &Addon) -> Rect {
    let m = &addon.manifest;
    let last = addons.len() - 1;
    let configure_url = m.behavior_hints.configurable.then(|| format!("{}/configure", base_url(&addon.manifest_url)));

    rect()
        .width(Size::fill())
        .padding(Gaps::new_all(16.))
        .spacing(8.)
        .corner_radius(8.)
        .background(theme().surface)
        .child(
            rect()
                .horizontal()
                .spacing(10.)
                .cross_align(Alignment::End)
                .child(label().text(m.name.clone()).font_size(19.).font_weight(FontWeight::BOLD))
                .child(label().text(format!("v{}", m.version)).font_size(13.).color(theme().muted))
                .maybe(m.behavior_hints.adult, |el| el.child(label().text("18+").font_size(13.).color(crate::theme::ERROR))),
        )
        .maybe_child(m.description.clone().map(|d| paragraph().max_lines(2).text_overflow(TextOverflow::Ellipsis).font_size(14.).color(theme().muted).span(d)))
        .child(label().text(m.types.join(" · ")).font_size(13.).color(theme().muted))
        .child(
            rect()
                .horizontal()
                .spacing(20.)
                .maybe(i > 0, |el| el.child(action(ctx, addons, "↑ Up", move |l| l.swap(i, i - 1))))
                .maybe(i < last, |el| el.child(action(ctx, addons, "↓ Down", move |l| l.swap(i, i + 1))))
                .map(configure_url, |el, url| {
                    el.child(label().text("Configure").font_size(14.).color((230, 230, 230)).on_press(move |_| open_in_browser(&url)))
                })
                .child(action(ctx, addons, "Remove", move |l| {
                    l.remove(i);
                })),
        )
}

/// An admin-published addon: read-only here, dimmed while switched off.
fn admin_card(addon: &Addon, on: bool) -> Rect {
    let m = &addon.manifest;
    rect()
        .width(Size::fill())
        .padding(Gaps::new_all(16.))
        .spacing(6.)
        .corner_radius(8.)
        .background(theme().surface)
        .maybe(!on, |el| el.opacity(0.45))
        .child(
            rect()
                .horizontal()
                .spacing(10.)
                .cross_align(Alignment::End)
                .child(label().text(m.name.clone()).font_size(19.).font_weight(FontWeight::BOLD))
                .child(label().text(format!("v{}", m.version)).font_size(13.).color(theme().muted)),
        )
        .maybe_child(m.description.clone().map(|d| paragraph().max_lines(2).text_overflow(TextOverflow::Ellipsis).font_size(14.).color(theme().muted).span(d)))
}

/// A text button that edits the addon list and saves it.
fn action(ctx: &Ctx, addons: &[Addon], text: &'static str, edit: impl Fn(&mut Vec<Addon>) + 'static) -> Label {
    let (ctx, list) = (ctx.clone(), addons.to_vec());
    label().text(text).font_size(14.).color((230, 230, 230)).on_press(move |_| {
        let mut list = list.clone();
        edit(&mut list);
        ctx.save_addons(list);
    })
}

/// Opens `url` in the system browser.
fn open_in_browser(url: &str) {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        // `start` treats the first quoted arg as a window title, hence the empty "".
        c.args(["/C", "start", ""]);
        c
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    if let Err(e) = cmd.arg(url).spawn() {
        eprintln!("opening {url}: {e}");
    }
}

/// Move addons between Relix and a Stremio account.
#[derive(PartialEq)]
struct StremioPanel;

impl Component for StremioPanel {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let email = use_state(String::new);
        let password = use_state(String::new);
        let mut busy = use_state(|| false);
        let mut message = use_state(|| None::<String>);
        let mut connected = use_state(|| stremio::connected(&ctx));

        // Runs one Stremio action, showing progress and its outcome.
        let run = {
            let ctx = ctx.clone();
            move |action: &'static str| {
                if *busy.peek() {
                    return;
                }
                let ctx = ctx.clone();
                let (mail, pass) = (email.read().trim().to_owned(), password.read().clone());
                busy.set(true);
                message.set(None);
                spawn(async move {
                    let outcome = match action {
                        "connect" if mail.is_empty() || pass.is_empty() => Err("Enter your Stremio email and password.".to_owned()),
                        "connect" => stremio::connect(&ctx, &mail, &pass).await.map(|()| "Connected.".to_owned()),
                        "import" => stremio::import_from_stremio(&ctx).await.map(|s| format!("Imported from Stremio: {s}.")),
                        "export" => stremio::export_to_stremio(&ctx).await.map(|s| format!("Exported to Stremio: {s}.")),
                        _ => {
                            stremio::disconnect(&ctx).await;
                            Ok("Disconnected.".to_owned())
                        }
                    };
                    busy.set(false);
                    connected.set(stremio::connected(&ctx));
                    message.set(Some(outcome.unwrap_or_else(|e| e)));
                });
            }
        };
        let action = |text: &'static str, name: &'static str, primary: bool| {
            let mut run = run.clone();
            let (bg, fg) = if primary { theme().primary } else { (theme().raised, (255, 255, 255)) };
            Focusable::new(button(text, bg, fg), move |()| run(name))
        };

        let body = match connected.read().clone() {
            Some(account) => rect()
                .spacing(12.)
                .child(label().text(format!("Connected as {account}")).font_size(15.).color(theme().muted))
                .child(
                    rect()
                        .horizontal()
                        .spacing(12.)
                        .child(action("Import from Stremio", "import", true))
                        .child(action("Export to Stremio", "export", false))
                        .child(action("Disconnect", "disconnect", false)),
                ),
            None => rect()
                .width(Size::px(560.))
                .spacing(10.)
                .child(Input::new(email).width(Size::fill()).placeholder("Stremio email"))
                .child(Input::new(password).width(Size::fill()).placeholder("Stremio password").mode(InputMode::new_password()))
                .child(action("Connect Stremio account", "connect", true)),
        };

        rect()
            .width(Size::fill())
            .padding(Gaps::new(24., 0., 0., 0.))
            .spacing(12.)
            .child(label().text("Stremio").font_size(22.).font_weight(FontWeight::BOLD))
            .child(
                paragraph()
                    .font_size(14.)
                    .color(theme().muted)
                    .span("Bring the addons from your Stremio account into Relix, or send Relix's addons to Stremio. Both merge: nothing is removed, and Stremio's built-in addons are left alone. Your password goes only to Stremio and isn't stored."),
            )
            .child(body)
            .maybe(*busy.read(), |el| el.child(label().text("Working…").font_size(14.).color(theme().muted)))
            .maybe_child(message.read().clone().map(|m| label().text(m).font_size(15.)))
    }
}
