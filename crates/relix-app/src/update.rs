//! Self-update (ADR 0009): on launch, ask the Relix server for the latest build, download and verify
//! its signed zip in the background, and swap the files in when the user restarts.

use std::path::{Path, PathBuf};

use freya::prelude::WritableUtils;
use relix_proto::update::{UpdateManifest, is_newer, signed_message, verify};

use crate::{Ctx, data_dir, get};

/// The release signing key's public half, set when building releases; no key, no updates.
const PUBLIC_KEY: Option<&str> = option_env!("RELIX_UPDATE_PUBKEY");

/// A newer version, downloaded and verified, waiting for a restart.
#[derive(Debug, Clone, PartialEq)]
pub struct Ready {
    pub version: String,
    pub install: Install,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Install {
    /// Folder holding the new relix-app.exe and its files, swapped in on restart.
    Files(PathBuf),
    /// The new .msi: a Program Files install can't replace its own files, so the installer
    /// upgrades it on restart (Windows asks for admin once).
    Msi(PathBuf),
}

/// Whether this copy can write to its own folder (not, say, Program Files without admin).
fn can_install() -> bool {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return false };
    let probe = dir.join(".relix-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

/// Checks for and prepares an update; sets `ctx.update` when one is ready.
pub async fn check(ctx: Ctx) {
    let (Some(key), false) = (PUBLIC_KEY, cfg!(debug_assertions)) else { return };
    match prepare(&ctx, key).await {
        Ok(Some(ready)) => {
            let mut update = ctx.update;
            update.set(Some(ready));
        }
        Ok(None) => {}
        Err(e) => eprintln!("update: {e}"),
    }
}

/// The "Check for updates" button in Settings: checks now and says what it found.
pub async fn check_now(ctx: Ctx) -> String {
    let (Some(key), false) = (PUBLIC_KEY, cfg!(debug_assertions)) else {
        return "Updates are off in development builds.".into();
    };
    match prepare(&ctx, key).await {
        Ok(Some(ready)) => {
            let version = ready.version.clone();
            let mut update = ctx.update;
            update.set(Some(ready));
            format!("Relix {version} is ready.")
        }
        Ok(None) => format!("You're up to date (Relix {}).", env!("CARGO_PKG_VERSION")),
        Err(e) => format!("Couldn't check for updates: {e}"),
    }
}

async fn prepare(ctx: &Ctx, key: &str) -> Result<Option<Ready>, String> {
    // A copy that can't write to its own folder is a Program Files (.msi) install on Windows; on
    // Linux that's the .deb, which apt updates.
    let msi = !can_install();
    if msi && !cfg!(windows) {
        return Ok(None);
    }
    let os = if msi { "msi" } else { std::env::consts::OS };
    let latest: UpdateManifest = get(ctx.http.clone(), crate::sync::server_url(&format!("/update/latest?os={os}"))).await.map_err(|e| e.to_string())?;
    if !is_newer(&latest.version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let url = crate::sync::server_url(&format!("/update/files/{}", latest.file));
    let res = ctx.http.get(url).send().await.and_then(reqwest::Response::error_for_status).map_err(|e| e.to_string())?;
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    if !verify(key, &signed_message(&latest.version, &bytes), &latest.signature) {
        return Err(format!("{} isn't signed by Relix; ignoring it", latest.file));
    }
    let dir = data_dir().join("update");
    let install = if msi {
        let path = dir.join(&latest.file);
        tokio::task::spawn_blocking(move || {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, &bytes)).map(|()| path)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
        .map(Install::Msi)?
    } else {
        Install::Files(tokio::task::spawn_blocking(move || unpack(&dir, &bytes)).await.map_err(|e| e.to_string())??)
    };
    Ok(Some(Ready { version: latest.version, install }))
}

/// Unpacks the zip into a fresh `dir` and returns the folder that holds relix-app.exe.
// shortcut: shells out to `tar`, which Windows 10+ ships (reads zip) and Linux has (reads .tar.gz); macOS
// builds will need their own package format.
fn unpack(dir: &Path, zip: &[u8]) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(dir);
    let out = dir.join("files");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let zip_path = dir.join("update.zip");
    std::fs::write(&zip_path, zip).map_err(|e| e.to_string())?;
    // Windows' own tar reads zips; another `tar` earlier on PATH (Git's GNU tar) can't.
    let tar = if cfg!(windows) { std::env::var("SystemRoot").map_or("tar".into(), |r| format!("{r}\\System32\\tar.exe")) } else { "tar".into() };
    let ok = std::process::Command::new(tar).arg("-xf").arg(&zip_path).arg("-C").arg(&out).status().map_err(|e| e.to_string())?;
    if !ok.success() {
        return Err("couldn't unpack the update".into());
    }
    // The zip holds one top folder (Relix-x.y.z-windows-x64-portable).
    [out.clone()]
        .into_iter()
        .chain(std::fs::read_dir(&out).map_err(|e| e.to_string())?.flatten().map(|e| e.path()))
        .find(|p| p.join(app_exe_name()).is_file())
        .ok_or_else(|| "the update has no relix-app".into())
}

fn app_exe_name() -> String {
    format!("relix-app{}", std::env::consts::EXE_SUFFIX)
}

/// [`apply`] on a background thread: copying a 100 MB install would freeze the window.
pub fn apply_in_background(ready: Ready) {
    std::thread::spawn(move || {
        if let Err(e) = apply(&ready) {
            eprintln!("update: {e}");
        }
    });
}

/// Copies the update over this install and starts the new version. Files in use (the running exe,
/// libmpv) are renamed to `*.old` first, which Windows allows; `cleanup` removes them next launch.
pub fn apply(ready: &Ready) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let install = exe.parent().ok_or(std::io::ErrorKind::NotFound)?;
    let files = match &ready.install {
        Install::Files(files) => files,
        Install::Msi(msi) => {
            // msiexec asks for admin itself and upgrades in place; Relix reopens unelevated after.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                std::process::Command::new("cmd")
                    // `&`, not `&&`: Relix reopens even when the install is declined or fails.
                    .raw_arg(format!("/C msiexec /i \"{}\" /passive & start \"\" \"{}\"", msi.display(), exe.display()))
                    .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                    .spawn()?;
                std::process::exit(0);
            }
            #[cfg(not(windows))]
            return Err(std::io::Error::other(format!("{} can only install on Windows", msi.display())));
        }
    };
    // The single-file portable build stays one file: rebuild it from the update's exe and libmpv.
    #[cfg(windows)]
    if crate::bundled::appended_len(&exe).is_some() {
        let old = old_name(&exe);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old)?;
        let new = (files.join(app_exe_name()), files.join("libmpv-2.dll"));
        if let Err(e) = crate::bundled::write(&exe, &new.0, &new.1) {
            let _ = std::fs::rename(&old, &exe);
            return Err(e);
        }
        std::process::Command::new(&exe).arg("--after-update").spawn()?;
        std::process::exit(0);
    }
    swap_files(files, install, |from, to| std::fs::copy(from, to).map(drop))?;
    std::process::Command::new(install.join(app_exe_name())).arg("--after-update").spawn()?;
    std::process::exit(0);
}

