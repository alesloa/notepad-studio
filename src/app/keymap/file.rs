// File menu + app-lifecycle keybindings (Quit, New / Open / Save, Close Tab,
// Restore Closed Tab). Tab cycling and tab-N jumps live in `tabs.rs`.
//
// Cross-platform pattern: register `cmd-*` and `ctrl-*` together. gpui
// dispatches both unconditionally; the unused chord on each platform is inert.

use gpui::{App, KeyBinding};

use crate::actions::*;

pub(crate) fn register(cx: &mut App) {
    // Quit: Cmd+Q on macOS only. Linux/Windows free Ctrl+Q for Notepad++ block
    // comment (Notepad++ has no default quit chord — File ▸ Exit / Alt+F4).
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-shift-n", FileNewWindow, None),
        KeyBinding::new("ctrl-shift-n", FileNewWindow, None),
        KeyBinding::new("cmd-n", FileNew, Some("Workspace")),
        KeyBinding::new("ctrl-n", FileNew, Some("Workspace")),
        KeyBinding::new("cmd-w", FileCloseTab, Some("Workspace")),
        KeyBinding::new("ctrl-w", FileCloseTab, Some("Workspace")),
        KeyBinding::new("cmd-shift-w", FileCloseAllTabs, Some("Workspace")),
        KeyBinding::new("ctrl-shift-w", FileCloseAllTabs, Some("Workspace")),
        // File I/O. Save As is `Ctrl+Alt+S` in Notepad++ on Windows/Linux,
        // but every Mac app expects `Cmd+Shift+S`. Register both — only the
        // platform-active modifier fires.
        KeyBinding::new("cmd-o", FileOpen, Some("Workspace")),
        KeyBinding::new("ctrl-o", FileOpen, Some("Workspace")),
        KeyBinding::new("cmd-s", FileSave, Some("Workspace")),
        KeyBinding::new("ctrl-s", FileSave, Some("Workspace")),
        KeyBinding::new("cmd-shift-s", FileSaveAs, Some("Workspace")),
        KeyBinding::new("ctrl-alt-s", FileSaveAs, Some("Workspace")),
        // Save All: macOS uses Cmd+Opt+S since Cmd+Shift+S is Save As; Linux
        // and Windows match Notepad++'s Ctrl+Shift+S.
        KeyBinding::new("cmd-alt-s", FileSaveAll, Some("Workspace")),
        KeyBinding::new("ctrl-shift-s", FileSaveAll, Some("Workspace")),
    ]);

    // Restore Recent Closed Tab — Cmd/Ctrl+Shift+T (Notepad++ Restore Recent
    // Closed File). Theme toggle in View is menu-only, so this chord is free.
    cx.bind_keys([
        KeyBinding::new("cmd-shift-t", FileRestoreClosedTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-t", FileRestoreClosedTab, Some("Workspace")),
    ]);
}
