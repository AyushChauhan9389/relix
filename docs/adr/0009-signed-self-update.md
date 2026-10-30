# Signed self-update from the Relix server

Decided 2026-10-30, replacing "notify only". Release builds check `GET /update/latest` on the Relix server at launch. If a newer version is listed, they download it in the background, check its ed25519 signature against the public key built into the app (`RELIX_UPDATE_PUBKEY`), and unpack it. A banner then offers "Restart". Restarting renames the running exe and libmpv to `*.old` (Windows allows renaming files in use), copies the new files in, and starts the new version; the next launch deletes the `*.old` files. The signature covers the version and the file together, so an old build can't be relabelled as new.

The server, not GitHub, hosts updates because the repository is private. The signing key's private half lives outside the repo (`%USERPROFILE%\.relix\update-signing.key` by default), so a compromised server can't push its own build.

## Consequences

- Windows (zip) and Linux (tar.gz) builds update themselves; each OS has its own `latest-<os>.json`, and plain `latest.json` stays the Windows one for builds that predate that. Both unpack with the system `tar`; macOS will need its own package format.
- Releases are built, signed and published only by the Release workflow in GitHub Actions (2026-10-30).
- Losing the private key means installed copies can't be updated until users download a build with a new key by hand.
- If copying fails halfway, the files done so far stay new; each failed file is put back from `*.old`.
