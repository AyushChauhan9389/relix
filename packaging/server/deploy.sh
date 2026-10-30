#!/usr/bin/env bash
# Redeploys relix-server: copies the committed source to the host, builds it there and restarts the service.
# First-time setup (user, /etc/relix/relix.env, systemd unit, Caddy) is described in packaging/server/README.md.
set -euo pipefail
HOST=${RELIX_HOST:-root@relix.carbonkit.tech}

# Keep target/ between deploys so rebuilds take seconds, not minutes.
git archive --format=tar HEAD | ssh "$HOST" 'mkdir -p /opt/relix-src && cd /opt/relix-src &&
  find . -mindepth 1 -maxdepth 1 ! -name target -exec rm -rf {} + && tar -x'
ssh "$HOST" 'cd /opt/relix-src && ~/.cargo/bin/cargo build --release -p relix-server &&
  install -m 755 target/release/relix-server /usr/local/bin/relix-server &&
  systemctl restart relix-server && systemctl is-active relix-server'

# The website and the Caddy config that serves it next to the API.
tar -cf - -C website . | ssh "$HOST" 'mkdir -p /var/www/relix && tar -xf - -C /var/www/relix && chmod -R a+rX /var/www/relix'
scp -q packaging/server/Caddyfile "$HOST":/etc/caddy/Caddyfile
ssh "$HOST" 'caddy validate --config /etc/caddy/Caddyfile >/dev/null && systemctl reload caddy'
