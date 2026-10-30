# Relix plan

A Netflix-style desktop streaming app built with Freya. It talks to Stremio addons directly (no stremio-core) and has its own local-first account and sync system.
Vocabulary is in [GLOSSARY.md](./GLOSSARY.md). Decisions that are hard to reverse are in [docs/adr](./docs/adr).

## Stack

| Layer | Pick |
|---|---|
| UI | `freya = "=0.5.0-rc.13"` (router, radio, remote-asset, sdk-tokio) |
| Addons | own client in `relix-proto` + `relix-app`, reqwest (rustls) + serde_json |
| Player | libmpv2 drawing inside Relix's window (`wid`), Relix's own Netflix-style Lua controls (embedded) |
| Local data | SQLite via rusqlite (bundled), `records(profile, kind, key, value, seq, dirty)` |
| Server | axum + SQLite (WAL), single binary, Server-Sent Events push |

Workspace crates: `relix-app` (Freya client), `relix-server` (sync/account), `relix-proto` (shared addon + sync types).

## Product decisions

- **Platforms:** Linux, Windows and macOS from day one. No mobile.
- **Home:** featured title (cross-fades between the first 5 items, falls back to a text title when there's no logo), Continue Watching, catalog rows (each profile can reorder or hide them, synced in settings), My List. Catalogs that need input never appear on Home. Rows page through with `skip` as you scroll.
- **Detail page:** streams grouped by addon, in addon order, filling in as each addon answers.
- **Search:** queries every catalog that supports `search` in parallel, one row per catalog, debounced at 300ms.
- **Addon install:** paste a manifest URL, or open a `stremio://` deep link (registered as a URL handler on all three OSes). "Configure" opens the addon's `/configure` page in the system browser.
- **Stremio import/export:** connect a Stremio account on the Addons screen (password sent only to Stremio; its session key is kept on the device). Import merges Stremio's addons into Relix (same addon takes Stremio's config, nothing removed, Stremio's Local Files addon skipped). Export merges the user's own addons into Stremio with their original manifests, leaving Stremio's protected built-ins and Stremio-only addons alone.
- **Torrents:** not supported ([ADR 0008](docs/adr/0008-no-built-in-torrent-playback.md)). Torrent streams are hidden; debrid-configured addons provide URL streams instead.
- **Profiles:** deferred (removed 2026-10-29). The app runs as one implicit profile; records still carry a profile id so profiles (and kids mode) can return without a migration.
- **Navigation:** spatial focus from phase 1. Arrow keys, Enter and Esc work on every screen.
- **Look:** dark, Netflix-like. All colours, spacing and type live in one theme module so a designed theme can drop in later.
- **Player:** plays inside Relix's window (separate window only on Wayland), not forced fullscreen, with Relix's own Netflix-style controls (`player.lua`, embedded in the app): back + title, centre play/pause and ±10s, red timeline, subtitles & audio panel, speed, next episode and a countdown card near the end. Finishing an episode (or pressing Next) auto-plays the next one from the same release (`bingeGroup`). Progress comes back from libmpv events.
- **Defaults:** Cinemeta and OpenSubtitles v3 come preinstalled.

## Accounts and sync

- **Local-first:** the app works fully without an account. Signing in only enables sync, to the official server by default or to a custom server URL.
- **What syncs, per profile:** the addon list (one record), library items, progress, watched flags and settings.
- **Conflicts:** the latest write wins per record, ordered by server sequence numbers. Watched flags only grow, and unwatching is an explicit entry ([ADR 0004](docs/adr/0004-sync-conflict-rules.md)).
- **Encryption:** Bitwarden-style ([ADR 0005](docs/adr/0005-client-side-encryption.md)). The addon list is always encrypted on the device. Library, progress and settings stay readable by the server.
- **Login:** email and password on every device (it unlocks the data key too). QR/pairing deferred until a phone or TV app exists. No OAuth or passkeys in v1.
- **Email:** the official server verifies the email address with a code over SMTP. Self-hosters can turn verification off.
- **Admin addons:** the server admin publishes addons (`relix-server admin add|list|remove`, `RELIX_ADMIN_TOKEN`) that every signed-in account receives live. Users have one switch, on by default and synced; their own copy of an addon wins. Signed-out users get none.

## Shipping

- **CI:** GitHub Actions runs clippy and the tests on all three OSes.
- **Releases:**
  - Windows: NSIS installer with `libmpv-2.dll` bundled.
  - Linux: Flatpak and AUR.
  - macOS: `.app` with the libmpv dylibs bundled, unsigned until an Apple Developer account exists.
- **Updates:** check GitHub Releases on launch and show a banner. No self-updating.
- **Telemetry:** none. Rotating local log file, with an "Open logs" button.
- **Licence:** GPL-3.0 for the app, AGPL-3.0 for the server.

## Performance budgets

- Cold start under 500ms: Home renders from SQLite before any network request.
- Frame time under 8ms while scrolling a Home screen with 20 rows (Freya hotpath bench).
- Idle RAM under 200MB, with the poster LRU cache bounded by size.
- Under 2s from picking a URL stream to its first frame, not counting the addon's own response time.

How to get there: request small posters, decode them off the main thread and shrink them to display size, keep a memory LRU plus a disk cache, virtualize rows and the row list, use one signal per data slice, and build release with `lto = "fat"` and `codegen-units = 1`.

## Testing

Targeted tests only: addon routing and extra encoding, sync merge rules, key derivation and encryption round-trips, and server integration tests on in-memory SQLite. No UI tests.

## Phases

0. **Spikes:** the workspace builds; the addon client fetches Cinemeta's home rows and a stream list, with the routing tests passing; a Freya window shows Cinemeta rows; the app launches mpv on a URL stream.
1. **MVP:** Home, detail page with seasons and episodes, stream list, URL playback, installing addons, spatial focus.
2. **Library:** library, continue-watching, search, discover, subtitles.
3. **Accounts:** server, sync, encryption, password login on every device.
4. **Polish:** auto-play the next episode, more player features, design pass.
