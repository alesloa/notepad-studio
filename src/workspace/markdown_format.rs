// Markdown formatting helpers — drive the toolbar's Markdown formatting
// group. Each public action wraps a small helper that mutates the active
// buffer through the existing `replace_byte_range` undo path.
//
// Shape of the helpers:
//   * `md_wrap_inline` — bold / italic / strike / inline-code. Toggles the
//     wrap delimiters on or off around the current selection. Empty
//     selection inserts a placeholder so the user lands typing-ready.
//   * `md_line_prefix` — headings, bullet list, task list, block quote.
//     Toggles a per-line prefix across every line touched by the selection
//     (or the current line). Skips blank lines.
//   * `md_ordered_list` — like `md_line_prefix` but increments the number
//     per non-empty line.
//   * `md_code_block` — fenced code block on its own lines.
//   * `md_link_or_image` — `[text](url)` / `![alt](url)`.
//   * `md_horizontal_rule` — inserts `---` on its own line.
//
// All helpers are gated by `md_guard` which requires the editor to be
// focused, the active tab's file to be a markdown file, and the view mode
// to be `Edit` (the formatting toolbar is hidden in preview anyway, but
// a stray keyboard chord or accessibility dispatch shouldn't bypass that).
//
// Each helper plans its edit with a pure `plan_*` function over the rope
// (tested at the bottom of this file) and applies it with `apply_md_edit`.

use gpui::{Context, Window, px};
use ropey::Rope;
use std::ops::Range;
use std::time::Duration;

use crate::actions::*;
use crate::workspace::Workspace;

impl Workspace {
    /// Mouse-driven selection update from the markdown preview pane.
    /// `extend = true` keeps the existing anchor (drag / shift-click);
    /// `start_drag = true` sets `is_dragging` so the per-paragraph
    /// `on_mouse_move` listeners know to keep updating the head until
    /// the window-level mouse-up clears the flag.
    pub(crate) fn preview_set_caret(
        &mut self,
        source_byte: usize,
        extend: bool,
        start_drag: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let byte = snap_byte(&self.buffer().rope, source_byte);
        let tab = self.active_tab_mut();
        if extend {
            tab.selection.head = byte;
        } else {
            tab.selection.anchor = byte;
            tab.selection.head = byte;
        }
        if start_drag {
            tab.selection.is_dragging = true;
        }
        tab.desired_col_x = None;
        // Mouse-driven selection never auto-scrolls — the user already
        // told us where they want to look by clicking there. Drop any
        // pending scroll request from an earlier keyboard / edit so the
        // next paint doesn't yank the viewport.
        self.preview_caret_scroll_pending = false;
        cx.notify();
    }

    pub(crate) fn preview_is_dragging(&self) -> bool {
        self.tabs[self.active].selection.is_dragging
    }

    pub(crate) fn preview_end_drag(&mut self, cx: &mut gpui::Context<Self>) {
        let tab = self.active_tab_mut();
        if tab.selection.is_dragging {
            tab.selection.is_dragging = false;
            cx.notify();
        }
    }

    /// Double-click in the preview pane → select the word (run of same
    /// word-class chars) under the source byte. Mirrors source-mode
    /// double-click; `is_dragging` stays off because v1 doesn't yet
    /// extend-by-word during a drag.
    pub(crate) fn preview_select_word(
        &mut self,
        source_byte: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let byte = snap_byte(&self.buffer().rope, source_byte);
        let range = self.word_range_at(byte);
        let tab = self.active_tab_mut();
        tab.selection.anchor = range.start;
        tab.selection.head = range.end;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        cx.notify();
    }

    /// Triple-click → select the line under the source byte (excluding
    /// trailing CR/LF), matching source-mode triple-click.
    pub(crate) fn preview_select_line(
        &mut self,
        source_byte: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let byte = snap_byte(&self.buffer().rope, source_byte);
        let range = self.line_range_at(byte);
        let tab = self.active_tab_mut();
        tab.selection.anchor = range.start;
        tab.selection.head = range.end;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        cx.notify();
    }

    /// Insert markdown image markup at `source_byte`. Used by the
    /// preview pane's drag-drop handler — drop a `.png` / `.jpg` / etc.
    /// onto a paragraph and we splice `![name](file://path)` in at the
    /// rendered byte under the cursor. Selection collapses to the end of
    /// the inserted text so the next keystroke continues from there.
    pub(crate) fn preview_insert_image_markup(
        &mut self,
        source_byte: usize,
        snippet: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let edit = plan_insert_image(&self.buffer().rope, source_byte, snippet);
        self.apply_md_edit(edit, cx);
    }

    /// Append image markup at the end of the buffer, preceded by a
    /// newline if the buffer doesn't already end with one. Called when a
    /// drop misses every paragraph / heading hitbox — the file lands
    /// somewhere predictable instead of getting silently dropped.
    pub(crate) fn preview_append_image_markup(
        &mut self,
        snippet: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let buf = self.buffer();
        let edit = plan_append_image(&buf.rope, &snippet, buf.line_ending.as_str());
        self.apply_md_edit(edit, cx);
    }

    /// Jump the preview viewport to the footnote definition matching
    /// `label`. Fired when the user cmd/ctrl-clicks a `[^label]` reference
    /// in preview mode — the synthetic `notepad-fn:def:<label>` link the
    /// reference run carries routes here from the click handler in
    /// `markdown_view::mod`.
    ///
    /// Strategy: scan the source rope for the literal `[^<label>]:` token
    /// (definition prefix) and drop the caret at the byte where the
    /// closing `]` sits. `ensure_cursor_visible` then schedules the next
    /// paint to scroll the holding block into view. Moving the caret
    /// gives a "you are here" anchor and lets the user immediately edit
    /// the def body without an extra click. No-op when the def isn't
    /// present (orphan reference) — keeps a stale link from yanking the
    /// viewport to byte 0.
    pub(crate) fn preview_jump_to_footnote_def(
        &mut self,
        label: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let source = self.buffer().rope.to_string();
        let needle = format!("[^{label}]:");
        let Some(start) = source.find(&needle) else {
            return;
        };
        let target = start + needle.len() - 1;
        let tab = self.active_tab_mut();
        tab.selection.anchor = target;
        tab.selection.head = target;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        self.caret_visible = true;
        self.ensure_cursor_visible(cx);
        cx.notify();
    }

