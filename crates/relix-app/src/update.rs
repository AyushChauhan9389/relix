//! Self-update (ADR 0009): on launch, ask the Relix server for the latest build, download and verify
//! its signed zip in the background, and swap the files in when the user restarts.

use std::path::{Path, PathBuf};

use freya::prelude::WritableUtils;
use relix_proto::update::{UpdateManifest, is_newer, signed_message, verify};

use crate::{Ctx, data_dir, get};

/// The release signing key's public half, set when building releases; no key, no updates.
const PUBLIC_KEY: Option<&str> = option_env!("RELIX_UPDATE_PUBKEY");

/// A downloaded, verified update unpacked and waiting for a restart.
#[derive(Debug, Clone, PartialEq)]
pub struct Ready {
    pub version: String,
    /// Folder holding the new relix-app.exe and its files.
    pub files: PathBuf,
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

async fn prepare(ctx: &Ctx, key: &str) -> Result<Option<Ready>, String> {
    let latest: UpdateManifest = get(ctx.http.clone(), crate::sync::server_url("/update/latest")).await.map_err(|e| e.to_string())?;
    if !is_newer(&latest.version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let url = crate::sync::server_url(&format!("/update/files/{}", latest.file));
    let res = ctx.http.get(url).send().await.and_then(reqwest::Response::error_for_status).map_err(|e| e.to_string())?;
    let zip = res.bytes().await.map_err(|e| e.to_string())?;
    if !verify(key, &signed_message(&latest.version, &zip), &latest.signature) {
        return Err(format!("{} isn't signed by Relix; ignoring it", latest.file));
    }
    let dir = data_dir().join("update");
    let files = tokio::task::spawn_blocking(move || unpack(&dir, &zip)).await.map_err(|e| e.to_string())??;
    Ok(Some(Ready { version: latest.version, files }))
}

/// Unpacks the zip into a fresh `dir` and returns the folder that holds relix-app.exe.
// shortcut: uses Windows' built-in tar (Windows 10+) because only a Windows zip is released; add a zip
// crate or per-OS packages when Linux/macOS builds update themselves.
fn unpack(dir: &Path, zip: &[u8]) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(dir);
    let out = dir.join("files");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let zip_path = dir.join("update.zip");
    std::fs::write(&zip_path, zip).map_err(|e| e.to_string())?;
    let ok = std::process::Command::new("tar").arg("-xf").arg(&zip_path).arg("-C").arg(&out).status().map_err(|e| e.to_string())?;
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

/// Copies the update over this install and starts the new version. Files in use (the running exe,
/// libmpv) are renamed to `*.old` first, which Windows allows; `cleanup` removes them next launch.
pub fn apply(ready: &Ready) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let install = exe.parent().ok_or(std::io::ErrorKind::NotFound)?;
    for entry in std::fs::read_dir(&ready.files)?.flatten().filter(|e| e.path().is_file()) {
        let target = install.join(entry.file_name());
        let old = old_name(&target);
        let _ = std::fs::remove_file(&old);
        let moved = target.exists() && std::fs::rename(&target, &old).is_ok();
        if let Err(e) = std::fs::copy(entry.path(), &target) {
            // Put the old file back so this install keeps working.
            if moved {
                let _ = std::fs::rename(&old, &target);
            }
            return Err(e);
        }
    }
    std::process::Command::new(install.join(app_exe_name())).spawn()?;
    std::process::exit(0);
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
    use super::*;

    #[test]
    fn old_files_keep_their_name_plus_old() {
        assert_eq!(old_name(Path::new("C:/Relix/libmpv-2.dll")), Path::new("C:/Relix/libmpv-2.dll.old"));
    }
}
