# Own addon client, no stremio-core

Relix speaks the Stremio addon protocol (plain HTTP + JSON) with its own client instead of embedding stremio-core, which would have given addons, library and Stremio account sync for free. We chose to rebuild because Relix has its own account system (profiles, client-side encryption) and owning the model end to end beats bending stremio-core's account and storage assumptions. Compatibility means talking to addons exactly like Stremio does; routing (resource/type/idPrefix matching) and `extra` path encoding are the subtle parts and are covered by tests.
