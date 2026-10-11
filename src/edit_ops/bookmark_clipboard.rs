// PowerEditor/src/Notepad_plus.cpp:2962–2976, 3094–3106 — Copy Bookmarked Lines.
// NppCommands.cpp:2389–2391, 4563 — command route and macro recording.
// Original Rust; the oracle supplies command behavior, not implementation.

use crate::{
    actions::CopyBookmarkedLines,
    buffer::ContentRevision,
    macros::{self, MacroEvent},
    tab::{InputFocus, TabViewMode},
    text::line_ending::LineEnding,
    workspace::Workspace,
};
use gpui::{AppContext, ClipboardItem, Context, Window};
use ropey::Rope;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use uuid::Uuid;

const MAX_COPY_BYTES: usize = 128 * 1024 * 1024;
const MAX_BOOKMARKS: usize = 1_000_000;
const FOREGROUND_BYTES: usize = 256 * 1024;
const FOREGROUND_BOOKMARKS: usize = 4096;
const PREPARING: &str = "Preparing Copy Bookmarked Lines; Escape cancels";

#[cfg(test)]
thread_local! {
    static PHYSICAL_LINE_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) struct PendingBookmarkCopy {
    id: Uuid,
    cancel: Arc<AtomicBool>,
}

impl Drop for PendingBookmarkCopy {
    fn drop(&mut self) { self.cancel.store(true, Ordering::Relaxed); }
}

struct CopySnapshot {
    tab_id: Uuid,
    source_epoch: crate::workspace::source_input::SourceInputToken,
    input_context: gpui::InputContextToken,
    instance: Arc<()>,
    revision: ContentRevision,
    activity: ContentRevision,
    ending: LineEnding,
    bookmarks: Arc<Vec<usize>>,
}

impl CopySnapshot {
    fn valid(&self, w: &Workspace, window: &Window) -> bool {
        let Some(tab) = w.tabs.get(w.active) else { return false; };
        !w.window_closed && window.is_window_active() && w.focus_handle.is_focused(window)
            && w.input_focus(window) == InputFocus::Editor && !w.is_composing_text(window)
            && tab.view_mode == TabViewMode::Edit && tab.buffer.loading.is_none()
            && tab.session_id == self.tab_id && Arc::ptr_eq(&tab.instance, &self.instance)
            && w.source_input_token() == self.source_epoch && window.input_context_token() == self.input_context
            && tab.buffer.raw_revision == self.revision && tab.content_activity == self.activity
            && tab.buffer.line_ending == self.ending
            && tab.buffer.bookmarks.iter().copied().eq(self.bookmarks.iter().copied())
    }
}

fn bookmarked_text(
    rope: &Rope,
    bookmarks: &[usize],
    ending: LineEnding,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<String, &'static str> {
    let mut output = String::new();
    let mut previous: Option<std::ops::Range<usize>> = None;
    for &at in bookmarks {
        if cancel.load(Ordering::Relaxed) { return Err("Copy Bookmarked Lines cancelled"); }
        if at > rope.len_bytes() { continue; }
        if previous.as_ref().is_some_and(|range| range.contains(&at) || range.start == at) { continue; }
        #[cfg(test)]
        PHYSICAL_LINE_LOOKUPS.with(|count| count.set(count.get() + 1));
        let range = super::menu_line_commands::physical_line_range(rope, at, cancel)?;
        if previous.as_ref().is_some_and(|last| last.start == range.start) { continue; }
        previous = Some(range.clone());
        let mut body_end = range.end;
        let suffix = if body_end > range.start && rope.get_byte(body_end - 1) == Some(b'\n') {
            body_end -= 1;
            if body_end > range.start && rope.get_byte(body_end - 1) == Some(b'\r') {
                body_end -= 1;
                if ending == LineEnding::Cr { "\r" } else { "\r\n" }
            } else { ending.as_str() }
        } else if body_end > range.start && rope.get_byte(body_end - 1) == Some(b'\r') {
            body_end -= 1;
            "\r"
        } else { "" };
        let required = output.len().checked_add(body_end.saturating_sub(range.start))
            .and_then(|length| length.checked_add(suffix.len()))
            .filter(|&length| length <= limit)
            .ok_or("Copy Bookmarked Lines exceeds the 128 MiB clipboard limit")?;
        if required > output.capacity() {
            let capacity = required.max(output.capacity().saturating_mul(2)).min(limit);
            output.try_reserve_exact(capacity.saturating_sub(output.len()))
                .map_err(|_| "Not enough memory to copy bookmarked lines")?;
        }
        let slice = rope.get_byte_slice(range.start..body_end)
            .ok_or("A bookmarked line is no longer available")?;
        for chunk in slice.chunks() {
            if cancel.load(Ordering::Relaxed) { return Err("Copy Bookmarked Lines cancelled"); }
            output.push_str(chunk);
        }
        output.push_str(suffix);
    }
    if cancel.load(Ordering::Relaxed) { return Err("Copy Bookmarked Lines cancelled"); }
    Ok(output)
}

