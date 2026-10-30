# Local-first, accounts optional

Relix works fully offline with no account: profiles, addon lists, library and progress live in a local SQLite database whose records have the same shape as the server's. Signing in only attaches existing profiles to an account and turns on sync, to the Relix server fixed at build time (ADR 0005). This keeps playback independent of the backend.
