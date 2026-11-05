# Relix for Android

Builds an APK (arm64 phones, tablets and Android TV) from `crates/relix-app`.

## Needs (Linux build machine)

- Rust with `rustup target add aarch64-linux-android` and `cargo install cargo-ndk`
- JDK 17+, Android SDK (platform 36), NDK r26d+
- `ANDROID_HOME` and `ANDROID_NDK_HOME` set

## Build

```sh
cd packaging/android
./fetch-mpv.sh              # once: libmpv + FFmpeg from mpv-android, checked by hash
./gradlew assembleRelease   # also runs cargo ndk for relix-app
```

The APK lands in `app/build/outputs/apk/release/app-release.apk`. Install with `adb install -r <apk>`.
Crashes are written to `crash.log` in the app's data folder (`adb shell run-as tech.carbonkit.relix cat files/crash.log`).

## Signing

Releases (the Release workflow) are signed with Relix's key, from the `ANDROID_KEYSTORE` (base64 of
the `.jks`) and `ANDROID_KEYSTORE_PASSWORD` secrets, alias `relix`. Locally, set `RELIX_KEYSTORE`
(path) and `RELIX_KEYSTORE_PASSWORD` to sign the same way; without them Gradle uses the debug key.
Android only installs an update signed like the installed app, so the key must never change.
