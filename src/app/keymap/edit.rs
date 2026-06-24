// Edit & clipboard, text-input, Notepad++ Edit ops, and caret-movement
// keybindings — every chord that drives the editor body lives here.
//
// Reserved chords (deliberately unbound until the feature ships):
//   F11 — full-screen toggle (Notepad++ default), wired in v0.0.6.

use gpui::{App, KeyBinding};

use crate::actions::*;

pub(crate) fn register(cx: &mut App) {
    // Edit & clipboard
    cx.bind_keys([
        KeyBinding::new("cmd-a", EditSelectAll, Some("Workspace")),
        KeyBinding::new("ctrl-a", EditSelectAll, Some("Workspace")),
        KeyBinding::new("cmd-c", EditCopy, Some("Workspace")),
        KeyBinding::new("ctrl-c", EditCopy, Some("Workspace")),
        KeyBinding::new("cmd-x", EditCut, Some("Workspace")),
        KeyBinding::new("ctrl-x", EditCut, Some("Workspace")),
        KeyBinding::new("cmd-v", EditPaste, Some("Workspace")),
        KeyBinding::new("ctrl-v", EditPaste, Some("Workspace")),
        KeyBinding::new("cmd-z", EditUndo, Some("Workspace")),
        KeyBinding::new("ctrl-z", EditUndo, Some("Workspace")),
        KeyBinding::new("cmd-shift-z", EditRedo, Some("Workspace")),
        KeyBinding::new("ctrl-shift-z", EditRedo, Some("Workspace")),
        // Notepad++ / Windows convention.
        KeyBinding::new("ctrl-y", EditRedo, Some("Workspace")),
    ]);

    // Text input
    cx.bind_keys([
        KeyBinding::new("backspace", EditBackspace, Some("Workspace")),
        KeyBinding::new("delete", EditDelete, Some("Workspace")),
        KeyBinding::new("enter", EditInsertNewline, Some("Workspace")),
        KeyBinding::new("tab", EditInsertTab, Some("Workspace")),
        // Notepad++: Shift+Tab outdents the current line / every line in the
        // selection. Notepad Studio wires it through the new EditOutdent action so
        // implementations can target it directly without colliding with the
        // editor input handler's tab-insert path.
        KeyBinding::new("shift-tab", EditOutdent, Some("Workspace")),
    ]);

    // Edit ops (Notepad++ parity).
    //
    // Move line up/down: Notepad++ uses Ctrl+Shift+Up/Down on Win/Linux; on
    // macOS Cmd+Shift+Up/Down already collide with Select-to-doc-start/end,
    // so the mac chord is Alt+Shift+Up/Down (VSCode/Sublime convention).
    //
    // Block / stream comment: Notepad++'s Ctrl+Q / Ctrl+Shift+Q. On macOS Cmd+Q
    // is reserved for Quit, so block comment becomes Cmd+/ (and stream is
    // Cmd+Shift+/). Notepad++ reserves Ctrl+/ for word completion, so we keep
    // Linux/Windows on Ctrl+Q to match.
    //
    // Delete line: Ctrl+Shift+L (Cmd+Shift+L on mac). Notepad++ uses Ctrl+L
    // for Delete-to-Start-of-Line, but we already use Cmd+L for Goto Line.
    cx.bind_keys([
        KeyBinding::new("cmd-d", EditDuplicateLine, Some("Workspace")),
        KeyBinding::new("ctrl-d", EditDuplicateLine, Some("Workspace")),
        // Cmd+R fill right (CSV grid only — no-op in editor / rich-doc).
        KeyBinding::new("cmd-r", EditFillRight, Some("Workspace")),
        KeyBinding::new("ctrl-r", EditFillRight, Some("Workspace")),
        // Cmd+; / Cmd+Shift+; — insert today's date / now-time. CSV-only.
        KeyBinding::new("cmd-;", EditInsertTodayDate, Some("Workspace")),
        KeyBinding::new("ctrl-;", EditInsertTodayDate, Some("Workspace")),
        KeyBinding::new("cmd-shift-;", EditInsertNowTime, Some("Workspace")),
        KeyBinding::new("ctrl-shift-;", EditInsertNowTime, Some("Workspace")),
        KeyBinding::new("ctrl-shift-up", EditMoveLineUp, Some("Workspace")),
        KeyBinding::new("ctrl-shift-down", EditMoveLineDown, Some("Workspace")),
        KeyBinding::new("alt-shift-up", EditMoveLineUp, Some("Workspace")),
        KeyBinding::new("alt-shift-down", EditMoveLineDown, Some("Workspace")),
        KeyBinding::new("cmd-shift-l", EditDeleteLine, Some("Workspace")),
        KeyBinding::new("ctrl-shift-l", EditDeleteLine, Some("Workspace")),
        KeyBinding::new("cmd-shift-u", EditUppercase, Some("Workspace")),
        KeyBinding::new("ctrl-shift-u", EditUppercase, Some("Workspace")),
        KeyBinding::new("cmd-u", EditLowercase, Some("Workspace")),
        KeyBinding::new("ctrl-u", EditLowercase, Some("Workspace")),
        KeyBinding::new("cmd-/", EditBlockComment, Some("Workspace")),
        KeyBinding::new("ctrl-q", EditBlockComment, Some("Workspace")),
        KeyBinding::new("cmd-shift-/", EditStreamComment, Some("Workspace")),
        KeyBinding::new("ctrl-shift-q", EditStreamComment, Some("Workspace")),
        KeyBinding::new("cmd-alt-enter", EditInsertBlankLineAbove, Some("Workspace")),
        KeyBinding::new("ctrl-alt-enter", EditInsertBlankLineAbove, Some("Workspace")),
        KeyBinding::new("cmd-shift-enter", EditInsertBlankLineBelow, Some("Workspace")),
        KeyBinding::new("ctrl-shift-enter", EditInsertBlankLineBelow, Some("Workspace")),
    ]);

    // Notepad++ Edit-menu keybindings — match NPP defaults where they exist.
    //
    // - Begin/End Select: Ctrl+Shift+B (NPP), mirrored to Cmd+Shift+B on macOS.
    // - Begin/End Select in Column Mode: Alt+Shift+B (NPP), same chord on mac.
    // - Toggle Read-Only: Ctrl+Alt+R (NPP), Cmd+Alt+R on macOS.
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-b", EditBeginEndSelect, Some("Workspace")),
        KeyBinding::new("cmd-shift-b", EditBeginEndSelect, Some("Workspace")),
        KeyBinding::new(
            "alt-shift-b",
            EditBeginEndSelectColumnMode,
            Some("Workspace"),
        ),
        KeyBinding::new("ctrl-alt-r", EditToggleReadOnly, Some("Workspace")),
        KeyBinding::new("cmd-alt-r", EditToggleReadOnly, Some("Workspace")),
    ]);

    // Caret movement
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, Some("Workspace")),
        KeyBinding::new("right", MoveRight, Some("Workspace")),
        KeyBinding::new("up", MoveUp, Some("Workspace")),
        KeyBinding::new("down", MoveDown, Some("Workspace")),
        KeyBinding::new("shift-left", SelectLeft, Some("Workspace")),
        KeyBinding::new("shift-right", SelectRight, Some("Workspace")),
        KeyBinding::new("shift-up", SelectUp, Some("Workspace")),
        KeyBinding::new("shift-down", SelectDown, Some("Workspace")),
        // Page nav — Notepad++ default (PageUp / PageDown bare, plus Shift
        // for selection extension).
        KeyBinding::new("pageup", EditPageUp, Some("Workspace")),
        KeyBinding::new("pagedown", EditPageDown, Some("Workspace")),
        KeyBinding::new("shift-pageup", SelectPageUp, Some("Workspace")),
        KeyBinding::new("shift-pagedown", SelectPageDown, Some("Workspace")),
    ]);

    // Line start/end. Home/End are universal; cmd-left/right are macOS-only.
    cx.bind_keys([
        KeyBinding::new("home", MoveLineStart, Some("Workspace")),
        KeyBinding::new("end", MoveLineEnd, Some("Workspace")),
        KeyBinding::new("shift-home", SelectLineStart, Some("Workspace")),
        KeyBinding::new("shift-end", SelectLineEnd, Some("Workspace")),
        KeyBinding::new("cmd-left", MoveLineStart, Some("Workspace")),
        KeyBinding::new("cmd-right", MoveLineEnd, Some("Workspace")),
        KeyBinding::new("cmd-shift-left", SelectLineStart, Some("Workspace")),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, Some("Workspace")),
    ]);

    // Word jump. macOS uses option (alt); Notepad++ / Linux use ctrl.
    cx.bind_keys([
        KeyBinding::new("alt-left", MoveWordLeft, Some("Workspace")),
        KeyBinding::new("alt-right", MoveWordRight, Some("Workspace")),
        KeyBinding::new("alt-shift-left", SelectWordLeft, Some("Workspace")),
        KeyBinding::new("alt-shift-right", SelectWordRight, Some("Workspace")),
        KeyBinding::new("ctrl-left", MoveWordLeft, Some("Workspace")),
        KeyBinding::new("ctrl-right", MoveWordRight, Some("Workspace")),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some("Workspace")),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, Some("Workspace")),
    ]);

    // Document start/end. macOS uses cmd-up/down or cmd-home/end; Notepad++
    // and most Linux apps use ctrl-home/end.
    cx.bind_keys([
        KeyBinding::new("cmd-up", MoveDocStart, Some("Workspace")),
        KeyBinding::new("cmd-down", MoveDocEnd, Some("Workspace")),
        KeyBinding::new("cmd-shift-up", SelectDocStart, Some("Workspace")),
        KeyBinding::new("cmd-shift-down", SelectDocEnd, Some("Workspace")),
        KeyBinding::new("cmd-home", MoveDocStart, Some("Workspace")),
        KeyBinding::new("cmd-end", MoveDocEnd, Some("Workspace")),
        KeyBinding::new("cmd-shift-home", SelectDocStart, Some("Workspace")),
        KeyBinding::new("cmd-shift-end", SelectDocEnd, Some("Workspace")),
        KeyBinding::new("ctrl-home", MoveDocStart, Some("Workspace")),
        KeyBinding::new("ctrl-end", MoveDocEnd, Some("Workspace")),
        KeyBinding::new("ctrl-shift-home", SelectDocStart, Some("Workspace")),
        KeyBinding::new("ctrl-shift-end", SelectDocEnd, Some("Workspace")),
    ]);
}
