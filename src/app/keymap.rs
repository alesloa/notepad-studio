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

use gpui::{Action, App};

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

/// Win32 menu-bar keys for the in-window menu bar (Windows and Linux; macOS
/// menus are native). Bound before the catalog so a user chord on the same
/// keys wins: among equally specific bindings gpui takes the later one.
/// The Alt+<key> list is the access keys of Notepad++'s top-level titles
/// (PowerEditor/src/Notepad_plus.rc, IDR_M30_MENU).
pub(crate) fn register_menu_bar_keys(cx: &mut App) {
    if cfg!(target_os = "macos") {
        return;
    }
    let mapper = cx.keyboard_mapper().clone();
    let mut bindings = Vec::new();
    for (chord, action) in menu_bar_chords() {
        match shortcuts::load_binding(&chord, action, Some("Workspace"), mapper.as_ref()) {
            Ok(binding) => bindings.push(binding),
            Err(err) => eprintln!("keymap: skipping menu bar chord {chord:?}: {err}"),
        }
    }
    cx.bind_keys(bindings);
}

/// Alt tap, F10 and Alt+<access key> for every top-level menu title.
fn menu_bar_chords() -> Vec<(String, Box<dyn Action>)> {
    let mut chords: Vec<(String, Box<dyn Action>)> = vec![
        ("alt".into(), act(MenuBarAltTap)),
        ("f10".into(), act(MenuBarActivate)),
        ("alt-shift-/".into(), act(MenuBarOpen { key: '?' })),
    ];
    for key in ['f', 'e', 's', 'v', 'n', 'l', 't', 'o', 'm', 'r', 'p', 'w'] {
        chords.push((format!("alt-{key}"), act(MenuBarOpen { key })));
    }
    chords
}

const FIND_DIALOG: &str = "FindDialog";

fn act(action: impl Action) -> Box<dyn Action> {
    Box::new(action)
}

/// Chords every platform shares inside the Find dialog: Esc / Enter / F3 /
/// Shift-F3 / Shift-Enter plus the plain caret keys.
fn dialog_common() -> Vec<(&'static str, Box<dyn Action>)> {
    vec![
        ("escape", act(EditFindClose)),
        ("f3", act(EditFindNext)),
        ("shift-f3", act(EditFindPrev)),
        ("shift-enter", act(EditInsertNewline)),
        ("enter", act(EditInsertNewline)),
        ("backspace", act(EditBackspace)),
        ("delete", act(EditDelete)),
        ("left", act(MoveLeft)),
        ("right", act(MoveRight)),
        ("shift-left", act(SelectLeft)),
        ("shift-right", act(SelectRight)),
        ("home", act(MoveLineStart)),
        ("end", act(MoveLineEnd)),
        ("shift-home", act(SelectLineStart)),
        ("shift-end", act(SelectLineEnd)),
    ]
}

/// Text-field chords inside the Find dialog on Windows and Linux: the
/// Ctrl-family word and document moves plus the clipboard keys, matching
/// a Windows edit control (Ctrl+Insert / Shift+Insert / Shift+Delete
/// included, as Notepad++ binds them in the editor).
fn dialog_windows_linux() -> Vec<(&'static str, Box<dyn Action>)> {
    vec![
        ("ctrl-left", act(MoveWordLeft)),
        ("ctrl-right", act(MoveWordRight)),
        ("ctrl-shift-left", act(SelectWordLeft)),
        ("ctrl-shift-right", act(SelectWordRight)),
        ("ctrl-home", act(MoveDocStart)),
        ("ctrl-end", act(MoveDocEnd)),
        ("ctrl-shift-home", act(SelectDocStart)),
        ("ctrl-shift-end", act(SelectDocEnd)),
        ("ctrl-a", act(EditSelectAll)),
        ("ctrl-c", act(EditCopy)),
        ("ctrl-insert", act(EditCopy)),
        ("ctrl-x", act(EditCut)),
        ("shift-delete", act(EditCut)),
        ("ctrl-v", act(EditPaste)),
        ("shift-insert", act(EditPaste)),
    ]
}

