#!/usr/bin/env bash
# Install Notepad Studio on Linux: copy the release binary into a system
# (or user) location, drop the .desktop file so the app appears in the
# application menus and in Files / Dolphin / Thunar "Open With" lists,
# install icons into the hicolor theme so the launcher and Open With
# submenu render the proper artwork, then refresh the desktop database
# + icon cache so the new entry shows up without a logout.
#
# Two install scopes:
#   --system  (default if run as root): /usr/local/bin, /usr/share/applications,
#             /usr/share/icons/hicolor — affects every user on the box.
#   --user    (default if non-root):    ~/.local/bin, ~/.local/share/applications,
#             ~/.local/share/icons/hicolor — affects only the current user.
#
# Usage:
#   ./scripts/install-linux.sh             # auto-pick scope from EUID
#   ./scripts/install-linux.sh --user
#   sudo ./scripts/install-linux.sh --system

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Pick scope.
SCOPE=""
for arg in "$@"; do
  case "$arg" in
    --system) SCOPE="system" ;;
    --user)   SCOPE="user" ;;
    *) echo "unknown arg: $arg (expected --system or --user)" >&2; exit 2 ;;
  esac
done
if [[ -z "$SCOPE" ]]; then
  if [[ $EUID -eq 0 ]]; then SCOPE="system"; else SCOPE="user"; fi
fi

if [[ "$SCOPE" == "system" ]]; then
  BIN_DIR="/usr/local/bin"
  APPS_DIR="/usr/share/applications"
  ICONS_DIR="/usr/share/icons/hicolor"
  echo "==> System install (scope: every user)"
else
  BIN_DIR="$HOME/.local/bin"
  APPS_DIR="$HOME/.local/share/applications"
  ICONS_DIR="$HOME/.local/share/icons/hicolor"
  echo "==> User install (scope: $USER only)"
fi

# Locate the release binary. Build it first if missing — no point copying a
# stale debug binary in.
BIN_SRC="$REPO_ROOT/target/release/notepad-studio"
if [[ ! -x "$BIN_SRC" ]]; then
  echo "==> target/release/notepad-studio not found, running cargo build --release"
  cargo build --release
fi
if [[ ! -x "$BIN_SRC" ]]; then
  echo "error: build did not produce $BIN_SRC" >&2
  exit 1
fi

DESKTOP_SRC="$REPO_ROOT/assets/notepad-studio.desktop"
ICONS_SRC="$REPO_ROOT/assets/linux"
if [[ ! -f "$DESKTOP_SRC" ]]; then
  echo "error: $DESKTOP_SRC not found" >&2
  exit 1
fi
if [[ ! -d "$ICONS_SRC" ]]; then
  echo "error: $ICONS_SRC not found" >&2
  exit 1
fi

# Make the destination directories so install -m below doesn't trip on a
# missing parent on a fresh user profile.
mkdir -p "$BIN_DIR" "$APPS_DIR"
for size in 16 32 48 64 128 256 512; do
  mkdir -p "$ICONS_DIR/${size}x${size}/apps"
done

# Stop any running instance — a live binary can't be overwritten on some
# filesystems and would also drop the old executable into the new install.
echo "==> Stopping any running notepad-studio instance"
pkill -x notepad-studio 2>/dev/null || true
sleep 0.3

echo "==> Installing binary to $BIN_DIR"
install -m 0755 "$BIN_SRC" "$BIN_DIR/notepad-studio"

echo "==> Installing desktop entry to $APPS_DIR"
install -m 0644 "$DESKTOP_SRC" "$APPS_DIR/notepad-studio.desktop"

echo "==> Installing icons into hicolor theme"
for size in 16 32 48 64 128 256 512; do
  src="$ICONS_SRC/icon_${size}.png"
  if [[ -f "$src" ]]; then
    install -m 0644 "$src" "$ICONS_DIR/${size}x${size}/apps/notepad-studio.png"
  fi
done

# Refresh the system caches so file managers + the launcher pick up the new
# .desktop + MimeType associations + icons without a logout. Both tools are
# part of standard freedesktop.org toolchains (shared-mime-info /
# desktop-file-utils / gtk3+); silently skip when not present (some minimal
# WMs don't ship them).
if command -v update-desktop-database >/dev/null 2>&1; then
  echo "==> Refreshing desktop database ($APPS_DIR)"
  update-desktop-database "$APPS_DIR" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  echo "==> Refreshing icon cache ($ICONS_DIR)"
  gtk-update-icon-cache -f -t "$ICONS_DIR" 2>/dev/null || true
fi

echo ""
echo "Installed:"
echo "  binary:  $BIN_DIR/notepad-studio"
echo "  desktop: $APPS_DIR/notepad-studio.desktop"
echo "  icons:   $ICONS_DIR/<size>/apps/notepad-studio.png"
echo ""
if [[ "$SCOPE" == "user" ]]; then
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Warning: $BIN_DIR is not on \$PATH. Add it or use --system." ;;
  esac
fi
echo "Notepad Studio should now appear in your file manager's"
echo "\"Open With\" menu for text and source-code files."