    /// Jump the preview viewport to the first `[^label]` reference
    /// matching `label`. Fired when the user clicks the back-link element
    /// on a footnote definition. The first non-definition occurrence wins
    /// — multiple references to the same footnote are rare and pandoc /
    /// GFM render only one back-arrow per def anyway, so this matches
    /// the common-case expectation.
    ///
    /// Distinguishes ref from def by skipping any `[^<label>]` whose next
    /// byte is `:` (that's the def itself). Scan stops at the first
    /// surviving match.
    pub(crate) fn preview_jump_to_footnote_ref(
        &mut self,
        label: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let source = self.buffer().rope.to_string();
        let needle = format!("[^{label}]");
        let mut search_from = 0;
        let target = loop {
            let Some(rel) = source[search_from..].find(&needle) else {
                return;
            };
            let abs = search_from + rel;
            let after = abs + needle.len();
            if source.as_bytes().get(after) != Some(&b':') {
                break abs;
            }
            search_from = after;
        };
        let tab = self.active_tab_mut();
        tab.selection.anchor = target;
        tab.selection.head = target;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        self.caret_visible = true;
        self.ensure_cursor_visible(cx);
        cx.notify();
    }

    /// Quadruple-click → select the entire buffer. Mirrors source-mode
    /// `on_mouse_down`'s 4-click path so the preview matches the native
    /// macOS / NPP behaviour. Drag flag stays off — a quadruple-click
    /// drag would be surprising and inherits no useful anchor.
    pub(crate) fn preview_select_all(&mut self, cx: &mut gpui::Context<Self>) {
        let total = self.buffer().rope.len_bytes();
        let tab = self.active_tab_mut();
        tab.selection.anchor = 0;
        tab.selection.head = total;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        cx.notify();
    }

    /// Pick the closest registered block to `position` (window-space
    /// pixels) and return its mapped source byte. Used by the outer
    /// pane's mouse-down handler when a click lands in dead space
    /// (margins, between blocks, scroll-past-end spacer) and by the
    /// window-level mouse-move listener when a drag passes through a
    /// gap between blocks.
    ///
    /// Selection rule:
    ///   * If `position.y` is inside any block's layout bounds → that
    ///     block (the in-bounds layout's own clamping handles the
    ///     column).
    ///   * Else → the block whose vertical center is closest to
    ///     `position.y`. When the cursor is below every block this
    ///     ends up being the last block, and `layout_index_at` clamps
    ///     to its end byte; above every block, it's the first block
    ///     and clamps to its start. That matches the "drag past the
    ///     bottom selects to end" behaviour of every native text input.
    ///
    /// Returns `None` only when there are zero registered blocks
    /// (preview hasn't painted yet, or the file is empty). Callers
    /// fall back to either a no-op or `preview_clear_selection`.
    pub(crate) fn preview_pick_source_byte(
        &self,
        position: gpui::Point<gpui::Pixels>,
    ) -> Option<usize> {
        let targets = &self.preview_block_targets;
        if targets.is_empty() {
            return None;
        }
        // Prefer the in-bounds match — within a single block, the
        // layout's own line-iteration logic in index_for_position
        // handles vertical position correctly.
        for target in targets {
            let bounds = target.layout.bounds();
            if position.y >= bounds.top() && position.y <= bounds.bottom() {
                return target.source_byte_at(position);
            }
        }
        // No in-bounds hit — pick the block whose vertical center is
        // closest. `index_for_position` on that block will clamp to
        // the nearest column in the nearest line.
        let mut best_idx = 0usize;
        let mut best_dist = gpui::Pixels::MAX;
        for (i, target) in targets.iter().enumerate() {
            let bounds = target.layout.bounds();
            let center_y = bounds.top() + bounds.size.height / 2.;
            let d = (center_y - position.y).abs();
            if d < best_dist {
                best_dist = d;
                best_idx = i;
            }
        }
        targets[best_idx].source_byte_at(position)
    }

    /// One-shot drag-or-click handler called from the outer preview-pane
    /// mouse-down and the window-level mouse-move listener. Hit-tests
    /// against the registered block list and applies the same selection
    /// semantics as the per-block on_mouse_down — keeping behaviour
    /// consistent whether the click landed in text or in dead space.
    pub(crate) fn preview_drag_to_pixel(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        extend: bool,
        start_drag: bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(byte) = self.preview_pick_source_byte(position) else {
            return false;
        };
        self.preview_set_caret(byte, extend, start_drag, cx);
        true
    }

