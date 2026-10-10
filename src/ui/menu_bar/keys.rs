// Keyboard helpers for the menu bar: Notepad++ accelerator text and Win32
// row navigation.
//
// PowerEditor/src/WinControls/shortcut/shortcut.cpp:294 — Shortcut::toString
// orders modifiers Ctrl+Alt+Shift; key names come from namedKeyArray (:35).

use gpui::Modifiers;

/// Accelerator text for one keystroke, e.g. `Ctrl+Shift+S`, `Alt+Down`.
pub(crate) fn chord_label(m: &Modifiers, key: &str) -> String {
    let mut s = String::new();
    if m.control {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    if m.platform {
        s.push_str(if cfg!(target_os = "windows") { "Win+" } else { "Super+" });
    }
    s.push_str(&key_name(key));
    s
}

/// Notepad++'s name for a gpui key.
pub(crate) fn key_name(key: &str) -> String {
    let named = match key {
        "backspace" => "Backspace",
        "tab" => "Tab",
        "enter" => "Enter",
        "escape" => "Esc",
        "space" => "Spacebar",
        "pageup" => "Page up",
        "pagedown" => "Page down",
        "end" => "End",
        "home" => "Home",
        "left" => "Left",
        "up" => "Up",
        "right" => "Right",
        "down" => "Down",
        "insert" => "INS",
        "delete" => "DEL",
        // shortcut.cpp:276 shows the backtick key as `~` on US layouts.
        "`" => "~",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    if let Some(n) = key.strip_prefix('f') {
        if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
            return format!("F{n}");
        }
    }
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The next row to highlight moving one step `forward` (or back) from
/// `from`, wrapping at either end and skipping rows that can't take the
/// highlight. `from = None` starts outside the list, so the first step lands
/// on the first (or last) selectable row.
pub(crate) fn step(selectable: &[bool], from: Option<usize>, forward: bool) -> Option<usize> {
    let n = selectable.len();
    if n == 0 {
        return None;
    }
    let mut i = match from {
        Some(i) if i < n => i,
        _ if forward => n - 1,
        _ => 0,
    };
    for _ in 0..n {
        i = if forward { (i + 1) % n } else { (i + n - 1) % n };
        if selectable.get(i).copied().unwrap_or(false) {
            return Some(i);
        }
    }
    None
}

/// Rows whose access key is `typed`, starting after `from` and wrapping, so
/// repeated presses cycle through rows that share a key.
pub(crate) fn matching(keys: &[Option<char>], typed: char, from: Option<usize>) -> Vec<usize> {
    let n = keys.len();
    let start = from.map_or(0, |i| i.saturating_add(1));
    (0..n)
        .map(|k| (start + k) % n)
        .filter(|&i| keys.get(i).copied().flatten() == Some(typed))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(control: bool, alt: bool, shift: bool) -> Modifiers {
        Modifiers {
            control,
            alt,
            shift,
            ..Default::default()
        }
    }

    #[test]
    fn modifier_order_matches_notepad_plus_plus() {
        assert_eq!(chord_label(&mods(true, true, true), "s"), "Ctrl+Alt+Shift+S");
        assert_eq!(chord_label(&mods(true, false, true), "up"), "Ctrl+Shift+Up");
        assert_eq!(chord_label(&mods(false, true, false), "f4"), "Alt+F4");
        assert_eq!(chord_label(&mods(false, false, false), "f3"), "F3");
    }

    #[test]
    fn named_keys() {
        assert_eq!(key_name("escape"), "Esc");
        assert_eq!(key_name("space"), "Spacebar");
        assert_eq!(key_name("pageup"), "Page up");
        assert_eq!(key_name("delete"), "DEL");
        assert_eq!(key_name("insert"), "INS");
        assert_eq!(key_name("f12"), "F12");
        assert_eq!(key_name("`"), "~");
        assert_eq!(key_name("/"), "/");
        assert_eq!(key_name("a"), "A");
        assert_eq!(key_name("f"), "F");
        assert_eq!(key_name(""), "");
    }

    #[test]
    fn step_wraps_and_skips_separators() {
        let sel = [true, false, true, true];
        assert_eq!(step(&sel, None, true), Some(0));
        assert_eq!(step(&sel, None, false), Some(3));
        assert_eq!(step(&sel, Some(0), true), Some(2));
        assert_eq!(step(&sel, Some(3), true), Some(0));
        assert_eq!(step(&sel, Some(0), false), Some(3));
        assert_eq!(step(&sel, Some(2), false), Some(0));
    }

    #[test]
    fn step_on_empty_or_unselectable_lists() {
        assert_eq!(step(&[], None, true), None);
        assert_eq!(step(&[false, false], Some(0), true), None);
        assert_eq!(step(&[true], Some(0), true), Some(0));
        // A stale index past the end restarts from the edge.
        assert_eq!(step(&[true, true], Some(9), true), Some(0));
    }

    #[test]
    fn matching_cycles_from_the_current_row() {
        let keys = [Some('s'), Some('a'), Some('s'), None, Some('s')];
        assert_eq!(matching(&keys, 's', None), vec![0, 2, 4]);
        assert_eq!(matching(&keys, 's', Some(0)), vec![2, 4, 0]);
        assert_eq!(matching(&keys, 's', Some(4)), vec![0, 2, 4]);
        assert_eq!(matching(&keys, 'a', Some(1)), vec![1]);
        assert!(matching(&keys, 'z', None).is_empty());
        assert!(matching(&[], 'z', Some(3)).is_empty());
    }
}
