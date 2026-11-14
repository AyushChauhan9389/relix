// Relix for Android: Gradle builds the Rust app (crates/relix-app) with cargo-ndk and packs it with
// libmpv into an APK. Run ./fetch-mpv.sh once first. Based on Freya's Android example.
plugins {
    alias(libs.plugins.android.application)
}

val repo = rootProject.projectDir.resolve("../..")
val jniLibs = projectDir.resolve("src/main/jniLibs")
// Phones and newer TVs are 64-bit ARM; many older TVs run a 32-bit system. `-Prelix.abis=armeabi-v7a`
// builds just one, for a quicker test build.
val abis = (findProperty("relix.abis") as String?)?.split(',') ?: listOf("arm64-v8a", "armeabi-v7a")
val rustTargets = mapOf("arm64-v8a" to "aarch64-linux-android", "armeabi-v7a" to "armv7-linux-androideabi")
val relixVersion = repo.resolve("crates/relix-app/Cargo.toml").readLines()
    .first { it.startsWith("version") }.substringAfter('"').substringBefore('"')

android {
    namespace = "tech.carbonkit.relix"
    compileSdk {
        version = release(36) {
            minorApiLevel = 1
        }
    }

    defaultConfig {
        applicationId = "tech.carbonkit.relix"
        // Android 9: older Android TVs (Mi TV and the like) never got further.
        minSdk = 28
        targetSdk = 36
        // 0.1.9 -> 109: every release must count up.
        versionCode = relixVersion.split('.').fold(0) { acc, part -> acc * 100 + part.toInt() }
        versionName = relixVersion
        ndk { abiFilters += abis }
    }

    // Releases are signed with Relix's own key (RELIX_KEYSTORE, a .jks path, and its password):
    // Android only installs an update signed like the installed app. Without it, the debug key.
    val keystore = System.getenv("RELIX_KEYSTORE")
    signingConfigs {
        if (keystore != null) {
            create("relix") {
                storeFile = file(keystore)
                storePassword = System.getenv("RELIX_KEYSTORE_PASSWORD")
                keyAlias = "relix"
                keyPassword = System.getenv("RELIX_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName(if (keystore != null) "relix" else "debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    sourceSets["main"].res.srcDir(file("build/generated/relix-res"))
}

// The desktop logo as the launcher icon and TV banner.
val relixIcon by tasks.registering(Copy::class) {
    from(repo.resolve("crates/relix-app/assets/icon.png"))
    into(file("build/generated/relix-res/drawable-nodpi"))
    rename { "ic_launcher.png" }
}

// relix-app is a plain library for desktop builds; Android loads it as a shared one (android_main).
val buildRust by tasks.registering(Exec::class) {
    workingDir(repo)
    val androidHome = System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT") ?: ""
    environment("ANDROID_HOME", androidHome)
    // The sync server is fixed at build time (sync.rs); without it a phone would call itself (localhost).
    environment("RELIX_SERVER", System.getenv("RELIX_SERVER") ?: "https://relix.carbonkit.tech")
    // Freya's theme detection (mundy) compiles a little Java against it.
    environment("ANDROID_JAR", "$androidHome/platforms/android-36/android.jar")
    // Skia has no prebuilt 32-bit ARM Android library, so that one builds Skia from source, which
    // looks for the NDK here.
    System.getenv("ANDROID_NDK_HOME")?.let { environment("ANDROID_NDK", it) }
    // 32-bit ARM generates its libmpv bindings (no bundled ones); the NDK's headers reject
    // clang's target unless it names the API level, which Rust's armv7 triple doesn't.
    environment("BINDGEN_EXTRA_CLANG_ARGS_armv7_linux_androideabi", "--target=armv7a-linux-androideabi28")
    commandLine(listOf("cargo", "ndk") + abis.flatMap { listOf("-t", it) } + listOf("--platform", "28",
        "rustc", "-p", "relix-app", "--lib", "--release", "--crate-type", "cdylib"))
    doFirst {
        for (abi in abis) {
            check(jniLibs.resolve("$abi/libmpv.so").exists()) { "libmpv missing: run packaging/android/fetch-mpv.sh first" }
        }
    }
    doLast {
        for (abi in abis) {
            repo.resolve("target/${rustTargets.getValue(abi)}/release/librelix_app.so")
                .copyTo(jniLibs.resolve("$abi/librelix_app.so"), overwrite = true)
        }
    }
}

tasks.named("preBuild") {
    dependsOn(relixIcon, buildRust)
}

dependencies {
    implementation(libs.androidx.appcompat)
}