    /// While a drag is in progress, nudge the preview scroll offset when
    /// the cursor moves past the top or bottom of the viewport so the
    /// user can keep extending the selection into content that's still
    /// scrolled off. Returns the applied delta in pixels (always 0 when
    /// no scroll was needed). Magnitude scales with how far past the
    /// edge the cursor is, capped at ~3 line-heights per call — fast
    /// enough to feel responsive without overshooting on a single
    /// mouse-move event.
    pub(crate) fn preview_edge_auto_scroll(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Pixels {
        let viewport = self.md_preview_scroll_handle.bounds();
        let viewport_h = viewport.size.height;
        if viewport_h <= px(0.) {
            return px(0.);
        }
        let top = viewport.origin.y;
        let bottom = top + viewport_h;
        // Use a fixed step rather than window.line_height() because this
        // runs inside an `on_mouse_event` closure where Window isn't
        // available — pick a body-text-sized step that feels natural.
        let step = px(20.);
        let cap = step * 3.;
        let mut offset = self.md_preview_scroll_handle.offset();
        let max_y = self.md_preview_scroll_handle.max_offset().height;
        let delta;
        if position.y < top {
            // Cursor past top edge — scroll content down (offset.y toward
            // 0). Scale by how far past the edge so the further out the
            // cursor goes, the faster it scrolls (up to the cap).
            let past = top - position.y;
            let want = past.min(cap);
            let new_y = (offset.y + want).min(px(0.));
            delta = new_y - offset.y;
            offset.y = new_y;
        } else if position.y > bottom {
            let past = position.y - bottom;
            let want = past.min(cap);
            let new_y = (offset.y - want).max(-max_y);
            delta = new_y - offset.y;
            offset.y = new_y;
        } else {
            return px(0.);
        }
        if delta.abs() > px(0.) {
            self.md_preview_scroll_handle.set_offset(offset);
            cx.notify();
        }
        delta
    }

    /// Kick off (or no-op if already running) a repeating timer that keeps
    /// re-firing edge auto-scroll while the user holds the drag cursor past
    /// the top/bottom edge of the preview viewport. Without this, scrolling
    /// stalls the instant mouse movement stops — native NPP keeps going.
    ///
    /// Stores the captured cursor position in `preview_last_drag_pos` so the
    /// tick loop has something to re-feed into the edge check. The spawned
    /// task self-cancels when the drag ends, the cursor moves back inside
    /// the viewport, or the captured position clears.
    ///
    /// Tick interval is 30ms — fast enough to feel native (roughly matches
    /// macOS NSTextView's auto-scroll cadence) without burning idle frames.
    pub(crate) fn preview_start_continuous_autoscroll(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.preview_last_drag_pos = Some(position);
        if self.preview_autoscroll_task.is_some() {
            return;
        }
        // Only start the timer if the cursor is currently past an edge.
        // A drag entirely inside the viewport doesn't need the fallback.
        let viewport = self.md_preview_scroll_handle.bounds();
        let viewport_h = viewport.size.height;
        if viewport_h <= px(0.) {
            return;
        }
        let top = viewport.origin.y;
        let bottom = top + viewport_h;
        if position.y >= top && position.y <= bottom {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            loop {
                gpui::Timer::after(Duration::from_millis(30)).await;
                let keep_going = this
                    .update(cx, |w, cx| {
                        if !w.preview_is_dragging() {
                            w.preview_autoscroll_task = None;
                            w.preview_last_drag_pos = None;
                            return false;
                        }
                        let Some(pos) = w.preview_last_drag_pos else {
                            w.preview_autoscroll_task = None;
                            return false;
                        };
                        let viewport = w.md_preview_scroll_handle.bounds();
                        let viewport_h = viewport.size.height;
                        if viewport_h <= px(0.) {
                            w.preview_autoscroll_task = None;
                            return false;
                        }
                        let top = viewport.origin.y;
                        let bottom = top + viewport_h;
                        if pos.y >= top && pos.y <= bottom {
                            // Cursor walked back inside the viewport — stop
                            // ticking. The next MouseMoveEvent past an edge
                            // will restart us.
                            w.preview_autoscroll_task = None;
                            return false;
                        }
                        w.preview_edge_auto_scroll(pos, cx);
                        w.preview_drag_to_pixel(pos, true, false, cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        });
        self.preview_autoscroll_task = Some(task);
    }

    /// Cancel the continuous-autoscroll timer (if any) and clear the captured
    /// drag position. Called from MouseUpEvent so the timer doesn't outlive
    /// the drag. Dropping the `Task` cancels the spawned future.
    pub(crate) fn preview_stop_continuous_autoscroll(&mut self, _cx: &mut gpui::Context<Self>) {
        self.preview_autoscroll_task = None;
        self.preview_last_drag_pos = None;
    }

    /// Vertical caret movement inside the markdown preview pane. Walks
    /// up/down by `window.line_height()` against the per-block layouts
    /// registered by the last paint (`preview_block_targets`). Returns
    /// `true` if a move was applied so the caller can fall through to
    /// the source-mode path when the preview hasn't painted yet.
    ///
    /// Algorithm:
    ///   1. Find the block whose hit-runs cover the current `head`.
    ///   2. Resolve the caret's window-space pixel `(x, y)` from that
    ///      block's `TextLayout`.
    ///   3. Compute the target visual y = `caret_y + line_height * delta`
    ///      and preserve `desired_col_x` across moves (matches source-mode
    ///      column behaviour).
    ///   4. Try the current block first (handles multi-wrap paragraphs).
    ///      If the target y is outside the current block, fall through to
    ///      the block whose bounds contain the target y, or the nearest
    ///      block above/below when none does.
    ///   5. `layout.index_for_position` on the chosen block clamps the
    ///      target x to that line's nearest column.
    ///   6. Set `head` to the resulting source byte; call
    ///      `ensure_cursor_visible` so the new caret position scrolls
    ///      into view via the [[preview_caret_scroll_pending]] flag.
    pub(crate) fn preview_move_vertical(
        &mut self,
        delta: i32,
        extend: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        use crate::ui::markdown_view::{
            layout_index_at, render_byte_to_source, source_byte_to_render,
        };
        if self.preview_block_targets.is_empty() {
            return false;
        }
        let head = self.selection().head;
        let line_height = window.line_height();
        // Locate the block currently holding the caret. `source_byte_to_render`
        // returns `Some` exactly when `head` falls inside this block's
        // source-byte span — same gate `CaretOverlay::paint` uses.
        let cur = self
            .preview_block_targets
            .iter()
            .find_map(|t| source_byte_to_render(&t.hits, head).map(|r| (t.clone(), r)));
        let Some((cur_target, rendered_byte)) = cur else {
            return false;
        };
        let Some(origin) = cur_target.layout.position_for_index(rendered_byte) else {
            return false;
        };
        // Preserve column across vertical moves. First Up/Down after a
        // horizontal move captures the current pixel x; subsequent moves
        // reuse it so the caret tracks the same column even through lines
        // of varying length.
        let desired_x = self.tabs[self.active]
            .desired_col_x
            .unwrap_or(origin.x);
        self.active_tab_mut().desired_col_x = Some(desired_x);

        // Target sample point — half a line-height past the current line's
        // top in the move direction so we land cleanly inside the next
        // wrapped row (avoids index_for_position's row-boundary fuzziness
        // when probing exactly on a baseline).
        let target_y = origin.y + line_height * (delta as f32 + 0.5);
        let target = gpui::point(desired_x, target_y);

        // Try the current block first — handles multi-wrap paragraphs
        // where Up/Down stays within the same block.
        let cur_bounds = cur_target.layout.bounds();
        let in_cur =
            target_y >= cur_bounds.top() && target_y <= cur_bounds.bottom();
        let chosen_byte = if in_cur {
            layout_index_at(&cur_target.layout, target)
                .map(|i| render_byte_to_source(&cur_target.hits, i))
        } else {
            // Walk all blocks to find one whose y span straddles target_y;
            // if none does, pick the nearest in the direction of `delta`.
            let mut straddler: Option<usize> = None;
            for (i, t) in self.preview_block_targets.iter().enumerate() {
                let b = t.layout.bounds();
                if target_y >= b.top() && target_y <= b.bottom() {
                    straddler = Some(i);
                    break;
                }
            }
            let chosen_idx = straddler.or_else(|| {
                // Past every block in this direction — clamp to the
                // first/last block by vertical center so Up at the top
                // lands on byte 0 and Down at the bottom lands on len.
                let mut best_idx = 0usize;
                let mut best_dist = gpui::Pixels::MAX;
                for (i, t) in self.preview_block_targets.iter().enumerate() {
                    let b = t.layout.bounds();
                    let center = b.top() + b.size.height / 2.;
                    let d = (center - target_y).abs();
                    if d < best_dist {
                        best_dist = d;
                        best_idx = i;
                    }
                }
                Some(best_idx)
            });
            let Some(idx) = chosen_idx else { return false; };
            let chosen = self.preview_block_targets[idx].clone();
            let b = chosen.layout.bounds();
            // Clamp target_y inside the chosen block so index_for_position
            // doesn't fall off the layout entirely.
            let clamped_y = target_y.max(b.top()).min(b.bottom() - px(1.));
            let probe = gpui::point(desired_x, clamped_y);
            layout_index_at(&chosen.layout, probe)
                .map(|i| render_byte_to_source(&chosen.hits, i))
        };

        let Some(byte) = chosen_byte else { return false; };
        let new_byte = snap_byte(&self.buffer().rope, byte);
        let tab = self.active_tab_mut();
        if extend {
            tab.selection.head = new_byte;
        } else {
            tab.selection.collapse_to(new_byte);
        }
        self.caret_visible = true;
        self.ensure_cursor_visible(cx);
        cx.notify();
        true
    }

    /// Collapse the active tab's selection. Called from the preview
    /// pane's outer mouse-down so a click that misses every paragraph /
    /// heading drops the highlight, the same way clicking blank space in
    /// a regular text field clears the selection.
    pub(crate) fn preview_clear_selection(&mut self, cx: &mut gpui::Context<Self>) {
        let tab = self.active_tab_mut();
        if tab.selection.head != tab.selection.anchor || tab.selection.is_dragging {
            tab.selection.head = tab.selection.anchor;
            tab.selection.is_dragging = false;
            cx.notify();
        }
    }

    fn md_guard(&mut self, _window: &mut Window) -> bool {
        // Markdown formatting is exposed via the secondary toolbar that's
        // only visible in preview mode, but the helpers themselves run in
        // either source or preview view — they always operate on the
        // persisted source selection. Gate purely on file extension so a
        // stray dispatch on a non-markdown tab does nothing.
        let tab = &self.tabs[self.active];
        crate::ui::markdown_view::is_markdown_path(tab.buffer.file_path.as_deref())
    }

    /// Replace the planned range and select the planned result. Every
    /// formatting command and image drop goes through here. A read-only
    /// buffer refuses the edit, so its selection stays put rather than
    /// pointing into text that was never inserted.
    fn apply_md_edit(&mut self, edit: MdEdit, cx: &mut Context<Self>) {
        let read_only = self.buffer().read_only;
        self.replace_byte_range(edit.range, &edit.text, cx);
        if read_only {
            return;
        }
        let tab = self.active_tab_mut();
        if edit.anchor == edit.head {
            tab.selection.collapse_to(edit.head);
        } else {
            tab.selection.anchor = edit.anchor;
            tab.selection.head = edit.head;
        }
        tab.desired_col_x = None;
        cx.notify();
    }

    /// Wrap the current selection (or insert at the caret) with `prefix`/`suffix`.
    /// Toggles off when the selection is already exactly `prefix..suffix`.
    fn md_wrap_inline(
        &mut self,
        prefix: &str,
        suffix: &str,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) {
        let r = self.selection().ordered();
        let edit = plan_wrap_inline(&self.buffer().rope, r, prefix, suffix, placeholder);
        self.apply_md_edit(edit, cx);
    }

    /// Add or remove `prefix` from the start of every non-empty line in the
    /// selection's line block. Toggles off only when every non-empty line
    /// already begins with the prefix.
    fn md_line_prefix(&mut self, prefix: &str, cx: &mut Context<Self>) {
        let r = self.selection().ordered();
        if let Some(edit) = plan_line_prefix(&self.buffer().rope, r, prefix) {
            self.apply_md_edit(edit, cx);
        }
    }

    /// Number every non-empty line in the selection block (`1. `, `2. `, …).
    /// Toggles off when every non-empty line already begins with `\d+\. `.
    fn md_ordered_list(&mut self, cx: &mut Context<Self>) {
        let r = self.selection().ordered();
        if let Some(edit) = plan_ordered_list(&self.buffer().rope, r) {
            self.apply_md_edit(edit, cx);
        }
    }

    /// Wrap the selection (or insert at the caret) with a fenced code block.
    /// Always lands on its own lines.
    fn md_code_block(&mut self, cx: &mut Context<Self>) {
        let r = self.selection().ordered();
        let buf = self.buffer();
        let edit = plan_code_block(&buf.rope, r, buf.line_ending.as_str());
        self.apply_md_edit(edit, cx);
    }

    /// Insert / wrap a markdown link or image.
    /// * No selection: `[text](url)` (or `![alt](url)`), with `text`/`alt` selected.
    /// * Selection: `[selected](url)` with the `url` placeholder selected so
    ///   the user can paste / type the URL straight away.
    fn md_link_or_image(&mut self, image: bool, cx: &mut Context<Self>) {
        let r = self.selection().ordered();
        let edit = plan_link_or_image(&self.buffer().rope, r, image);
        self.apply_md_edit(edit, cx);
    }

    /// Insert a horizontal rule (`---`) on its own line.
    fn md_horizontal_rule(&mut self, cx: &mut Context<Self>) {
        let r = self.selection().ordered();
        let buf = self.buffer();
        let edit = plan_horizontal_rule(&buf.rope, r, buf.line_ending.as_str());
        self.apply_md_edit(edit, cx);
    }

    // ---- Action wrappers (one per toolbar button). ---------------------------

    pub(crate) fn md_bold(
        &mut self,
        _: &MdBold,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_wrap_inline("**", "**", "bold text", cx);
    }

    pub(crate) fn md_italic(
        &mut self,
        _: &MdItalic,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_wrap_inline("*", "*", "italic text", cx);
    }

    pub(crate) fn md_strikethrough(
        &mut self,
        _: &MdStrikethrough,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_wrap_inline("~~", "~~", "strikethrough", cx);
    }

    pub(crate) fn md_inline_code(
        &mut self,
        _: &MdInlineCode,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_wrap_inline("`", "`", "code", cx);
    }

    pub(crate) fn md_heading_1(
        &mut self,
        _: &MdHeading1,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_line_prefix("# ", cx);
    }

    pub(crate) fn md_heading_2(
        &mut self,
        _: &MdHeading2,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_line_prefix("## ", cx);
    }

    pub(crate) fn md_heading_3(
        &mut self,
        _: &MdHeading3,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_line_prefix("### ", cx);
    }

    pub(crate) fn md_bullet_list(
        &mut self,
        _: &MdBulletList,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_line_prefix("- ", cx);
    }

    pub(crate) fn md_ordered_list_action(
        &mut self,
        _: &MdOrderedList,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_ordered_list(cx);
    }

    pub(crate) fn md_task_list(
        &mut self,
        _: &MdTaskList,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_line_prefix("- [ ] ", cx);
    }

    pub(crate) fn md_block_quote(
        &mut self,
        _: &MdBlockQuote,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_line_prefix("> ", cx);
    }

    pub(crate) fn md_code_block_action(
        &mut self,
        _: &MdCodeBlock,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_code_block(cx);
    }

    pub(crate) fn md_link(
        &mut self,
        _: &MdLink,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_link_or_image(false, cx);
    }

    pub(crate) fn md_image(
        &mut self,
        _: &MdImage,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_link_or_image(true, cx);
    }

    pub(crate) fn md_horizontal_rule_action(
        &mut self,
        _: &MdHorizontalRule,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.md_guard(w) {
            return;
        }
        self.md_horizontal_rule(cx);
    }
}

/// One buffer edit planned by a Markdown formatting command: replace
/// `range` with `text`, then select `anchor..head` (a caret when equal).
/// The planners below are pure functions of the rope and the ordered
/// selection, so the commands can be tested without a window.
#[derive(Debug, PartialEq, Eq)]
struct MdEdit {
    range: Range<usize>,
    text: String,
    anchor: usize,
    head: usize,
}

/// `Buffer::slice_bytes` over a bare rope.
fn slice_bytes(rope: &Rope, range: Range<usize>) -> String {
    let total = rope.len_bytes();
    let start = range.start.min(total);
    let end = range.end.min(total);
    if start >= end {
        return String::new();
    }
    rope.byte_slice(start..end).to_string()
}

/// `byte` clamped to the rope and moved back to the start of the char it
/// falls inside. Selections and preview hit-tests can point past the end
/// or into a multi-byte char, and ropey panics on either.
fn snap_byte(rope: &Rope, byte: usize) -> usize {
    let byte = byte.min(rope.len_bytes());
    rope.char_to_byte(rope.byte_to_char(byte))
}

fn snap_range(rope: &Rope, r: Range<usize>) -> Range<usize> {
    snap_byte(rope, r.start)..snap_byte(rope, r.end)
}

/// Whether the rope holds exactly the bytes of `s` starting at `start`.
fn bytes_at(rope: &Rope, start: usize, s: &str) -> bool {
    s.bytes()
        .enumerate()
        .all(|(i, b)| start.checked_add(i).and_then(|at| rope.get_byte(at)) == Some(b))
}

/// Bold / italic / strike / inline-code: wrap the selection in
/// `prefix`/`suffix`, or unwrap it when the markers are already there.
fn plan_wrap_inline(
    rope: &Rope,
    r: Range<usize>,
    prefix: &str,
    suffix: &str,
    placeholder: &str,
) -> MdEdit {
    let r = snap_range(rope, r);
    let selected = slice_bytes(rope, r.clone());
    let total = rope.len_bytes();
    let has_selection = r.start != r.end;

    // Toggle off, case A: selection itself includes the markers — the
    // user picked "**foo**" wholesale.
    if has_selection
        && selected.starts_with(prefix)
        && selected.ends_with(suffix)
        && selected.len() >= prefix.len() + suffix.len()
    {
        let inner = selected[prefix.len()..selected.len() - suffix.len()].to_string();
        let head = r.start + inner.len();
        return MdEdit { anchor: r.start, head, range: r, text: inner };
    }

    // Toggle off, case B: selection is the inner text between existing
    // markers — happens when the user double-clicks a word inside
    // "**foo**" (preview-mode selection covers the rendered "foo", not
    // the asterisks). Strip the surrounding markers and keep the same
    // visible text selected.
    // Compared byte by byte: the byte before the selection can be the
    // tail of a multi-byte char, which a rope slice can't start at.
    if has_selection
        && r.start >= prefix.len()
        && r.end + suffix.len() <= total
        && bytes_at(rope, r.start - prefix.len(), prefix)
        && bytes_at(rope, r.end, suffix)
    {
        let outer_start = r.start - prefix.len();
        let outer_end = r.end + suffix.len();
        let head = outer_start + selected.len();
        return MdEdit {
            range: outer_start..outer_end,
            text: selected,
            anchor: outer_start,
            head,
        };
    }

    if !has_selection {
        // Insert placeholder so a fresh `Bold` click lands selecting the
        // word "bold text" (or whatever the placeholder is) for the user
        // to type over.
        let anchor = r.start + prefix.len();
        return MdEdit {
            range: r,
            text: format!("{prefix}{placeholder}{suffix}"),
            anchor,
            head: anchor + placeholder.len(),
        };
    }

    // Multi-paragraph wrap: selection spans a blank-line gap, which
    // pulldown-cmark reads as separate paragraphs. A naive
    // `{prefix}{selected}{suffix}` would put markers around the whole
    // span, breaking across the gap (the blank line closes the first
    // paragraph's bold, the prefix on para 2 looks like an unrelated
    // start). Wrap each paragraph individually instead, preserving
    // the original blank-line gaps verbatim.
    let paragraphs = paragraph_ranges_in(&selected);
    if paragraphs.len() > 1 {
        let mut out = String::with_capacity(
            selected.len() + paragraphs.len() * (prefix.len() + suffix.len()),
        );
        let mut cursor = 0usize;
        for p in &paragraphs {
            if p.start > cursor {
                out.push_str(&selected[cursor..p.start]);
            }
            out.push_str(prefix);
            out.push_str(&selected[p.start..p.end]);
            out.push_str(suffix);
            cursor = p.end;
        }
        if cursor < selected.len() {
            out.push_str(&selected[cursor..]);
        }
        let head = r.start + out.len();
        return MdEdit { anchor: r.start, head, range: r, text: out };
    }

    // Single-paragraph wrap (or selection with no recognisable content
    // — fall back to wrap-whole for safety).
    let anchor = r.start + prefix.len();
    let head = anchor + selected.len();
    MdEdit {
        range: r,
        text: format!("{prefix}{selected}{suffix}"),
        anchor,
        head,
    }
}

/// Range of bytes covering the lines touched by the selection `r`.
/// Mirrors the line-block math used by `apply_line_comment` so the two
/// behave the same: a selection that ends exactly on a line boundary
/// excludes the trailing blank line, matching NPP's "current line range"
/// convention.
fn line_block_range(rope: &Rope, r: Range<usize>) -> Option<Range<usize>> {
    let total_lines = rope.len_lines();
    if total_lines == 0 {
        return None;
    }
    let r = snap_range(rope, r);
    let first = rope.byte_to_line(r.start);
    let last_raw = rope.byte_to_line(r.end);
    let last = if r.end > r.start
        && last_raw > first
        && rope.line_to_byte(last_raw) == r.end
    {
        last_raw - 1
    } else {
        last_raw
    };
    let block_start = rope.line_to_byte(first);
    let block_end = if last + 1 < total_lines {
        rope.line_to_byte(last + 1)
    } else {
        rope.len_bytes()
    };
    Some(block_start..block_end)
}

/// `text` split after each CRLF, CR or LF, each line keeping its line
/// ending: `str::split_inclusive` for all three of Notepad++'s EOLs.
fn split_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let end = match b {
            b'\n' => i + 1,
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => i + 2,
            b'\r' => i + 1,
            _ => {
                i += 1;
                continue;
            }
        };
        if let Some(line) = text.get(start..end) {
            lines.push(line);
        }
        start = end;
        i = end;
    }
    if let Some(rest) = text.get(start..).filter(|rest| !rest.is_empty()) {
        lines.push(rest);
    }
    lines
}

/// Selects the rewritten line block, or `None` when nothing changes.
fn plan_block_rewrite(range: Range<usize>, block: &str, new_block: String) -> Option<MdEdit> {
    if new_block == block {
        return None;
    }
    let head = range.start + new_block.len();
    Some(MdEdit { anchor: range.start, head, range, text: new_block })
}

/// Headings, bullet list, task list, block quote: toggle `prefix` on
/// every non-empty line of the selection's line block.
fn plan_line_prefix(rope: &Rope, r: Range<usize>, prefix: &str) -> Option<MdEdit> {
    let range = line_block_range(rope, r)?;
    let block = rope.byte_slice(range.clone()).to_string();

    let all_prefixed = split_lines(&block)
        .into_iter()
        .filter(|l| !l.trim_end_matches(['\n', '\r']).is_empty())
        .all(|l| {
            let body = l.trim_end_matches(['\n', '\r']);
            body.starts_with(prefix)
        });
    let any_non_empty = split_lines(&block)
        .into_iter()
        .any(|l| !l.trim_end_matches(['\n', '\r']).is_empty());

    let new_block: String = if all_prefixed && any_non_empty {
        let mut out = String::new();
        for line in split_lines(&block) {
            let body = line.trim_end_matches(['\n', '\r']);
            if body.is_empty() {
                out.push_str(line);
            } else if let Some(stripped) = line.strip_prefix(prefix) {
                out.push_str(stripped);
            } else {
                out.push_str(line);
            }
        }
        out
    } else {
        let mut out = String::new();
        for line in split_lines(&block) {
            let body = line.trim_end_matches(['\n', '\r']);
            if body.is_empty() {
                out.push_str(line);
            } else {
                out.push_str(prefix);
                out.push_str(line);
            }
        }
        out
    };
    plan_block_rewrite(range, &block, new_block)
}

/// `line` without its leading `\d+\. `, or `None` when it has none.
fn strip_ordered_prefix(line: &str) -> Option<&str> {
    let body = line;
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    if i + 1 >= bytes.len() {
        return None;
    }
    if bytes[i] != b'.' || bytes[i + 1] != b' ' {
        return None;
    }
    Some(&body[i + 2..])
}

/// Ordered list: number every non-empty line of the selection's line
/// block, or strip the numbers when every line already has one.
fn plan_ordered_list(rope: &Rope, r: Range<usize>) -> Option<MdEdit> {
    let range = line_block_range(rope, r)?;
    let block = rope.byte_slice(range.clone()).to_string();

    let all_numbered = split_lines(&block)
        .into_iter()
        .filter(|l| !l.trim_end_matches(['\n', '\r']).is_empty())
        .all(|l| strip_ordered_prefix(l).is_some());
    let any_non_empty = split_lines(&block)
        .into_iter()
        .any(|l| !l.trim_end_matches(['\n', '\r']).is_empty());

    let new_block: String = if all_numbered && any_non_empty {
        let mut out = String::new();
        for line in split_lines(&block) {
            let body = line.trim_end_matches(['\n', '\r']);
            if body.is_empty() {
                out.push_str(line);
            } else if let Some(stripped) = strip_ordered_prefix(line) {
                out.push_str(stripped);
            } else {
                out.push_str(line);
            }
        }
        out
    } else {
        let mut out = String::new();
        let mut n: usize = 1;
        for line in split_lines(&block) {
            let body = line.trim_end_matches(['\n', '\r']);
            if body.is_empty() {
                out.push_str(line);
            } else {
                out.push_str(&format!("{n}. "));
                out.push_str(line);
                n += 1;
            }
        }
        out
    };
    plan_block_rewrite(range, &block, new_block)
}

/// Whether a block inserted over `r` needs a line break before it (it
/// doesn't start a line) and after it (more text follows on the line).
fn block_insert_breaks(rope: &Rope, r: &Range<usize>) -> (bool, bool) {
    let is_eol = |at: usize| matches!(rope.get_byte(at), Some(b'\n' | b'\r'));
    let total = rope.len_bytes();
    let needs_lead = r.start > 0 && !is_eol(r.start - 1);
    let needs_tail = r.end < total && !is_eol(r.end);
    (needs_lead, needs_tail)
}

/// Fenced code block around the selection, or an empty one with the
/// caret inside. Line breaks are the document's `eol`.
fn plan_code_block(rope: &Rope, r: Range<usize>, eol: &str) -> MdEdit {
    let r = snap_range(rope, r);
    let selected = slice_bytes(rope, r.clone());
    let (needs_lead, needs_tail) = block_insert_breaks(rope, &r);
    let lead = if needs_lead { eol } else { "" };
    let tail = if needs_tail { eol } else { "" };

    if selected.is_empty() {
        // Place caret on the empty line between the fences.
        let caret = r.start + lead.len() + "```".len() + eol.len();
        return MdEdit {
            range: r,
            text: format!("{lead}```{eol}{eol}```{eol}{tail}"),
            anchor: caret,
            head: caret,
        };
    }
    let body = if selected.ends_with(['\n', '\r']) {
        selected
    } else {
        format!("{selected}{eol}")
    };
    let text = format!("{lead}```{eol}{body}```{eol}{tail}");
    let head = r.start + text.len();
    MdEdit { anchor: r.start, head, range: r, text }
}

/// `[text](url)` / `![alt](url)`, selecting the part to type next.
fn plan_link_or_image(rope: &Rope, r: Range<usize>, image: bool) -> MdEdit {
    let r = snap_range(rope, r);
    let selected = slice_bytes(rope, r.clone());
    let bracket_open = if image { "![" } else { "[" };
    let url_placeholder = "url";
    let label_placeholder = if image { "alt" } else { "text" };

    if r.start == r.end {
        let label_start = r.start + bracket_open.len();
        return MdEdit {
            range: r,
            text: format!("{bracket_open}{label_placeholder}]({url_placeholder})"),
            anchor: label_start,
            head: label_start + label_placeholder.len(),
        };
    }
    // After insertion, the URL placeholder lives at:
    //   r.start + bracket_open.len() + selected.len() + "](".len()
    // and runs for `url_placeholder.len()` bytes.
    let url_start = r.start + bracket_open.len() + selected.len() + "](".len();
    MdEdit {
        range: r,
        text: format!("{bracket_open}{selected}]({url_placeholder})"),
        anchor: url_start,
        head: url_start + url_placeholder.len(),
    }
}

/// `---` on its own line, caret after it.
fn plan_horizontal_rule(rope: &Rope, r: Range<usize>, eol: &str) -> MdEdit {
    let r = snap_range(rope, r);
    let (needs_lead, needs_tail) = block_insert_breaks(rope, &r);
    let mut text = String::new();
    if needs_lead {
        text.push_str(eol);
    }
    text.push_str("---");
    text.push_str(eol);
    if needs_tail {
        text.push_str(eol);
    }
    let caret = r.start + text.len();
    MdEdit { range: r, text, anchor: caret, head: caret }
}

/// Image markup dropped onto the preview at `source_byte`, caret after it.
fn plan_insert_image(rope: &Rope, source_byte: usize, snippet: String) -> MdEdit {
    let byte = snap_byte(rope, source_byte);
    let caret = byte + snippet.len();
    MdEdit { range: byte..byte, text: snippet, anchor: caret, head: caret }
}

/// Image markup appended on its own line at the end of the buffer.
fn plan_append_image(rope: &Rope, snippet: &str, eol: &str) -> MdEdit {
    let total = rope.len_bytes();
    let (needs_lead, _) = block_insert_breaks(rope, &(total..total));
    let text = if needs_lead {
        format!("{eol}{snippet}{eol}")
    } else {
        format!("{snippet}{eol}")
    };
    let caret = total + text.len();
    MdEdit { range: total..total, text, anchor: caret, head: caret }
}

/// Find the byte ranges of content paragraphs inside `text`. A blank
/// line (a line ending, optional whitespace, and another line ending;
/// CRLF, CR and LF all count) closes one paragraph and opens the next.
/// Returned ranges cover only the content bytes; trailing whitespace at paragraph end is trimmed so
/// the suffix marker doesn't end up after stray spaces. Blank-line
/// gaps between paragraphs aren't returned but their bytes can be
/// recovered from the gaps between consecutive ranges.
///
/// Used by `md_wrap_inline` to wrap each paragraph individually for
/// multi-paragraph selections — otherwise the suffix marker on the
/// first paragraph never closes (the blank-line gap breaks the inline
/// emphasis under CommonMark, so `**A\n\nB**` renders as a literal
/// `**A` followed by `B**` rather than two bold paragraphs).
fn paragraph_ranges_in(text: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    let mut paragraphs: Vec<std::ops::Range<usize>> = Vec::new();
    let mut content_start: Option<usize> = None;
    let mut line_start = 0usize;
    for line in split_lines(text) {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if let Some(start) = content_start.take() {
                // Trim trailing ASCII whitespace so the suffix sits
                // tight against the last non-whitespace byte.
                let mut end = line_start;
                while end > start && bytes[end - 1].is_ascii_whitespace() {
                    end -= 1;
                }
                if end > start {
                    paragraphs.push(start..end);
                }
            }
        } else if content_start.is_none() {
            content_start = Some(line_start);
        }
        line_start += line.len();
    }
    if let Some(start) = content_start {
        let mut end = bytes.len();
        while end > start && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if end > start {
            paragraphs.push(start..end);
        }
    }
    paragraphs
}

#[cfg(test)]
mod paragraph_split_tests {
    use super::paragraph_ranges_in;

