#!/usr/bin/env bash
# Frees space on the server before a deploy builds there: deletes update files no manifest names
# (earlier releases' installers, all still on GitHub; the server also does this on each upload once
# it runs a build that knows how), and the build's incremental cache. Prints disk use before and after.
set -euo pipefail
HOST=${RELIX_HOST:-root@relix.carbonkit.tech}
ssh "$HOST" 'set -eu
df -h / | tail -1
cd /var/lib/relix/updates 2>/dev/null && {
  keep=$(cat *.json 2>/dev/null | grep -o "\"file\":\"[^\"]*\"" | cut -d\" -f4 | sort -u)
  n=0
  for f in *; do
    case "$f" in *.json) continue ;; esac
    [ -f "$f" ] || continue
    printf "%s\n" "$keep" | grep -qxF -- "$f" && continue
    rm -f -- "$f" && n=$((n + 1))
  done
  echo "removed $n old update files"
}
rm -rf /opt/relix-src/target/release/incremental
df -h / | tail -1'
