#!/usr/bin/env bash
# Build a macOS .app bundle and patch its Info.plist for Tahoe.
#
# cargo-bundle (v0.6.1-zed) writes an Info.plist that looks like a legacy
# pre-2000 Carbon app: LSRequiresCarbon=true, no SDK markers, no
# LSMinimumSystemVersion. macOS Tahoe (macOS 26) reads this and treats the
# bundle as legacy, wrapping its icon in a glass "container" tile.
#
# VS Code ships a plist marked with DT* keys (DTSDKName=macosx26.x,
# DTXcode=26xx, DTPlatformName=macosx) and no LSRequiresCarbon — Tahoe
# treats it as a native build and renders the icon without the auto-
# wrapped container. This script runs cargo bundle, deletes the legacy
# Carbon flag, and injects the same SDK markers VS Code uses so our icon
# renders cleanly on Tahoe.
#
# Usage: ./scripts/bundle-mac.sh [--release]
#   --release  Build via `cargo bundle --release` (default if omitted)
#   --debug    Build via `cargo bundle` (debug)

set -euo pipefail

PROFILE="release"
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE="debug"
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ICON_SRC="$REPO_ROOT/assets/AppIcon.icon"
if [[ ! -d "$ICON_SRC" ]]; then
  echo "error: $ICON_SRC not found (expected the Icon Composer bundle)" >&2
  exit 1
fi

echo "==> cargo bundle ($PROFILE)"
if [[ "$PROFILE" == "release" ]]; then
  cargo bundle --release
  APP="$REPO_ROOT/target/release/bundle/osx/Notepad Studio.app"
else
  cargo bundle
  APP="$REPO_ROOT/target/debug/bundle/osx/Notepad Studio.app"
fi

if [[ ! -d "$APP" ]]; then
  echo "error: bundle not found at $APP" >&2
  exit 1
fi

PLIST="$APP/Contents/Info.plist"

# Patch Info.plist to declare this bundle as a Tahoe-native build.
# Tahoe applies the glass-container "tile" to icons of apps that look
# legacy (LSRequiresCarbon set, no DT* SDK markers). VS Code ships these
# keys and its dock icon renders cleanly without an auto-wrapped tile;
# we mirror its Info.plist to get the same behaviour.
plist_set() {
  local key="$1" type="$2" value="$3"
  /usr/libexec/PlistBuddy -c "Delete :$key" "$PLIST" 2>/dev/null || true
  /usr/libexec/PlistBuddy -c "Add :$key $type $value" "$PLIST"
}

/usr/libexec/PlistBuddy -c "Delete :LSRequiresCarbon" "$PLIST" 2>/dev/null || true

plist_set CFBundleIconName        string "icon"
plist_set LSMinimumSystemVersion  string "12.0"
plist_set DTSDKName               string "macosx26.1"
plist_set DTSDKBuild              string "25B74"
plist_set DTXcode                 string "2611"
plist_set DTXcodeBuild            string "17B100"
plist_set DTPlatformName          string "macosx"
plist_set DTPlatformVersion       string "26.1"
plist_set DTPlatformBuild         string "25B74"

echo "==> Patched Info.plist with Tahoe SDK metadata (matches VS Code)"

# Declare CFBundleDocumentTypes so Notepad Studio appears in Finder's
# "Open With" menu for every text-ish file we can handle. cargo-bundle
# does not emit this key from Cargo.toml metadata, so we inject the whole
# array here via plistlib (PlistBuddy syntax for deeply-nested arrays of
# dicts is unreadable; Python is always available on macOS).
#
# LSHandlerRank = "Alternate" advertises Notepad Studio as a candidate
# without claiming to be the default for any type — users keep their
# existing defaults (Numbers for CSV, TextEdit for TXT) and pick us from
# the Open With submenu.
#
# Each dict groups extensions and UTIs by purpose so future edits stay
# readable. Adding a new file type: either extend an existing dict's
# `LSItemContentTypes` / `CFBundleTypeExtensions` arrays or append a new
# dict to the list.
python3 - "$PLIST" <<'PY'
import plistlib, sys
plist_path = sys.argv[1]
with open(plist_path, "rb") as f:
    info = plistlib.load(f)

doc_types = [
    {
        "CFBundleTypeName": "Plain Text Document",
        "CFBundleTypeRole": "Editor",
        "LSHandlerRank": "Alternate",
        "LSItemContentTypes": [
            "public.plain-text",
            "public.text",
            "public.utf8-plain-text",
            "public.utf16-plain-text",
            "public.utf8-tab-separated-values-text",
            "public.log",
        ],
    },
    {
        "CFBundleTypeName": "Delimiter Separated Values",
        "CFBundleTypeRole": "Editor",
        "LSHandlerRank": "Alternate",
        "LSItemContentTypes": [
            "public.comma-separated-values-text",
            "public.tab-separated-values-text",
        ],
    },
    {
        "CFBundleTypeName": "Source Code",
        "CFBundleTypeRole": "Editor",
        "LSHandlerRank": "Alternate",
        "LSItemContentTypes": [
            "public.source-code",
            "public.script",
            "public.shell-script",
            "public.python-script",
            "public.ruby-script",
            "public.perl-script",
            "public.json",
            "public.xml",
            "public.html",
            "public.css",
            "public.javascript",
            "com.netscape.javascript-source",
            "public.yaml",
            "net.daringfireball.markdown",
        ],
    },
    {
        "CFBundleTypeName": "Rich Text Document",
        "CFBundleTypeRole": "Editor",
        "LSHandlerRank": "Alternate",
        "LSItemContentTypes": [
            "public.rtf",
            "org.oasis-open.opendocument.text",
            "com.microsoft.word.openxml.document",
            "com.microsoft.word.doc",
        ],
    },
    {
        "CFBundleTypeName": "Notepad Studio Document",
        "CFBundleTypeRole": "Editor",
        "LSHandlerRank": "Alternate",
        "CFBundleTypeExtensions": [
            "txt", "text", "log", "md", "markdown", "mkd", "mdown",
            "csv", "tsv",
            "json", "jsonc", "json5",
            "xml", "svg", "xhtml", "xsl", "xslt", "plist",
            "html", "htm", "shtml",
            "css", "scss", "sass", "less",
            "js", "mjs", "cjs", "jsx",
            "ts", "tsx",
            "rs",
            "py", "pyi", "pyw",
            "c", "h",
            "cpp", "cxx", "cc", "hpp", "hxx", "hh",
            "cs", "csx",
            "java",
            "go",
            "rb", "rake", "gemspec",
            "php", "phtml",
            "swift",
            "kt", "kts",
            "lua",
            "sh", "bash", "zsh", "ksh", "fish",
            "ps1", "psm1", "psd1",
            "yaml", "yml",
            "toml",
            "ini", "conf", "cfg", "env", "editorconfig", "gitignore",
            "sql",
            "rtf", "doc", "docx", "odt",
            "patch", "diff",
            "tex", "bib",
            "r", "R",
            "dart",
            "vue", "svelte",
        ],
    },
]
info["CFBundleDocumentTypes"] = doc_types
with open(plist_path, "wb") as f:
    plistlib.dump(info, f)
print(f"==> Injected {len(doc_types)} CFBundleDocumentTypes group(s) into Info.plist")
PY

touch "$APP"

echo ""
echo "Bundle ready: $APP"
echo ""
echo "Install:"
echo "  mv \"/Applications/Notepad Studio.app\" \"\$HOME/.Trash/\""
echo "  ditto \"$APP\" \"/Applications/Notepad Studio.app\""
echo "  killall Dock"
