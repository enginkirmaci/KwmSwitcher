//! Minimal file logger on the `log` facade: one rolling-by-day file, line
//! buffered with an immediate flush so a hard crash loses at most the line
//! being written.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

struct FileLogger {
    file: Mutex<Option<File>>,
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

        let mut guard = match self.file.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        // Reopen lazily if the file was rotated away underneath us.
        if guard.is_none() {
            *guard = open_log_file(&self.path);
        }
        if let Some(file) = guard.as_mut() {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = file.flush();
            }
        }
    }
}

fn open_log_file(path: &Path) -> Option<File> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Installs the logger writing to `path`. Safe to call once; later calls are
/// ignored (the supervisor and supervised child are separate processes).
pub fn init(path: PathBuf, level: log::LevelFilter) {
    let file = open_log_file(&path);
    let _ = log::set_boxed_logger(Box::new(FileLogger {
        file: Mutex::new(file),
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
    let c_path = match std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
        Ok(p) => p,
        Err(_) => return,
    };
    const O_WRONLY: i32 = 0x1;
    const O_CREAT: i32 = 0x40;
    const O_APPEND: i32 = 0x400;
    unsafe {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let fd = libc::open(c_path.as_ptr(), O_WRONLY | O_CREAT | O_APPEND, 0o644);
        if fd >= 0 {
            libc::dup2(fd, 2);
            libc::close(fd);
        }
    }
}
