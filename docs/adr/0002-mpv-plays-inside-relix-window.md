# mpv renders into Relix's own picture

Video plays inside Relix's window on every platform, Wayland included. mpv renders each frame through its render API (`vo=libmpv`) into an OpenGL framebuffer owned by Skia, and a custom Freya element (`video.rs`) draws it like any image. Freya is told to use its OpenGL renderer for this. Relix's Netflix-style controls are still the Lua script drawn on mpv's OSD; Relix forwards the mouse (`mouse`, `keydown/keyup MBTN_LEFT`) and keys (`keypress`) to mpv, and handles Esc, fullscreen and next episode itself (`script-message`).

Changed 2026-10-30: this replaced a separate mpv window, which users found jarring.
Changed 2026-10-31: this replaced window embedding (`wid`), which Wayland doesn't allow (mpv opened its own window there, and Back crashed on Linux).

## Considered Options

- Separate mpv window (the original choice): simplest, but playback "leaves" the app.
- Window embedding (`wid`): native mpv output with HDR, but no Wayland support.
- CPU frames drawn by Freya/Skia: works everywhere, but copies every frame through the CPU (heavy at 4K, no HDR).

## Consequences

- Freya renders with OpenGL everywhere (no Vulkan/Metal). macOS's OpenGL is deprecated but still present.
- HDR output and hardware-decode zero-copy depend on the GL interop mpv finds; check 4K/HDR when it matters.
- Linux finds GL functions through EGL (Freya's default there); a macOS build needs a CGL lookup in `video.rs`.
- The render context must be freed before mpv is destroyed: `VideoRenderer` owns both in that order.