    #[test]
    fn single_paragraph_returns_one_range() {
        let r = paragraph_ranges_in("hello world");
        assert_eq!(r.len(), 1);
        assert_eq!(&"hello world"[r[0].clone()], "hello world");
    }

    #[test]
    fn two_paragraphs_split_on_blank_line() {
        let s = "para one\n\npara two";
        let r = paragraph_ranges_in(s);
        assert_eq!(r.len(), 2);
        assert_eq!(&s[r[0].clone()], "para one");
        assert_eq!(&s[r[1].clone()], "para two");
    }

    #[test]
    fn soft_break_stays_in_same_paragraph() {
        let s = "line one\nline two\n\nnext para";
        let r = paragraph_ranges_in(s);
        assert_eq!(r.len(), 2);
        assert_eq!(&s[r[0].clone()], "line one\nline two");
        assert_eq!(&s[r[1].clone()], "next para");
    }

    #[test]
    fn multiple_blank_lines_collapse_to_single_break() {
        let s = "first\n\n\n\nsecond";
        let r = paragraph_ranges_in(s);
        assert_eq!(r.len(), 2);
        assert_eq!(&s[r[0].clone()], "first");
        assert_eq!(&s[r[1].clone()], "second");
    }

    #[test]
    fn whitespace_only_blank_lines_count_as_breaks() {
        let s = "alpha\n   \n\t\nbeta";
        let r = paragraph_ranges_in(s);
        assert_eq!(r.len(), 2);
        assert_eq!(&s[r[0].clone()], "alpha");
        assert_eq!(&s[r[1].clone()], "beta");
    }

