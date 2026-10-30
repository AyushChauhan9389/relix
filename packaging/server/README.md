# Production server: relix.carbonkit.tech

Ubuntu VM on Google Cloud. relix-server runs under systemd as the `relix` user, listening on `127.0.0.1:8787`; Caddy
serves it on HTTPS with an automatic Let's Encrypt certificate.

| What | Where |
|---|---|
| Binary | `/usr/local/bin/relix-server` |
| Settings (root only; admin token, SMTP) | `/etc/relix/relix.env` |
| Database and update files | `/var/lib/relix/` (`relix-server.db`, `updates/`) |
| Service | `/etc/systemd/system/relix-server.service` (copy here) |
| HTTPS | `/etc/caddy/Caddyfile` (copy here) |
| Logs | `journalctl -u relix-server -f` |

**Redeploy** after committing: `bash packaging/server/deploy.sh` (builds on the server from `git archive HEAD`).

**Publish an app update:** run the Release workflow on GitHub; it uploads the builds and `latest-<os>.json` with
`PUT /admin/update/<name>` (admin token).

**Redeploy also** uploads `website/` to `/var/www/relix` and the Caddyfile.

**Settings change:** edit `/etc/relix/relix.env`, then `systemctl restart relix-server`. Adding `RELIX_SMTP_URL` and
`RELIX_SMTP_FROM` turns on emailed verification codes.

**First-time setup on a new machine:** install `build-essential caddy` and rustup; create the `relix` system user;
write `/etc/relix/relix.env` (`RELIX_DB`, `RELIX_ADDR=127.0.0.1:8787`, `RELIX_UPDATE_DIR`, `RELIX_ADMIN_TOKEN`, mode
600); copy the service and Caddyfile; `systemctl enable --now relix-server`; `systemctl reload caddy`; then deploy.
