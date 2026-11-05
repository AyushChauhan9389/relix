#!/bin/sh
# Puts libmpv (and the FFmpeg and C++ libraries it needs) for Android into app/src/main/jniLibs,
# taken from an official mpv-android release (GPL, like Relix). Pinned and checked by hash.
set -eu
TAG=2026-09-17
SHA256=0b0e0a534dd6b8d2b117f06952e0600064ad97702e48bff02543e3e32107803d
URL="https://github.com/mpv-android/mpv-android/releases/download/$TAG/app-default-arm64-v8a-release.apk"

cd "$(dirname "$0")"
out=app/src/main/jniLibs/arm64-v8a
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL -o "$tmp/mpv.apk" "$URL"
echo "$SHA256  $tmp/mpv.apk" | sha256sum -c -
unzip -q -o "$tmp/mpv.apk" 'lib/arm64-v8a/*' -d "$tmp"
mkdir -p "$out"
# libplayer.so is mpv-android's own Java glue; Relix talks to libmpv directly.
for lib in "$tmp"/lib/arm64-v8a/*.so; do
    [ "$(basename "$lib")" = libplayer.so ] || cp "$lib" "$out/"
done
ls "$out"
