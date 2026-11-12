//! Single-instance guard: a second copy of the tray app would register a
//! second tray icon, run a second engine, and fight the first one over the
//! i2c bus, so only the first instance is allowed to run.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use std::time::Duration;

use crate::tray::UiCommand;

/// Holds the locked descriptor for the process lifetime; closing it (on exit
/// or crash) releases the lock for the next instance. Stored in a `static`
/// rather than leaked, so the intent is visible.
static LOCK: OnceLock<File> = OnceLock::new();

/// The only message the socket accepts: "show your main window".
const OPEN_MESSAGE: &[u8] = b"open\n";

/// Takes an exclusive, non-blocking lock on `<state>/kwmswitcher.lock`.
///
/// Returns `true` when this process may run: the lock was acquired, or it
/// could not even be attempted (read-only state dir, filesystem without
/// `flock`), because a broken lock must not keep the app from starting.
pub fn acquire() -> bool {
    if LOCK.get().is_some() {
        return true;
    }
    let path = crate::paths::lock_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
    {
        Ok(file) => file,
        Err(err) => {
            log::warn!(
                "Cannot open instance lock {}: {err}; continuing without it",
                path.display()
            );
            return true;
        }
    };
    // SAFETY: `file` is a live descriptor owned by this function, and `flock`
    // only changes its lock state; the descriptor stays open for the process
    // lifetime because `LOCK` takes ownership of it. Closing it (process
    // exit) releases the lock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        let _ = LOCK.set(file);
        return true;
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        return false;
    }
    log::warn!(
        "Cannot lock {}: {err}; continuing without it",
        path.display()
    );
    true
}

/// Tells an already-running instance to open its main window.
///
/// Returns `false` when nobody is listening (the other instance may still be
/// starting, or the socket is stale); the caller exits either way, because the
/// instance lock already proved another copy owns the app.
pub fn notify_running_instance() -> bool {
    let Ok(mut stream) = UnixStream::connect(crate::paths::ipc_socket()) else {
        return false;
    };
    stream.write_all(OPEN_MESSAGE).is_ok() && stream.flush().is_ok()
}

/// Listens for launch hand-offs for the process lifetime.
///
/// A second launch (the launcher entry, a terminal command) connects here
/// instead of starting a second engine; every connection asks the UI to show
/// the main window. Failures only cost the hand-off, never the app.
pub fn start_open_listener(ui_tx: Sender<UiCommand>) {
    let path = crate::paths::ipc_socket();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // A socket file left behind by a previous run would make `bind` fail.
    let _ = std::fs::remove_file(&path);

    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(err) => {
            log::warn!(
                "Cannot listen for launch hand-off at {}: {err}",
                path.display()
            );
            return;
        }
    };

    let spawned = std::thread::Builder::new()
        .name("ipc".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                // Never let one stuck client block later hand-offs.
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut message = [0u8; OPEN_MESSAGE.len()];
                let _ = stream.read_exact(&mut message);
                if ui_tx.send(UiCommand::OpenMainWindow).is_err() {
                    break;
                }
            }
        });
    if let Err(err) = spawned {
        log::warn!("Failed to spawn the launch listener: {err}");
    }
}
