# Relix

Netflix-style desktop streaming app built with [Freya](https://freyaui.dev), compatible with Stremio addons. See [PLAN.md](PLAN.md).

## Build

Needs Rust and libmpv.

- **Linux:** install libmpv (`mpv` / `libmpv-dev` / `mpv-libs`).
- **macOS:** `brew install mpv`.
- **Windows:** download the latest `mpv-dev-x86_64-v3-*.7z` from [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake/releases), extract it into `vendor/mpv/`, then generate the import library from a VS Developer prompt:

  ```sh
  cd vendor/mpv
  dumpbin -exports libmpv-2.dll   # copy the mpv_* names into mpv.def under "LIBRARY libmpv-2" / "EXPORTS"
  lib -def:mpv.def -name:libmpv-2.dll -out:mpv.lib -machine:x64
  ```

  The build copies `libmpv-2.dll` next to the exe.

```sh
cargo test                                  # addon protocol tests
cargo run -p relix-app                      # Home screen
cargo run -p relix-app -- --play <url>      # playback spike
```

## Sync server

`relix-server` is one binary with one SQLite file; accounts are optional and the app works without one.

```sh
cargo run -p relix-server        # listens on 0.0.0.0:8787, prints verification codes to stdout
```

Configure with environment variables: `RELIX_DB`, `RELIX_ADDR`, `RELIX_SMTP_URL` + `RELIX_SMTP_FROM` (emailed codes),
`RELIX_VERIFY_EMAIL=true|false`. In the app, open **Account** and point it at the server URL.
The addon list is encrypted on the device before upload; the server never sees addon URLs or debrid keys.

### Admin addons

Set `RELIX_ADMIN_TOKEN` on the server to publish addons to every signed-in account. Users see them under
**Addons → From your server** with one switch (on by default, synced); their own copy of the same addon wins.

```sh
export RELIX_ADMIN_TOKEN=...                 # same value the server runs with
relix-server admin add https://anime-kitsu.strem.fun/manifest.json
relix-server admin list
relix-server admin remove 1                  # RELIX_SERVER_URL defaults to http://127.0.0.1:8787
```

