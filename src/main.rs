//! KwmSwitcher — USB KVM switcher that flips monitor inputs over DDC/CI.
//!
//! Process modes:
//! - default: the gpui tray application
//! - `--supervise`: run the external supervisor (spawn + relaunch on crash)
//! - `--crash-test [native|managed]`: deliberately crash, to exercise the
//!   supervisor's relaunch behavior

mod autostart;
mod config;
mod ddc;
mod engine;
mod input_source;
mod logging;
mod paths;
mod supervisor;
mod tray;
mod usb;
mod ui;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Supervisor mode: spawn the app as a child and relaunch on crash.
    // Dispatched before any logging setup so the supervisor stays lightweight.
    if args.iter().any(|arg| arg == "--supervise") {
        let code = supervisor::run(&args);
        std::process::exit(code);
    }

    logging::init(paths::log_file(), log::LevelFilter::Debug);
    // A detached tray app has no terminal; but under a debugger (set
    // KWMSWITCHER_KEEP_STDERR=1) keep OS stderr so panic backtraces reach
    // the debug console instead of the log file.
    if std::env::var_os("KWMSWITCHER_KEEP_STDERR").is_none() {
        logging::redirect_stderr(&paths::stderr_log_file());
    }
    install_panic_hook();

    // Diagnostic hook: crash on demand so the supervisor's relaunch behavior
    // can be exercised. `native` aborts (SIGABRT, uncatchable from Rust code
    // paths); `managed` panics on a background thread.
    if let Some(mode) = crash_test_mode(&args) {
        log::warn!("Crash-test hook firing in '{mode}' mode");
        trigger_crash(&mode);
        return;
    }

    log::debug!("Starting KwmSwitcher application");
    ui::run();
}

/// Returns the crash-test mode if `--crash-test` was passed.
fn crash_test_mode(args: &[String]) -> Option<String> {
    let index = args.iter().position(|arg| arg == "--crash-test")?;
    let mode = args
        .get(index + 1)
        .filter(|next| !next.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| "native".into());
    Some(mode.to_lowercase())
}

fn trigger_crash(mode: &str) {
    // Give the log a moment to flush before the fault.
    std::thread::sleep(std::time::Duration::from_millis(200));
    if mode == "managed" {
        std::thread::Builder::new()
            .name("crash-test".into())
            .spawn(|| {
                panic!("crash-test: deliberate unhandled panic");
            })
            .expect("spawn crash thread");
        loop {
            std::thread::park();
        }
    } else {
        // Simulate an unrecoverable native fault the way a wedged i2c bus
        // would: immediate abort, bypassing unwind and exit handlers.
        std::process::abort();
    }
}

/// Panics anywhere tear the process down with a non-zero exit so the
/// supervisor relaunches — matching the C# version's `UnhandledException`
/// behavior.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let message = format!("Panic: {info}");
        log::error!("{message}");
        logging::append_crash_log(&paths::crash_log_file(), &message);
        std::process::exit(1);
    }));
}
