use gpui::{Context, MouseDownEvent, Window};

use crate::buffer::BlockSelection;
use crate::edit_ops::delimiter_select;
use crate::settings;
use crate::text::url_scan;
use crate::workspace::{DragMode, TextDragPhase, TextDragState, Workspace};

impl Workspace {
    pub(crate) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_autocomplete(cx);
        let offset = self.offset_at_position(event.position);
        let shift = event.modifiers.shift;
        let alt = event.modifiers.alt;
        let ctrl = event.modifiers.control;
        let cmd = event.modifiers.platform;

        // Cmd+click on a URL → open in default browser. Single click only;
        // multi-click falls through to normal selection so Cmd+double-click
        // can still extend behaviours later. Skips when the feature is
        // disabled in Preferences → Cloud & Link.
        if cmd && !alt && !shift && !ctrl && event.click_count == 1 {
            let snap = settings::snapshot();
            if snap.clickable_urls_enabled {
                if let Some(url) = url_at_offset(self, offset) {
                    let _ = std::process::Command::new("open").arg(url).spawn();
                    cx.notify();
                    return;
                }
            }
        }

        // NPP Ctrl+double-click → select between delimiters. Reads the
        // configured pair from Preferences (`delimiter_left/right`) and
        // honors `delimiter_match_entire_document`. Falls through to
        // normal double-click word selection on miss.
        if ctrl && !alt && !shift && event.click_count == 2 {
            if let Some(range) = delimiter_pair_range(self, offset) {
                self.clear_block_selection();
                let tab = self.active_tab_mut();
                tab.selection.anchor = range.start;
                tab.selection.head = range.end;
                tab.selection.is_dragging = false;
                self.drag_mode = DragMode::Char;
                self.drag_anchor_range = None;
                self.caret_visible = true;
                cx.notify();
                return;
            }
        }

        // Alt+drag (Option+drag on macOS) starts a Notepad++ column-mode
        // selection. Anchor + head are kept in (line, char_col) so virtual
        // columns past EOL still work. Linear `Selection.is_dragging` stays
        // off — the per-frame mouse-move handler routes block updates to
        // `tab.block_selection` instead.
        if alt && event.click_count <= 1 {
            let (line, col) = self.line_col_at_byte(offset);
            self.set_block_selection(Some(BlockSelection {
                anchor_line: line,
                anchor_col: col,
                head_line: line,
                head_col: col,
                is_dragging: true,
            }));
            let active = self.active;
            self.tabs[active].selection.collapse_to(offset);
            self.drag_mode = DragMode::Char;
            self.drag_anchor_range = None;
            self.caret_visible = true;
            cx.notify();
            return;
        }

        // Shift+click always extends the existing selection by character —
        // ignore click_count so users can shift-click after a double/triple
        // without strange snap-by-word behavior.
        if shift {
            // Plain shift-click cancels any in-progress block selection
            // — switching back to linear extend semantics.
            self.clear_block_selection();
            let tab = self.active_tab_mut();
            tab.selection.head = offset;
            tab.selection.is_dragging = true;
            self.drag_mode = DragMode::Char;
            self.drag_anchor_range = None;
            self.caret_visible = true;
            cx.notify();
            return;
        }

        // Plain click clears any prior block selection.
        self.clear_block_selection();

        // Selected-text drag-and-drop. If the click landed inside the active
        // selection (and no modifiers are in play above), arm a pending drag.
        // The MouseMove listener decides whether to promote it to a real drag
        // (pointer moves past TEXT_DRAG_THRESHOLD_PX) or treat it as a plain
        // click (mouse-up with no movement collapses the selection). Gated
        // off when the user has set `Editing 1 ▸ Disable selected text
        // drag-and-drop`.
        if event.click_count == 1 && !settings::snapshot().disable_selected_text_drag_drop {
            let cur_sel = self.selection();
            let lo = cur_sel.anchor.min(cur_sel.head);
            let hi = cur_sel.anchor.max(cur_sel.head);
            if lo != hi && offset >= lo && offset < hi {
                let source_text = self
                    .buffer()
                    .rope
                    .byte_slice(lo..hi)
                    .to_string();
                self.text_drag = Some(TextDragState {
                    phase: TextDragPhase::Pending,
                    source_range: lo..hi,
                    source_text,
                    start_position: event.position,
                    drop_target: offset,
                });
                self.drag_mode = DragMode::Char;
                self.drag_anchor_range = None;
                self.caret_visible = true;
                // Do NOT touch the selection — the source range stays
                // highlighted so the user can see what they're dragging.
                cx.notify();
                return;
            }
        }