impl Workspace {
    pub(crate) fn copy_bookmarked_lines(&mut self, _: &CopyBookmarkedLines, window: &mut Window, cx: &mut Context<Self>) {
        self.run_copy_bookmarked_lines(window, cx);
    }

    pub(crate) fn cancel_bookmark_copy(&mut self, cx: &mut Context<Self>) {
        self.pending_bookmark_copy = None;
        if self.status_message.as_deref() == Some(PREPARING) {
            self.status_message = None;
            cx.notify();
        }
    }

    pub(crate) fn run_copy_bookmarked_lines(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.cancel_bookmark_copy(cx);
        let Some(tab) = self.tabs.get(self.active) else { return false; };
        let refusal = if self.window_closed || tab.view_mode != TabViewMode::Edit {
            Some("Switch to Source or Raw View to copy bookmarked lines")
        } else if !window.is_window_active() || !self.focus_handle.is_focused(window)
            || self.input_focus(window) != InputFocus::Editor
        {
            Some("Focus the active Source or Raw View editor to copy bookmarked lines")
        } else if tab.buffer.loading.is_some() {
            Some("Wait for the file to finish loading before copying bookmarked lines")
        } else if self.is_composing_text(window) {
            Some("Finish composing text before copying bookmarked lines")
        } else if tab.buffer.bookmarks.len() > MAX_BOOKMARKS {
            Some("Too many bookmarks to copy; the limit is 1,000,000")
        } else { None };
        if let Some(message) = refusal {
            self.set_status(message); cx.notify(); return false;
        }
        let background = !tab.buffer.bookmarks.is_empty()
            && (tab.buffer.rope.len_bytes() > FOREGROUND_BYTES || tab.buffer.bookmarks.len() > FOREGROUND_BOOKMARKS);
        if background && macros::execution_active() {
            self.set_status("Large Copy Bookmarked Lines is unavailable during macro recording or playback");
            cx.notify(); return false;
        }
        let mut bookmarks = Vec::new();
        if bookmarks.try_reserve_exact(tab.buffer.bookmarks.len()).is_err() {
            self.set_status("Not enough memory to copy bookmarked lines"); cx.notify(); return false;
        }
        bookmarks.extend(tab.buffer.bookmarks.iter().copied());
        let snapshot = CopySnapshot { tab_id: tab.session_id, source_epoch: self.source_input_token(),
            input_context: window.input_context_token(), instance: tab.instance.clone(),
            revision: tab.buffer.raw_revision.clone(), activity: tab.content_activity.clone(),
            ending: tab.buffer.line_ending, bookmarks: Arc::new(bookmarks) };
        let rope = tab.buffer.rope.clone();
        if !background {
            match bookmarked_text(&rope, &snapshot.bookmarks, snapshot.ending, MAX_COPY_BYTES, &AtomicBool::new(false)) {
                Ok(text) => {
                    crate::clipboard_history::record_copy(&text);
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    macros::record(MacroEvent::CopyBookmarkedLines);
                    return true;
                }
                Err(message) => { self.set_status(message); cx.notify(); return false; }
            }
        }
        let id = Uuid::new_v4();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let bookmarks = snapshot.bookmarks.clone();
        let ending = snapshot.ending;
        #[cfg(test)]
        let gate = self.bookmark_copy_test_gate.take();
        let work = cx.background_spawn(async move {
            #[cfg(test)]
            if let Some(gate) = gate { let _ = gate.await; }
            #[cfg(test)]
            let started = std::time::Instant::now();
            let result = bookmarked_text(&rope, &bookmarks, ending, MAX_COPY_BYTES, &worker_cancel);
            #[cfg(test)]
            if rope.len_bytes() > 10 * 1024 * 1024 {
                eprintln!("bookmark copy worker: source={} bytes, elapsed={:?}", rope.len_bytes(), started.elapsed());
            }
            result
        });
        self.pending_bookmark_copy = Some(PendingBookmarkCopy { id, cancel });
        self.set_status(PREPARING); cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let updated = this.update_in(cx, |w, window, cx| {
                if !w.pending_bookmark_copy.as_ref().is_some_and(|pending| pending.id == id) { return; }
                let preparing = w.status_message.as_deref() == Some(PREPARING);
                w.cancel_bookmark_copy(cx);
                if !snapshot.valid(w, window) {
                    if preparing {
                        w.set_status("Copy Bookmarked Lines cancelled because the editor changed; clipboard unchanged");
                        cx.notify();
                    }
                    return;
                }
                match result {
                    Ok(text) => {
                        crate::clipboard_history::record_copy(&text);
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        w.set_status("Copied bookmarked lines");
                    }
                    Err(message) => w.set_status(message),
                }
                cx.notify();
            });
            if updated.is_err() {
                let _ = this.update(cx, |w, cx| {
                    if w.pending_bookmark_copy.as_ref().is_some_and(|pending| pending.id == id) { w.cancel_bookmark_copy(cx); }
                });
            }
        }).detach();
        true
    }
}

#[cfg(test)]
#[path = "bookmark_clipboard_tests.rs"]
pub(crate) mod tests;
