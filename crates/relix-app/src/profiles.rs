use freya::prelude::*;
use freya::router::*;

use crate::home::button;
use crate::store::Profile;
use crate::{Ctx, Route};

const MUTED: (u8, u8, u8) = (170, 170, 170);
const AVATAR_COLORS: [(u8, u8, u8); 5] = [(0, 113, 235), (229, 9, 20), (70, 180, 105), (240, 170, 30), (150, 80, 220)];

/// A profile's avatar colour, stable for its id.
pub fn avatar_color(p: &Profile) -> (u8, u8, u8) {
    AVATAR_COLORS[p.id.bytes().map(usize::from).sum::<usize>() % AVATAR_COLORS.len()]
}

/// "Who's watching?": pick a profile or add one.
#[derive(PartialEq)]
pub struct Profiles;

impl Component for Profiles {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let mut list = use_state(|| ctx.store.profiles().unwrap_or_default());
        let mut adding = use_state(|| false);
        let mut name = use_state(String::new);
        let kids = use_state(|| false);
        let mut pin = use_state(String::new);
        let mut error = use_state(|| None::<String>);

        let create = {
            let ctx = ctx.clone();
            move |_| {
                let (n, k, p) = (name.read().trim().to_owned(), *kids.read(), pin.read().trim().to_owned());
                if n.is_empty() {
                    return error.set(Some("Give the profile a name.".into()));
                }
                if k && !p.is_empty() && !(p.len() == 4 && p.chars().all(|c| c.is_ascii_digit())) {
                    return error.set(Some("The PIN must be 4 digits.".into()));
                }
                let result = ctx.store.create_profile(&n, k).and_then(|mut profile| {
                    profile.pin = (k && !p.is_empty()).then_some(p);
                    ctx.store.save_profile(&profile)
                });
                match result {
                    Ok(()) => {
                        list.set(ctx.store.profiles().unwrap_or_default());
                        adding.set(false);
                        name.set(String::new());
                        pin.set(String::new());
                        error.set(None);
                    }
                    Err(e) => error.set(Some(format!("Couldn't create the profile: {e}"))),
                }
            }
        };

        rect()
            .expanded()
            .center()
            .spacing(36.)
            .child(label().text("Who's watching?").font_size(44.).font_weight(FontWeight::BOLD))
            .child(
                rect()
                    .horizontal()
                    .spacing(28.)
                    .children(list.read().iter().map(|p| {
                        let (ctx, p2) = (ctx.clone(), p.clone());
                        avatar(p)
                            .on_press(move |_| {
                                ctx.switch_profile(p2.clone());
                                let _ = RouterContext::get().replace(Route::Home);
                            })
                            .into_element()
                    }))
                    .child(add_tile().on_press(move |_| adding.toggle())),
            )
            .maybe(*adding.read(), |el| {
                el.child(
                    rect()
                        .width(Size::px(420.))
                        .padding(Gaps::new_all(20.))
                        .spacing(14.)
                        .corner_radius(8.)
                        .background((32, 32, 32))
                        .child(Input::new(name).width(Size::fill()).placeholder("Name").auto_focus(true))
                        .child(
                            rect()
                                .horizontal()
                                .spacing(12.)
                                .cross_align(Alignment::Center)
                                .child(Switch::new(kids))
                                .child(label().text("Kids profile: only addons you allow, never adult ones").font_size(14.).color(MUTED)),
                        )
                        .maybe(*kids.read(), |el| {
                            el.child(Input::new(pin).width(Size::fill()).placeholder("PIN to lock its addons (optional, 4 digits)"))
                        })
                        .maybe_child(error.read().clone().map(|e| label().text(e).font_size(14.).color((229, 9, 20))))
                        .child(button("Create", (229, 9, 20), (255, 255, 255)).on_press(create)),
                )
            })
    }
}

fn avatar(p: &Profile) -> Rect {
    let initial = p.name.chars().next().unwrap_or('?').to_uppercase().to_string();
    rect()
        .spacing(10.)
        .cross_align(Alignment::Center)
        .child(
            rect()
                .width(Size::px(140.))
                .height(Size::px(140.))
                .corner_radius(8.)
                .background(avatar_color(p))
                .center()
                .child(label().text(initial).font_size(64.).font_weight(FontWeight::BOLD)),
        )
        .child(label().text(p.name.clone()).font_size(18.).color((200, 200, 200)))
        .maybe(p.kids, |el| el.child(label().text("KIDS").font_size(12.).font_weight(FontWeight::BOLD).color((70, 180, 105))))
}

fn add_tile() -> Rect {
    rect()
        .spacing(10.)
        .cross_align(Alignment::Center)
        .child(
            rect()
                .width(Size::px(140.))
                .height(Size::px(140.))
                .corner_radius(8.)
                .border(Border::new().fill((90, 90, 90)).width(2.))
                .center()
                .child(label().text("+").font_size(64.).color(MUTED)),
        )
        .child(label().text("Add profile").font_size(18.).color(MUTED))
}