        // gpui's `MouseDownEvent::click_count` is fed by the OS click-grouping
        // state machine (NSEvent.clickCount on macOS, equivalent state on
        // Linux/Windows), so we get system-correct double-click time and
        // movement thresholds for free.
        let click_count = event.click_count;
        match click_count {
            0 | 1 => {
                let tab = self.active_tab_mut();
                tab.selection.anchor = offset;
                tab.selection.head = offset;
                tab.selection.is_dragging = true;
                self.drag_mode = DragMode::Char;
                self.drag_anchor_range = None;
            }
            2 => {
                let range = self.word_range_at(offset);
                let tab = self.active_tab_mut();
                tab.selection.anchor = range.start;
                tab.selection.head = range.end;
                tab.selection.is_dragging = true;
                self.drag_mode = DragMode::Word;
                self.drag_anchor_range = Some(range);
            }
            3 => {
                let range = self.line_range_at(offset);
                let tab = self.active_tab_mut();
                tab.selection.anchor = range.start;
                tab.selection.head = range.end;
                tab.selection.is_dragging = true;
                self.drag_mode = DragMode::Line;
                self.drag_anchor_range = Some(range);
            }
            _ => {
                // Quadruple-click → select the entire document. NPP/macOS
                // standard. Drag after this point reverts to char-level so
                // a 4-click-and-drag doesn't surprise the user.
                let total = self.buffer().rope.len_bytes();
                let tab = self.active_tab_mut();
                tab.selection.anchor = 0;
                tab.selection.head = total;
                tab.selection.is_dragging = false;
                self.drag_mode = DragMode::Char;
                self.drag_anchor_range = None;
            }
        }

        self.caret_visible = true;
        cx.notify();
    }

    /// Commit a selected-text drag-and-drop release. `cmd_held = true` keeps
    /// the source intact and copies the text to the drop position; `false`
    /// moves the text (deletes source, inserts at drop position). When the
    /// drop target falls inside the source range, the drop is a no-op
    /// (would self-destruct); the selection is restored at the source.
    pub(crate) fn commit_text_drag(
        &mut self,
        state: TextDragState,
        cmd_held: bool,
        cx: &mut Context<Self>,
    ) {
        let source = state.source_range.clone();
        let target = state.drop_target;
        // Drop inside source is a no-op — would erase the dragged text into
        // itself. Leave the selection as it was; mirrors macOS Finder /
        // Notepad++.
        if !cmd_held && target >= source.start && target <= source.end {
            self.caret_visible = true;
            cx.notify();
            return;
        }
        if self.buffer().read_only {
            self.set_status("Buffer is read-only");
            cx.notify();
            return;
        }
        self.begin_edit_group();
        if cmd_held {
            // Copy: leave the source intact, just insert at target.
            self.replace_byte_range(target..target, &state.source_text, cx);
            self.end_edit_group(cx);
            let inserted_end = target + state.source_text.len();
            let tab = self.active_tab_mut();
            tab.selection.anchor = target;
            tab.selection.head = inserted_end;
            tab.selection.is_dragging = false;
        } else {
            // Move: delete source first, then insert at the target adjusted
            // for the deletion when the target was to the right of source.
            let adjusted_target = if target >= source.end {
                target - state.source_text.len()
            } else {
                target
            };
            self.replace_byte_range(source.clone(), "", cx);
            self.replace_byte_range(adjusted_target..adjusted_target, &state.source_text, cx);
            self.end_edit_group(cx);
            let inserted_end = adjusted_target + state.source_text.len();
            let tab = self.active_tab_mut();
            tab.selection.anchor = adjusted_target;
            tab.selection.head = inserted_end;
            tab.selection.is_dragging = false;
        }
        self.drag_mode = DragMode::Char;
        self.drag_anchor_range = None;
        self.caret_visible = true;
        self.ensure_cursor_visible(cx);
        cx.notify();
    }
}

/// Read the configured delimiter pair from settings and run
/// `select_between_delimiters` against the active buffer at `offset`.
/// Returns `None` if either configured side is empty (degenerate config) or
/// if no surrounding pair was found.
fn delimiter_pair_range(
    ws: &Workspace,
    offset: usize,
) -> Option<std::ops::Range<usize>> {
    let snap = settings::snapshot();
    let left = snap.delimiter_left.chars().next()?;
    let right = snap.delimiter_right.chars().next()?;
    delimiter_select::select_between_delimiters(
        &ws.buffer().rope,
        offset,
        left,
        right,
        snap.delimiter_match_entire_document,
    )
}

/// If `offset` falls inside a URL on its line, return the URL string.
/// URLs are detected per-line via `url_scan::find_urls`; URLs that span
/// hard line breaks are not joined (each line scanned independently).
fn url_at_offset(ws: &Workspace, offset: usize) -> Option<String> {
    let rope = &ws.buffer().rope;
    let total = rope.len_bytes();
    if offset > total {
        return None;
    }
    let line_idx = rope.byte_to_line(offset);
    let line_byte_start = rope.line_to_byte(line_idx);
    let line_slice = rope.line(line_idx);
    let raw = line_slice.to_string();
    let trimmed = raw.trim_end_matches(['\n', '\r']);
    let local = offset.saturating_sub(line_byte_start);
    for r in url_scan::find_urls(trimmed) {
        if r.start <= local && local < r.end {
            return Some(trimmed[r.start..r.end].to_string());
        }
    }
    None
}
