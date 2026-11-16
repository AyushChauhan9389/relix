#!/usr/bin/env bash
# Builds the web app into web/dist: Relix (Rust, wasm32-unknown-emscripten), the page and its player.
# Needs the wasm32-unknown-emscripten Rust target and Emscripten 4 (emcc on PATH); npm install first.
set -euo pipefail
cd "$(dirname "$0")"
target=${CARGO_TARGET_DIR:-../target}
cargo build --release -p relix-app --bin relix-app --target wasm32-unknown-emscripten
mkdir -p dist
cp "$target/wasm32-unknown-emscripten/release/relix-app.js" "$target/wasm32-unknown-emscripten/release/relix_app.wasm" dist/
cp index.html dist/
cp ../crates/relix-app/assets/icon.png dist/
npx esbuild src/app.js --bundle --format=esm --minify --sourcemap --outfile=dist/app.js
echo "built web/dist"
