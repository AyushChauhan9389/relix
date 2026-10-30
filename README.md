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

## Portable Windows build

```powershell
powershell -ExecutionPolicy Bypass -File packaging\portable.ps1
```

Produces `dist\Relix-<version>-windows-x64-portable.zip`: `relix-app.exe`, `libmpv-2.dll`, `relix-server.exe` and a
`portable` marker. With the marker next to the exe, Relix keeps its data in a `data` folder beside it instead of
`%APPDATA%
elix`, so the folder can be carried around. Release builds open without a console window.

## Sync server

`relix-server` is one binary with one SQLite file; accounts are optional and the app works without one.

```sh
cargo run -p relix-server        # listens on 0.0.0.0:8787, prints verification codes to stdout
```

Configure with environment variables: `RELIX_DB`, `RELIX_ADDR`, `RELIX_SMTP_URL` + `RELIX_SMTP_FROM` (emailed codes),
`RELIX_VERIFY_EMAIL=true|false`. Like Stremio, the server stores each account's addons, library, progress and
settings readable (debrid keys included), so run it behind HTTPS.

The app has no server field: build it against your server with `RELIX_SERVER=https://api.example.com` set
(`cargo build --release`, or the portable script). Without it, builds talk to `http://localhost:8787`.

### Admin addons

Set `RELIX_ADMIN_TOKEN` on the server to publish addons to every signed-in account. Users see them under
**Addons → From your server** with one switch (on by default, synced); their own copy of the same addon wins.

```sh
export RELIX_ADMIN_TOKEN=...                 # same value the server runs with
relix-server admin add https://anime-kitsu.strem.fun/manifest.json
relix-server admin list
relix-server admin remove 1                  # RELIX_SERVER_URL defaults to http://127.0.0.1:8787
```

## Releasing an update

1. Bump `version` in `crates/relix-app/Cargo.toml`.
2. Run `packaging/portable.ps1` (it builds against `https://relix.carbonkit.tech` unless `RELIX_SERVER` is set). It builds the app with the
   public key in `packaging/update-key.pub` and signs the zip with `%USERPROFILE%/.relix/update-signing.key`
   (or `RELIX_UPDATE_KEY`), writing `dist/latest.json`.
3. Copy the zip and `latest.json` into the server's `RELIX_UPDATE_DIR`: see `packaging/server/README.md`. Running copies pick it up
   on their next launch and offer "Restart".

Keep `update-signing.key` safe and out of git: whoever has it can ship code to every install. A new key is made with
`relix-server update keygen <file>` (it prints the public key for `packaging/update-key.pub`).

