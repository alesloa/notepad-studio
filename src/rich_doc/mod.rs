//! Rich-document support (DOCX, RTF, ODT, .doc). Phases 1-4: DOCX + RTF + ODT + legacy .doc.
//!
//! `open(path)` is the cross-format entry point used by `io.rs`.
//! The parsed `Document` AST lives on `Buffer::rich_doc` (see `RichDocPayload`).

pub mod ast;
pub mod container;
pub mod doc;
pub mod docx;
pub mod edits;
pub mod import_html;
mod match_blocks;
pub mod odt;
pub mod rtf;

use std::path::Path;

/// Refuse recursive document input deeper than this before calling a parser.
pub(crate) const MAX_NESTING_DEPTH: usize = 256;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DocFormat {
    Docx,
    Rtf,
    Odt,
    Doc,
}

#[derive(Debug, thiserror::Error)]
pub enum RichDocError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("docx parse error: {0}")]
    Docx(String),
    #[error("rtf parse error: {0}")]
    Rtf(String),
    #[error("odt parse error: {0}")]
    Odt(String),
    #[error("doc parse error: {0}")]
    Doc(String),
    #[error("zip error: {0}")]
    Zip(String),
    #[error("save as required: this format must be saved via Save As .{suggested_ext}")]
    SaveAsRequired { suggested_ext: &'static str },
    #[error("unsupported extension")]
    Unsupported,
    #[error("saving edited documents with tables or images is not supported; the file has not been changed")]
    UnsupportedContent,
    #[error("raw source edits cannot be saved in this native format or combined with Doc View edits; undo the source edits or copy them into a new text tab. No file has been changed")]
    UnsupportedRawEdits,
}

pub type Result<T> = std::result::Result<T, RichDocError>;

fn ensure_text_only(doc: &ast::Document) -> Result<()> {
    if doc.blocks.iter().any(|block| matches!(block, ast::Block::Table { .. } | ast::Block::Image { .. })) {
        return Err(RichDocError::UnsupportedContent);
    }
    Ok(())
}

fn write_document_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::io::write_file_safely(path, bytes)
        .map_err(|err| RichDocError::Io(std::io::Error::other(err)))
}

/// Detect rich-doc format from path extension. Phases 1-4 recognise `.docx`, `.rtf`, `.odt`, `.doc`.
pub fn detect_format(path: &Path) -> Option<DocFormat> {
    match path.extension().and_then(|e| e.to_str())?.to_ascii_lowercase().as_str() {
        "docx" => Some(DocFormat::Docx),
        "rtf" => Some(DocFormat::Rtf),
        "odt" => Some(DocFormat::Odt),
        "doc" => Some(DocFormat::Doc),
        _ => None,
    }
}

/// Byte-precise position inside a single inline run.
///
/// Phase 2g — `byte` is a UTF-8 byte offset inside `ast.blocks[block]`'s `runs[run].text`.
/// Always sits on a char boundary; mutators clamp / round to the nearest boundary
/// before storing. `block` indexes `ast.blocks`; `run` indexes the inline runs of
/// that block's `Paragraph` / `Heading`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextPos {
    pub block: usize,
    pub run: usize,
    pub byte: usize,
}

/// Caret + selection state inside the rich-doc Doc View.
///
/// Single-block-only in Phase 2g.A: `anchor` and `cursor` always share the same
/// `block` index. `range_in_block` returns `None` if they diverge (a guard for
/// future cross-block work in Phase 2g.B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: TextPos,
    pub cursor: TextPos,
}

