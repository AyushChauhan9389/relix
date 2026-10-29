# Rust axum + SQLite sync server

The sync/account server is Rust (axum) on SQLite in WAL mode, shipped as one binary with WebSocket push. Rust lets the server share `relix-proto` types with the client so sync and crypto formats cannot drift; one binary with an embedded database makes self-hosting "download and run". Go and Elysia (Bun) were considered and rejected because both would duplicate the types in a second language. Postgres is deferred until a single machine can't keep up.
