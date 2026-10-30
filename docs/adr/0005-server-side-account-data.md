# Account data lives readable on the server, like Stremio

Decided 2026-10-30, replacing client-side encryption of the addon list. Like Stremio, the server stores each account's records (addon list included) as plain JSON and checks the password itself: the app sends email and password over HTTPS, the server stores an Argon2id hash. There is no data key or recovery key. The app has no server URL field: the address is fixed when building (`RELIX_SERVER`), defaulting to localhost for development.

The earlier design (password-derived master key wrapping a data key, recovery key at signup) kept debrid keys away from the server, but made a forgotten password lose the addon list and ruled out password reset by email. Matching Stremio was preferred.

## Consequences

- Whoever runs the server can read users' addon links, including debrid API keys. It must use HTTPS and protect its database.
- Password reset by email becomes possible (not built yet).
- Accounts created under the encrypted design don't carry over; there was no production server.
