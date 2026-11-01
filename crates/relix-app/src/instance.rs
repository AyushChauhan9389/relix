//! One Relix per data folder: a second launch brings the running window forward and exits.
//! A lock file says one is running; the port it listens on (127.0.0.1 only) is how it's told.

use std::fs::{File, TryLockError};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub enum Claim {
    /// This is the running Relix: keep the lock alive and [`serve`] the listener.
    Primary(File, TcpListener),
    /// Another Relix runs here and was asked to show its window.
    Shown,
    /// Another Relix holds the lock but can't be reached; run anyway rather than not at all.
    Unsure,
}

/// Claims `dir` for this process. With `wait` (relaunched after an update) it waits for the
/// previous Relix to exit instead of handing over to it.
pub fn claim(dir: &Path, wait: bool) -> Claim {
    let _ = std::fs::create_dir_all(dir);
    let Ok(lock) = File::options().create(true).truncate(false).write(true).open(dir.join("relix.lock")) else { return Claim::Unsure };
    let deadline = Instant::now() + Duration::from_secs(if wait { 10 } else { 0 });
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(TryLockError::WouldBlock) => return if show_other(dir) { Claim::Shown } else { Claim::Unsure },
            Err(TryLockError::Error(_)) => return Claim::Unsure,
        }
    }
    // The port lives in its own file: Windows won't let another process read a locked one.
    match TcpListener::bind("127.0.0.1:0").and_then(|l| Ok((l.local_addr()?.port(), l))) {
        Ok((port, listener)) if std::fs::write(dir.join("relix.port"), port.to_string()).is_ok() => Claim::Primary(lock, listener),
        _ => Claim::Unsure,
    }
}

fn show_other(dir: &Path) -> bool {
    let Some(port) = std::fs::read_to_string(dir.join("relix.port")).ok().and_then(|p| p.trim().parse::<u16>().ok()) else { return false };
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(1))
        .and_then(|mut s| s.write_all(b"show"))
        .is_ok()
}

/// Requests to show the window, for the UI to take once it's up.
pub static SHOW: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<()>>> = Mutex::new(None);

/// Answers later launches on a background thread.
pub fn serve(listener: TcpListener) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    *SHOW.lock().unwrap() = Some(rx);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 4];
            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
            if stream.read_exact(&mut buf).is_ok() && &buf == b"show" {
                let _ = tx.send(());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_claim_shows_the_first() {
        let dir = std::env::temp_dir().join(format!("relix-instance-test-{}", std::process::id()));
        let Claim::Primary(_lock, listener) = claim(&dir, false) else { panic!("first claim should win") };
        assert!(matches!(claim(&dir, false), Claim::Shown));
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4];
        conn.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"show");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
