//! Minimal file logger on the `log` facade: one append-only file, capped in
//! size and rotated once, written straight to the kernel so a hard crash
//! loses at most the line being written.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Rotate once the file passes this size; one previous file is kept.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
/// Back-off after a failed open: without it, every log line would retry the
/// same failing `open`, which turns into a syscall storm when the state dir
/// is unwritable.
const OPEN_RETRY_INTERVAL: Duration = Duration::from_secs(30);

struct LogFile {
    file: Option<File>,
    written: u64,
    retry_at: Option<Instant>,
}

struct FileLogger {
    inner: Mutex<LogFile>,
    path: PathBuf,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Our own code logs at the configured level; dependencies (zbus,
        // cosmic-text, gpui) are clamped to Warn+ so DBus chatter and font
        // scans don't flood the file.
        let internal = record.target().starts_with(env!("CARGO_CRATE_NAME"));
        if !internal && record.level() > log::Level::Warn {
            return;
        }
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!(
            "{timestamp} [{}] {}{}\n",
            record.level(),
            record.args(),
            if internal {
                record
                    .file()
                    .map(|f| format!(" ({f}:{})", record.line().unwrap_or(0)))
                    .unwrap_or_default()
            } else {
                format!(" ({})", record.target())
            }
        );

        // The panic hook writes to its own file and never calls the logger,
        // so a panic while this lock is held cannot deadlock the process.
        let mut inner = self.inner.lock().expect("log lock poisoned");

        if inner.file.is_none() {
            if inner
                .retry_at
                .is_some_and(|retry_at| Instant::now() < retry_at)
            {
                return;
            }
            let Some((file, len)) = open_log_file(&self.path) else {
                inner.retry_at = Some(Instant::now() + OPEN_RETRY_INTERVAL);
                return;
            };
            inner.file = Some(file);
            inner.written = len;
            inner.retry_at = None;
        }

        if inner.written >= MAX_LOG_BYTES {
            if let Some(err) = rotate(&self.path, &mut inner) {
                // Drop the lock before logging: `log::warn!` re-enters this
                // logger, and locking it twice on one thread would deadlock.
                drop(inner);
                log::warn!("Failed to rotate log file {}: {err}", self.path.display());
                return;
            }
        }

        if let Some(file) = inner.file.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                inner.written += line.len() as u64;
            } else {
                // Reopen (and retry) on the next line instead of failing forever.
                inner.file = None;
                inner.retry_at = Some(Instant::now() + OPEN_RETRY_INTERVAL);
            }
        }
    }

    fn flush(&self) {
        let mut inner = self.inner.lock().expect("log lock poisoned");
        if let Some(file) = inner.file.as_mut() {
            let _ = file.flush();
        }
    }
}

fn open_log_file(path: &Path) -> Option<(File, u64)> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let len = std::fs::metadata(path).map_or(0, |meta| meta.len());
    Some((file, len))
}

/// Renames the current file to `*.log.1` and reopens a fresh one. Returns the
/// rename error, if any; the caller logs it after releasing the lock.
fn rotate(path: &Path, inner: &mut LogFile) -> Option<std::io::Error> {
    // Drop the handle before renaming so no write lands in the rotated file.
    inner.file = None;
    let rotated = path.with_extension("log.1");
    let rename_error = std::fs::rename(path, &rotated).err();
    if let Some((file, len)) = open_log_file(path) {
        inner.file = Some(file);
        // On a failed rename the old file is still there: reset the counter
        // so the next line does not retry the rename in a loop.
        inner.written = if rename_error.is_some() { 0 } else { len };
    } else {
        inner.written = 0;
        inner.retry_at = Some(Instant::now() + OPEN_RETRY_INTERVAL);
    }
    rename_error
}

/// Installs the logger writing to `path`. Safe to call once; later calls are
/// ignored (the supervisor and supervised child are separate processes).
pub fn init(path: PathBuf, level: log::LevelFilter) {
    let (file, len) = match open_log_file(&path) {
        Some((file, len)) => (Some(file), len),
        None => (None, 0),
    };
    // `init` can't log yet: the logger it reports to is the one being set.
    let _ = log::set_boxed_logger(Box::new(FileLogger {
        inner: Mutex::new(LogFile {
            file,
            written: len,
            retry_at: None,
        }),
        path,
    }));
    log::set_max_level(level);
}

/// Appends a line to `crash.log`; used by panic/signal handlers where the
/// regular logger may already be poisoned or flushed.
pub fn append_crash_log(path: &Path, message: &str) {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{timestamp}] {message}");
    }
}

/// Points the OS-level stderr (fd 2) at `path`. A detached tray app has no
/// terminal, so without this, panic backtraces and libc-level failures on
/// stderr vanish. Best effort; silently ignored when unsupported.
pub fn redirect_stderr(path: &Path) {
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok();
    let Some(c_path) = c_path else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // SAFETY: `c_path` is a valid NUL-terminated C string built from the
    // path's bytes, the flags are the platform's O_* constants, and the
    // descriptor is closed on the same side it was opened.
    unsafe {
        let fd = libc::open(
            c_path.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
            0o644,
        );
        if fd < 0 {
            return;
        }
        if libc::dup2(fd, libc::STDERR_FILENO) < 0 {
            let _ = libc::close(fd);
            return;
        }
        // `dup2` did not take ownership of `fd`; stderr now holds its own.
        let _ = libc::close(fd);
    }
}
