//! mpv on desktop and Android; on the web a stand-in with the same calls, which control the page's
//! own player (web.rs → web/src/app.js): pause, speed, seeking, its position.

#[cfg(not(target_os = "emscripten"))]
pub use libmpv2::Mpv;

#[cfg(target_os = "emscripten")]
#[derive(Default)]
pub struct Mpv;

#[cfg(target_os = "emscripten")]
impl Mpv {
    pub fn set_property<T: std::fmt::Display>(&self, name: &str, value: T) -> Result<(), String> {
        crate::web::player_set(name, &value.to_string());
        Ok(())
    }

    pub fn get_property<T: std::str::FromStr>(&self, name: &str) -> Result<T, String> {
        crate::web::player_get(name).and_then(|v| v.parse().ok()).ok_or_else(|| format!("no {name}"))
    }

    pub fn command(&self, name: &str, args: &[&str]) -> Result<(), String> {
        crate::web::player_command(name, args);
        Ok(())
    }
}
