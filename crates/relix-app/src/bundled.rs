//! The single-file portable exe: libmpv is appended to relix-app.exe as `<exe><dll><len: u64 LE><MAGIC>`,
//! unpacked into the data folder on first run and loaded from there (it's delay-loaded, see build.rs).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"RELIXMPV";

/// The appended libmpv's length, if `exe` carries one.
pub fn appended_len(exe: &Path) -> Option<u64> {
    let mut f = File::open(exe).ok()?;
    let size = f.metadata().ok()?.len();
    f.seek(SeekFrom::End(-16)).ok()?;
    let mut tail = [0u8; 16];
    f.read_exact(&mut tail).ok()?;
    let len = u64::from_le_bytes(tail[..8].try_into().ok()?);
    (&tail[8..] == MAGIC && len + 16 <= size).then_some(len)
}

/// Writes `out` = `exe` + `dll` + trailer: a portable Relix in one file.
pub fn write(out: &Path, exe: &Path, dll: &Path) -> io::Result<()> {
    let mut o = File::create(out)?;
    io::copy(&mut File::open(exe)?, &mut o)?;
    let len = io::copy(&mut File::open(dll)?, &mut o)?;
    o.write_all(&len.to_le_bytes())?;
    o.write_all(MAGIC)
}

/// Unpacks the appended libmpv into `dir` (again only when its size changed) and has Windows load it from there.
pub fn unpack(exe: &Path, len: u64, dir: &Path) -> io::Result<()> {
    let dll = dir.join("libmpv-2.dll");
    let unpacked = (|| {
        if std::fs::metadata(&dll).map(|m| m.len()).ok() != Some(len) {
            std::fs::create_dir_all(dir)?;
            let mut f = File::open(exe)?;
            f.seek(SeekFrom::End(-16 - len as i64))?;
            let part = dll.with_extension("part");
            io::copy(&mut f.take(len), &mut File::create(&part)?)?;
            std::fs::rename(&part, &dll)?;
        }
        Ok(())
    })();
    // Even when refreshing it failed (a copy still in use), the one already there works.
    set_dll_directory(dir)?;
    unpacked
}

fn set_dll_directory(dir: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetDllDirectoryW(path: *const u16) -> i32;
    }
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
    match unsafe { SetDllDirectoryW(wide.as_ptr()) } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_unpacks_libmpv() {
        let dir = std::env::temp_dir().join(format!("relix-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (exe, dll, out) = (dir.join("app.exe"), dir.join("lib.dll"), dir.join("portable.exe"));
        std::fs::write(&exe, b"MZ exe bytes").unwrap();
        std::fs::write(&dll, b"dll bytes!").unwrap();

        assert_eq!(appended_len(&exe), None, "a plain exe carries nothing");
        write(&out, &exe, &dll).unwrap();
        assert_eq!(appended_len(&out), Some(10));

        unpack(&out, 10, &dir.join("mpv")).unwrap();
        assert_eq!(std::fs::read(dir.join("mpv/libmpv-2.dll")).unwrap(), b"dll bytes!");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
