use std::collections::BTreeSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui::{Bounds, Pixels, ShapedLine, SharedString};

/// Compute the smallest unused "new N" number from the labels currently in
/// use. Walks `existing_labels` (typically every open tab's `new_label`),
/// pulls out integers from `"new N"` strings, and returns the lowest
/// positive integer not in that set. Notepad++-style — closing `new 3`
/// then opening a new tab while `new 1`, `new 2`, `new 4` are open yields
/// `new 3`.
pub(crate) fn next_new_label_for<'a, I>(existing_labels: I) -> SharedString
where
    I: IntoIterator<Item = Option<&'a SharedString>>,
{
    let used: BTreeSet<usize> = existing_labels
        .into_iter()
        .filter_map(|label| {
            label
                .and_then(|s| s.strip_prefix("new "))
                .and_then(|rest| rest.parse::<usize>().ok())
        })
        .collect();
    let n = (1..).find(|n| !used.contains(n)).unwrap_or(1);
    SharedString::from(format!("new {n}"))
}

use crate::syntax::{HighlightSpan, LanguageId};
use crate::text::encoding::Encoding;
use crate::text::line_ending::LineEnding;

/// Live state of an in-flight chunked load (files > `STREAM_THRESHOLD_BYTES`).
/// Present only while bytes are still being read from disk; cleared the moment
/// the streaming task finishes (or is cancelled by the user from the status
/// bar).
pub(crate) struct LoadProgress {
    pub(crate) bytes_read: u64,
    pub(crate) total_bytes: u64,
    /// Shared cancel flag — flipped by the status-bar Cancel button. The
    /// streaming task checks it between every chunk read and exits early.
    pub(crate) cancel: Arc<AtomicBool>,
}

pub(crate) struct Buffer {
    pub(crate) rope: ropey::Rope,
    pub(crate) file_path: Option<PathBuf>,
    pub(crate) dirty: bool,
    pub(crate) line_ending: LineEnding,
    /// Per-buffer text encoding. Drives both the on-disk byte format at save
    /// time and the encoding chip on the status bar. New / scratch buffers
    /// default to UTF-8; loaded buffers inherit whatever `decode` sniffed.
    pub(crate) encoding: Encoding,
    /// Byte offsets of bookmarked line starts. Each bookmark anchors at the
    /// start byte of its line; `apply_edit` shifts these offsets to track
    /// rope mutations and `normalize_bookmarks` snaps them back to the
    /// containing line's start. Per-buffer, in-memory only.
    pub(crate) bookmarks: BTreeSet<usize>,
    /// Persistent byte ranges painted under the editor text in the
    /// `theme.mark_highlight` colour. Populated by the Mark tab's "Mark All"
    /// button; cleared by "Clear all marks". Per-buffer so switching tabs
    /// doesn't bleed marks from one buffer's content onto another's. Edits
    /// shift these offsets in `apply_edit`, the same way bookmarks track.
    pub(crate) marks: Vec<Range<usize>>,
    /// Detected language (by file extension on load). `None` means plain
    /// text — no parsing, no highlights.
    pub(crate) language: Option<LanguageId>,
    /// Cached tree-sitter highlight spans, sorted by `range.start`.
    /// Refreshed inside `apply_edit` after every rope mutation.
    pub(crate) highlights: Vec<HighlightSpan>,
    /// Streaming-load progress for the current file. `Some` while a large
    /// file is still reading in chunks off the bg executor; cleared when the
    /// load completes or the user cancels. Edits / Save / Find behaviour
    /// continues to work on the partial rope while this is set — the UI
    /// just shows progress in the status bar instead of the usual chips.
    pub(crate) loading: Option<LoadProgress>,
    /// Per-buffer display label for untitled / scratch buffers. `Some("new 1")`
    /// for the first scratch buffer in a session. Cleared once `file_path`
    /// is set via Save As. Allocated by `next_new_label_for` against the
    /// workspace's current open tabs so a freshly created tab fills the
    /// lowest-unused slot. Persisted in the session manifest so restored
    /// untitled tabs keep their original label across launches.
    pub(crate) new_label: Option<SharedString>,
    /// Notepad++ "Read-Only" flag. When `true`, every edit op short-circuits
    /// at `Workspace::replace_byte_range` and the status bar reports
    /// "Buffer is read-only". Set by the tab right-click menu; persisted in
    /// the session manifest. Independent of OS-level file permissions —
    /// "Clear Read-Only Flag" additionally chmods the file writable.
    pub(crate) read_only: bool,
    /// User-applied tab color (`Some(1..=5)`) → painted as a thin top stripe
    /// on the tab strip. NPP-canonical palette: 1 yellow, 2 green, 3 blue,
    /// 4 orange, 5 pink. `None` = no stripe.
    pub(crate) tab_color: Option<u8>,
    /// True when this tab is the auto-opened `change.log`. Excludes the tab
    /// from session persistence so the next launch's auto-open logic owns
    /// it, instead of restoring a stale copy.
    pub(crate) is_changelog: bool,
    /// Set when the buffer was loaded from a rich-document format (DOCX, etc.).
    /// When `Some`, `rope` holds the Raw View text (extracted document.xml) and
    /// `rich_doc.ast` is the source of truth for Doc View rendering.
    pub(crate) rich_doc: Option<crate::rich_doc::RichDocPayload>,
    /// Lazy `rope.to_string()` cache. Find-as-you-type calls `refresh_matches`
    /// on every keystroke in the find box; without this each call re-flattened
    /// the entire rope (allocate + copy = O(n) on every char) which made search
    /// crawl on multi-MB files. The cache is built on first read after an edit
    /// and reused until the next rope mutation invalidates it via
    /// `invalidate_flat_text`.
    pub(crate) flat_text_cache: Option<String>,
}

