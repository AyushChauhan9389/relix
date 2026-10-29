# No built-in torrent playback

Relix does not stream torrents itself (decided 2026-10-29; this replaces the planned librqbit engine and its P2P opt-in). Torrent (`infoHash`) streams from addons are dropped before display, so the stream list only offers what Relix can play. Users who want torrent-indexed content configure those addons with a debrid service, which turns them into ordinary URL streams. This keeps a P2P engine, its seeding and legal exposure, and the store/notarization questions it raises out of the app.

## Consequences

- Torrentio and similar addons without debrid show no streams; the detail page says why.
- If torrent support is ever wanted, it should come back as its own decision, not a toggle on this one.
