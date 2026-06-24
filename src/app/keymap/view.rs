// View keybindings. Theme toggle is menu-only (View ▸ Toggle Theme); Notepad++
// doesn't bind theme either. Word Wrap and Show Whitespace are menu-only —
// NPP doesn't bind a default chord for either.
//
// Zoom (VS Code-style): Cmd/Ctrl + `=` for zoom in (also Shift+= so the literal
// `+` keypress works), Cmd/Ctrl + `-` for zoom out, Cmd/Ctrl + `0` for reset.
//
// Full screen: F11 — matches Notepad++ on every platform. Bare key, no
// modifier. macOS users still get the OS-standard ctrl-cmd-f via the system
// menu; F11 is the cross-platform NPP convention.

use gpui::{App, KeyBinding};

use crate::actions::*;

pub(crate) fn register(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-=", ViewZoomIn, Some("Workspace")),
        KeyBinding::new("ctrl-=", ViewZoomIn, Some("Workspace")),
        KeyBinding::new("cmd-shift-=", ViewZoomIn, Some("Workspace")),
        KeyBinding::new("ctrl-shift-=", ViewZoomIn, Some("Workspace")),
        KeyBinding::new("cmd--", ViewZoomOut, Some("Workspace")),
        KeyBinding::new("ctrl--", ViewZoomOut, Some("Workspace")),
        KeyBinding::new("cmd-0", ViewZoomReset, Some("Workspace")),
        KeyBinding::new("ctrl-0", ViewZoomReset, Some("Workspace")),
        KeyBinding::new("f11", ViewToggleFullScreen, Some("Workspace")),
        // Preferences mirrors the main window's zoom — the chord pressed
        // here delegates to the active Workspace's `set_zoom` so both
        // windows stay in sync (the dialog observes Workspace.zoom_level
        // and re-applies via `set_rem_size`).
        KeyBinding::new("cmd-=", ViewZoomIn, Some("PreferencesDialog")),
        KeyBinding::new("ctrl-=", ViewZoomIn, Some("PreferencesDialog")),
        KeyBinding::new("cmd-shift-=", ViewZoomIn, Some("PreferencesDialog")),
        KeyBinding::new("ctrl-shift-=", ViewZoomIn, Some("PreferencesDialog")),
        KeyBinding::new("cmd--", ViewZoomOut, Some("PreferencesDialog")),
        KeyBinding::new("ctrl--", ViewZoomOut, Some("PreferencesDialog")),
        KeyBinding::new("cmd-0", ViewZoomReset, Some("PreferencesDialog")),
        KeyBinding::new("ctrl-0", ViewZoomReset, Some("PreferencesDialog")),
    ]);
}
