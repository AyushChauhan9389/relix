# mpv draws inside Relix's window

Video plays inside Relix's own window: Relix hands mpv its native window (`wid`: HWND on Windows, NSView on macOS, X11 window on Linux) and mpv renders as a child surface there, with gpu-next, hardware decoding and HDR intact. Relix's Netflix-style controls are a Lua script drawn on mpv's OSD; fullscreen and Esc are forwarded to Relix (`script-message`) because embedded mpv can't go fullscreen itself.

Changed 2026-10-30: this replaced a separate mpv window, which users found jarring. Wayland has no foreign-window embedding, so there mpv still opens its own window.

## Considered Options

- Separate mpv window (the original choice): simplest, but playback "leaves" the app.
- CPU frames drawn by Freya/Skia: works on Wayland and allows Freya-drawn controls, but copies every frame through the CPU (heavy at 4K, no HDR).
- Shared GL texture: forces Freya onto OpenGL, impossible on macOS (Metal).

## Consequences

- Relix's UI can't draw over the video; player controls live in the Lua script.
- libmpv's handle drop only detaches; the player sends `quit` explicitly or the core (kept alive by the script) plays on.
