// Bookmark keybindings — Notepad++ defaults.

use gpui::{App, KeyBinding};

use crate::actions::*;

pub(crate) fn register(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("f2", BookmarkToggle, Some("Workspace")),
        KeyBinding::new("ctrl-f2", BookmarkNext, Some("Workspace")),
        KeyBinding::new("shift-f2", BookmarkPrev, Some("Workspace")),
        // Clear-all takes both Cmd-Shift-F2 (mac) and Ctrl-Shift-F2 (Linux/Win)
        // per the cross-platform binding policy.
        KeyBinding::new("cmd-shift-f2", BookmarkClearAll, Some("Workspace")),
        KeyBinding::new("ctrl-shift-f2", BookmarkClearAll, Some("Workspace")),
    ]);
}
