#!/usr/bin/env bash
# Install the freshly-built Notepad Studio.app into /Applications.
#
# Run after `./scripts/bundle-mac.sh --release` has produced the bundle at
# target/release/bundle/osx/Notepad Studio.app.
#
# Steps:
#   1. Kills any running instance.
#   2. Moves the currently-installed /Applications/Notepad Studio.app to ~/.Trash
#      (timestamped, recoverable).
#   3. Copies the new bundle into /Applications via `ditto` (preserves xattrs).
#   4. Refreshes the Dock icon cache via `killall Dock`.
#   5. Launches the new app.
#
# Usage: ./scripts/install-mac.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NEW_APP="$REPO_ROOT/target/release/bundle/osx/Notepad Studio.app"
INSTALLED="/Applications/Notepad Studio.app"

if [[ ! -d "$NEW_APP" ]]; then
  echo "error: $NEW_APP not found" >&2
  echo "       run ./scripts/bundle-mac.sh --release first" >&2
  exit 1
fi

echo "==> Stopping any running instance"
pkill -f "Notepad Studio" 2>/dev/null || true
pkill -f notepad-studio 2>/dev/null || true
sleep 0.5

if [[ -d "$INSTALLED" ]]; then
  TS=$(date +%Y%m%d-%H%M%S)
  TRASH_NAME="Notepad Studio (replaced $TS).app"
  echo "==> Moving existing install to ~/.Trash/$TRASH_NAME"
  mv "$INSTALLED" "$HOME/.Trash/$TRASH_NAME"
fi

echo "==> Installing new bundle into /Applications"
ditto "$NEW_APP" "$INSTALLED"
touch "$INSTALLED"

echo "==> Refreshing Launch Services + Dock icon cache"
/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister \
  -f "$INSTALLED" 2>/dev/null || true
killall Dock 2>/dev/null || true
sleep 1

echo "==> Launching"
open "$INSTALLED"

echo ""
echo "Installed: $INSTALLED"
echo "Old install (if any) is in ~/.Trash/"
