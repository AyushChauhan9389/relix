// Relix for Android: Gradle builds the Rust app (crates/relix-app) with cargo-ndk and packs it with
// libmpv into an APK. Run ./fetch-mpv.sh once first. Based on Freya's Android example.
plugins {
    alias(libs.plugins.android.application)
}

val repo = rootProject.projectDir.resolve("../..")
val jniLibs = projectDir.resolve("src/main/jniLibs")
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
        minSdk = 31
        targetSdk = 36
        // 0.1.9 -> 109: every release must count up.
        versionCode = relixVersion.split('.').fold(0) { acc, part -> acc * 100 + part.toInt() }
        versionName = relixVersion
        ndk { abiFilters += "arm64-v8a" }
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
    commandLine("cargo", "ndk", "-t", "arm64-v8a", "--platform", "31",
        "rustc", "-p", "relix-app", "--lib", "--release", "--crate-type", "cdylib")
    doFirst {
        check(jniLibs.resolve("arm64-v8a/libmpv.so").exists()) { "libmpv missing: run packaging/android/fetch-mpv.sh first" }
    }
    doLast {
        repo.resolve("target/aarch64-linux-android/release/librelix_app.so")
            .copyTo(jniLibs.resolve("arm64-v8a/librelix_app.so"), overwrite = true)
    }
}

tasks.named("preBuild") {
    dependsOn(relixIcon, buildRust)
}

dependencies {
    implementation(libs.androidx.appcompat)
}