impl TextSelection {
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.cursor
    }

    /// Within a single block, returns ordered (run_lo, byte_lo, run_hi, byte_hi)
    /// where (run_lo, byte_lo) ≤ (run_hi, byte_hi) by run-then-byte ordering.
    /// Returns `None` when anchor and cursor sit in different blocks.
    pub fn range_in_block(&self) -> Option<(usize, usize, usize, usize)> {
        if self.anchor.block != self.cursor.block {
            return None;
        }
        let a = (self.anchor.run, self.anchor.byte);
        let c = (self.cursor.run, self.cursor.byte);
        let (lo, hi) = if a <= c { (a, c) } else { (c, a) };
        Some((lo.0, lo.1, hi.0, hi.1))
    }

    /// Cross-block-aware ordered range. Returns `(lo, hi)` where `lo <= hi`
    /// using (block, run, byte) ordering. Used by Phase 2g.B navigation /
    /// editing across paragraph boundaries.
    pub fn range_full(&self) -> (TextPos, TextPos) {
        let a = (self.anchor.block, self.anchor.run, self.anchor.byte);
        let c = (self.cursor.block, self.cursor.run, self.cursor.byte);
        if a <= c {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    /// True when anchor and cursor sit in the same block.
    pub fn is_single_block(&self) -> bool {
        self.anchor.block == self.cursor.block
    }
}

/// Full parsed payload stored on `Buffer::rich_doc`.
#[derive(Debug, Clone)]
pub struct RichDocPayload {
    pub(crate) revision: crate::buffer::ContentRevision,
    pub format: DocFormat,
    pub ast: ast::Document,
    /// Raw text shown in Raw View. For DOCX this is the extracted `word/document.xml`.
    pub raw_view_text: String,
    /// Original file bytes captured at open time. Used by Phase 2 save to preserve
    /// every part of the container we don't touch (media, headers/footers, styles,
    /// `[Content_Types].xml`, rels, etc.). Saving with no edits re-writes this
    /// byte-for-byte; saving with edits opens this as a zip, swaps in a freshly
    /// re-serialized `word/document.xml`, and writes back.
    pub original_bytes: std::sync::Arc<Vec<u8>>,
    /// Structured edits applied since open. Empty → save byte-perfect from
    /// `original_bytes`; non-empty → save re-serializes `word/document.xml`
    /// from `ast` and swaps it into the original zip container.
    pub edit_log: Vec<edits::Edit>,
    /// Block the user clicked into most recently — the target for toolbar
    /// formatting actions (Bold / Italic / Underline). `None` means the user
    /// hasn't clicked into any block yet; toolbar actions are no-ops in that
    /// state instead of silently mutating the wrong paragraph.
    pub active_block: Option<usize>,
    /// Caret + selection state inside the active block. `None` when nothing
    /// is selected / no caret is placed. Phase 2g.A keeps anchor.block ==
    /// cursor.block; Phase 2g.B allows cross-block.
    pub selection: Option<TextSelection>,
    /// True between a mouse-down inside the active editable block and the
    /// matching mouse-up. While set, the window-global `MouseMoveEvent`
    /// handler extends the cursor (anchor unchanged) so click-and-drag
    /// produces a selection. Cleared on every mouse-up regardless of phase.
    pub drag_active: bool,
    /// Phase 2g.E — anchor position armed by a blank-area mouse-down but not
    /// yet committed to a real drag. Pure click → release on a blank area
    /// keeps `pending_drag_anchor = Some(_)` until mouse-up clears it without
    /// the selection ever changing (so "click in nothing" stays a deselect-
    /// in-place). On the first `MouseMoveEvent` with the button held, the
    /// pane-root move handler promotes this anchor into a real drag (sets
    /// `selection = Some({anchor: pending, cursor: pending})`, `drag_active
    /// = true`, `active_block = pending.block`), so dragging from any blank
    /// area starts selecting from a sensible spot.
    pub pending_drag_anchor: Option<TextPos>,
    /// Document headers. Each entry is one header section's block sequence
    /// (DOCX has multiple if first/odd/even pages differ; ODT one per master
    /// page). Rendered out-of-flow at the bottom of Doc View. Read-only in
    /// Phase 5a — toolbar edits never target these and save preserves them
    /// verbatim via the container-replay path (separate zip entries).
    pub headers: Vec<Vec<ast::Block>>,
    /// Document footers. Same shape and lifecycle as `headers`.
    pub footers: Vec<Vec<ast::Block>>,
    /// Snapshot-based undo stack. Each entry captures the pre-action ast +
    /// selection + active_block + edit_log. apply_edit pushes a
    /// snapshot of the current state before mutating, then clears redo so
    /// any new edit forks the timeline. Capped at UNDO_STACK_LIMIT entries
    /// (oldest dropped on overflow) to bound memory on long sessions.
    pub undo_stack: Vec<UndoSnapshot>,
    /// Snapshot-based redo stack. Populated when undo runs (the popped
    /// undo snapshot's "from" state goes here so redo can restore it).
    /// Cleared by every fresh apply_edit so a redo never crosses a forked
    /// timeline.
    pub redo_stack: Vec<UndoSnapshot>,
    /// Transient outer edit group's starting log length. The first edit saves
    /// its pre-action state; later edits in the group reuse that history entry.
    pub(crate) edit_group_start: Option<usize>,
}

/// Snapshot of a `RichDocPayload`'s mutable state at one point in time —
/// what undo/redo restore. Stores the full ast (cheap to clone for typical
/// documents — Document derives Clone) plus the caret / selection / active
/// block / full edit log so a restore puts the user back exactly where
/// they were and save_replay can still decide "any edits?" via
/// `edit_log.is_empty()`.
#[derive(Debug, Clone)]
pub struct UndoSnapshot {
    pub(crate) revision: crate::buffer::ContentRevision,
    pub ast: ast::Document,
    pub selection: Option<TextSelection>,
    pub active_block: Option<usize>,
    pub edit_log: Vec<edits::Edit>,
}

/// Cap on undo + redo stack size. Bounds memory on long editing sessions —
/// most users never undo past the last few dozen edits, so 200 is generous.
pub const UNDO_STACK_LIMIT: usize = 200;

impl RichDocPayload {
    /// Apply an edit and record it on the log. Marks the payload as
    /// "needs container-replay save" implicitly via `edit_log.is_empty()`.
    /// Saves one pre-action snapshot for a group (or each standalone edit)
    /// and wipes redo so the edit forks any prior redo timeline.
    pub fn apply_edit(&mut self, edit: edits::Edit) {
        if self.edit_group_start.is_none_or(|start| self.edit_log.len() == start) {
            self.push_undo_snapshot();
            self.redo_stack.clear();
        }
        edits::apply(&edit, &mut self.ast);
        self.revision = crate::buffer::ContentRevision::default();
        self.edit_log.push(edit);
    }

    // PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp:2523-2525 —
    // Replace All is one Undo action; empty actions retain the Redo timeline.
    pub(crate) fn begin_edit_group(&mut self) -> Option<usize> {
        let previous = self.edit_group_start;
        if previous.is_none() {
            self.edit_group_start = Some(self.edit_log.len());
        }
        previous
    }

    pub(crate) fn end_edit_group(&mut self, previous: Option<usize>) {
        self.edit_group_start = previous;
    }

    /// Snapshot the current mutable state for undo. Internal — apply_edit
    /// calls this before mutating. Caps the stack at UNDO_STACK_LIMIT and
    /// drops the oldest entry on overflow so memory stays bounded.
    fn push_undo_snapshot(&mut self) {
        let snap = self.current_snapshot();
        if self.undo_stack.len() >= UNDO_STACK_LIMIT {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snap);
    }

    fn current_snapshot(&self) -> UndoSnapshot {
        UndoSnapshot {
            revision: self.revision.clone(),
            ast: self.ast.clone(),
            selection: self.selection,
            active_block: self.active_block,
            edit_log: self.edit_log.clone(),
        }
    }

    fn restore_snapshot(&mut self, snap: UndoSnapshot) {
        self.revision = snap.revision;
        self.ast = snap.ast;
        self.selection = snap.selection;
        self.active_block = snap.active_block;
        self.edit_log = snap.edit_log;
        // Any in-flight drag state is meaningless after a state-restore —
        // wipe so a stray mouse-up doesn't smear the just-restored caret.
        self.drag_active = false;
        self.pending_drag_anchor = None;
    }

    /// Reverse the most recent edit by restoring the previous snapshot.
    /// Returns true when an undo actually ran (false when the stack is
    /// empty). The pre-undo state goes on redo_stack so a redo can
    /// reinstate it.
    pub fn undo(&mut self) -> bool {
        let Some(snap) = self.undo_stack.pop() else {
            return false;
        };
        let current = self.current_snapshot();
        if self.redo_stack.len() >= UNDO_STACK_LIMIT {
            self.redo_stack.remove(0);
        }
        self.redo_stack.push(current);
        self.restore_snapshot(snap);
        true
    }

    /// Reverse the most recent undo. Mirror image of `undo`.
    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.redo_stack.pop() else {
            return false;
        };
        let current = self.current_snapshot();
        if self.undo_stack.len() >= UNDO_STACK_LIMIT {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(current);
        self.restore_snapshot(snap);
        true
    }

    /// Returns true if every run in `block` has `bold` set. Used to drive the
    /// pressed-state of the toolbar Bold button. Empty / missing blocks are
    /// treated as not-bold.
    pub fn block_is_bold(&self, block: usize) -> bool {
        self.block_all_runs(block, |s| s.bold)
    }

    pub fn block_is_italic(&self, block: usize) -> bool {
        self.block_all_runs(block, |s| s.italic)
    }

    pub fn block_is_underline(&self, block: usize) -> bool {
        self.block_all_runs(block, |s| s.underline)
    }

    pub fn block_is_strike(&self, block: usize) -> bool {
        self.block_all_runs(block, |s| s.strike)
    }

    /// Returns the heading level of `block` (1..=6) when it is a `Heading`,
    /// or `None` for paragraphs / tables / images / out-of-range indices.
    /// Drives the toolbar Promote / Demote Heading buttons.
    pub fn block_heading_level(&self, block: usize) -> Option<u8> {
        match self.ast.blocks.get(block)? {
            ast::Block::Heading { level, .. } => Some(*level),
            _ => None,
        }
    }

    /// Returns the shared color across every run in `block`, or `None` if the
    /// block is empty / has mixed colors / has no color. Used to drive the
    /// pressed-state swatch on the toolbar Color button.
    pub fn block_color(&self, block: usize) -> Option<u32> {
        let Some(b) = self.ast.blocks.get(block) else {
            return None;
        };
        let runs = match b {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            ast::Block::Table { .. } | ast::Block::Image { .. } => return None,
        };
        let first = runs.first()?.style.color?;
        if runs.iter().all(|r| r.style.color == Some(first)) {
            Some(first)
        } else {
            None
        }
    }

    /// Returns the paragraph alignment and indent level of `block`. Defaults
    /// to (Left, 0) for out-of-range indices, tables, and images so the
    /// toolbar shows a sensible default state even when active_block isn't
    /// a styleable paragraph.
    pub fn block_props(&self, block: usize) -> (ast::Alignment, u8) {
        let Some(b) = self.ast.blocks.get(block) else {
            return (ast::Alignment::Left, 0);
        };
        match b {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => {
                (props.alignment, props.indent_level)
            }
            ast::Block::Table { .. } | ast::Block::Image { .. } => (ast::Alignment::Left, 0),
        }
    }

    /// Returns the list marker on `block`, or `None` for plain paragraphs,
    /// tables, and images. Drives the toolbar pressed-state on the bulleted
    /// and numbered list buttons.
    pub fn block_list(&self, block: usize) -> Option<ast::ListMarker> {
        let b = self.ast.blocks.get(block)?;
        match b {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => props.list,
            ast::Block::Table { .. } | ast::Block::Image { .. } => None,
        }
    }

    /// Returns the shared link URL across every run in `block`, or `None`.
    /// Same semantics as `block_color`.
    pub fn block_link(&self, block: usize) -> Option<String> {
        let Some(b) = self.ast.blocks.get(block) else {
            return None;
        };
        let runs = match b {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            ast::Block::Table { .. } | ast::Block::Image { .. } => return None,
        };
        let first = runs.first()?.style.link.clone()?;
        if runs.iter().all(|r| r.style.link.as_deref() == Some(first.as_str())) {
            Some(first)
        } else {
            None
        }
    }

    /// Read-only access to the inline runs of `block`. Returns `None` for
    /// out-of-range indices, tables, and images. Used by the editable
    /// renderer + selection walkers.
    pub fn block_runs(&self, block: usize) -> Option<&[ast::InlineRun]> {
        let b = self.ast.blocks.get(block)?;
        match b {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => Some(runs),
            ast::Block::Table { .. } | ast::Block::Image { .. } => None,
        }
    }

    /// Total byte length across every run in `block`. Returns 0 for blocks
    /// without runs (tables, images, out-of-range).
    pub fn block_total_bytes(&self, block: usize) -> usize {
        self.block_runs(block).map(|runs| runs.iter().map(|r| r.text.len()).sum()).unwrap_or(0)
    }

    /// Concatenated text of every run in `block`. Returns empty string for
    /// non-paragraph blocks / out-of-range indices.
    pub fn block_text(&self, block: usize) -> String {
        let Some(runs) = self.block_runs(block) else {
            return String::new();
        };
        let mut s = String::with_capacity(runs.iter().map(|r| r.text.len()).sum());
        for r in runs {
            s.push_str(&r.text);
        }
        s
    }

    /// Flat byte offset (sum of preceding runs' lengths + intra-run byte)
    /// for a `TextPos` in `block`. Used to talk to GPUI's `ShapedLine` which
    /// indexes a single concatenated string.
    pub fn flat_byte_for(&self, pos: TextPos) -> usize {
        let Some(runs) = self.block_runs(pos.block) else {
            return 0;
        };
        let mut acc = 0usize;
        for (i, r) in runs.iter().enumerate() {
            if i < pos.run {
                acc += r.text.len();
            } else if i == pos.run {
                return acc + pos.byte.min(r.text.len());
            } else {
                break;
            }
        }
        acc
    }

    /// Inverse of `flat_byte_for` — map a concatenated-text byte offset back
    /// to (run, intra-run byte). Clamps past-end to the final run's end.
    pub fn pos_from_flat_byte(&self, block: usize, flat: usize) -> TextPos {
        let Some(runs) = self.block_runs(block) else {
            return TextPos { block, run: 0, byte: 0 };
        };
        if runs.is_empty() {
            return TextPos { block, run: 0, byte: 0 };
        }
        let mut acc = 0usize;
        for (i, r) in runs.iter().enumerate() {
            let len = r.text.len();
            if flat < acc + len {
                let intra = (flat - acc).min(len);
                // Round to nearest char boundary (down).
                let intra = nearest_char_boundary_down(&r.text, intra);
                return TextPos { block, run: i, byte: intra };
            }
            acc += len;
        }
        // Past the end — sit at end of last run.
        let last = runs.len() - 1;
        TextPos { block, run: last, byte: runs[last].text.len() }
    }

    /// Collapsed cursor at the start of `block`, run 0, byte 0.
    pub fn block_start(&self, block: usize) -> TextPos {
        TextPos { block, run: 0, byte: 0 }
    }

    /// Collapsed cursor at the end of `block` — past the last byte of the
    /// last run. For empty blocks falls back to (0, 0).
    pub fn block_end(&self, block: usize) -> TextPos {
        let Some(runs) = self.block_runs(block) else {
            return TextPos { block, run: 0, byte: 0 };
        };
        if runs.is_empty() {
            return TextPos { block, run: 0, byte: 0 };
        }
        let last = runs.len() - 1;
        TextPos { block, run: last, byte: runs[last].text.len() }
    }

    /// Build a flat searchable string covering every textual block, plus a
    /// per-block offset map. The flat string joins each textual block's
    /// concatenated run text with `\n` between blocks, so user-facing search
    /// matches what they actually see — not the raw XML behind it. Non-
    /// textual blocks (tables, images) are skipped entirely; they aren't
    /// editable inline anyway and have no plain-text representation that
    /// matches what's rendered.
    ///
    /// The offset map is `Vec<(block_idx, flat_start, flat_end)>` — for each
    /// textual block, the half-open flat-text range it occupies. Find
    /// navigation uses this to translate a flat-text match into a TextPos
    /// pair (start block + intra-block flat byte → TextPos via
    /// `pos_from_flat_byte`).
    pub fn flat_search_text(&self) -> (String, Vec<(usize, usize, usize)>) {
        let mut text = String::new();
        let mut offsets: Vec<(usize, usize, usize)> = Vec::new();
        for (idx, _) in self.ast.blocks.iter().enumerate() {
            let Some(runs) = self.block_runs(idx) else {
                continue;
            };
            if !offsets.is_empty() {
                text.push('\n');
            }
            let start = text.len();
            for r in runs {
                text.push_str(&r.text);
            }
            let end = text.len();
            offsets.push((idx, start, end));
        }
        (text, offsets)
    }

    /// Map a global flat-text byte offset (produced by `flat_search_text`)
    /// back to a `TextPos`. Returns `None` when the offset doesn't land in
    /// any textual block (i.e. it's past the end). Past-end is rounded to
    /// the end of the final textual block when `clamp_past_end` is true.
    pub fn pos_from_global_flat(
        &self,
        offsets: &[(usize, usize, usize)],
        global: usize,
        clamp_past_end: bool,
    ) -> Option<TextPos> {
        for &(block_idx, start, end) in offsets {
            if global >= start && global <= end {
                let intra = global - start;
                return Some(self.pos_from_flat_byte(block_idx, intra));
            }
        }
        if clamp_past_end {
            if let Some(&(block_idx, _start, end)) = offsets.last() {
                return Some(self.pos_from_flat_byte(block_idx, end));
            }
        }
        None
    }

    /// Translate flat-text find matches into per-block highlight ranges.
    /// Returns `Vec<(block_idx, lo_in_block, hi_in_block, match_idx)>`.
    /// Each input match is sliced across every textual block it touches,
    /// so a single match spanning two paragraphs produces two entries (one
    /// per block). `match_idx` indexes back into the original `matches`
    /// slice so the caller can mark the "current" match with a different
    /// color.
    pub fn matches_per_block(
        &self,
        offsets: &[(usize, usize, usize)],
        matches: &[std::ops::Range<usize>],
    ) -> Vec<(usize, usize, usize, usize)> {
        match_blocks::project(offsets, matches)
    }

    /// Inverse of `pos_from_global_flat` — map a `TextPos` to the global
    /// flat-text byte offset (so we can locate the cursor's position in the
    /// joined search string). Returns 0 when `pos.block` isn't in `offsets`.
    pub fn global_flat_for_pos(&self, offsets: &[(usize, usize, usize)], pos: TextPos) -> usize {
        let intra = self.flat_byte_for(pos);
        for &(block_idx, start, _end) in offsets {
            if block_idx == pos.block {
                return start + intra;
            }
        }
        0
    }

    /// Extract the text inside `selection`. Single-block selections return
    /// the run slice; cross-block selections join paragraphs with `\n` so
    /// the clipboard text looks correct in any plain-text receiver.
    pub fn selected_text(&self) -> String {
        let Some(sel) = self.selection else {
            return String::new();
        };
        if sel.is_collapsed() {
            return String::new();
        }
        let (lo, mut hi) = sel.range_full();
        if lo.block >= self.ast.blocks.len() {
            return String::new();
        }
        if hi.block >= self.ast.blocks.len() {
            hi = self.block_end(self.ast.blocks.len().saturating_sub(1));
        }
        if lo.block == hi.block {
            return self.text_between_in_block(lo.block, lo.run, lo.byte, hi.run, hi.byte);
        }
        let mut out = String::new();
        // First block: lo→end
        if let Some(runs) = self.block_runs(lo.block) {
            let last_run = runs.len().saturating_sub(1);
            let end_byte = runs.last().map(|r| r.text.len()).unwrap_or(0);
            out.push_str(&self.text_between_in_block(
                lo.block, lo.run, lo.byte, last_run, end_byte,
            ));
        }
        // Middle blocks: whole text
        for b in lo.block.saturating_add(1)..hi.block {
            out.push('\n');
            out.push_str(&self.block_text(b));
        }
        // Last block: start→hi
        out.push('\n');
        out.push_str(&self.text_between_in_block(hi.block, 0, 0, hi.run, hi.byte));
        out
    }

    fn text_between_in_block(
        &self,
        block: usize,
        run_lo: usize,
        byte_lo: usize,
        run_hi: usize,
        byte_hi: usize,
    ) -> String {
        let Some(runs) = self.block_runs(block) else {
            return String::new();
        };
        let mut out = String::new();
        for (i, r) in runs.iter().enumerate() {
            if i < run_lo || i > run_hi {
                continue;
            }
            let start = if i == run_lo { nearest_char_boundary_down(&r.text, byte_lo) } else { 0 };
            let end = if i == run_hi {
                nearest_char_boundary_down(&r.text, byte_hi)
            } else {
                r.text.len()
            };
            if let Some(slice) = r.text.get(start..end) {
                out.push_str(slice);
            }
        }
        out
    }

    fn block_all_runs(&self, block: usize, f: impl Fn(&ast::InlineStyle) -> bool) -> bool {
        let Some(b) = self.ast.blocks.get(block) else {
            return false;
        };
        let runs = match b {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            // Tables/images don't have top-level runs; toolbar treats them as
            // not-styled so B/I/U buttons render unpressed.
            ast::Block::Table { .. } | ast::Block::Image { .. } => return false,
        };
        !runs.is_empty() && runs.iter().all(|r| f(&r.style))
    }
}

/// Round `byte` down to the nearest UTF-8 char boundary inside `s`. Used
/// when mapping click positions (which arrive at byte granularity) onto
/// char-safe AST byte offsets.
pub fn nearest_char_boundary_down(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}

/// Round `byte` up to the nearest UTF-8 char boundary inside `s`. Used by
/// the keyboard handler when stepping the cursor right past a multi-byte
/// codepoint so we never park on a mid-codepoint offset.
pub fn nearest_char_boundary_up(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while b < s.len() && !s.is_char_boundary(b) {
        b += 1;
    }
    b
}

/// docx-rust's content-control readers overflow a 1 MiB stack on ordinary
/// nesting. Reserve a fixed stack for input within MAX_NESTING_DEPTH.
const PARSE_STACK_BYTES: usize = 32 * 1024 * 1024;

/// Open a rich-doc file by path. Reads the file fresh; doesn't take pre-loaded bytes
/// (each format owns its read strategy so DOCX zip handling and RTF text handling
/// don't bleed into a shared loader). Parsing runs on a helper thread with a
/// `PARSE_STACK_BYTES` stack; the call still blocks until it finishes.
pub fn open(path: &Path) -> Result<RichDocPayload> {
    open_as(path, detect_format(path).ok_or(RichDocError::Unsupported)?)
}

/// Parse a session backup whose extension does not identify its native format.
pub(crate) fn open_as(path: &Path, format: DocFormat) -> Result<RichDocPayload> {
    let path = path.to_path_buf();
    let worker = std::thread::Builder::new()
        .name("rich-doc-open".into())
        .stack_size(PARSE_STACK_BYTES)
        .spawn(move || match format {
            DocFormat::Docx => docx::open(&path),
            DocFormat::Rtf => rtf::open(&path),
            DocFormat::Odt => odt::open(&path),
            DocFormat::Doc => doc::open(&path),
        })?;
    worker
        .join()
        .map_err(|_| RichDocError::Io(std::io::Error::other("rich-doc parser thread panicked")))?
}

/// Write a rich-doc payload back to disk in its native format.
///
/// For RTF, `raw_view_override` carries the current rope text — when set and the
/// edit_log is empty, RTF saves the rope directly (raw view is editable).
/// Unsupported raw edits are refused before writing instead of discarded.
/// Legacy `.doc` returns `SaveAsRequired` — the binary format can't be
/// regenerated safely, so the only persistence path is Save As .docx.
pub fn save(payload: &RichDocPayload, path: &Path, raw_view_override: Option<&str>) -> Result<()> {
    validate_raw_view(payload, raw_view_override)?;
    match payload.format {
        DocFormat::Docx => docx::save(payload, path),
        DocFormat::Rtf => rtf::save(payload, path, raw_view_override),
        DocFormat::Odt => odt::save(payload, path),
        DocFormat::Doc => Err(RichDocError::SaveAsRequired { suggested_ext: "docx" }),
    }
}

pub(crate) fn validate_raw_view(payload: &RichDocPayload, raw: Option<&str>) -> Result<()> {
    let Some(raw) = raw else { return Ok(()); };
    if raw == payload.raw_view_text || normalized_lines(raw).eq(normalized_lines(&payload.raw_view_text)) {
        return Ok(());
    }
    if payload.format == DocFormat::Rtf && payload.edit_log.is_empty() {
        return Ok(());
    }
    Err(RichDocError::UnsupportedRawEdits)
}

fn normalized_lines(text: &str) -> impl Iterator<Item = char> + '_ {
    let mut chars = text.chars().peekable();
    std::iter::from_fn(move || {
        let ch = chars.next()?;
        if ch == '\r' {
            if chars.peek() == Some(&'\n') { chars.next(); }
            Some('\n')
        } else {
            Some(ch)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_validation_handles_line_endings_and_legacy_conversion() {
        let mut payload = rtf::open_bytes(br"{\rtf1\ansi text}".to_vec()).expect("fixture");
        payload.raw_view_text = "café 中😀\r\nnext\rlast".into();
        for format in [DocFormat::Docx, DocFormat::Odt, DocFormat::Doc, DocFormat::Rtf] {
            payload.format = format;
            assert!(validate_raw_view(&payload, None).is_ok());
            assert!(validate_raw_view(&payload, Some("café 中😀\nnext\nlast")).is_ok());
            let changed = validate_raw_view(&payload, Some("changed 中😀\nnext\nlast"));
            if format == DocFormat::Rtf {
                assert!(changed.is_ok());
            } else {
                assert!(matches!(changed, Err(RichDocError::UnsupportedRawEdits)));
            }
        }
        payload.apply_edit(edits::Edit::SetBlockBold { block: 0, on: true });
        assert!(matches!(validate_raw_view(&payload, Some("changed source")), Err(RichDocError::UnsupportedRawEdits)));
        assert!(validate_raw_view(&payload, Some("café 中😀\nnext\nlast")).is_ok());
    }

    #[test]
    fn edited_image_blocks_are_refused_before_writing() {
        let dir = crate::persist::TempDir::new("rich-save-image");
        let path = dir.join("source.rtf");
        let original = b"{\\rtf1 keep this source}";
        std::fs::write(&path, original).expect("write fixture");
        let mut payload = rtf::open(&path).expect("open");
        payload.ast.blocks.push(ast::Block::Image {
            source: ast::ImageSource::Inline {
                bytes: std::sync::Arc::new(vec![1, 2, 3]), mime: "image/png".into(),
            },
            width_px: None, height_px: None, alt: Some("keep image".into()),
        });
        payload.apply_edit(edits::Edit::SetBlockBold { block: 0, on: true });
        for format in [DocFormat::Docx, DocFormat::Odt, DocFormat::Rtf] {
            payload.format = format;
            assert!(matches!(save(&payload, &path, None), Err(RichDocError::UnsupportedContent)));
            assert_eq!(std::fs::read(&path).expect("read"), original);
        }
        assert!(matches!(doc::save_as_docx(&payload, &path), Err(RichDocError::UnsupportedContent)));
        assert_eq!(std::fs::read(&path).expect("read"), original);
    }
    use std::path::PathBuf;

    fn nested_sdt_document_xml(depth: usize) -> String {
        let body = format!("{}<w:p><w:r><w:t>x</w:t></w:r></w:p>{}",
            "<w:sdt><w:sdtContent>".repeat(depth), "</w:sdtContent></w:sdt>".repeat(depth));
        format!(r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#)
    }

    #[test]
    fn open_does_not_depend_on_the_callers_stack() {
        let dir = crate::persist::TempDir::new("docx-caller-stack");
        let path = dir.join("document.docx");
        let bytes = doc::build_minimal_docx(&nested_sdt_document_xml(20)).expect("container");
        std::fs::write(&path, bytes).expect("fixture");
        let worker = std::thread::Builder::new().stack_size(1024 * 1024)
            .spawn(move || open(&path).map(|payload| payload.format)).expect("spawn");
        assert_eq!(worker.join().expect("join").expect("open"), DocFormat::Docx);
    }

    #[test]
    #[ignore = "release timing on an archive above the streaming threshold"]
    fn large_rich_document_open_timing() {
        use std::io::Write;
        let dir = crate::persist::TempDir::new("rich-open-timing");
        let path = dir.join("document.docx");
        let original = doc::build_minimal_docx(&nested_sdt_document_xml(0)).expect("container");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(original)).expect("archive");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).expect("file"));
        for i in 0..archive.len() {
            writer.raw_copy_file(archive.by_index(i).expect("entry")).expect("copy");
        }
        writer.start_file("padding.bin", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).expect("entry");
        writer.write_all(&vec![0; 11 * 1024 * 1024]).expect("padding");
        writer.finish().expect("finish");
        assert!(std::fs::metadata(&path).expect("metadata").len() > crate::io_stream::STREAM_THRESHOLD_BYTES);
        let started = std::time::Instant::now();
        for _ in 0..5 {
            assert_eq!(open(&path).expect("open").format, DocFormat::Docx);
        }
        println!("five opens of 11 MiB DOCX: {:?}", started.elapsed());
    }

    #[test]
    fn docx_nested_past_the_limit_is_an_error() {
        let dir = crate::persist::TempDir::new("docx-depth-limit");
        let path = dir.join("document.docx");
        let bytes = doc::build_minimal_docx(&nested_sdt_document_xml(100_000)).expect("container");
        std::fs::write(&path, bytes).expect("fixture");
        assert!(matches!(open(&path), Err(RichDocError::Zip(_))));
    }

    #[test]
    fn detects_docx_lowercase() {
        assert_eq!(detect_format(&PathBuf::from("x.docx")), Some(DocFormat::Docx));
    }

    #[test]
    fn detects_docx_uppercase() {
        assert_eq!(detect_format(&PathBuf::from("X.DOCX")), Some(DocFormat::Docx));
    }

    #[test]
    fn detects_rtf_lowercase() {
        assert_eq!(detect_format(&PathBuf::from("x.rtf")), Some(DocFormat::Rtf));
    }

    #[test]
    fn detects_rtf_uppercase() {
        assert_eq!(detect_format(&PathBuf::from("X.RTF")), Some(DocFormat::Rtf));
    }

    #[test]
    fn detects_odt_lowercase() {
        assert_eq!(detect_format(&PathBuf::from("x.odt")), Some(DocFormat::Odt));
    }

    #[test]
    fn detects_odt_uppercase() {
        assert_eq!(detect_format(&PathBuf::from("X.ODT")), Some(DocFormat::Odt));
    }

    #[test]
    fn detects_doc_lowercase() {
        assert_eq!(detect_format(&PathBuf::from("x.doc")), Some(DocFormat::Doc));
    }

    #[test]
    fn detects_doc_uppercase() {
        assert_eq!(detect_format(&PathBuf::from("X.DOC")), Some(DocFormat::Doc));
    }

    #[test]
    fn save_for_doc_returns_save_as_required() {
        let payload = RichDocPayload {
            revision: crate::buffer::ContentRevision::default(),
            format: DocFormat::Doc,
            ast: ast::Document { blocks: vec![] },
            raw_view_text: String::new(),
            original_bytes: std::sync::Arc::new(Vec::new()),
            edit_log: vec![],
            active_block: None,
            selection: None,
            drag_active: false,
            pending_drag_anchor: None,
            headers: Vec::new(),
            footers: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            edit_group_start: None,
        };
        let target = std::env::temp_dir().join("notepad-studio-doc-save-rejected.doc");
        let err = save(&payload, &target, None).expect_err("save must reject .doc");
        assert!(matches!(err, RichDocError::SaveAsRequired { suggested_ext: "docx" }));
    }

    #[test]
    fn ignores_unknown_extension() {
        assert_eq!(detect_format(&PathBuf::from("x.txt")), None);
    }

    #[test]
    fn ignores_missing_extension() {
        assert_eq!(detect_format(&PathBuf::from("README")), None);
    }

    fn single_para_payload(text: &str) -> RichDocPayload {
        RichDocPayload {
            revision: crate::buffer::ContentRevision::default(),
            format: DocFormat::Docx,
            ast: ast::Document {
                blocks: vec![ast::Block::Paragraph {
                    runs: vec![ast::InlineRun {
                        text: text.to_string(),
                        style: ast::InlineStyle::default(),
                    }],
                    props: ast::BlockProps::default(),
                }],
            },
            raw_view_text: String::new(),
            original_bytes: std::sync::Arc::new(Vec::new()),
            edit_log: vec![],
            active_block: None,
            selection: None,
            drag_active: false,
            pending_drag_anchor: None,
            headers: Vec::new(),
            footers: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            edit_group_start: None,
        }
    }

    #[test]
    fn selected_text_with_mid_char_offsets_does_not_panic() {
        // "aé中😀b": é is 2 bytes, 中 3, 😀 4. Every offset inside one of
        // them is off a char boundary.
        let text = "a\u{e9}\u{4e2d}\u{1f600}b";
        let mut p = single_para_payload(text);
        for lo in 0..=text.len() + 2 {
            for hi in 0..=text.len() + 2 {
                p.selection = Some(TextSelection {
                    anchor: TextPos { block: 0, run: 0, byte: lo },
                    cursor: TextPos { block: 0, run: 0, byte: hi },
                });
                let got = p.selected_text();
                assert!(text.contains(got.as_str()), "{lo}..{hi} gave {got:?}");
            }
        }
        // Cross-block selection whose ends sit mid-char (inside é and 中);
        // both ends round down to the char start.
        p.ast.blocks.push(p.ast.blocks[0].clone());
        p.selection = Some(TextSelection {
            anchor: TextPos { block: 0, run: 0, byte: 2 },
            cursor: TextPos { block: 1, run: 0, byte: 5 },
        });
        assert_eq!(p.selected_text(), "\u{e9}\u{4e2d}\u{1f600}b\na\u{e9}");
    }

    #[test]
    fn selected_text_does_not_invent_missing_blocks() {
        let mut payload = single_para_payload("hello");
        for last_block in [10_000, usize::MAX] {
            payload.selection = Some(TextSelection {
                anchor: TextPos { block: 0, run: 0, byte: 0 },
                cursor: TextPos { block: last_block, run: 0, byte: 0 },
            });
            assert_eq!(payload.selected_text(), "hello");
        }
        payload.selection = Some(TextSelection {
            anchor: TextPos { block: usize::MAX - 1, run: 0, byte: 0 },
            cursor: TextPos { block: usize::MAX, run: 0, byte: 0 },
        });
        assert!(payload.selected_text().is_empty());
        payload.ast.blocks.clear();
        assert!(payload.selected_text().is_empty());
    }

    #[test]
    fn undo_restores_pre_edit_state() {
        let mut p = single_para_payload("hello");
        p.apply_edit(edits::Edit::InsertText {
            block: 0,
            run: 0,
            byte: 5,
            text: " world".to_string(),
        });
        assert_eq!(p.block_text(0), "hello world");
        assert!(p.undo());
        assert_eq!(p.block_text(0), "hello");
        assert!(p.edit_log.is_empty(), "edit log should clear after undo back to origin");
    }

    #[test]
    fn redo_reapplies_undone_edit() {
        let mut p = single_para_payload("hello");
        p.apply_edit(edits::Edit::InsertText {
            block: 0,
            run: 0,
            byte: 5,
            text: " world".to_string(),
        });
        assert!(p.undo());
        assert!(p.redo());
        assert_eq!(p.block_text(0), "hello world");
        assert_eq!(p.edit_log.len(), 1);
    }

    #[test]
    fn fresh_edit_after_undo_clears_redo() {
        let mut p = single_para_payload("hello");
        p.apply_edit(edits::Edit::InsertText {
            block: 0,
            run: 0,
            byte: 5,
            text: " world".to_string(),
        });
        assert!(p.undo());
        // Branch the timeline — redo should now be impossible.
        p.apply_edit(edits::Edit::InsertText {
            block: 0,
            run: 0,
            byte: 5,
            text: "!".to_string(),
        });
        assert!(!p.redo(), "redo must be cleared by a new edit");
        assert_eq!(p.block_text(0), "hello!");
    }

    #[test]
    fn undo_on_empty_stack_is_noop() {
        let mut p = single_para_payload("hello");
        assert!(!p.undo());
        assert_eq!(p.block_text(0), "hello");
    }

    #[test]
    fn undo_restores_selection_and_active_block() {
        let mut p = single_para_payload("hello");
        let pos = TextPos { block: 0, run: 0, byte: 3 };
        p.selection = Some(TextSelection { anchor: pos, cursor: pos });
        p.active_block = Some(0);
        p.apply_edit(edits::Edit::InsertText {
            block: 0,
            run: 0,
            byte: 5,
            text: "X".to_string(),
        });
        // After undo, caret should sit back at byte 3.
        assert!(p.undo());
        let sel = p.selection.expect("selection restored");
        assert_eq!(sel.cursor.byte, 3);
        assert_eq!(p.active_block, Some(0));
    }

    #[test]
    fn undo_stack_caps_at_limit() {
        let mut p = single_para_payload("hello");
        for _ in 0..(UNDO_STACK_LIMIT + 10) {
            p.apply_edit(edits::Edit::InsertText {
                block: 0,
                run: 0,
                byte: 0,
                text: "x".to_string(),
            });
        }
        assert_eq!(p.undo_stack.len(), UNDO_STACK_LIMIT);
    }

    #[test]
    fn nested_edit_groups_restore_the_full_log_and_selection() {
        let mut p = single_para_payload("café");
        let pos = p.block_end(0);
        p.selection = Some(TextSelection { anchor: p.block_start(0), cursor: pos });
        p.active_block = Some(0);
        let before = p.current_snapshot();
        let outer = p.begin_edit_group();
        p.apply_edit(edits::Edit::InsertText { block: 0, run: 0, byte: 0, text: "中".into() });
        let inner = p.begin_edit_group();
        p.apply_edit(edits::Edit::InsertText { block: 0, run: 0, byte: 0, text: "😀".into() });
        p.end_edit_group(inner);
        p.apply_edit(edits::Edit::SetBlockBold { block: 0, on: true });
        p.end_edit_group(outer);
        assert!(p.edit_group_start.is_none());
        assert_eq!(p.undo_stack.len(), 1);
        assert_eq!(p.edit_log.len(), 3);
        let after = p.current_snapshot();
        assert!(p.undo());
        assert_eq!(p.ast, before.ast);
        assert_eq!(p.selection, before.selection);
        assert_eq!(p.active_block, before.active_block);
        assert_eq!(p.edit_log, before.edit_log);
        assert!(p.redo());
        assert_eq!(p.ast, after.ast);
        assert_eq!(p.edit_log, after.edit_log);
        // Ending the outer scope must let the next edit start a new Undo action.
        p.apply_edit(edits::Edit::InsertText { block: 0, run: 0, byte: 0, text: "next".into() });
        assert_eq!(p.undo_stack.len(), 2);
        assert!(p.undo());
        assert_eq!(p.ast, after.ast);
    }

    #[test]
    fn empty_nested_edit_groups_preserve_redo() {
        let mut p = single_para_payload("café");
        p.apply_edit(edits::Edit::InsertText { block: 0, run: 0, byte: 0, text: "中".into() });
        assert!(p.undo());
        let outer = p.begin_edit_group();
        let inner = p.begin_edit_group();
        p.end_edit_group(inner);
        p.end_edit_group(outer);
        assert!(p.edit_group_start.is_none());
        assert!(p.undo_stack.is_empty());
        assert_eq!(p.redo_stack.len(), 1);
        assert!(p.redo());
        assert_eq!(p.block_text(0), "中café");
    }

    #[test]
    fn large_edit_group_preserves_prior_history_at_the_cap() {
        let mut p = single_para_payload("");
        for _ in 0..UNDO_STACK_LIMIT {
            p.apply_edit(edits::Edit::InsertText { block: 0, run: 0, byte: 0, text: "x".into() });
        }
        let before = p.current_snapshot();
        let group = p.begin_edit_group();
        for _ in 0..UNDO_STACK_LIMIT + 20 {
            p.apply_edit(edits::Edit::InsertText { block: 0, run: 0, byte: 0, text: "中".into() });
        }
        p.end_edit_group(group);
        assert_eq!(p.undo_stack.len(), UNDO_STACK_LIMIT);
        assert!(p.undo());
        assert_eq!(p.ast, before.ast);
        assert_eq!(p.edit_log, before.edit_log);
        for _ in 1..UNDO_STACK_LIMIT {
            assert!(p.undo(), "the group must evict only the oldest prior action");
        }
        assert!(!p.undo());
        assert_eq!(p.block_text(0), "x");
    }
}
