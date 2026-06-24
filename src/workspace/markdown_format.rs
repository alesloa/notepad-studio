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

use gpui::{Context, Window, px};
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
        let total = self.buffer().rope.len_bytes();
        let byte = source_byte.min(total);
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
        let total = self.buffer().rope.len_bytes();
        let byte = source_byte.min(total);
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
        let total = self.buffer().rope.len_bytes();
        let byte = source_byte.min(total);
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
        let total = self.buffer().rope.len_bytes();
        let byte = source_byte.min(total);
        self.replace_byte_range(byte..byte, &snippet, cx);
        let tab = self.active_tab_mut();
        let new_pos = byte + snippet.len();
        tab.selection.anchor = new_pos;
        tab.selection.head = new_pos;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        cx.notify();
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
        let total = self.buffer().rope.len_bytes();
        let needs_lead = if total == 0 {
            false
        } else {
            let last = self.buffer().slice_bytes(total.saturating_sub(1)..total);
            !last.ends_with('\n')
        };
        let final_snippet = if needs_lead {
            format!("\n{snippet}\n")
        } else {
            format!("{snippet}\n")
        };
        self.replace_byte_range(total..total, &final_snippet, cx);
        let tab = self.active_tab_mut();
        let new_pos = total + final_snippet.len();
        tab.selection.anchor = new_pos;
        tab.selection.head = new_pos;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        cx.notify();
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
        let total = self.buffer().rope.len_bytes();
        let new_byte = byte.min(total);
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

    /// Wrap the current selection (or insert at the caret) with `prefix`/`suffix`.
    /// Toggles off when the selection is already exactly `prefix..suffix`.
    fn md_wrap_inline(
        &mut self,
        prefix: &str,
        suffix: &str,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) {
        let sel = self.selection();
        let r = sel.ordered();
        let selected = self.buffer().slice_bytes(r.clone());
        let total = self.buffer().rope.len_bytes();

        // Toggle off, case A: selection itself includes the markers — the
        // user picked "**foo**" wholesale.
        if !sel.is_empty()
            && selected.starts_with(prefix)
            && selected.ends_with(suffix)
            && selected.len() >= prefix.len() + suffix.len()
        {
            let inner = &selected[prefix.len()..selected.len() - suffix.len()];
            let inner_owned = inner.to_string();
            let inner_len = inner_owned.len();
            self.replace_byte_range(r.clone(), &inner_owned, cx);
            let tab = self.active_tab_mut();
            tab.selection.anchor = r.start;
            tab.selection.head = r.start + inner_len;
            tab.desired_col_x = None;
            cx.notify();
            return;
        }

        // Toggle off, case B: selection is the inner text between existing
        // markers — happens when the user double-clicks a word inside
        // "**foo**" (preview-mode selection covers the rendered "foo", not
        // the asterisks). Strip the surrounding markers and keep the same
        // visible text selected.
        if !sel.is_empty()
            && r.start >= prefix.len()
            && r.end + suffix.len() <= total
        {
            let pre_bytes = self
                .buffer()
                .slice_bytes(r.start - prefix.len()..r.start);
            let suf_bytes = self
                .buffer()
                .slice_bytes(r.end..r.end + suffix.len());
            if pre_bytes == prefix && suf_bytes == suffix {
                let outer_start = r.start - prefix.len();
                let outer_end = r.end + suffix.len();
                let inner_owned = selected.clone();
                let inner_len = inner_owned.len();
                self.replace_byte_range(outer_start..outer_end, &inner_owned, cx);
                let tab = self.active_tab_mut();
                tab.selection.anchor = outer_start;
                tab.selection.head = outer_start + inner_len;
                tab.desired_col_x = None;
                cx.notify();
                return;
            }
        }

        if sel.is_empty() {
            // Insert placeholder so a fresh `Bold` click lands selecting the
            // word "bold text" (or whatever the placeholder is) for the user
            // to type over.
            let new_text = format!("{prefix}{placeholder}{suffix}");
            self.replace_byte_range(r.clone(), &new_text, cx);
            let tab = self.active_tab_mut();
            let inner_start = r.start + prefix.len();
            let inner_end = inner_start + placeholder.len();
            tab.selection.anchor = inner_start;
            tab.selection.head = inner_end;
            tab.desired_col_x = None;
            cx.notify();
            return;
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
            let new_len = out.len();
            self.replace_byte_range(r.clone(), &out, cx);
            let tab = self.active_tab_mut();
            tab.selection.anchor = r.start;
            tab.selection.head = r.start + new_len;
            tab.desired_col_x = None;
            cx.notify();
            return;
        }

        // Single-paragraph wrap (or selection with no recognisable content
        // — fall back to wrap-whole for safety).
        let new_text = format!("{prefix}{selected}{suffix}");
        let inner_start = r.start + prefix.len();
        let inner_end = inner_start + selected.len();
        self.replace_byte_range(r.clone(), &new_text, cx);
        let tab = self.active_tab_mut();
        tab.selection.anchor = inner_start;
        tab.selection.head = inner_end;
        tab.desired_col_x = None;
        cx.notify();
    }

    /// Range of bytes covering the lines touched by the current selection.
    /// Mirrors the line-block math used by `apply_line_comment` so the two
    /// behave the same: a selection that ends exactly on a line boundary
    /// excludes the trailing blank line, matching NPP's "current line range"
    /// convention.
    fn line_block_range(&self) -> Option<(std::ops::Range<usize>, usize)> {
        let sel = self.selection();
        let r = sel.ordered();
        let rope = &self.buffer().rope;
        let total_lines = rope.len_lines();
        if total_lines == 0 {
            return None;
        }
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
        Some((block_start..block_end, total_lines))
    }

    /// Add or remove `prefix` from the start of every non-empty line in the
    /// selection's line block. Toggles off only when every non-empty line
    /// already begins with the prefix.
    fn md_line_prefix(&mut self, prefix: &str, cx: &mut Context<Self>) {
        let Some((range, _)) = self.line_block_range() else {
            return;
        };
        let block = self
            .buffer()
            .rope
            .byte_slice(range.clone())
            .to_string();

        let all_prefixed = block
            .split_inclusive('\n')
            .filter(|l| !l.trim_end_matches(['\n', '\r']).is_empty())
            .all(|l| {
                let body = l.trim_end_matches(['\n', '\r']);
                body.starts_with(prefix)
            });
        let any_non_empty = block
            .split_inclusive('\n')
            .any(|l| !l.trim_end_matches(['\n', '\r']).is_empty());

        let new_block: String = if all_prefixed && any_non_empty {
            let mut out = String::new();
            for line in block.split_inclusive('\n') {
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
            for line in block.split_inclusive('\n') {
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

        if new_block == block {
            return;
        }
        let new_len = new_block.len();
        let block_start = range.start;
        self.replace_byte_range(range, &new_block, cx);
        let tab = self.active_tab_mut();
        tab.selection.anchor = block_start;
        tab.selection.head = block_start + new_len;
        tab.desired_col_x = None;
        cx.notify();
    }

    /// Number every non-empty line in the selection block (`1. `, `2. `, …).
    /// Toggles off when every non-empty line already begins with `\d+\. `.
    fn md_ordered_list(&mut self, cx: &mut Context<Self>) {
        let Some((range, _)) = self.line_block_range() else {
            return;
        };
        let block = self
            .buffer()
            .rope
            .byte_slice(range.clone())
            .to_string();

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

        let all_numbered = block
            .split_inclusive('\n')
            .filter(|l| !l.trim_end_matches(['\n', '\r']).is_empty())
            .all(|l| strip_ordered_prefix(l).is_some());
        let any_non_empty = block
            .split_inclusive('\n')
            .any(|l| !l.trim_end_matches(['\n', '\r']).is_empty());

        let new_block: String = if all_numbered && any_non_empty {
            let mut out = String::new();
            for line in block.split_inclusive('\n') {
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
            let mut n: u32 = 1;
            for line in block.split_inclusive('\n') {
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

        if new_block == block {
            return;
        }
        let new_len = new_block.len();
        let block_start = range.start;
        self.replace_byte_range(range, &new_block, cx);
        let tab = self.active_tab_mut();
        tab.selection.anchor = block_start;
        tab.selection.head = block_start + new_len;
        tab.desired_col_x = None;
        cx.notify();
    }

    /// Wrap the selection (or insert at the caret) with a fenced code block.
    /// Always lands on its own lines.
    fn md_code_block(&mut self, cx: &mut Context<Self>) {
        let sel = self.selection();
        let r = sel.ordered();
        let selected = self.buffer().slice_bytes(r.clone());
        let buf = self.buffer();
        let total = buf.rope.len_bytes();

        // Insert leading `\n` only if we aren't already at the start of a line.
        let needs_lead = r.start > 0
            && self
                .buffer()
                .slice_bytes(r.start.saturating_sub(1)..r.start)
                != "\n";
        // Insert trailing `\n` only if we aren't already followed by `\n`.
        let after = if r.end < total {
            self.buffer()
                .slice_bytes(r.end..(r.end + 1).min(total))
        } else {
            String::new()
        };
        let needs_tail = !after.is_empty() && after != "\n";

        let lead = if needs_lead { "\n" } else { "" };
        let tail = if needs_tail { "\n" } else { "" };

        let new_text;
        let caret_at;
        if selected.is_empty() {
            new_text = format!("{lead}```\n\n```\n{tail}");
            // Place caret on the empty line between the fences.
            caret_at = r.start + lead.len() + "```\n".len();
        } else {
            let body = if selected.ends_with('\n') {
                selected.clone()
            } else {
                format!("{selected}\n")
            };
            new_text = format!("{lead}```\n{body}```\n{tail}");
            caret_at = r.start + new_text.len();
        }
        let new_len = new_text.len();
        self.replace_byte_range(r.clone(), &new_text, cx);
        let tab = self.active_tab_mut();
        if selected.is_empty() {
            tab.selection.collapse_to(caret_at);
        } else {
            tab.selection.anchor = r.start;
            tab.selection.head = r.start + new_len;
        }
        tab.desired_col_x = None;
        cx.notify();
    }

    /// Insert / wrap a markdown link or image.
    /// * No selection: `[text](url)` (or `![alt](url)`), with `text`/`alt` selected.
    /// * Selection: `[selected](url)` with the `url` placeholder selected so
    ///   the user can paste / type the URL straight away.
    fn md_link_or_image(&mut self, image: bool, cx: &mut Context<Self>) {
        let sel = self.selection();
        let r = sel.ordered();
        let selected = self.buffer().slice_bytes(r.clone());
        let bracket_open = if image { "![" } else { "[" };
        let url_placeholder = "url";
        let label_placeholder = if image { "alt" } else { "text" };

        if sel.is_empty() {
            let new_text = format!(
                "{bracket_open}{label_placeholder}]({url_placeholder})"
            );
            self.replace_byte_range(r.clone(), &new_text, cx);
            let tab = self.active_tab_mut();
            let label_start = r.start + bracket_open.len();
            let label_end = label_start + label_placeholder.len();
            tab.selection.anchor = label_start;
            tab.selection.head = label_end;
            tab.desired_col_x = None;
            cx.notify();
        } else {
            let new_text = format!("{bracket_open}{selected}]({url_placeholder})");
            // After insertion, the URL placeholder lives at:
            //   r.start + bracket_open.len() + selected.len() + "](".len()
            // and runs for `url_placeholder.len()` bytes.
            let url_start = r.start + bracket_open.len() + selected.len() + "](".len();
            let url_end = url_start + url_placeholder.len();
            self.replace_byte_range(r.clone(), &new_text, cx);
            let tab = self.active_tab_mut();
            tab.selection.anchor = url_start;
            tab.selection.head = url_end;
            tab.desired_col_x = None;
            cx.notify();
        }
    }

    /// Insert a horizontal rule (`---`) on its own line.
    fn md_horizontal_rule(&mut self, cx: &mut Context<Self>) {
        let sel = self.selection();
        let r = sel.ordered();
        let total = self.buffer().rope.len_bytes();
        let needs_lead = r.start > 0
            && self
                .buffer()
                .slice_bytes(r.start.saturating_sub(1)..r.start)
                != "\n";
        let after = if r.end < total {
            self.buffer()
                .slice_bytes(r.end..(r.end + 1).min(total))
        } else {
            String::new()
        };
        let needs_tail = !after.is_empty() && after != "\n";

        let mut new_text = String::new();
        if needs_lead {
            new_text.push('\n');
        }
        new_text.push_str("---\n");
        if needs_tail {
            new_text.push('\n');
        }
        let new_len = new_text.len();
        self.replace_byte_range(r.clone(), &new_text, cx);
        let tab = self.active_tab_mut();
        tab.selection.collapse_to(r.start + new_len);
        tab.desired_col_x = None;
        cx.notify();
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

/// Find the byte ranges of content paragraphs inside `text`. A blank
/// line (`\n` followed by optional whitespace and another `\n`) closes
/// one paragraph and opens the next. Returned ranges cover only the
/// content bytes; trailing whitespace at paragraph end is trimmed so
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
    loop {
        let mut line_end = line_start;
        while line_end < bytes.len() && bytes[line_end] != b'\n' {
            line_end += 1;
        }
        let is_blank = text[line_start..line_end].trim().is_empty();
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
        if line_end >= bytes.len() {
            break;
        }
        line_start = line_end + 1;
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
