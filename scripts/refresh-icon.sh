#!/usr/bin/env bash
# Regenerate every icon asset from a fresh Icon Composer (.icon) bundle.
#
# Pass the path to the source .icon directory as the first argument, or
# the script defaults to $HOME/Downloads/new-icon-notepad.icon.
#
# What it does:
#   1. Replaces assets/AppIcon.icon with the source bundle
#   2. Forces icon.json scale to 1 (full-canvas) regardless of what
#      Icon Composer exported (it often exports 0.22 / 0.26 / 0.28)
#   3. Regenerates assets/icon.png (1024x1024, sRGB, TrueColor) from
#      the source PNG inside the .icon bundle's Assets/ folder
#   4. Regenerates assets/icon.iconset/ (10 sizes for the .icns)
#   5. Compiles assets/icon.icns via iconutil
#   6. Regenerates assets/linux/icon_*.png (7 sizes)
#   7. Regenerates assets/icon.ico (Windows multi-size)
#   8. Runs ./scripts/bundle-mac.sh --release
#   9. Runs ./scripts/install-mac.sh
#
# Requires: magick (ImageMagick 7), iconutil, sips (macOS).
#
# Usage:
#   ./scripts/refresh-icon.sh
#   ./scripts/refresh-icon.sh /path/to/MyIcon.icon

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

SRC_ICON_DIR="${1:-$HOME/Downloads/new-icon-notepad.icon}"

if [[ ! -d "$SRC_ICON_DIR" ]]; then
  echo "error: source .icon bundle not found at $SRC_ICON_DIR" >&2
  exit 1
fi
if [[ ! -d "$SRC_ICON_DIR/Assets" ]]; then
  echo "error: $SRC_ICON_DIR has no Assets/ directory" >&2
  exit 1
fi

SRC_PNG="$(find "$SRC_ICON_DIR/Assets" -maxdepth 1 -type f -name '*.png' | head -n 1)"
if [[ -z "$SRC_PNG" ]]; then
  echo "error: no .png found in $SRC_ICON_DIR/Assets/" >&2
  exit 1
fi
echo "==> Source icon bundle: $SRC_ICON_DIR"
echo "==> Source PNG:         $SRC_PNG"

for tool in magick iconutil sips; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "error: required tool '$tool' not on PATH" >&2
    exit 1
  fi
done

# 1. Replace assets/AppIcon.icon with the new source bundle.
echo "==> Replacing assets/AppIcon.icon/"
rm -rf assets/AppIcon.icon
cp -R "$SRC_ICON_DIR" assets/AppIcon.icon

# 2. Force scale=1 in the copy's icon.json so the design fills the canvas.
#    Icon Composer often exports 0.22 / 0.26 / 0.28 which leaves padding.
PROJ_JSON="assets/AppIcon.icon/icon.json"
echo "==> Forcing icon.json scale = 1 in $PROJ_JSON"
python3 - "$PROJ_JSON" <<'PY'
import json, sys
path = sys.argv[1]
with open(path) as f:
    data = json.load(f)
for grp in data.get("groups", []):
    for layer in grp.get("layers", []):
        pos = layer.setdefault("position", {})
        pos["scale"] = 1
        pos["translation-in-points"] = [0, 0]
with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PY

# 3. Regenerate the canonical 1024x1024 PNG from the new source.
echo "==> Regenerating assets/icon.png (1024x1024, sRGB)"
magick "$SRC_PNG" -resize 1024x1024 \
  -define png:color-type=6 -type TrueColor -colorspace sRGB \
  assets/icon.png

# 4. Regenerate the 10-file iconset.
echo "==> Regenerating assets/icon.iconset/ (10 files)"
mkdir -p assets/icon.iconset
for s in 16 32 128 256 512; do
  d=$((s * 2))
  magick assets/icon.png -resize ${s}x${s} -colorspace sRGB \
    "assets/icon.iconset/icon_${s}x${s}.png"
  magick assets/icon.png -resize ${d}x${d} -colorspace sRGB \
    "assets/icon.iconset/icon_${s}x${s}@2x.png"
done

# 5. Compile assets/icon.icns from the iconset.
echo "==> Compiling assets/icon.icns"
iconutil -c icns -o assets/icon.icns assets/icon.iconset

# 6. Regenerate Linux PNGs (7 sizes).
echo "==> Regenerating assets/linux/ (7 files)"
mkdir -p assets/linux
for s in 16 32 48 64 128 256 512; do
  magick assets/icon.png -resize ${s}x${s} -colorspace sRGB \
    "assets/linux/icon_${s}.png"
done

# 7. Regenerate Windows .ico (multi-size).
echo "==> Regenerating assets/icon.ico"
magick assets/icon.png \
  -define icon:auto-resize=256,128,64,48,32,16 \
  assets/icon.ico

# 8. Build the macOS bundle.
echo ""
echo "==> Building macOS bundle (release)"
./scripts/bundle-mac.sh --release

# 9. Install.
echo ""
echo "==> Installing"
./scripts/install-mac.sh

echo ""
echo "Done. New icon is live at /Applications/Notepad Studio.app."