    #[test]
    fn trailing_whitespace_trimmed_from_paragraph() {
        let s = "content   \n\nnext";
        let r = paragraph_ranges_in(s);
        assert_eq!(r.len(), 2);
        assert_eq!(&s[r[0].clone()], "content");
    }

    #[test]
    fn empty_input_yields_no_ranges() {
        assert!(paragraph_ranges_in("").is_empty());
        assert!(paragraph_ranges_in("\n\n\n").is_empty());
        assert!(paragraph_ranges_in("   \n\t\n").is_empty());
    }
}

#[cfg(test)]
mod format_plan_tests {
    use super::*;

    /// `text` after `edit`, and the text the edit leaves selected.
    fn apply(text: &str, edit: &MdEdit) -> (String, String) {
        assert!(
            text.is_char_boundary(edit.range.start) && text.is_char_boundary(edit.range.end),
            "edit range {:?} is off a char boundary of {text:?}",
            edit.range
        );
        let mut rope = Rope::from_str(text);
        let start = rope.byte_to_char(edit.range.start);
        let end = rope.byte_to_char(edit.range.end);
        rope.remove(start..end);
        rope.insert(start, &edit.text);
        let after = rope.to_string();
        let sel = edit.anchor.min(edit.head)..edit.anchor.max(edit.head);
        let selected = after.get(sel).map(str::to_string);
        (after, selected.unwrap_or_else(|| "<selection off a char boundary>".into()))
    }

