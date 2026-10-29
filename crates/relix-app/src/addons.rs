use freya::prelude::*;
use relix_proto::addon::{Addon, Manifest, base_url, manifest_url};

use crate::home::button;
use crate::{Ctx, get};

const MUTED: (u8, u8, u8) = (170, 170, 170);

/// Installed addons: install by URL or stremio:// link, reorder, configure, remove.
#[derive(PartialEq)]
pub struct Addons;

impl Component for Addons {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let mut url = use_state(String::new);
        let mut status = use_state(|| None::<String>);
        let addons = ctx.addons.read().clone();

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

        ScrollView::new().child(
            rect()
                .width(Size::fill())
                .padding(Gaps::new(100., 48., 48., 48.))
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
                        .child(button("Install", (229, 9, 20), (255, 255, 255)).on_press(move |_| install(url.read().clone()))),
                )
                .maybe_child(status.read().clone().map(|s| label().text(s).font_size(15.).color(MUTED)))
                .children(addons.iter().enumerate().map(|(i, addon)| card(&ctx, &addons, i, addon).into_element())),
        )
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
        .background((32, 32, 32))
        .child(
            rect()
                .horizontal()
                .spacing(10.)
                .cross_align(Alignment::End)
                .child(label().text(m.name.clone()).font_size(19.).font_weight(FontWeight::BOLD))
                .child(label().text(format!("v{}", m.version)).font_size(13.).color(MUTED))
                .maybe(m.behavior_hints.adult, |el| el.child(label().text("18+").font_size(13.).color((229, 9, 20)))),
        )
        .maybe_child(m.description.clone().map(|d| paragraph().max_lines(2).text_overflow(TextOverflow::Ellipsis).font_size(14.).color(MUTED).span(d)))
        .child(label().text(m.types.join(" · ")).font_size(13.).color(MUTED))
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
