#!/usr/bin/env bash
# Installs KwmSwitcher for the current user and (re)starts it.
#
#   ./scripts/install.sh              build, install, restart the app
#   ./scripts/install.sh --no-start   build and install only
#
# Installs under $PREFIX (default: ~/.local):
#   $PREFIX/bin/kwmswitcher
#   $PREFIX/share/applications/kwmswitcher.desktop
#   $PREFIX/share/icons/hicolor/{16,32,48,64,128,256}x*/apps/kwmswitcher.png
#
# The app itself keeps the autostart entries in sync on start (XDG autostart
# plus the Omarchy Hyprland block), so no config files are edited here.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PREFIX=${PREFIX:-$HOME/.local}
BIN="$PREFIX/bin/kwmswitcher"
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor"
START=1

for arg in "$@"; do
    case "$arg" in
        --no-start) START=0 ;;
        -h | --help)
            sed -n '2,13p' "$0"
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

echo "Building release binary…"
cargo build --release --manifest-path "$ROOT/Cargo.toml"

# A running copy must stop first: its supervisor would relaunch the old binary,
# and a running executable cannot be overwritten (ETXTBSY).
if pgrep -f 'kwmswitcher --supervise' >/dev/null; then
    echo "Stopping the running instance…"
    pkill -TERM -f 'kwmswitcher --supervise' || true
    sleep 1
fi
pkill -TERM -x kwmswitcher 2>/dev/null || true

echo "Installing $BIN"
install -Dm755 "$ROOT/target/release/kwmswitcher" "$BIN"

echo "Installing icons"
for size in 16 32 48 64 128 256; do
    target="$ICONS/${size}x${size}/apps/kwmswitcher.png"
    mkdir -p "$(dirname "$target")"
    if command -v rsvg-convert >/dev/null 2>&1; then
        rsvg-convert -w "$size" -h "$size" "$ROOT/assets/logo.svg" -o "$target"
    else
        install -m644 "$ROOT/assets/logo.png" "$target"
    fi
done

echo "Installing the launcher entry"
mkdir -p "$APPS"
cat >"$APPS/kwmswitcher.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=KWM Switcher
GenericName=USB KVM Switcher
Comment=Switch the monitor input when the USB switch moves
Exec="$BIN" --supervise
Icon=kwmswitcher
Terminal=false
Categories=Utility;
Keywords=KWM;KVM;USB;monitor;DDC;input;switch;
StartupNotify=false
StartupWMClass=kwmswitcher
EOF
update-desktop-database "$APPS" 2>/dev/null || true
gtk-update-icon-cache -f -t "$ICONS" 2>/dev/null || true

if [ "$START" = 0 ]; then
    echo "Installed. Start it with: $BIN --supervise"
    exit 0
fi

if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ]; then
    echo "No graphical session in this shell; installed but not started." >&2
    echo "Start it from the launcher or run: $BIN --supervise" >&2
    exit 0
fi

# The app treats OMARCHY_PATH as "this is an Omarchy session" and updates the
# Hyprland autostart block when it is present.
if [ -z "${OMARCHY_PATH:-}" ] && [ -d /usr/share/omarchy ]; then
    export OMARCHY_PATH=/usr/share/omarchy
fi

echo "Starting KwmSwitcher"
setsid "$BIN" --supervise >/dev/null 2>&1 </dev/null &