    fn wrap(text: &str, r: Range<usize>) -> (String, String) {
        let edit = plan_wrap_inline(&Rope::from_str(text), r, "**", "**", "bold text");
        apply(text, &edit)
    }

    fn prefix(text: &str, r: Range<usize>, p: &str) -> String {
        match plan_line_prefix(&Rope::from_str(text), r, p) {
            Some(edit) => apply(text, &edit).0,
            None => text.to_string(),
        }
    }

    fn ordered(text: &str, r: Range<usize>) -> String {
        match plan_ordered_list(&Rope::from_str(text), r) {
            Some(edit) => apply(text, &edit).0,
            None => text.to_string(),
        }
    }

    #[test]
    fn wrap_and_unwrap_inline() {
        assert_eq!(wrap("hello", 0..5), ("**hello**".into(), "hello".into()));
        assert_eq!(wrap("**hello**", 0..9), ("hello".into(), "hello".into()));
        assert_eq!(wrap("**hello**", 2..7), ("hello".into(), "hello".into()));
        assert_eq!(wrap("", 0..0), ("**bold text**".into(), "bold text".into()));
        assert_eq!(wrap("a\n\nb", 0..4).0, "**a**\n\n**b**");
    }

    #[test]
    fn line_prefix_and_ordered_list() {
        assert_eq!(prefix("a\nb\n", 0..3, "# "), "# a\n# b\n");
        assert_eq!(prefix("# a\n# b\n", 0..7, "# "), "a\nb\n");
        assert_eq!(prefix("a\n\nb", 0..4, "- "), "- a\n\n- b");
        assert_eq!(ordered("a\n\nb", 0..4), "1. a\n\n2. b");
        assert_eq!(ordered("1. a\n\n2. b", 0..10), "a\n\nb");
    }