impl Buffer {
    pub(crate) fn untitled() -> Self {
        let placeholder = "// Welcome to Notepad Studio\n\
                           //\n\
                           // A fast, lightweight Notepad++-style editor for macOS / Linux / Windows.\n\
                           //\n\
                           // Files & tabs\n\
                           //   * Cmd/Ctrl+N        new tab\n\
                           //   * Cmd/Ctrl+O        open file\n\
                           //   * Cmd/Ctrl+S        save     /   Cmd/Ctrl+Shift+S  save as\n\
                           //   * Cmd/Ctrl+W        close tab\n\
                           //   * Cmd/Ctrl+Shift+T  reopen last closed tab\n\
                           //   * Ctrl+Tab          cycle tabs (MRU)\n\
                           //   * Cmd/Ctrl+1..9     jump to tab N\n\
                           //   * File ▸ Open Folder…   workspace sidebar\n\
                           //\n\
                           // Find, replace, navigate\n\
                           //   * Cmd/Ctrl+F            find bar (Esc closes)\n\
                           //   * F3  /  Shift+F3       next / previous match\n\
                           //   * Aa  /  W  /  .*       match case, whole word, regex\n\
                           //   * Ctrl+H                replace\n\
                           //   * Cmd/Ctrl+G            go to line\n\
                           //   * Cmd/Ctrl+Shift+F      find in files\n\
                           //\n\
                           // Editing\n\
                           //   * Cmd/Ctrl+D            multi-cursor: select next match\n\
                           //   * Alt+Click             add cursor\n\
                           //   * Cmd/Ctrl+/            toggle line comment (24 langs)\n\
                           //   * Cmd/Ctrl+Z  /  Y      undo / redo\n\
                           //\n\
                           // View\n\
                           //   * F11                   full screen\n\
                           //   * Cmd/Ctrl+Shift+T      toggle theme\n\
                           //   * View menu             word wrap, whitespace, bracket match\n\
                           //\n\
                           // Tools\n\
                           //   * Edit ▸ Text Tools     prefix/suffix, dedup, columns, sort, …\n\
                           //   * Edit ▸ Preferences    fonts, indent, EOL, encoding, themes\n\
                           //   * Macro ▸ Record / Replay\n\
                           //   * Right-click a tab     close, save as, reveal, color, rename, …\n\
                           //\n\
                           // See change.log for the full release history.\n";
        Self {
            rope: ropey::Rope::from_str(placeholder),
            file_path: None,
            dirty: false,
            line_ending: crate::settings::resolved_default_line_ending(),
            encoding: crate::settings::resolved_default_encoding(),
            bookmarks: BTreeSet::new(),
            marks: Vec::new(),
            language: None,
            highlights: Vec::new(),
            loading: None,
            new_label: None,
            read_only: false,
            tab_color: None,
            is_changelog: false,
            rich_doc: None,
            flat_text_cache: None,
        }
    }

