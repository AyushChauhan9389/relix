# Client-side encryption of the addon list

Addon manifest URLs often embed Debrid API keys, so the whole addon list record is always encrypted on the device; the server never sees it. Keys are Bitwarden-style: the password is stretched with Argon2id into a master key, the server stores only a hash derived from it for login, the master key wraps a random data key, and a recovery key shown at signup can also unwrap it. New devices sign in with email and password, which derives the same master key and unwraps the data key. (QR pairing over X25519 was dropped for v1 on 2026-10-29: Relix is desktop-only, so nothing can scan the code; revisit with a phone or TV app.) Library, progress and settings stay readable by the server so they remain debuggable.

## Consequences

- Forgetting the password and losing the recovery key loses the addon list (library and progress survive).
- OAuth and passkeys are out of v1: they don't derive a key, so each would need another wrapper around the data key.