    #[test]
    fn block_inserts_land_on_their_own_lines() {
        let rope = Rope::from_str("abc");
        let edit = plan_code_block(&rope, 3..3, "\n");
        assert_eq!(apply("abc", &edit).0, "abc\n```\n\n```\n");
        assert_eq!((edit.anchor, edit.head), (8, 8));
        let edit = plan_code_block(&rope, 0..3, "\n");
        assert_eq!(apply("abc", &edit), ("```\nabc\n```\n".into(), "```\nabc\n```\n".into()));
        let edit = plan_horizontal_rule(&Rope::from_str("ab"), 1..1, "\n");
        assert_eq!(apply("ab", &edit).0, "a\n---\n\nb");
    }

    #[test]
    fn links_and_images() {
        let rope = Rope::from_str("x");
        assert_eq!(apply("x", &plan_link_or_image(&rope, 0..1, false)), ("[x](url)".into(), "url".into()));
        assert_eq!(apply("x", &plan_link_or_image(&rope, 1..1, true)), ("x![alt](url)".into(), "alt".into()));
        let edit = plan_append_image(&Rope::from_str("abc"), "![i](p)", "\n");
        assert_eq!(apply("abc", &edit).0, "abc\n![i](p)\n");
        let edit = plan_insert_image(&Rope::from_str("abc"), 1, "![i](p)".into());
        assert_eq!(apply("abc", &edit).0, "a![i](p)bc");
    }