    /// Clone enough of a buffer to spawn a sibling tab in another window.
    /// Drops in-flight `loading` progress — the shared `Arc<AtomicBool>` cancel
    /// flag would otherwise tie the two windows' status-bar Cancel buttons
    /// together. Caller is expected to refuse the transfer when `loading`
    /// is still `Some`, or accept that the new copy starts loaded-to-current.
    pub(crate) fn clone_for_transfer(&self) -> Self {
        Self {
            rope: self.rope.clone(),
            file_path: self.file_path.clone(),
            dirty: self.dirty,
            line_ending: self.line_ending,
            encoding: self.encoding,
            bookmarks: self.bookmarks.clone(),
            marks: self.marks.clone(),
            language: self.language,
            highlights: self.highlights.clone(),
            loading: None,
            new_label: self.new_label.clone(),
            read_only: self.read_only,
            tab_color: self.tab_color,
            is_changelog: self.is_changelog,
            rich_doc: self.rich_doc.clone(),
            flat_text_cache: self.flat_text_cache.clone(),
        }
    }

    /// A truly empty buffer for tabs created after the first.
    pub(crate) fn empty() -> Self {
        Self {
            rope: ropey::Rope::new(),
            file_path: None,
            dirty: false,
            line_ending: crate::settings::resolved_default_line_ending(),
            encoding: crate::settings::resolved_default_encoding(),
            bookmarks: BTreeSet::new(),
            marks: Vec::new(),
            language: crate::settings::snapshot().default_language,
            highlights: Vec::new(),
            loading: None,
            new_label: None,
            read_only: false,
            tab_color: None,
            is_changelog: false,
            rich_doc: None,
            flat_text_cache: None,
        }
    }

    /// Lazily build (or reuse) the flat `String` snapshot of `rope` used by
    /// search / highlight scans. The cache is dropped by `invalidate_flat_text`
    /// on every rope mutation (see `Workspace::apply_edit`, the streaming-load
    /// chunk inserts, and the session-restore rope swap) so it never goes
    /// stale. Repeated calls between edits are O(1) and reuse the same `String`
    /// allocation — the cost of `rope.to_string()` is paid once per edit
    /// instead of once per find-as-you-type keystroke.
    pub(crate) fn flat_text(&mut self) -> &str {
        let rope = &self.rope;
        self.flat_text_cache.get_or_insert_with(|| rope.to_string())
    }

    /// Drop the cached flat text. Call this whenever `rope` is mutated so the
    /// next `flat_text()` rebuilds against the new content.
    pub(crate) fn invalidate_flat_text(&mut self) {
        self.flat_text_cache = None;
    }

    /// Re-run the tree-sitter highlighter against the rope contents and
    /// store the resulting spans. Cheap no-op when `language` is `None`.
    /// Called from `Workspace::apply_edit` after every rope mutation and
    /// once on file load.
    pub(crate) fn reparse_highlights(&mut self) {
        let Some(lang) = self.language else {
            self.highlights.clear();
            return;
        };
        let source = self.rope.to_string();
        let snap = crate::settings::snapshot();
        if crate::syntax::should_skip_syntax(source.len() as u64, &snap) {
            self.highlights.clear();
            return;
        }
        self.highlights = crate::syntax::highlight(lang, &source);
    }

    pub(crate) fn display_short_name(&self) -> SharedString {
        if let Some(p) = self.file_path.as_ref() {
            if let Some(n) = p.file_name() {
                return n.to_string_lossy().into_owned().into();
            }
        }
        if crate::settings::snapshot().untitled_uses_first_line {
            if let Some(line) = first_nonempty_line(&self.rope) {
                return line.into();
            }
        }
        self.new_label
            .clone()
            .unwrap_or_else(|| SharedString::from("new"))
    }

