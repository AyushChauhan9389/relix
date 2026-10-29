# mpv owns the playback window

Video plays in a window libmpv creates itself (vo=gpu-next, Relix's own Lua controls drawn on mpv's OSD), not inside Freya's window. libmpv's render API only supports OpenGL or a CPU buffer, while Freya uses Vulkan on Linux and Metal on macOS, and embedding a native child window fails on Wayland. A separate window gives zero-copy hardware decode and HDR on all three OSes with no per-platform code. An in-UI preview (CPU frames drawn through Skia) may be added later for trailers, where copy cost doesn't matter.

## Considered Options

- Embed mpv's native window as a child (HWND/NSView/X11): no Wayland.
- CPU buffer drawn by Skia: heavy at 4K60, no HDR.
- Shared GL texture: forces Freya onto OpenGL, impossible on macOS.
