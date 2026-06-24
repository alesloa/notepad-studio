// Find / Replace / Goto Line / Find in Files / Mark keybindings.
//
// Notepad++ defaults with macOS adaptations for the chords that collide with
// system shortcuts:
//   * Replace    — Ctrl+H (Notepad++) → Cmd+Opt+F on macOS (Cmd+H = Hide).
//   * Goto Line  — Ctrl+G (Notepad++) → Cmd+L on macOS (Cmd+G = Find Next).
//
// Bindings live on the `Workspace` key context (the editor window) and on the
// `FindDialog` context (the dialog window) — gpui dispatches actions against
// whichever window is focused, so registering both contexts lets keystrokes
// like F3 / Esc / Shift-Enter work no matter which window has focus.

use gpui::{App, KeyBinding};

use crate::actions::*;

pub(crate) fn register(cx: &mut App) {
    cx.bind_keys([
        // Find — opens the dialog on the Find tab (or focuses if open).
        KeyBinding::new("cmd-f", EditFind, Some("Workspace")),
        KeyBinding::new("ctrl-f", EditFind, Some("Workspace")),
        // Esc closes whichever search surface is up — dialog or inline goto.
        KeyBinding::new("escape", EditFindClose, Some("Workspace")),
        KeyBinding::new("escape", EditFindClose, Some("FindDialog")),
        // F3 / Shift-F3 — Find Next / Previous, same chord regardless of
        // which window is focused.
        KeyBinding::new("f3", EditFindNext, Some("Workspace")),
        KeyBinding::new("shift-f3", EditFindPrev, Some("Workspace")),
        KeyBinding::new("f3", EditFindNext, Some("FindDialog")),
        KeyBinding::new("shift-f3", EditFindPrev, Some("FindDialog")),
        // Shift-Enter — Find Previous when a find/replace/mark input is
        // focused. Editor-context shift-enter is a no-op (find_prev short-
        // circuits when there are no cached matches).
        KeyBinding::new("shift-enter", EditFindPrev, Some("Workspace")),
        KeyBinding::new("shift-enter", EditFindPrev, Some("FindDialog")),
        // Enter inside the dialog routes through Workspace::insert_newline
        // — the same handler the editor uses — so the per-input dispatch
        // (Find Next / Replace / FIF run / Mark all) lands on the right
        // workspace method based on which input is focused.
        KeyBinding::new("enter", EditInsertNewline, Some("FindDialog")),
        // Replace — opens the dialog on the Replace tab.
        KeyBinding::new("cmd-alt-f", EditReplace, Some("Workspace")),
        KeyBinding::new("ctrl-h", EditReplace, Some("Workspace")),
        // Goto Line stays inline above the editor.
        KeyBinding::new("cmd-l", EditGotoLine, Some("Workspace")),
        KeyBinding::new("ctrl-g", EditGotoLine, Some("Workspace")),
        // Find in Files — opens the dialog on the FIF tab.
        KeyBinding::new("cmd-shift-f", SearchFindInFiles, Some("Workspace")),
        KeyBinding::new("ctrl-shift-f", SearchFindInFiles, Some("Workspace")),
        // Mark — opens the dialog on the Mark tab. Notepad++ doesn't reserve
        // a default chord for Mark; route it to Cmd/Ctrl+M, which Notepad++
        // already uses for "Trace Mark Line" in some menus.
        KeyBinding::new("cmd-m", SearchMark, Some("Workspace")),
        KeyBinding::new("ctrl-m", SearchMark, Some("Workspace")),
    ]);

    // Editing chords inside the dialog inputs. The dialog window has its own
    // dispatch tree under the `FindDialog` key context, so the editor-window
    // bindings (which scope to `Workspace`) never fire here. We mirror the
    // universal text-input chords (backspace / delete / arrows / home / end /
    // word jumps / clipboard / select-all) so the dialog inputs behave like
    // any other native single-line text field.
    cx.bind_keys([
        // Character delete.
        KeyBinding::new("backspace", EditBackspace, Some("FindDialog")),
        KeyBinding::new("delete", EditDelete, Some("FindDialog")),
        // Caret movement.
        KeyBinding::new("left", MoveLeft, Some("FindDialog")),
        KeyBinding::new("right", MoveRight, Some("FindDialog")),
        KeyBinding::new("shift-left", SelectLeft, Some("FindDialog")),
        KeyBinding::new("shift-right", SelectRight, Some("FindDialog")),
        // Line start / end. Home/End are universal; cmd-left/right are macOS.
        KeyBinding::new("home", MoveLineStart, Some("FindDialog")),
        KeyBinding::new("end", MoveLineEnd, Some("FindDialog")),
        KeyBinding::new("shift-home", SelectLineStart, Some("FindDialog")),
        KeyBinding::new("shift-end", SelectLineEnd, Some("FindDialog")),
        KeyBinding::new("cmd-left", MoveLineStart, Some("FindDialog")),
        KeyBinding::new("cmd-right", MoveLineEnd, Some("FindDialog")),
        KeyBinding::new("cmd-shift-left", SelectLineStart, Some("FindDialog")),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, Some("FindDialog")),
        // Word jumps. macOS = alt; Notepad++ / Linux = ctrl.
        KeyBinding::new("alt-left", MoveWordLeft, Some("FindDialog")),
        KeyBinding::new("alt-right", MoveWordRight, Some("FindDialog")),
        KeyBinding::new("alt-shift-left", SelectWordLeft, Some("FindDialog")),
        KeyBinding::new("alt-shift-right", SelectWordRight, Some("FindDialog")),
        KeyBinding::new("ctrl-left", MoveWordLeft, Some("FindDialog")),
        KeyBinding::new("ctrl-right", MoveWordRight, Some("FindDialog")),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some("FindDialog")),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, Some("FindDialog")),
        // Document jumps — for a single-line input these collapse to
        // line-start / line-end, but the editor's MoveDocStart/End handlers
        // already branch on input focus, so binding the same chord keeps
        // user muscle memory portable across the editor and dialog inputs.
        KeyBinding::new("cmd-up", MoveDocStart, Some("FindDialog")),
        KeyBinding::new("cmd-down", MoveDocEnd, Some("FindDialog")),
        KeyBinding::new("cmd-shift-up", SelectDocStart, Some("FindDialog")),
        KeyBinding::new("cmd-shift-down", SelectDocEnd, Some("FindDialog")),
        KeyBinding::new("cmd-home", MoveDocStart, Some("FindDialog")),
        KeyBinding::new("cmd-end", MoveDocEnd, Some("FindDialog")),
        KeyBinding::new("cmd-shift-home", SelectDocStart, Some("FindDialog")),
        KeyBinding::new("cmd-shift-end", SelectDocEnd, Some("FindDialog")),
        KeyBinding::new("ctrl-home", MoveDocStart, Some("FindDialog")),
        KeyBinding::new("ctrl-end", MoveDocEnd, Some("FindDialog")),
        KeyBinding::new("ctrl-shift-home", SelectDocStart, Some("FindDialog")),
        KeyBinding::new("ctrl-shift-end", SelectDocEnd, Some("FindDialog")),
        // Clipboard.
        KeyBinding::new("cmd-a", EditSelectAll, Some("FindDialog")),
        KeyBinding::new("ctrl-a", EditSelectAll, Some("FindDialog")),
        KeyBinding::new("cmd-c", EditCopy, Some("FindDialog")),
        KeyBinding::new("ctrl-c", EditCopy, Some("FindDialog")),
        KeyBinding::new("cmd-x", EditCut, Some("FindDialog")),
        KeyBinding::new("ctrl-x", EditCut, Some("FindDialog")),
        KeyBinding::new("cmd-v", EditPaste, Some("FindDialog")),
        KeyBinding::new("ctrl-v", EditPaste, Some("FindDialog")),
    ]);
}