/// Text-field chords inside the Find dialog on macOS: Option for words,
/// Cmd for line ends, document ends and the clipboard.
fn dialog_mac() -> Vec<(&'static str, Box<dyn Action>)> {
    vec![
        ("cmd-left", act(MoveLineStart)),
        ("cmd-right", act(MoveLineEnd)),
        ("cmd-shift-left", act(SelectLineStart)),
        ("cmd-shift-right", act(SelectLineEnd)),
        ("alt-left", act(MoveWordLeft)),
        ("alt-right", act(MoveWordRight)),
        ("alt-shift-left", act(SelectWordLeft)),
        ("alt-shift-right", act(SelectWordRight)),
        ("cmd-up", act(MoveDocStart)),
        ("cmd-down", act(MoveDocEnd)),
        ("cmd-shift-up", act(SelectDocStart)),
        ("cmd-shift-down", act(SelectDocEnd)),
        ("cmd-home", act(MoveDocStart)),
        ("cmd-end", act(MoveDocEnd)),
        ("cmd-shift-home", act(SelectDocStart)),
        ("cmd-shift-end", act(SelectDocEnd)),
        ("cmd-a", act(EditSelectAll)),
        ("cmd-c", act(EditCopy)),
        ("cmd-x", act(EditCut)),
        ("cmd-v", act(EditPaste)),
    ]
}

/// The Find dialog's chord list for the platform this binary targets.
fn dialog_chords() -> Vec<(&'static str, Box<dyn Action>)> {
    let mut chords = dialog_common();
    if cfg!(target_os = "macos") {
        chords.extend(dialog_mac());
    } else {
        chords.extend(dialog_windows_linux());
    }
    chords
}

/// Bind every chord that lives only in the FindDialog window — the dialog's
/// own copy of the editor's text-input keymap, plus dialog-specific actions
/// like `EditFindClose` whose chord on the editor side is already in the
/// catalog.
///
/// Called from both startup (after `register_catalog`) and from the
/// Shortcut Mapper's reload path so the dialog's text-input behaviour stays
/// intact across rebinds.
pub(crate) fn register_dialog_only(cx: &mut App) {
    let mapper = cx.keyboard_mapper().clone();
    let mut bindings = Vec::new();
    for (chord, action) in dialog_chords() {
        match shortcuts::load_binding(chord, action, Some(FIND_DIALOG), mapper.as_ref()) {
            Ok(binding) => bindings.push(binding),
            Err(err) => eprintln!("keymap: skipping Find dialog chord {chord:?}: {err}"),
        }
    }
    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn assert_list_loads(list: Vec<(&'static str, Box<dyn Action>)>, name: &str) {
        let mut seen = BTreeSet::new();
        for (chord, action) in list {
            assert!(
                seen.insert(shortcuts::canonical_chord(chord)),
                "{name}: {chord} listed twice"
            );
            assert!(
                shortcuts::load_binding(chord, action, Some(FIND_DIALOG), &gpui::DummyKeyboardMapper)
                    .is_ok(),
                "{name}: {chord} does not load"
            );
        }
    }

    #[test]
    fn find_dialog_chords_load_on_every_platform() {
        let mut win = dialog_common();
        win.extend(dialog_windows_linux());
        assert_list_loads(win, "Windows/Linux");
        let mut mac = dialog_common();
        mac.extend(dialog_mac());
        assert_list_loads(mac, "macOS");
    }

    #[test]
    fn find_dialog_has_no_cross_platform_strays() {
        for (chord, _) in dialog_windows_linux() {
            assert!(!chord.contains("cmd"), "{chord} in the Windows/Linux list");
        }
        for (chord, _) in dialog_mac() {
            assert!(!chord.starts_with("ctrl"), "{chord} in the macOS list");
        }
    }
}
