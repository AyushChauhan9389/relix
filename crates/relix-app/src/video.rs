//! Video drawn inside Relix's own window on every platform, Wayland included: mpv renders each
//! frame into an OpenGL framebuffer that Skia then draws like any image. Needs Freya's OpenGL
//! renderer (`RendererPreference::OpenGl`).

use std::any::Any;
use std::borrow::Cow;
use std::cell::RefCell;
use std::ffi::{CString, c_char, c_void};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use freya::prelude::*;
use freya_core::integration::*;
use libmpv2::Mpv;
use libmpv2::render::{OpenGLInitParams, RenderContext as MpvRenderContext, RenderParam, RenderParamApiType};
use skia_safe::gpu::{self, Budgeted, SurfaceOrigin};
use skia_safe::surface::BackendHandleAccess;
use skia_safe::{ImageInfo, Paint, Rect as SkRect, Surface};

/// Owns mpv's GL side. Created by the player; drawn by [`video`] elements.
pub struct VideoRenderer {
    // Declaration order is drop order: the GL objects must go before the mpv core they borrow.
    gl: RefCell<Option<Gl>>,
    /// Only `None` while dropping.
    mpv: Option<Arc<Mpv>>,
    on_frame: Arc<dyn Fn() + Send + Sync>,
    /// Tells the player the render context is up, so it can open the file.
    ready: RefCell<Option<Sender<()>>>,
}

impl Drop for VideoRenderer {
    fn drop(&mut self) {
        // The GL side goes here, where Freya's GL context is current, and before the core it borrows.
        let started = std::time::Instant::now();
        // libmpv2 frees the frame callback before the context, so swap in one that owns nothing first.
        if let Some(gl) = self.gl.borrow_mut().as_mut() {
            gl.ctx.set_update_callback(|| {});
        }
        self.gl.borrow_mut().take();
        crate::log!("player: video renderer released in {} ms (UI thread)", started.elapsed().as_millis());
        // Destroying mpv waits for its network and decoder threads to stop: never on the UI thread.
        if let Some(mpv) = self.mpv.take() {
            std::thread::spawn(move || drop(mpv));
        }
    }
}

struct Gl {
    ctx: MpvRenderContext<'static>,
    target: Option<((i32, i32), Surface)>,
}

impl VideoRenderer {
    /// `mpv` must be created with `vo=libmpv`. `on_frame` runs on mpv's thread whenever a new frame
    /// is ready; it should make the UI repaint the video element.
    pub fn new(mpv: Arc<Mpv>, ready: Sender<()>, on_frame: impl Fn() + Send + Sync + 'static) -> Self {
        Self { gl: RefCell::new(None), mpv: Some(mpv), on_frame: Arc::new(on_frame), ready: RefCell::new(Some(ready)) }
    }

    /// The player, for the controls (pause, seek, volume, tracks).
    pub fn mpv(&self) -> &Mpv {
        self.mpv.as_deref().expect("mpv is only taken while dropping")
    }

    /// Renders the current frame at `size` pixels and returns it as a Skia image.
    fn frame(&self, dctx: &mut gpu::DirectContext, size: (i32, i32)) -> Option<skia_safe::Image> {
        let mut gl = self.gl.borrow_mut();
        if gl.is_none() {
            let params = [
                RenderParam::ApiType(RenderParamApiType::OpenGl),
                RenderParam::InitParams(OpenGLInitParams { get_proc_address: gl_proc, ctx: () }),
            ];
            let ctx = self.mpv().create_render_context(params).map_err(|e| crate::log!("mpv render context: {e}")).ok()?;
            // SAFETY: `ctx` borrows `self.mpv`, which this struct keeps alive and drops after `gl`.
            let mut ctx: MpvRenderContext<'static> = unsafe { std::mem::transmute(ctx) };
            let on_frame = self.on_frame.clone();
            ctx.set_update_callback(move || on_frame());
            *gl = Some(Gl { ctx, target: None });
            // If the player stopped waiting (the window didn't draw in time) the file plays without
            // video; this brings it back. Before the file opens it just fails, harmlessly.
            let _ = self.mpv().command("video-reload", &[]);
            if let Some(ready) = self.ready.borrow_mut().take() {
                let _ = ready.send(());
            }
        }
        let gl = gl.as_mut()?;
        if gl.target.as_ref().is_none_or(|(s, _)| *s != size) {
            let info = ImageInfo::new_n32_premul(size, None);
            let surface = gpu::surfaces::render_target(dctx, Budgeted::Yes, &info, None, SurfaceOrigin::BottomLeft, None, false, None)?;
            gl.target = Some((size, surface));
        }
        let (_, surface) = gl.target.as_mut()?;
        let target = gpu::surfaces::get_backend_render_target(surface, BackendHandleAccess::FlushWrite)?;
        let fbo = target.gl_framebuffer_info()?.fboid;
        // Skia's queued work must reach the GPU before mpv draws, and mpv leaves GL state behind.
        dctx.flush_and_submit();
        // Skia's bottom-left-origin target wants the image flipped.
        let drawn = gl.ctx.render::<()>(fbo as i32, size.0, size.1, true);
        dctx.reset(None);
        if let Err(e) = drawn {
            crate::log!("mpv render: {e}");
        }
        Some(surface.image_snapshot())
    }
}

#[cfg(windows)]
fn gl_proc(_: &(), name: &str) -> *mut c_void {
    #[link(name = "opengl32")]
    unsafe extern "system" {
        fn wglGetProcAddress(name: *const std::ffi::c_char) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleA(name: *const std::ffi::c_char) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
    }
    let Ok(name) = CString::new(name) else { return std::ptr::null_mut() };
    // wgl only knows extension and post-1.1 functions; the rest live in opengl32.dll itself.
    let p = unsafe { wglGetProcAddress(name.as_ptr()) };
    if !matches!(p as isize, -1..=3) {
        return p;
    }
    unsafe { GetProcAddress(GetModuleHandleA(c"opengl32.dll".as_ptr()), name.as_ptr()) }
}