    #[test]
    fn formatting_next_to_multibyte_chars() {
        // The byte before / after the selection is the tail / head of a
        // multi-byte char.
        assert_eq!(wrap("中x中", 3..4), ("中**x**中".into(), "x".into()));
        assert_eq!(wrap("aax中", 2..3), ("aa**x**中".into(), "x".into()));
        assert_eq!(wrap("é**x**👍", 4..5), ("éx👍".into(), "x".into()));
        let edit = plan_code_block(&Rope::from_str("café"), 5..5, "\n");
        assert_eq!(apply("café", &edit).0, "café\n```\n\n```\n");
        let edit = plan_code_block(&Rope::from_str("é"), 0..0, "\n");
        assert_eq!(apply("é", &edit).0, "```\n\n```\n\né");
        let edit = plan_horizontal_rule(&Rope::from_str("👍x👍"), 4..5, "\n");
        assert_eq!(apply("👍x👍", &edit).0, "👍\n---\n\n👍");
        let edit = plan_append_image(&Rope::from_str("café"), "![i](p)", "\n");
        assert_eq!(apply("café", &edit).0, "café\n![i](p)\n");
    }

    #[test]
    fn selection_past_end_or_inside_a_char() {
        // Past the end: the selection snaps to the end of the buffer.
        assert_eq!(prefix("ab", 10..10, "# "), "# ab");
        assert_eq!(ordered("ab", 5..9), "1. ab");
        assert_eq!(wrap("ab", 5..7), ("ab**bold text**".into(), "bold text".into()));
        let edit = plan_code_block(&Rope::from_str("ab"), 9..9, "\n");
        assert_eq!(apply("ab", &edit).0, "ab\n```\n\n```\n");
        let edit = plan_horizontal_rule(&Rope::from_str("ab"), 3..4, "\n");
        assert_eq!(apply("ab", &edit).0, "ab\n---\n");
        let edit = plan_link_or_image(&Rope::from_str(""), 1..3, false);
        assert_eq!(apply("", &edit), ("[text](url)".into(), "text".into()));
        let edit = plan_insert_image(&Rope::from_str("ab"), 7, "![i](p)".into());
        assert_eq!(apply("ab", &edit).0, "ab![i](p)");

        // Inside a char: each end snaps back to the start of its char.
        assert_eq!(wrap("é", 1..2), ("**é**".into(), "é".into()));
        assert_eq!(wrap("é", 1..1), ("**bold text**é".into(), "bold text".into()));
        assert_eq!(prefix("中", 2..2, "> "), "> 中");
        let edit = plan_code_block(&Rope::from_str("é"), 1..1, "\n");
        assert_eq!(apply("é", &edit).0, "```\n\n```\n\né");
        let edit = plan_link_or_image(&Rope::from_str("👍"), 1..3, true);
        assert_eq!(apply("👍", &edit).0, "![alt](url)👍");
        let edit = plan_insert_image(&Rope::from_str("👍"), 2, "![i](p)".into());
        assert_eq!(apply("👍", &edit).0, "![i](p)👍");
    }

    fn code_block(text: &str, r: Range<usize>, eol: &str) -> String {
        apply(text, &plan_code_block(&Rope::from_str(text), r, eol)).0
    }

    fn rule(text: &str, r: Range<usize>, eol: &str) -> String {
        apply(text, &plan_horizontal_rule(&Rope::from_str(text), r, eol)).0
    }

    fn append_image(text: &str, eol: &str) -> String {
        apply(text, &plan_append_image(&Rope::from_str(text), "![i](p)", eol)).0
    }

    #[test]
    fn crlf_and_cr_only_documents() {
        // Every CR-terminated line is a line.
        assert_eq!(prefix("a\rb\r", 0..3, "# "), "# a\r# b\r");
        assert_eq!(prefix("# a\r# b", 0..7, "# "), "a\rb");
        assert_eq!(ordered("a\r\rb", 0..4), "1. a\r\r2. b");
        assert_eq!(wrap("a\r\rb", 0..4).0, "**a**\r\r**b**");
        assert_eq!(prefix("a\r\nb\r\n", 0..4, "- [ ] "), "- [ ] a\r\n- [ ] b\r\n");
        assert_eq!(ordered("a\r\n\r\nb", 0..6), "1. a\r\n\r\n2. b");
        assert_eq!(wrap("a\r\n\r\nb", 0..6).0, "**a**\r\n\r\n**b**");

        // Inserted line breaks match the document's, and a CR counts as
        // the line break before or after the insertion.
        assert_eq!(code_block("ab\r\ncd", 2..2, "\r\n"), "ab\r\n```\r\n\r\n```\r\n\r\ncd");
        let edit = plan_code_block(&Rope::from_str("ab\r\ncd"), 2..2, "\r\n");
        assert_eq!((edit.anchor, edit.head), (9, 9));
        assert_eq!(code_block("ab\r", 3..3, "\r"), "ab\r```\r\r```\r");
        assert_eq!(code_block("x\r\ny", 0..1, "\r\n"), "```\r\nx\r\n```\r\n\r\ny");
        assert_eq!(rule("ab\r\ncd", 4..4, "\r\n"), "ab\r\n---\r\n\r\ncd");
        assert_eq!(rule("ab\rcd", 2..2, "\r"), "ab\r---\r\rcd");
        assert_eq!(append_image("abc\r\n", "\r\n"), "abc\r\n![i](p)\r\n");
        assert_eq!(append_image("abc\r", "\r"), "abc\r![i](p)\r");
    }
}
