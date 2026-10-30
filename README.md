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

## Builds

Release builds happen only in GitHub Actions. **CI** (every push) lints and tests on Windows and Linux. **Release**
(Actions → Release → Run workflow) refuses a version that's already tagged or not newer than the one the server
publishes, then builds `Relix-<version>-windows-x64-portable.zip` and `Relix-<version>-linux-x64.tar.gz`, signs them,
creates the GitHub release and uploads both to the server for auto-update.

Each build carries a `portable` marker, so Relix keeps its data in a `data` folder beside the app; delete it to use the
per-user folder instead. The Linux build needs the system libmpv.

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

1. Bump `version` in `crates/relix-app/Cargo.toml` and push.
2. Run the **Release** workflow. Running copies pick the new version up on their next launch and offer "Restart".

The workflow signs with the `RELIX_UPDATE_SIGNING_KEY` secret and uploads with `RELIX_ADMIN_TOKEN`. The public half of
the signing key is `packaging/update-key.pub`; a new pair is made with `relix-server update keygen <file>`. Whoever
has the private key can ship code to every install.