#[cfg(not(windows))]
fn gl_proc(_: &(), name: &str) -> *mut c_void {
    // Freya uses EGL on Wayland and GLX first on X11: look functions up the way the current context
    // was made. shortcut: Linux only; macOS needs CGL lookup when a Mac build exists.
    unsafe extern "C" {
        fn dlopen(file: *const c_char, flags: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    type GetProc = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type GetContext = unsafe extern "C" fn() -> *mut c_void;
    const RTLD_LAZY: i32 = 1;
    let Ok(name) = CString::new(name) else { return std::ptr::null_mut() };
    let sym = |lib: &std::ffi::CStr, sym: &std::ffi::CStr| unsafe {
        let handle = dlopen(lib.as_ptr(), RTLD_LAZY);
        if handle.is_null() { std::ptr::null_mut() } else { dlsym(handle, sym.as_ptr()) }
    };
    unsafe {
        let current = sym(c"libEGL.so.1", c"eglGetCurrentContext");
        if !current.is_null() && !std::mem::transmute::<*mut c_void, GetContext>(current)().is_null() {
            let get = sym(c"libEGL.so.1", c"eglGetProcAddress");
            return if get.is_null() { get } else { std::mem::transmute::<*mut c_void, GetProc>(get)(name.as_ptr()) };
        }
        let get = [c"libGLX.so.0", c"libGL.so.1"].into_iter().map(|lib| sym(lib, c"glXGetProcAddressARB")).find(|p| !p.is_null());
        get.map_or(std::ptr::null_mut(), |get| std::mem::transmute::<*mut c_void, GetProc>(get)(name.as_ptr()))
    }
}

/// An element that draws the renderer's current frame filling its area. `frame` should change
/// with every new frame so Freya repaints it.
pub fn video(renderer: Rc<VideoRenderer>, frame: u64) -> Video {
    Video {
        key: DiffKey::None,
        element: VideoElement {
            renderer,
            frame,
            layout: LayoutData::default(),
            event_handlers: Default::default(),
            accessibility: AccessibilityData::default(),
        },
    }
}

pub struct Video {
    key: DiffKey,
    element: VideoElement,
}

impl From<Video> for Element {
    fn from(value: Video) -> Self {
        Element::Element { key: value.key, element: Rc::new(value.element), elements: vec![] }
    }
}

impl LayoutExt for Video {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.element.layout
    }
}

impl ContainerExt for Video {}

impl AccessibilityExt for Video {
    fn accessibility_data_mut(&mut self) -> &mut AccessibilityData {
        &mut self.element.accessibility
    }
}

impl EventHandlersExt for Video {
    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.element.event_handlers
    }
}

#[derive(Clone)]
pub struct VideoElement {
    renderer: Rc<VideoRenderer>,
    frame: u64,
    layout: LayoutData,
    event_handlers: EventHandlers,
    accessibility: AccessibilityData,
}

impl PartialEq for VideoElement {
    fn eq(&self, other: &Self) -> bool {
        self.layout == other.layout && self.frame == other.frame && Rc::ptr_eq(&self.renderer, &other.renderer)
    }
}

impl ElementExt for VideoElement {
    fn changed(&self, other: &Rc<dyn ElementExt>) -> bool {
        (other.as_ref() as &dyn Any).downcast_ref::<VideoElement>().is_none_or(|o| self != o)
    }

    fn diff(&self, other: &Rc<dyn ElementExt>) -> DiffModifies {
        let Some(o) = (other.as_ref() as &dyn Any).downcast_ref::<VideoElement>() else { return DiffModifies::all() };
        let mut diff = DiffModifies::empty();
        if self.layout != o.layout {
            diff.insert(DiffModifies::LAYOUT);
        }
        if self.frame != o.frame || !Rc::ptr_eq(&self.renderer, &o.renderer) {
            diff.insert(DiffModifies::STYLE);
        }
        if self.event_handlers != o.event_handlers {
            diff.insert(DiffModifies::EVENT_HANDLERS);
        }
        diff
    }

    fn layout(&'_ self) -> Cow<'_, LayoutData> {
        Cow::Borrowed(&self.layout)
    }

    fn effect(&'_ self) -> Option<Cow<'_, EffectData>> {
        None
    }

    fn style(&'_ self) -> Cow<'_, StyleState> {
        Cow::Owned(StyleState::default())
    }

    fn text_style(&'_ self) -> Cow<'_, TextStyleData> {
        Cow::Owned(TextStyleData::default())
    }

    fn accessibility(&'_ self) -> Cow<'_, AccessibilityData> {
        Cow::Borrowed(&self.accessibility)
    }

    fn events_handlers(&'_ self) -> Option<Cow<'_, EventHandlers>> {
        Some(Cow::Borrowed(&self.event_handlers))
    }

    fn should_measure_inner_children(&self) -> bool {
        false
    }

    fn render(&self, context: RenderContext) {
        let area = context.layout_node.visible_area();
        let size = (area.width().round() as i32, area.height().round() as i32);
        if size.0 <= 0 || size.1 <= 0 {
            return;
        }
        let Some(mut dctx) = context.canvas.direct_context() else { return };
        if let Some(image) = self.renderer.frame(&mut dctx, size) {
            let rect = SkRect::new(area.min_x(), area.min_y(), area.max_x(), area.max_y());
            context.canvas.draw_image_rect(&image, None, rect, &Paint::default());
        }
    }
}
