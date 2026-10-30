# Signed self-update from the Relix server

Decided 2026-10-30, replacing "notify only". Release builds check `GET /update/latest` on the Relix server at launch. If a newer version is listed, they download it in the background, check its ed25519 signature against the public key built into the app (`RELIX_UPDATE_PUBKEY`), and unpack it. A banner then offers "Restart". Restarting renames the running exe and libmpv to `*.old` (Windows allows renaming files in use), copies the new files in, and starts the new version; the next launch deletes the `*.old` files. The signature covers the version and the file together, so an old build can't be relabelled as new.

The server, not GitHub, hosts updates because the repository is private. The signing key's private half lives outside the repo (`%USERPROFILE%\.relix\update-signing.key` by default), so a compromised server can't push its own build.

## Consequences

- Only the portable Windows build updates itself (it unpacks with Windows' built-in `tar`); Linux and macOS will use their package managers or need their own updater.
- Losing the private key means installed copies can't be updated until users download a build with a new key by hand.
- If copying fails halfway, the files done so far stay new; each failed file is put back from `*.old`.
