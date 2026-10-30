#!/bin/sh
# Nightly copy of the account database (SQLite's online backup, safe while the server runs); keeps 7 days.
set -eu
dir=/var/lib/relix/backups
mkdir -p "$dir"
file="$dir/relix-$(date +%F).db"
sqlite3 /var/lib/relix/relix-server.db ".backup '$file'"
gzip -f "$file"
cp "$file.gz" "$dir/latest.db.gz"
find "$dir" -name 'relix-*.db.gz' -mtime +7 -delete
