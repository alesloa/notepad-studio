// Tab cycling + direct tab-N jump keybindings.

use gpui::{App, KeyBinding};

use crate::actions::*;

pub(crate) fn register(cx: &mut App) {
    // Cmd-Tab is reserved by macOS for app switching, so cycling falls back to
    // ctrl-tab everywhere. cmd-tab is registered too for parity — it simply
    // never fires on macOS.
    cx.bind_keys([
        KeyBinding::new("ctrl-tab", TabNext, Some("Workspace")),
        KeyBinding::new("cmd-tab", TabNext, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", TabPrev, Some("Workspace")),
        KeyBinding::new("cmd-shift-tab", TabPrev, Some("Workspace")),
        // Notepad++ visual-order cycle. Mac keyboards rarely have PageUp /
        // PageDown but we keep the chord on every platform for the keyboards
        // that do; trackpad / touchbar Macs simply won't fire it.
        KeyBinding::new("ctrl-pagedown", TabNextSequential, Some("Workspace")),
        KeyBinding::new("ctrl-pageup", TabPrevSequential, Some("Workspace")),
        // Notepad++ binds tab-N to Ctrl+NumPad1..9. Many Mac keyboards have
        // no NumPad, so we register the top-row digits everywhere — that's
        // also the cross-platform browser/editor convention.
        KeyBinding::new("cmd-1", TabGoto1, Some("Workspace")),
        KeyBinding::new("ctrl-1", TabGoto1, Some("Workspace")),
        KeyBinding::new("cmd-2", TabGoto2, Some("Workspace")),
        KeyBinding::new("ctrl-2", TabGoto2, Some("Workspace")),
        KeyBinding::new("cmd-3", TabGoto3, Some("Workspace")),
        KeyBinding::new("ctrl-3", TabGoto3, Some("Workspace")),
        KeyBinding::new("cmd-4", TabGoto4, Some("Workspace")),
        KeyBinding::new("ctrl-4", TabGoto4, Some("Workspace")),
        KeyBinding::new("cmd-5", TabGoto5, Some("Workspace")),
        KeyBinding::new("ctrl-5", TabGoto5, Some("Workspace")),
        KeyBinding::new("cmd-6", TabGoto6, Some("Workspace")),
        KeyBinding::new("ctrl-6", TabGoto6, Some("Workspace")),
        KeyBinding::new("cmd-7", TabGoto7, Some("Workspace")),
        KeyBinding::new("ctrl-7", TabGoto7, Some("Workspace")),
        KeyBinding::new("cmd-8", TabGoto8, Some("Workspace")),
        KeyBinding::new("ctrl-8", TabGoto8, Some("Workspace")),
        KeyBinding::new("cmd-9", TabGoto9, Some("Workspace")),
        KeyBinding::new("ctrl-9", TabGoto9, Some("Workspace")),
    ]);
}
