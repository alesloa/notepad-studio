// Per-window state the menus draw as checkmarks and radio marks (Notepad++
// checks Word wrap, the current encoding, the current language and so on).
//
// Menu builders can't read the `Workspace` entity: `rebuild_menus` runs from
// inside workspace updates (io.rs, macros), and reading an entity that is
// mid-update panics. So the active workspace pushes a snapshot here on every
// render, and the builders read the snapshot.

use std::sync::{Mutex, OnceLock};

use crate::syntax::LanguageId;
use crate::text::encoding::Encoding;
use crate::text::line_ending::LineEnding;

/// What the menus need to know about the active window's document.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct MenuState {
    /// False until a workspace has rendered once (or when none is open, as on
    /// macOS after the last window closes); builders show nothing checked.
    pub(crate) has_document: bool,
    pub(crate) word_wrap: bool,
    pub(crate) show_whitespace: bool,
    pub(crate) overtype: bool,
    pub(crate) encoding: Option<Encoding>,
    pub(crate) line_ending: Option<LineEnding>,
    /// `None` with `has_document` is a plain-text buffer ("Normal Text").
    pub(crate) language: Option<LanguageId>,
    pub(crate) tab_color: Option<u8>,
}

static STATE: OnceLock<Mutex<MenuState>> = OnceLock::new();

fn slot() -> &'static Mutex<MenuState> {
    STATE.get_or_init(|| Mutex::new(MenuState::default()))
}

/// Store `next`; returns true when it differs from what was there, so the
/// caller can rebuild native menus only on a real change.
pub(crate) fn publish(next: MenuState) -> bool {
    let Ok(mut current) = slot().lock() else {
        return false;
    };
    if *current == next {
        return false;
    }
    *current = next;
    true
}

/// The last published state, or the default when the lock is poisoned.
pub(crate) fn current() -> MenuState {
    slot().lock().map(|s| s.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_reports_only_real_changes() {
        let a = MenuState {
            has_document: true,
            word_wrap: true,
            ..MenuState::default()
        };
        publish(a.clone());
        assert!(!publish(a.clone()));
        assert_eq!(current(), a);
        let b = MenuState {
            word_wrap: false,
            ..a
        };
        assert!(publish(b.clone()));
        assert_eq!(current(), b);
    }
}
