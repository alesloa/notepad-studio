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
use std::sync::atomic::{AtomicBool, Ordering};

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

// Bulk callers prepare the second owned payload off-thread. Publication keeps
// clipboard order and returns unused/evicted text for off-thread disposal.
pub(crate) fn prepare_copy(text: &str, cancel: &AtomicBool) -> Result<String, &'static str> {
    if text.len() > 128 * 1024 * 1024 { return Err("Clipboard history exceeds the 128 MiB limit"); }
    let mut owned = String::new();
    owned.try_reserve_exact(text.len()).map_err(|_| "Not enough memory to prepare clipboard history")?;
    let mut offset = 0usize;
    while offset < text.len() {
        if cancel.load(Ordering::Relaxed) { return Err("Clipboard history preparation cancelled"); }
        let mut end = offset.saturating_add(64 * 1024).min(text.len());
        while end > offset && !text.is_char_boundary(end) { end -= 1; }
        let part = text.get(offset..end).ok_or("Invalid clipboard history text offset")?;
        owned.push_str(part); offset = end;
    }
    if cancel.load(Ordering::Relaxed) { return Err("Clipboard history preparation cancelled"); }
    Ok(owned)
}

pub(crate) fn record_copy_owned(text: String) -> Option<String> {
    let Ok(mut history) = global().lock() else { return Some(text); };
    history.record_owned(text)
}

impl History {
    fn record_owned(&mut self, text: String) -> Option<String> {
        if text.is_empty() || self.entries.front().is_some_and(|head| head == &text) { return Some(text); }
        if self.entries.len() < CAPACITY && self.entries.try_reserve(1).is_err() { return Some(text); }
        let evicted = if self.entries.len() == CAPACITY { self.entries.pop_back() } else { None };
        self.entries.push_front(text);
        evicted
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_publication_moves_payload_preserves_dedup_order_and_returns_evictions() {
        let mut history = History::default();
        let text = "é😀".repeat(50_000); let address = text.as_ptr();
        assert!(history.record_owned(text.clone()).is_none());
        assert_eq!(history.record_owned(text.clone()), Some(text.clone()));
        assert_eq!(history.record_owned(String::new()), Some(String::new()));
        let mut fresh = History::default(); assert!(fresh.record_owned(text).is_none());
        assert_eq!(fresh.entries.front().unwrap().as_ptr(), address);
        for index in 0..CAPACITY { assert!(history.record_owned(index.to_string()).is_none() || index == CAPACITY - 1); }
        assert_eq!(history.entries.front().unwrap(), &(CAPACITY - 1).to_string());
        assert_eq!(history.record_owned("newest".into()), Some("0".into()));
        assert_eq!(history.entries.len(), CAPACITY);
        assert!(prepare_copy("é😀", &AtomicBool::new(true)).is_err());
        assert_eq!(prepare_copy(&"é😀".repeat(50_000), &AtomicBool::new(false)).unwrap(), "é😀".repeat(50_000));
    }
}