/// Copies every file in `from` over `install`, all or nothing: replaced files are kept as `.old`
/// (a running exe can be renamed, not overwritten) and every one goes back if any copy fails.
fn swap_files(from: &Path, install: &Path, copy: impl Fn(&Path, &Path) -> std::io::Result<()>) -> std::io::Result<()> {
    let mut done: Vec<(PathBuf, bool)> = vec![];
    for entry in std::fs::read_dir(from)?.flatten().filter(|e| e.path().is_file()) {
        let target = install.join(entry.file_name());
        let old = old_name(&target);
        let _ = std::fs::remove_file(&old);
        let moved = target.exists() && std::fs::rename(&target, &old).is_ok();
        done.push((target.clone(), moved));
        if let Err(e) = copy(&entry.path(), &target) {
            for (target, moved) in done.iter().rev() {
                let _ = std::fs::remove_file(target);
                if *moved {
                    let _ = std::fs::rename(old_name(target), target);
                }
            }
            return Err(e);
        }
    }
    Ok(())
}

fn old_name(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    path.with_file_name(name)
}

/// Removes files left over from the last update.
pub fn cleanup() {
    let Some(install) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return };
    for entry in std::fs::read_dir(install).into_iter().flatten().flatten() {
        if entry.path().extension().is_some_and(|e| e == "old") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let _ = std::fs::remove_dir_all(data_dir().join("update"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_failed_swap_puts_every_old_file_back() {
        let root = std::env::temp_dir().join(format!("relix-swap-{}", std::process::id()));
        let (new, install) = (root.join("new"), root.join("install"));
        let _ = std::fs::remove_dir_all(&root);
        for dir in [&new, &install] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for name in ["a", "b", "c"] {
            std::fs::write(install.join(name), "old").unwrap();
            std::fs::write(new.join(name), "new").unwrap();
        }
        std::fs::write(new.join("added"), "new").unwrap();

        // The third copy fails, whatever order the folder lists in.
        let copies = std::cell::Cell::new(0);
        let failing = |from: &std::path::Path, to: &std::path::Path| {
            copies.set(copies.get() + 1);
            if copies.get() == 3 { Err(std::io::ErrorKind::Other.into()) } else { std::fs::copy(from, to).map(drop) }
        };
        assert!(super::swap_files(&new, &install, failing).is_err());
        for name in ["a", "b", "c"] {
            assert_eq!(std::fs::read_to_string(install.join(name)).unwrap(), "old", "{name}");
            assert!(!install.join(format!("{name}.old")).exists());
        }
        assert!(!install.join("added").exists(), "files the update added are gone too");

        super::swap_files(&new, &install, |from, to| std::fs::copy(from, to).map(drop)).unwrap();
        for name in ["a", "b", "c", "added"] {
            assert_eq!(std::fs::read_to_string(install.join(name)).unwrap(), "new");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    use super::*;

    #[test]
    fn old_files_keep_their_name_plus_old() {
        assert_eq!(old_name(Path::new("C:/Relix/libmpv-2.dll")), Path::new("C:/Relix/libmpv-2.dll.old"));
    }
}
