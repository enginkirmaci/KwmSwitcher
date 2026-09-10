# KwmSwitcher

A USB KVM switcher for Linux: plug your keyboard/mouse into a USB switch, and
KwmSwitcher watches the device list and flips your monitor to the right input
over DDC/CI automatically. When your peripherals disappear (you switched to the
other machine), it switches the monitor back.

Rewritten from the original C#/Avalonia version in **Rust** with
[**GPUI Kit**](https://gpui-kit.com) (the shadcn-style UI kit for Zed's GPUI).

![deps](https://img.shields.io/badge/ui-gpui__kit%200.6-6E56CF)

## Features

- **Automatic input switching** — polls `/sys/bus/usb/devices`, diffs the
  device set, and switches the monitor input when a tracked device appears or
  disappears (3 s switch cooldown prevents flapping).
- **PiP / PBP awareness** — reads the monitor's PiP mode; auto-switching is
  suspended while PiP/PBP is active, and the main window / tray menu can
  toggle it.
- **Standard DDC/CI and LG protocols** — supports both the standard input
  VCP code (0x60) and LG's vendor-specific registers (0xF4 with custom i2c
  source addresses).
- **Multi-monitor targeting** — detect monitors via `ddcutil detect` and pick
  the target, so laptops don't switch their built-in panel.
- **Tray icon** (StatusNotifierItem) with quick actions: switch local/remote,
  toggle PiP, open windows, quit.
- **Crash-proof supervision** — `--supervise` runs the app as a supervised
  child and relaunches after any non-zero exit (including native faults),
  with exponential backoff and crash-loop detection.
- **Autostart** — FreeDesktop `.desktop` entry in the user autostart dir.
- **Follows system theme** — light/dark via the XDG desktop portal, violet
  accent, client-side decorations.

## UI

The main window is a live diagram: **Local → Monitor → Remote**, with the
active path lit. Switch manually with the buttons, or let the USB watcher do
it. Settings cover tracked USB devices, input sources per protocol, target
monitor and startup options.

## Build

Rust 1.85+ (edition 2024 dependencies) and the usual GPUI Linux build deps
(Wayland/X11 client libraries, fontconfig; a Vulkan or GL capable GPU to run).

```sh
cargo build --release
./target/release/kwmswitcher
```

`ddcutil` must be installed (`pacman -S ddcutil`); the user typically needs
write access to the i2c bus (see `ddcutil` docs / `i2c-dev` group).

## Usage

```
kwmswitcher                 # run the app (starts minimized to tray by default)
kwmswitcher --supervise     # run under the built-in crash supervisor
kwmswitcher --crash-test [native|managed]   # deliberately crash (tests the supervisor)
```

- Settings are at `~/.config/KwmSwitcher/config.json` — the same file the
  previous Avalonia version used, so existing settings carry over.
- Logs go to `~/.local/state/KwmSwitcher/` (`kwmswitcher.log`,
  `supervisor.log`, `crash.log`, `stderr.log`).

## Layout

```
src/
  main.rs          entry point, flags, panic hook
  supervisor.rs    crash supervisor (relaunch with backoff)
  engine.rs        switch state machine (own thread)
  usb.rs           sysfs USB enumeration + poll thread
  ddc.rs           ddcutil runner (serialized, 15 s timeout)
  input_source.rs  VCP codes + LG/standard protocol mappings
  config.rs        JSON config (C#-compatible schema)
  autostart.rs     FreeDesktop autostart entry
  tray.rs          StatusNotifierItem tray (ksni)
  logging.rs       file logger + stderr redirection
  paths.rs         XDG paths
  ui/              gpui-kit UI: bridge, main window, settings window, assets
```

## Debugging in VS Code

Install the recommended extensions (CodeLLDB + rust-analyzer; VS Code offers
them on open). The provided debug profiles all live in `.vscode/launch.json`:

| Profile | What it does |
|---|---|
| **Debug kwmswitcher (GUI)** | Builds and launches the app under LLDB |
| **Debug supervisor mode** | Launches `--supervise` (debugs the supervisor; use Attach for the child) |
| **Crash test (native / managed)** | Runs the crash hooks so you can watch the fault in the debugger |
| **Debug unit tests** | Builds the test harness binary and runs it under LLDB; set `args` to a filter like `["config::"]` for a subset |
| **Attach to running kwmswitcher** | Attaches to a running instance (e.g. the supervised child) |

Launch profiles set `KWMSWITCHER_KEEP_STDERR=1`, which keeps OS stderr
attached to the debugger instead of redirecting it to `stderr.log`, so panic
backtraces show up in the debug console.

## Notes

- The Rust rewrite is Linux-only (the C# version also supported Windows via
  WMI; that path was not ported — the `main` branch preserves it).
- Icons are from [Lucide](https://lucide.dev) (ISC), bundled in `assets/`.
