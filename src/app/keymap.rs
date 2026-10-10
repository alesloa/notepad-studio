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
    register_menu_bar_keys(cx);
    shortcuts::register_catalog(cx, &overrides);
    register_dialog_only(cx);
}

/// Bind through the platform keyboard mapper. `KeyBinding::new` skips it, and
/// Windows reports Shift+<digit or punctuation> as the shifted character with
/// Shift cleared (Ctrl+Shift+= arrives as `ctrl-+`), so a `ctrl-shift-=`
/// binding built by `new` alone never fires there. The mapper rewrites it to
/// what the active layout sends; on macOS and Linux it leaves bindings as is.
pub(crate) fn bind_mapped(cx: &mut App, bindings: impl IntoIterator<Item = KeyBinding>) {
    let mapper = cx.keyboard_mapper().clone();
    let mapped: Vec<KeyBinding> = bindings
        .into_iter()
        .map(|binding| {
            let source = binding
                .keystrokes()
                .iter()
                .map(|k| k.unparse())
                .collect::<Vec<_>>()
                .join(" ");
            KeyBinding::load(
                &source,
                binding.action().boxed_clone(),
                binding.predicate(),
                false,
                None,
                mapper.as_ref(),
            )
            .unwrap_or(binding)
        })
        .collect();
    cx.bind_keys(mapped);
}

/// Win32 menu-bar keys for the in-window menu bar (Windows and Linux; macOS
/// menus are native). Bound before the catalog so a user chord on the same
/// keys wins: among equally specific bindings gpui takes the later one.
/// The Alt+<key> list is the access keys of Notepad++'s top-level titles
/// (PowerEditor/src/Notepad_plus.rc, IDR_M30_MENU).
pub(crate) fn register_menu_bar_keys(cx: &mut App) {
    if cfg!(target_os = "macos") {
        return;
    }
    let ws = Some("Workspace");
    bind_mapped(
        cx,
        [
            KeyBinding::new("alt", MenuBarAltTap, ws),
            KeyBinding::new("f10", MenuBarActivate, ws),
            KeyBinding::new("alt-shift-/", MenuBarOpen { key: '?' }, ws),
        ],
    );
    bind_mapped(
        cx,
        ['f', 'e', 's', 'v', 'n', 'l', 't', 'o', 'm', 'r', 'p', 'w']
            .into_iter()
            .map(|key| KeyBinding::new(&format!("alt-{key}"), MenuBarOpen { key }, ws)),
    );
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
    bind_mapped(cx, [
        // Esc / Enter / F3 / Shift-F3 / Shift-Enter inside the dialog.
        KeyBinding::new("escape", EditFindClose, Some("FindDialog")),
        KeyBinding::new("f3", EditFindNext, Some("FindDialog")),
        KeyBinding::new("shift-f3", EditFindPrev, Some("FindDialog")),
        KeyBinding::new("shift-enter", EditFindPrev, Some("FindDialog")),
        KeyBinding::new("enter", EditInsertNewline, Some("FindDialog")),
    ]);

    // Editing chords inside the dialog inputs (mirrors of the editor's
    // text-input keymap, scoped to the FindDialog context).
    bind_mapped(cx, [
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
