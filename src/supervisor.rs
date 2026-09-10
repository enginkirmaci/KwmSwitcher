//! External supervisor: spawns the app as a child and relaunches after a
//! crash (any non-zero exit, including native faults this Rust process can't
//! catch from inside the child). Exit-code contract:
//!
//! - `0`   = clean exit (user Quit) → do not relaunch
//! - other = crash → relaunch with exponential backoff, give up after a
//!   crash loop

use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};

use crate::paths;

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Consecutive crashes (with short uptime) before giving up.
const CRASH_LOOP_THRESHOLD: u32 = 5;
/// Uptime after which a crash counts as a fresh incident.
const HEALTHY_UPTIME: Duration = Duration::from_secs(60);

struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Strips `--supervise` so the child doesn't recurse into supervising.
fn strip_supervise_flag<'a>(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|a| a.as_str() != "--supervise")
        .cloned()
        .collect()
}

/// Runs the supervision loop; returns the process exit code to propagate.
pub fn run(args: &[String]) -> i32 {
    crate::logging::init(paths::supervisor_log_file(), log::LevelFilter::Info);

    let child_args = strip_supervise_flag(args);
    log::info!(
        "KwmSwitcher supervisor starting; child args: {}",
        if child_args.is_empty() { "(none)".into() } else { child_args.join(" ") }
    );

    let stop = StopFlag(Arc::new(AtomicBool::new(false)));
    for signal in [SIGTERM, SIGINT, SIGHUP] {
        // On a session end we stop *without* relaunching.
        if signal_hook::flag::register(signal, stop.0.clone()).is_err() {
            log::warn!("Failed to register handler for signal {signal}");
        }
    }

    let candidate = std::env::var("KWMSWITCHER_SUPERVISE_CHILD")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(str::to_string))
        });
    let Some(child_path) = candidate.filter(|p| std::path::Path::new(p).exists()) else {
        log::error!("Cannot determine executable path to relaunch; giving up.");
        return 1;
    };

    let mut consecutive_crashes: u32 = 0;
    let mut backoff = INITIAL_BACKOFF;

    while !stop.is_set() {
        let start = Instant::now();
        let exit_code = spawn_and_wait(&child_path, &child_args, &stop);
        let uptime = start.elapsed();

        if stop.is_set() {
            log::info!("Supervisor stopping; child exited with code {exit_code}.");
            break;
        }
        if exit_code == 0 {
            log::info!("Child exited cleanly (code 0). Supervisor will not relaunch.");
            break;
        }

        log::warn!(
            "Child crashed with exit code {exit_code} after {}s.",
            uptime.as_secs()
        );

        if uptime >= HEALTHY_UPTIME {
            consecutive_crashes = 0;
            backoff = INITIAL_BACKOFF;
            log::info!("Uptime >= {HEALTHY_UPTIME:?}; treating as a fresh incident.");
        }
        consecutive_crashes += 1;

        if consecutive_crashes > CRASH_LOOP_THRESHOLD {
            let msg = format!(
                "KwmSwitcher crashed {consecutive_crashes} times in a row without staying up. \
                 Supervisor giving up to avoid a crash loop. Please restart manually."
            );
            log::error!("{msg}");
            notify_user("KwmSwitcher crash loop", &msg);
            return 1;
        }

        log::info!(
            "Relaunching in {}s (consecutive crash #{consecutive_crashes}).",
            backoff.as_secs()
        );
        if wait_with_stop(backoff, &stop) {
            break; // interrupted by a stop signal
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }

    log::info!("KwmSwitcher supervisor exiting.");
    0
}

fn spawn_and_wait(child_path: &str, child_args: &[String], stop: &StopFlag) -> i32 {
    let mut child: Child = match Command::new(child_path).args(child_args).spawn() {
        Ok(child) => child,
        Err(err) => {
            log::error!("Failed to spawn {child_path}: {err}; treating as crash.");
            return -1;
        }
    };

    // Poll so a stop signal interrupts the wait promptly.
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code().unwrap_or(-1),
            Ok(None) => {
                if stop.is_set() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return 0;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(err) => {
                log::error!("Failed to wait for child: {err}");
                return -1;
            }
        }
    }
}

/// Sleeps `delay`, returning `true` when a stop signal arrived in the meantime.
fn wait_with_stop(delay: Duration, stop: &StopFlag) -> bool {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline {
        if stop.is_set() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    stop.is_set()
}

/// Best-effort desktop notification so a user who can't see the tray knows the
/// supervisor gave up. Purely cosmetic.
fn notify_user(title: &str, body: &str) {
    let _ = Command::new("notify-send").arg(title).arg(body).spawn();
}
