// Cross-platform key bindings.
//
// User-customisable chords (every action surfaced in `Settings ▸ Shortcut
// Mapper`) live in `crate::shortcuts::CATALOG`. The catalog is the single
// source of truth for defaults *and* the registry of which action types
// exist; `register_catalog` builds `KeyBinding`s from the catalog plus any
// loaded user overrides.
//
// Dialog-internal chords (text editing inside the Find dialog inputs) are
// NOT exposed to the user — they live in `register_dialog_only` here so the
// catalog stays focused on the actions a user might rebind.

use gpui::{App, KeyBinding};

use crate::actions::*;
use crate::shortcuts;

/// Bind every chord in the catalog (defaults + user overrides) plus the
/// dialog-internal mirrors. Called once on app start; the Shortcut Mapper
/// re-runs the equivalent path via `shortcuts::reload_keymap` after a save.
pub(crate) fn bind_app_keys(cx: &mut App) {
    let overrides = shortcuts::load_overrides();
    shortcuts::register_catalog(cx, &overrides);
    register_dialog_only(cx);
}

/// Bind every chord that lives only in the FindDialog window — the dialog's
/// own copy of the editor's text-input keymap, plus dialog-specific actions
/// like `EditFindClose` / `FindInFilesRun` whose chord on the editor side is
/// already in the catalog.
///
/// Called from both startup (after `register_catalog`) and from the
/// Shortcut Mapper's reload path so the dialog's text-input behaviour stays
/// intact across rebinds.
pub(crate) fn register_dialog_only(cx: &mut App) {
    cx.bind_keys([
        // Esc / Enter / F3 / Shift-F3 / Shift-Enter inside the dialog.
        KeyBinding::new("escape", EditFindClose, Some("FindDialog")),
        KeyBinding::new("f3", EditFindNext, Some("FindDialog")),
        KeyBinding::new("shift-f3", EditFindPrev, Some("FindDialog")),
        KeyBinding::new("shift-enter", EditFindPrev, Some("FindDialog")),
        KeyBinding::new("enter", EditInsertNewline, Some("FindDialog")),
    ]);

    // Editing chords inside the dialog inputs (mirrors of the editor's
    // text-input keymap, scoped to the FindDialog context).
    cx.bind_keys([
        KeyBinding::new("backspace", EditBackspace, Some("FindDialog")),
        KeyBinding::new("delete", EditDelete, Some("FindDialog")),
        KeyBinding::new("left", MoveLeft, Some("FindDialog")),
        KeyBinding::new("right", MoveRight, Some("FindDialog")),
        KeyBinding::new("shift-left", SelectLeft, Some("FindDialog")),
        KeyBinding::new("shift-right", SelectRight, Some("FindDialog")),
        KeyBinding::new("home", MoveLineStart, Some("FindDialog")),
        KeyBinding::new("end", MoveLineEnd, Some("FindDialog")),
        KeyBinding::new("shift-home", SelectLineStart, Some("FindDialog")),
        KeyBinding::new("shift-end", SelectLineEnd, Some("FindDialog")),
        KeyBinding::new("cmd-left", MoveLineStart, Some("FindDialog")),
        KeyBinding::new("cmd-right", MoveLineEnd, Some("FindDialog")),
        KeyBinding::new("cmd-shift-left", SelectLineStart, Some("FindDialog")),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, Some("FindDialog")),
        KeyBinding::new("alt-left", MoveWordLeft, Some("FindDialog")),
        KeyBinding::new("alt-right", MoveWordRight, Some("FindDialog")),
        KeyBinding::new("alt-shift-left", SelectWordLeft, Some("FindDialog")),
        KeyBinding::new("alt-shift-right", SelectWordRight, Some("FindDialog")),
        KeyBinding::new("ctrl-left", MoveWordLeft, Some("FindDialog")),
        KeyBinding::new("ctrl-right", MoveWordRight, Some("FindDialog")),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some("FindDialog")),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, Some("FindDialog")),
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