    /// Absolute path string for the OS title bar. Returns the file's full
    /// path when saved, otherwise the "new N" label.
    pub(crate) fn title_bar_label(&self) -> String {
        if let Some(p) = self.file_path.as_ref() {
            return p.display().to_string();
        }
        if crate::settings::snapshot().untitled_uses_first_line {
            if let Some(line) = first_nonempty_line(&self.rope) {
                return line;
            }
        }
        self.new_label
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "new".to_string())
    }

    #[allow(dead_code)]
    pub(crate) fn line_ending_label(&self) -> &'static str {
        self.line_ending.label()
    }

    /// Notepad++-style label shown in the status bar EOL chip.
    pub(crate) fn line_ending_status_label(&self) -> &'static str {
        self.line_ending.status_label()
    }

    /// Returns `Some(format)` when this buffer was loaded from a rich-document format.
    pub(crate) fn rich_doc_format(&self) -> Option<crate::rich_doc::DocFormat> {
        self.rich_doc.as_ref().map(|p| p.format)
    }

    pub(crate) fn is_rich_doc(&self) -> bool {
        self.rich_doc.is_some()
    }

    /// Text in `range`, clamped to the rope and with both ends floored to a
    /// char boundary (ropey's `byte_slice` panics on a mid-char offset).
    pub(crate) fn slice_bytes(&self, range: Range<usize>) -> String {
        let range = char_boundary_range(&self.rope, range);
        if range.is_empty() {
            return String::new();
        }
        self.rope.byte_slice(range).to_string()
    }

    // ---- Bookmarks ----------------------------------------------------------------

    /// Toggle a bookmark on the line containing `byte`. Stored as the byte
    /// offset of that line's start so it survives rope mutations via the
    /// shift-and-normalize hook in `apply_edit`.
    pub(crate) fn toggle_bookmark_at_byte(&mut self, byte: usize) {
        let total = self.rope.len_bytes();
        let clamped = byte.min(total);
        let line = self.rope.byte_to_line(clamped);
        let line_start = self.rope.line_to_byte(line);
        if !self.bookmarks.remove(&line_start) {
            self.bookmarks.insert(line_start);
        }
    }

    pub(crate) fn clear_bookmarks(&mut self) {
        self.bookmarks.clear();
    }

    /// Returns the line index (0-based) of every bookmark, in document order.
    pub(crate) fn bookmark_lines(&self) -> Vec<usize> {
        let total = self.rope.len_bytes();
        self.bookmarks
            .iter()
            .map(|b| self.rope.byte_to_line((*b).min(total)))
            .collect()
    }

    /// True if line `line` (0-based) has a bookmark.
    pub(crate) fn is_line_bookmarked(&self, line: usize) -> bool {
        let total_lines = self.rope.len_lines();
        if line >= total_lines.max(1) {
            return false;
        }
        let line_start = self.rope.line_to_byte(line);
        self.bookmarks.contains(&line_start)
    }

    /// Next bookmarked line strictly after `from`, wrapping to the first
    /// bookmark if there is none after.
    pub(crate) fn next_bookmark_line_after(&self, from: usize) -> Option<usize> {
        let lines = self.bookmark_lines();
        if lines.is_empty() {
            return None;
        }
        lines
            .iter()
            .find(|&&l| l > from)
            .copied()
            .or_else(|| lines.first().copied())
    }

    /// Previous bookmarked line strictly before `from`, wrapping to the last
    /// bookmark if there is none before.
    pub(crate) fn prev_bookmark_line_before(&self, from: usize) -> Option<usize> {
        let lines = self.bookmark_lines();
        if lines.is_empty() {
            return None;
        }
        lines
            .iter()
            .rev()
            .find(|&&l| l < from)
            .copied()
            .or_else(|| lines.last().copied())
    }

    /// Translate bookmark byte offsets to track an edit at `[start, end)`
    /// being replaced by `new_len` bytes. Bookmarks before `start` are
    /// untouched; bookmarks at or past `end` shift by the size delta;
    /// bookmarks strictly inside the deleted region are dropped.
    pub(crate) fn shift_bookmarks_for_edit(
        &mut self,
        start: usize,
        end: usize,
        new_len: usize,
    ) {
        if self.bookmarks.is_empty() {
            return;
        }
        let removed_len = end.saturating_sub(start);
        if removed_len == 0 && new_len == 0 {
            return;
        }
        let delta: isize = new_len as isize - removed_len as isize;
        let shifted: BTreeSet<usize> = self
            .bookmarks
            .iter()
            .filter_map(|&b| {
                if b <= start {
                    Some(b)
                } else if b >= end {
                    let signed = b as isize + delta;
                    if signed < 0 { None } else { Some(signed as usize) }
                } else {
                    // Bookmark sits strictly inside the deleted region.
                    None
                }
            })
            .collect();
        self.bookmarks = shifted;
    }

    // ---- Marks (Mark tab persistent highlights) ----------------------------------

    /// Replace every mark with `ranges`. Used by "Mark All".
    pub(crate) fn set_marks(&mut self, ranges: Vec<Range<usize>>) {
        self.marks = ranges;
    }

    /// Append `range` to the mark layer, preserving existing marks. Used when
    /// "Purge for each search" is off so successive Mark All runs accumulate.
    pub(crate) fn add_mark(&mut self, range: Range<usize>) {
        if range.start >= range.end {
            return;
        }
        // Skip exact duplicates — re-marking the same span shouldn't bloat
        // the layer.
        if self.marks.iter().any(|r| r.start == range.start && r.end == range.end) {
            return;
        }
        self.marks.push(range);
    }

    pub(crate) fn clear_marks(&mut self) {
        self.marks.clear();
    }

    /// Translate mark ranges across an edit at `[start, end)` being replaced
    /// by `new_len` bytes — same shape as `shift_bookmarks_for_edit`. Marks
    /// that overlap the edited region are dropped (their underlying text just
    /// changed); marks past the edit shift by the size delta.
    pub(crate) fn shift_marks_for_edit(&mut self, start: usize, end: usize, new_len: usize) {
        if self.marks.is_empty() {
            return;
        }
        let removed_len = end.saturating_sub(start);
        if removed_len == 0 && new_len == 0 {
            return;
        }
        let delta: isize = new_len as isize - removed_len as isize;
        self.marks.retain(|r| {
            // Drop any mark that overlaps the edited region — its content is
            // gone or transformed, so the highlight is stale.
            r.end <= start || r.start >= end
        });
        for r in self.marks.iter_mut() {
            if r.start >= end {
                let s = r.start as isize + delta;
                let e = r.end as isize + delta;
                r.start = if s < 0 { 0 } else { s as usize };
                r.end = if e < 0 { 0 } else { e as usize };
            }
        }
        // Clamp tail-end of the buffer in case the rope shrank below a stored
        // offset — defensive, retain above already drops most of these.
        let total = self.rope.len_bytes();
        self.marks
            .retain(|r| r.start < total && r.end <= total && r.start < r.end);
    }

    /// Snap every bookmark byte offset to the start of its current line.
    /// Called after `shift_bookmarks_for_edit` so subsequent
    /// `is_line_bookmarked` lookups can use exact-equality on a line start.
    pub(crate) fn normalize_bookmarks(&mut self) {
        if self.bookmarks.is_empty() {
            return;
        }
        let total_bytes = self.rope.len_bytes();
        let normalized: BTreeSet<usize> = self
            .bookmarks
            .iter()
            .map(|&b| {
                let clamped = b.min(total_bytes);
                let line = self.rope.byte_to_line(clamped);
                self.rope.line_to_byte(line)
            })
            .collect();
        self.bookmarks = normalized;
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Selection {
    pub(crate) anchor: usize,
    pub(crate) head: usize,
    pub(crate) is_dragging: bool,
}

impl Selection {
    pub(crate) fn ordered(&self) -> Range<usize> {
        if self.anchor <= self.head {
            self.anchor..self.head
        } else {
            self.head..self.anchor
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub(crate) fn collapse_to(&mut self, byte_offset: usize) {
        self.anchor = byte_offset;
        self.head = byte_offset;
        self.is_dragging = false;
    }

    /// This selection with both ends clamped to `rope` and floored to a
    /// char boundary. A selection restored from undo history, a session file
    /// or a plugin call can point past the end of a shorter document.
    pub(crate) fn clamped(self, rope: &ropey::Rope) -> Self {
        Self {
            anchor: floor_char_boundary(rope, self.anchor),
            head: floor_char_boundary(rope, self.head),
            is_dragging: self.is_dragging,
        }
    }
}

/// Largest char boundary at or below `byte`, after clamping `byte` to the
/// rope's length. Offsets from files, sessions, plugins or stale state can
/// point past the end or into a multi-byte UTF-8 sequence; ropey panics on
/// both in `byte_slice`, `char_to_byte` and friends.
pub(crate) fn floor_char_boundary(rope: &ropey::Rope, byte: usize) -> usize {
    let byte = byte.min(rope.len_bytes());
    // `byte_to_char` maps a mid-char byte to the char containing it.
    rope.char_to_byte(rope.byte_to_char(byte))
}

/// `range` clamped to the rope with both ends floored to char boundaries;
/// a reversed range collapses to an empty one at its start.
pub(crate) fn char_boundary_range(rope: &ropey::Rope, range: Range<usize>) -> Range<usize> {
    let start = floor_char_boundary(rope, range.start);
    let end = floor_char_boundary(rope, range.end).max(start);
    start..end
}

/// Notepad++ column-mode (block / rectangular) selection. Coordinates are
/// in (logical_line, char_col) — char column lets a UTF-8 file with
/// multi-byte chars still index visually. Lives on `Tab` alongside the
/// linear `Selection`; when `Some`, paint and edit funnels switch to the
/// rectangular path.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BlockSelection {
    pub(crate) anchor_line: usize,
    pub(crate) anchor_col: usize,
    pub(crate) head_line: usize,
    pub(crate) head_col: usize,
    pub(crate) is_dragging: bool,
}

impl BlockSelection {
    pub(crate) fn min_line(&self) -> usize {
        self.anchor_line.min(self.head_line)
    }
    pub(crate) fn max_line(&self) -> usize {
        self.anchor_line.max(self.head_line)
    }
    pub(crate) fn min_col(&self) -> usize {
        self.anchor_col.min(self.head_col)
    }
    pub(crate) fn max_col(&self) -> usize {
        self.anchor_col.max(self.head_col)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.min_line() == self.max_line() && self.min_col() == self.max_col()
    }
}

#[derive(Clone)]
pub(crate) struct LineGeo {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) line_byte_start: usize,
    pub(crate) line_byte_len: usize,
    pub(crate) shaped: ShapedLine,
}

#[derive(Clone)]
pub(crate) struct EditOp {
    pub(crate) byte_start: usize,
    pub(crate) removed: String,
    pub(crate) inserted: String,
    pub(crate) sel_before_anchor: usize,
    pub(crate) sel_before_head: usize,
}

/// One slot on the per-tab undo / redo stack. Most edits are `Single` — a
/// single keystroke / paste / replace pushes one `EditOp`. Multi-edit
/// operations like "Replace All" wrap their per-match edits in `Group` so a
/// single Undo press reverses every replacement and restores the caret to
/// where it sat before the operation began.
#[derive(Clone)]
pub(crate) enum UndoEntry {
    Single(EditOp),
    /// Ordered list of the individual edits that made up the group, in
    /// application order. `sel_before_anchor` / `sel_before_head` snapshot
    /// the caret right before the group opened so undo restores it there.
    Group {
        ops: Vec<EditOp>,
        sel_before_anchor: usize,
        sel_before_head: usize,
    },
}

/// State of an open `begin_edit_group` / `end_edit_group` transaction. Lives
/// on the active `Tab` while a transaction is open. While `Some`, every
/// `replace_byte_range` call appends its `EditOp` to `ops` instead of
/// pushing a fresh `UndoEntry::Single` to `undo_stack`. `end_edit_group`
/// consumes this and emits one `UndoEntry::Group`.
pub(crate) struct EditTransaction {
    pub(crate) ops: Vec<EditOp>,
    pub(crate) sel_before_anchor: usize,
    pub(crate) sel_before_head: usize,
}

// ---- Undo-stack persistence codec ----------------------------------------
//
// Hand-rolled little-endian binary format so the per-tab undo stack can be
// written alongside the `.bak` shadow file and survive app restarts. Format
// stays compact (no JSON / TOML overhead per keystroke) and dependency-free.
//
//   File layout:
//     magic        4 bytes "NSU\x01"
//     version      u32     = 1
//     n_entries    u32
//     entries      [Entry; n_entries]
//
//   Entry:
//     tag          u8      0 = Single(EditOp), 1 = Group { … }
//     tag=0:       EditOp
//     tag=1:       u64 sel_before_anchor, u64 sel_before_head, u32 n_ops, [EditOp; n_ops]
//
//   EditOp:
//     u64 byte_start
//     u32 removed_len, [u8; removed_len]
//     u32 inserted_len, [u8; inserted_len]
//     u64 sel_before_anchor
//     u64 sel_before_head
//
// Cap on serialize keeps the file bounded — the oldest entries (front of the
// vec) drop first so the user keeps recent undo across restart even on a
// long-typed buffer.

const UNDO_FILE_MAGIC: &[u8; 4] = b"NSU\x01";
const UNDO_FILE_VERSION: u32 = 1;
/// Hard cap on entries written to disk. In-memory stack stays unbounded; only
/// the persisted copy gets the haircut so a buffer with 50k keystrokes still
/// undoes to within ~2k operations after restart.
pub(crate) const UNDO_PERSIST_CAP: usize = 2000;

fn write_u32_le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u64_le(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_bytes_with_len(out: &mut Vec<u8>, bytes: &[u8]) {
    write_u32_le(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}

fn write_edit_op(out: &mut Vec<u8>, op: &EditOp) {
    write_u64_le(out, op.byte_start as u64);
    write_bytes_with_len(out, op.removed.as_bytes());
    write_bytes_with_len(out, op.inserted.as_bytes());
    write_u64_le(out, op.sel_before_anchor as u64);
    write_u64_le(out, op.sel_before_head as u64);
}

/// Serialize an undo stack to the on-disk binary format. Returns an empty
/// `Vec` when the stack is empty so callers can shortcut to `delete_undo`.
pub(crate) fn serialize_undo_stack(stack: &[UndoEntry]) -> Vec<u8> {
    if stack.is_empty() {
        return Vec::new();
    }
    let start = stack.len().saturating_sub(UNDO_PERSIST_CAP);
    let slice = &stack[start..];

    let mut out = Vec::with_capacity(16 + slice.len() * 64);
    out.extend_from_slice(UNDO_FILE_MAGIC);
    write_u32_le(&mut out, UNDO_FILE_VERSION);
    write_u32_le(&mut out, slice.len() as u32);
    for entry in slice {
        match entry {
            UndoEntry::Single(op) => {
                out.push(0u8);
                write_edit_op(&mut out, op);
            }
            UndoEntry::Group {
                ops,
                sel_before_anchor,
                sel_before_head,
            } => {
                out.push(1u8);
                write_u64_le(&mut out, *sel_before_anchor as u64);
                write_u64_le(&mut out, *sel_before_head as u64);
                write_u32_le(&mut out, ops.len() as u32);
                for op in ops {
                    write_edit_op(&mut out, op);
                }
            }
        }
    }
    out
}

struct UndoReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> UndoReader<'a> {
    fn read_u32(&mut self) -> Option<u32> {
        let end = self.pos.checked_add(4)?;
        if end > self.bytes.len() {
            return None;
        }
        let v = u32::from_le_bytes(self.bytes[self.pos..end].try_into().ok()?);
        self.pos = end;
        Some(v)
    }

    fn read_u64(&mut self) -> Option<u64> {
        let end = self.pos.checked_add(8)?;
        if end > self.bytes.len() {
            return None;
        }
        let v = u64::from_le_bytes(self.bytes[self.pos..end].try_into().ok()?);
        self.pos = end;
        Some(v)
    }

    fn read_u8(&mut self) -> Option<u8> {
        let v = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }

    fn read_string(&mut self) -> Option<String> {
        let len = self.read_u32()? as usize;
        let end = self.pos.checked_add(len)?;
        if end > self.bytes.len() {
            return None;
        }
        let s = std::str::from_utf8(&self.bytes[self.pos..end]).ok()?.to_owned();
        self.pos = end;
        Some(s)
    }

    fn read_edit_op(&mut self) -> Option<EditOp> {
        let byte_start = self.read_u64()? as usize;
        let removed = self.read_string()?;
        let inserted = self.read_string()?;
        let sel_before_anchor = self.read_u64()? as usize;
        let sel_before_head = self.read_u64()? as usize;
        Some(EditOp {
            byte_start,
            removed,
            inserted,
            sel_before_anchor,
            sel_before_head,
        })
    }
}

/// Best-effort deserialize. Returns an empty `Vec` on any malformed input —
/// losing restored undo is preferable to refusing to open the tab.
pub(crate) fn deserialize_undo_stack(bytes: &[u8]) -> Vec<UndoEntry> {
    if bytes.len() < 12 || &bytes[..4] != UNDO_FILE_MAGIC {
        return Vec::new();
    }
    let mut r = UndoReader {
        bytes,
        pos: 4,
    };
    let Some(version) = r.read_u32() else {
        return Vec::new();
    };
    if version != UNDO_FILE_VERSION {
        return Vec::new();
    }
    let Some(n) = r.read_u32() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let Some(tag) = r.read_u8() else {
            return out;
        };
        match tag {
            0 => {
                let Some(op) = r.read_edit_op() else {
                    return out;
                };
                out.push(UndoEntry::Single(op));
            }
            1 => {
                let Some(sel_before_anchor) = r.read_u64() else {
                    return out;
                };
                let Some(sel_before_head) = r.read_u64() else {
                    return out;
                };
                let Some(n_ops) = r.read_u32() else {
                    return out;
                };
                let mut ops = Vec::with_capacity(n_ops as usize);
                for _ in 0..n_ops {
                    let Some(op) = r.read_edit_op() else {
                        return out;
                    };
                    ops.push(op);
                }
                out.push(UndoEntry::Group {
                    ops,
                    sel_before_anchor: sel_before_anchor as usize,
                    sel_before_head: sel_before_head as usize,
                });
            }
            _ => return out,
        }
    }
    out
}

/// First non-empty line of the rope, trimmed and truncated to 30 chars
/// NPP-style. Returns `None` if the rope has no non-empty line. Used by
/// `display_short_name` / `title_bar_label` when
/// `Settings.untitled_uses_first_line` is true.
fn first_nonempty_line(rope: &ropey::Rope) -> Option<String> {
    const MAX_CHARS: usize = 30;
    let total = rope.len_lines();
    for i in 0..total {
        let line = rope.line(i);
        let s = line.to_string();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut out: String = trimmed.chars().take(MAX_CHARS).collect();
        if trimmed.chars().count() > MAX_CHARS {
            out.push('…');
        }
        return Some(out);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    /// "aé中😀" is 1 + 2 + 3 + 4 bytes: boundaries at 0, 1, 3, 6, 10.
    const MIXED: &str = "aé中😀";

    #[test]
    fn floor_char_boundary_snaps_into_range() {
        let rope = Rope::from_str(MIXED);
        let cases = [
            (0, 0), (1, 1), (2, 1), (3, 3), (4, 3), (5, 3), (6, 6),
            (7, 6), (9, 6), (10, 10), (11, 10), (usize::MAX, 10),
        ];
        for (byte, want) in cases {
            assert_eq!(floor_char_boundary(&rope, byte), want, "byte {byte}");
        }
        assert_eq!(floor_char_boundary(&Rope::new(), 5), 0);
    }

    #[test]
    fn slice_bytes_tolerates_mid_char_and_past_end_ranges() {
        let mut buf = Buffer::empty();
        buf.rope = Rope::from_str(MIXED);
        // Mid-char ends used to reach `byte_slice` and panic.
        assert_eq!(buf.slice_bytes(2..5), "é");
        assert_eq!(buf.slice_bytes(7..usize::MAX), "😀");
        assert_eq!(buf.slice_bytes(8..4), "");
        assert_eq!(buf.slice_bytes(50..60), "");
    }

    #[test]
    fn selection_clamped_to_shorter_document() {
        let rope = Rope::from_str("é\r\n");
        let sel = Selection { anchor: 1, head: 99, is_dragging: false }.clamped(&rope);
        assert_eq!((sel.anchor, sel.head), (0, 4));
    }

    #[test]
    fn bookmark_lines_ignores_offsets_past_the_end() {
        let mut buf = Buffer::empty();
        buf.rope = Rope::from_str("one\ntwo");
        buf.bookmarks.insert(4);
        // A bookmark that outlived a shrinking edit or came from an old
        // session used to panic in `byte_to_line`.
        buf.bookmarks.insert(1_000);
        assert_eq!(buf.bookmark_lines(), vec![1, 1]);
    }

}
