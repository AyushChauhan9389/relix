#!/usr/bin/env bash
# Signs a release's auto-update payloads in dist/ and uploads them to the update server.
# Usage: publish-update.sh VERSION [nightly]. Needs dist/relix-server (the signer), KEY (the
# signing key), TOKEN (the server's admin token) and RELIX_SERVER. Used by release.yml and
# publish-update.yml. Builds without an APK (before Android updated itself) skip Android.
set -euo pipefail
version=$1 nightly=${2:-}
chmod +x dist/relix-server
printf '%s' "$KEY" > signing.key
trap 'rm -f signing.key' EXIT
# os name -> the file its installs update from (.msi: Program Files installs upgrade through it).
declare -A files=(
  [windows]="Relix-$version-windows-x64-portable.zip"
  [linux]="Relix-$version-linux-x64.tar.gz"
  [msi]="Relix-$version-windows-x64.msi"
  [android]="Relix-$version-android.apk"
)
up() { curl -fsS -X PUT --data-binary "@dist/$1" -H "authorization: Bearer $TOKEN" "$RELIX_SERVER/admin/update/$1"; }
manifests=()
for os in windows linux msi android; do
  file=${files[$os]}
  [ -f "dist/$file" ] || { echo "no $file, skipping $os"; continue; }
  dist/relix-server update sign signing.key "dist/$file" "$version" > "dist/nightly-$os.json"
  up "$file"
  manifests+=("nightly-$os.json")
  # Stable reaches nightly users too: it's newer than its nightlies.
  if [ -z "$nightly" ]; then
    cp "dist/nightly-$os.json" "dist/latest-$os.json"
    manifests+=("latest-$os.json")
  fi
done
# Builds before per-OS manifests ask for latest.json; it's the Windows one.
if [ -z "$nightly" ] && [ -f dist/latest-windows.json ]; then
  cp dist/latest-windows.json dist/latest.json
  manifests+=(latest.json)
fi
# Manifests last, so no client sees a version whose file isn't there yet.
for m in "${manifests[@]}"; do up "$m"; done
# The web version at /app (the web job's files): index.html last, so a page never loads half of it.
if [ -f dist/relix_app.wasm ]; then
  web() { curl -fsS -X PUT --data-binary "@dist/$1" -H "authorization: Bearer $TOKEN" "$RELIX_SERVER/admin/web/$1"; }
  for f in relix_app.wasm relix-app.js app.js icon.png index.html; do web "$f"; done
  echo "published the web app"
fi
echo "published $version: ${manifests[*]}"
