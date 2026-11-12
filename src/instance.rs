//! Single-instance guard: a second copy of the tray app would register a
//! second tray icon, run a second engine, and fight the first one over the
//! i2c bus, so only the first instance is allowed to run.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::sync::OnceLock;

/// Holds the locked descriptor for the process lifetime; closing it (on exit
/// or crash) releases the lock for the next instance. Stored in a `static`
/// rather than leaked, so the intent is visible.
static LOCK: OnceLock<File> = OnceLock::new();

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
