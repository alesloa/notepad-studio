// Clipboard history — Edit ▸ Clipboard History panel feeds from this.
//
// Notepad++ source:
//   PowerEditor/src/WinControls/ClipboardHistoryPanel/ClipboardHistoryPanel.cpp
//   PowerEditor/src/NppCommands.cpp:1147 — IDM_EDIT_CLIPBOARDHISTORY_PANEL
//
// NPP keeps the panel's list in sync by hooking SCN_MODIFIED and pushing
// every copy/cut payload to the panel. Same idea here: every workspace
// copy/cut funnels through `record_copy` before/after `cx.write_to_clipboard`,
// so the popup panel renders straight from this process-global ring.
//
// History is in-memory only — not persisted across restarts, matching NPP.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// Hard cap on retained entries. Mirrors NPP's `_clipboardHistoryItemNbMax`
/// default (12 in the latest 8.x source). Sized so the panel never grows
/// past one screen of comfortably-readable previews.
const CAPACITY: usize = 16;

#[derive(Default)]
struct History {
    /// Front = most recent. Capped at `CAPACITY` — pushing past the cap
    /// drops the oldest entry.
    entries: VecDeque<String>,
}

static GLOBAL: OnceLock<Mutex<History>> = OnceLock::new();

fn global() -> &'static Mutex<History> {
    GLOBAL.get_or_init(|| Mutex::new(History::default()))
}

/// Push `text` onto the front of the history. Empty payloads are dropped
/// (matches NPP — Ctrl+C with no selection on an empty buffer doesn't
/// pollute the panel). If `text` matches the current head, the duplicate
/// is collapsed instead of being prepended, so repeatedly copying the
/// same selection doesn't fill the ring with identical rows.
pub(crate) fn record_copy(text: &str) {
    if text.is_empty() {
        return;
    }
    let Ok(mut g) = global().lock() else {
        return;
    };
    if g.entries.front().map(|s| s.as_str()) == Some(text) {
        return;
    }
    g.entries.push_front(text.to_owned());
    while g.entries.len() > CAPACITY {
        g.entries.pop_back();
    }
}

/// Snapshot of the current history, front-to-back (most-recent first).
/// Used by the popup panel to render its rows; cheap because the entries
/// are bounded by `CAPACITY`.
pub(crate) fn snapshot() -> Vec<String> {
    global()
        .lock()
        .map(|g| g.entries.iter().cloned().collect())
        .unwrap_or_default()
}

/// Look up an entry by index into the front-to-back snapshot order.
/// Returns `None` if the index is out of range — the panel uses this
/// when the user clicks/Enters a row to fetch the text to paste.
pub(crate) fn get(index: usize) -> Option<String> {
    global().lock().ok().and_then(|g| g.entries.get(index).cloned())
}

/// Drop every retained entry. Wired to the panel's `Clear` button so the
/// user can scrub history when copying something sensitive.
pub(crate) fn clear() {
    if let Ok(mut g) = global().lock() {
        g.entries.clear();
    }
}
