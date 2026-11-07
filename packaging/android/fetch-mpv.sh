#!/bin/sh
# Puts libmpv (and the FFmpeg and C++ libraries it needs) for Android into app/src/main/jniLibs,
# taken from an official mpv-android release (GPL, like Relix). Pinned and checked by hash.
# 64-bit ARM for phones and newer TVs, 32-bit ARM for older TVs (many run a 32-bit system).
set -eu
TAG=2026-09-17

cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fetch() { # abi sha256
    curl -fsSL -o "$tmp/$1.apk" "https://github.com/mpv-android/mpv-android/releases/download/$TAG/app-default-$1-release.apk"
    echo "$2  $tmp/$1.apk" | sha256sum -c -
    unzip -q -o "$tmp/$1.apk" "lib/$1/*" -d "$tmp"
    out=app/src/main/jniLibs/$1
    mkdir -p "$out"
    # libplayer.so is mpv-android's own Java glue; Relix talks to libmpv directly.
    for lib in "$tmp/lib/$1"/*.so; do
        [ "$(basename "$lib")" = libplayer.so ] || cp "$lib" "$out/"
    done
    ls "$out"
}

fetch arm64-v8a 0b0e0a534dd6b8d2b117f06952e0600064ad97702e48bff02543e3e32107803d
fetch armeabi-v7a 30c63f5528530f56cdece5d6466c749976f01bdcd8c8e67ae76e70a54b7243aa
