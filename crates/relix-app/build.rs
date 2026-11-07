// ponytail: links the vendored Windows libmpv (vendor/mpv, see PLAN.md) or Android's fetched one; Linux/macOS use the system libmpv.
fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    // Android links the libmpv that packaging/android/fetch-mpv.sh put where the APK takes it from.
    if os == "android" {
        let abi = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("aarch64") => "arm64-v8a",
            Ok("arm") => "armeabi-v7a",
            Ok(arch) => panic!("no libmpv for Android on {arch}: fetch-mpv.sh gets arm64-v8a and armeabi-v7a"),
            Err(e) => panic!("{e}"),
        };
        let libs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/android/app/src/main/jniLibs").join(abi);
        println!("cargo:rustc-link-search=native={}", libs.display());
        println!("cargo:rerun-if-changed={}", libs.join("libmpv.so").display());
        return;
    }
    if os != "windows" {
        return;
    }
    // The logo as the exe's icon in Explorer (the window icon is set at runtime).
    println!("cargo:rerun-if-changed=assets/icon.ico");
    winresource::WindowsResource::new().set_icon("assets/icon.ico").compile().expect("embed the exe icon");
    // libmpv loads on first use, not at startup, so the single-file portable exe can unpack it first.
    println!("cargo:rustc-link-arg-bins=/DELAYLOAD:libmpv-2.dll");
    println!("cargo:rustc-link-lib=delayimp");
    let vendor = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/mpv");
    println!("cargo:rustc-link-search=native={}", vendor.display());
    println!("cargo:rerun-if-changed={}", vendor.join("mpv.lib").display());

    // OUT_DIR is target/<profile>/build/<pkg>/out; the exe lives in target/<profile>.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let profile_dir = out.ancestors().nth(3).unwrap();
    std::fs::copy(vendor.join("libmpv-2.dll"), profile_dir.join("libmpv-2.dll")).expect("copy libmpv-2.dll");
}
