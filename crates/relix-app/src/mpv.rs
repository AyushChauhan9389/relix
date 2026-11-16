//! mpv on desktop and Android; on the web a stand-in with the same calls, which do nothing (the
//! page's own player plays there, web.rs).

#[cfg(not(target_os = "emscripten"))]
pub use libmpv2::Mpv;

#[cfg(target_os = "emscripten")]
#[derive(Default)]
pub struct Mpv;

#[cfg(target_os = "emscripten")]
impl Mpv {
    pub fn set_property<T>(&self, _: &str, _: T) -> Result<(), String> {
        Ok(())
    }

    pub fn get_property<T>(&self, _: &str) -> Result<T, String> {
        Err("no mpv on the web".into())
    }

    pub fn command(&self, _: &str, _: &[&str]) -> Result<(), String> {
        Ok(())
    }
}
