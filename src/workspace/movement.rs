use gpui::{Context, Window, px};

use crate::actions::{
    BookmarkClearAll, BookmarkNext, BookmarkPrev, BookmarkToggle, EditPageDown, EditPageUp,
    MoveDocEnd, MoveDocStart, MoveDown, MoveLeft, MoveLineEnd, MoveLineStart, MoveRight, MoveUp,
    MoveWordLeft, MoveWordRight, SelectDocEnd, SelectDocStart, SelectDown, SelectLeft,
    SelectLineEnd, SelectLineStart, SelectPageDown, SelectPageUp, SelectRight, SelectUp,
    SelectWordLeft, SelectWordRight,
};
use crate::tab::InputFocus;
use crate::workspace::Workspace;

impl Workspace {
    // ---- Cursor movement actions -----------------------------------------------------------

    pub(crate) fn move_left(&mut self, _: &MoveLeft, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_cursor(-1, false, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, 0, -1, false, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_move(window, -1, false);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveLeft);
        let sel = self.selection();
        let target = if sel.is_empty() {
            self.prev_char_byte(sel.head)
        } else {
            sel.ordered().start
        };
        self.move_to_byte(target, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn move_right(&mut self, _: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_cursor(1, false, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, 0, 1, false, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_move(window, 1, false);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveRight);
        let sel = self.selection();
        let target = if sel.is_empty() {
            self.next_char_byte(sel.head)
        } else {
            sel.ordered().end
        };
        self.move_to_byte(target, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_cursor(-1, true, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, 0, -1, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_move(window, -1, true);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectLeft);
        let target = self.prev_char_byte(self.selection().head);
        self.move_to_byte(target, true, true);
        self.commit_caret(cx);
    }

    pub(crate) fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_cursor(1, true, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, 0, 1, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_move(window, 1, true);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectRight);
        let target = self.next_char_byte(self.selection().head);
        self.move_to_byte(target, true, true);
        self.commit_caret(cx);
    }

    pub(crate) fn move_up(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if self.autocomplete_context_valid(window) {
            self.autocomplete_nav(-1, cx);
            return;
        }
        self.close_autocomplete(cx);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_vertical(-1, false, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, -1, 0, false, cx);
            return;
        }
        if focus != InputFocus::Editor { return; }
        crate::macros::record(crate::macros::MacroEvent::MoveUp);
        if self.is_in_markdown_preview()
            && self.preview_move_vertical(-1, false, window, cx)
        {
            return;
        }
        self.move_vertical(-1, false, cx);
    }

    pub(crate) fn move_down(&mut self, _: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if self.autocomplete_context_valid(window) {
            self.autocomplete_nav(1, cx);
            return;
        }
        self.close_autocomplete(cx);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_vertical(1, false, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, 1, 0, false, cx);
            return;
        }
        if focus != InputFocus::Editor { return; }
        crate::macros::record(crate::macros::MacroEvent::MoveDown);
        if self.is_in_markdown_preview()
            && self.preview_move_vertical(1, false, window, cx)
        {
            return;
        }
        self.move_vertical(1, false, cx);
    }

    pub(crate) fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_vertical(-1, true, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, -1, 0, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.close_autocomplete(cx);
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectUp);
        if self.is_in_markdown_preview()
            && self.preview_move_vertical(-1, true, window, cx)
        {
            return;
        }
        self.move_vertical(-1, true, cx);
    }

    pub(crate) fn select_down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_vertical(1, true, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, 1, 0, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.close_autocomplete(cx);
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectDown);
        if self.is_in_markdown_preview()
            && self.preview_move_vertical(1, true, window, cx)
        {
            return;
        }
        self.move_vertical(1, true, cx);
    }

    /// Number of full visible lines in the viewport, minimum 1. Returns 1
    /// before the first paint completes so page-nav is still functional.
    fn page_size_lines(&self) -> i32 {
        let lh = self.viewport.line_height;
        if lh <= px(0.) {
            return 1;
        }
        lines_per_page(self.viewport.bounds.size.height / lh)
    }

    fn move_page(&mut self, dir: i32, extend: bool, cx: &mut Context<Self>) {
        self.close_autocomplete(cx);
        self.clear_block_selection();
        let lines = self.page_size_lines();
        let head = self.selection().head;
        let rope = &self.buffer().rope;
        let cur_line = line_at_byte(rope, head);
        let target_line = page_target_line(cur_line, dir, lines, rope.len_lines());
        let target_byte = rope.line_to_byte(target_line);
        // Scroll the viewport by the same number of lines so the cursor's
        // relative position is preserved (matches Notepad++ behavior).
        let lh = self.viewport.line_height;
        if lh > px(0.) {
            let new_top = self.scroll_top() + lh * (dir as f32) * (lines as f32);
            self.set_scroll_top(new_top, cx);
        }
        if extend {
            self.active_tab_mut().selection.head = target_byte;
        } else {
            self.active_tab_mut().selection.collapse_to(target_byte);
        }
        self.active_tab_mut().desired_col_x = None;
        self.caret_visible = true;
        self.ensure_cursor_visible(cx);
        cx.notify();
    }

    pub(crate) fn page_up(&mut self, _: &EditPageUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focus(window) == InputFocus::RichDocBody {
            self.rich_doc_move_page(-1, false, cx);
            return;
        }
        // Markdown preview pane uses its own ScrollHandle, not the source
        // editor's viewport. `move_page` would adjust `viewport.scroll_top`
        // which the preview ignores. Scroll-only — caret stays put.
        if self.is_in_markdown_preview() {
            crate::macros::record(crate::macros::MacroEvent::PageUp);
            self.preview_page_scroll(-1, cx);
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::PageUp);
        self.move_page(-1, false, cx);
    }

    pub(crate) fn page_down(&mut self, _: &EditPageDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focus(window) == InputFocus::RichDocBody {
            self.rich_doc_move_page(1, false, cx);
            return;
        }
        if self.is_in_markdown_preview() {
            crate::macros::record(crate::macros::MacroEvent::PageDown);
            self.preview_page_scroll(1, cx);
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::PageDown);
        self.move_page(1, false, cx);
    }

    pub(crate) fn select_page_up(&mut self, _: &SelectPageUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focus(window) == InputFocus::RichDocBody {
            self.rich_doc_move_page(-1, true, cx);
            return;
        }
        // Shift+PageUp in preview: just scroll. Selection extension
        // requires visual-line knowledge the preview pane doesn't expose;
        // matching browser behaviour (scroll only) is the safer choice.
        if self.is_in_markdown_preview() {
            crate::macros::record(crate::macros::MacroEvent::SelectPageUp);
            self.preview_page_scroll(-1, cx);
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectPageUp);
        self.move_page(-1, true, cx);
    }

    pub(crate) fn select_page_down(&mut self, _: &SelectPageDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focus(window) == InputFocus::RichDocBody {
            self.rich_doc_move_page(1, true, cx);
            return;
        }
        if self.is_in_markdown_preview() {
            crate::macros::record(crate::macros::MacroEvent::SelectPageDown);
            self.preview_page_scroll(1, cx);
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectPageDown);
        self.move_page(1, true, cx);
    }

    pub(crate) fn move_line_start(&mut self, _: &MoveLineStart, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_to_edge(false, false, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_jump(window, false, false);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveLineStart);
        let target = self.line_start_byte(self.selection().head);
        self.move_to_byte(target, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn move_line_end(&mut self, _: &MoveLineEnd, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_to_edge(true, false, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_jump(window, true, false);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveLineEnd);
        let target = self.line_end_byte(self.selection().head);
        self.move_to_byte(target, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn select_line_start(
        &mut self,
        _: &SelectLineStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_to_edge(false, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_jump(window, false, true);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectLineStart);
        let target = self.line_start_byte(self.selection().head);
        self.move_to_byte(target, true, true);
        self.commit_caret(cx);
    }

    pub(crate) fn select_line_end(&mut self, _: &SelectLineEnd, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_to_edge(true, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_jump(window, true, true);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectLineEnd);
        let target = self.line_end_byte(self.selection().head);
        self.move_to_byte(target, true, true);
        self.commit_caret(cx);
    }

    pub(crate) fn move_doc_start(&mut self, _: &MoveDocStart, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_doc_edge(false, false, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_jump(window, false, false);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveDocStart);
        self.move_to_byte(0, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn move_doc_end(&mut self, _: &MoveDocEnd, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_doc_edge(true, false, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_jump(window, true, false);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveDocEnd);
        let total = self.buffer().rope.len_bytes();
        self.move_to_byte(total, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn select_doc_start(&mut self, _: &SelectDocStart, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_doc_edge(false, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_jump(window, false, true);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectDocStart);
        self.move_to_byte(0, true, true);
        self.commit_caret(cx);
    }

    pub(crate) fn select_doc_end(&mut self, _: &SelectDocEnd, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_doc_edge(true, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_jump(window, true, true);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectDocEnd);
        let total = self.buffer().rope.len_bytes();
        self.move_to_byte(total, true, true);
        self.commit_caret(cx);
    }

    pub(crate) fn move_word_left(&mut self, _: &MoveWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_word(-1, false, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_word_move(window, -1, false);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveWordLeft);
        let target = self.prev_word_boundary(self.selection().head);
        self.move_to_byte(target, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn move_word_right(&mut self, _: &MoveWordRight, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_word(1, false, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_word_move(window, 1, false);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveWordRight);
        let target = self.next_word_boundary(self.selection().head);
        self.move_to_byte(target, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn select_word_left(&mut self, _: &SelectWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_word(-1, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_word_move(window, -1, true);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectWordLeft);
        let target = self.prev_word_boundary(self.selection().head);
        self.move_to_byte(target, true, true);
        self.commit_caret(cx);
    }

    pub(crate) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_word(1, true, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_word_move(window, 1, true);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectWordRight);
        let target = self.next_word_boundary(self.selection().head);
        self.move_to_byte(target, true, true);
        self.commit_caret(cx);
    }

    // ---- Bookmark actions ------------------------------------------------------------------
    //
    // Bookmarks are per-buffer line markers. As in Notepad++, Ctrl-F2 toggles
    // (Cmd-F2 on macOS), F2 / Shift-F2 jump to the next / previous one
    // (wrapping), and Clear All has no default chord. The buffer holds the
    // offset state; these handlers translate cursor line ↔ bookmark line and
    // notify so the gutter repaints.

    pub(crate) fn toggle_bookmark(
        &mut self,
        _: &BookmarkToggle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let head = self.selection().head;
        let active = self.active;
        self.tabs[active].buffer.toggle_bookmark_at_byte(head);
        cx.notify();
    }

    pub(crate) fn goto_next_bookmark(
        &mut self,
        _: &BookmarkNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // F2 inside the CSV grid (no cell editor) opens the cell editor
        // on the active cell — Excel convention. Bookmarks are not
        // meaningful for a grid view so taking F2 here is safe.
        if self.input_focus(window) == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::open_active_cell_editor(self, window, cx);
            return;
        }
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let head = self.selection().head;
        let buffer = self.buffer();
        let cur_line = line_at_byte(&buffer.rope, head);
        let Some(target_line) = buffer.next_bookmark_line_after(cur_line) else {
            return;
        };
        let target_byte = self.buffer().rope.line_to_byte(target_line);
        self.move_to_byte(target_byte, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn goto_prev_bookmark(
        &mut self,
        _: &BookmarkPrev,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let head = self.selection().head;
        let buffer = self.buffer();
        let cur_line = line_at_byte(&buffer.rope, head);
        let Some(target_line) = buffer.prev_bookmark_line_before(cur_line) else {
            return;
        };
        let target_byte = self.buffer().rope.line_to_byte(target_line);
        self.move_to_byte(target_byte, false, true);
        self.commit_caret(cx);
    }

    pub(crate) fn clear_all_bookmarks(
        &mut self,
        _: &BookmarkClearAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let active = self.active;
        if self.tabs[active].buffer.bookmarks.is_empty() {
            return;
        }
        self.tabs[active].buffer.clear_bookmarks();
        cx.notify();
    }
}

/// Lines a page move steps for a viewport `rows` line-heights tall: one
/// less than the visible count, at least 1.
pub(super) fn lines_per_page(rows: f32) -> i32 {
    (rows.floor() as i32).saturating_sub(1).max(1)
}

/// Line holding byte `byte`, with a stale offset past EOF clamped to the
/// last line.
fn line_at_byte(rope: &ropey::Rope, byte: usize) -> usize {
    rope.byte_to_line(byte.min(rope.len_bytes()))
}

/// Line a page move lands on: `lines` lines from `cur_line` in direction
/// `dir`, kept inside a buffer of `total_lines` lines.
fn page_target_line(cur_line: usize, dir: i32, lines: i32, total_lines: usize) -> usize {
    let delta = dir.saturating_mul(lines) as isize;
    cur_line
        .saturating_add_signed(delta)
        .min(total_lines.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::{line_at_byte, lines_per_page, page_target_line};
    use ropey::Rope;

    #[test]
    fn lines_per_page_is_at_least_one() {
        assert_eq!(lines_per_page(30.0), 29);
        assert_eq!(lines_per_page(30.9), 29);
        assert_eq!(lines_per_page(1.0), 1);
        assert_eq!(lines_per_page(0.0), 1);
        assert_eq!(lines_per_page(-5.0), 1);
        assert_eq!(lines_per_page(f32::NAN), 1);
        assert_eq!(lines_per_page(f32::INFINITY), i32::MAX - 1);
        // A float that saturates to i32::MIN overflowed on the `- 1`.
        assert_eq!(lines_per_page(f32::NEG_INFINITY), 1);
    }

    #[test]
    fn line_at_byte_clamps_stale_offsets() {
        let rope = Rope::from_str("a\r\nb\rc\n");
        assert_eq!(line_at_byte(&rope, 0), 0);
        assert_eq!(line_at_byte(&rope, 3), 1);
        assert_eq!(line_at_byte(&rope, 5), 2);
        assert_eq!(line_at_byte(&rope, rope.len_bytes()), 3);
        // A head past EOF used to panic in `byte_to_line`.
        assert_eq!(line_at_byte(&rope, 99), 3);
        assert_eq!(line_at_byte(&Rope::new(), 5), 0);
        assert_eq!(line_at_byte(&Rope::from_str("é中"), 2), 0);
    }

    #[test]
    fn page_target_stays_in_buffer() {
        assert_eq!(page_target_line(0, -1, 30, 100), 0);
        assert_eq!(page_target_line(50, -1, 30, 100), 20);
        assert_eq!(page_target_line(50, 1, 30, 100), 80);
        assert_eq!(page_target_line(90, 1, 30, 100), 99);
        assert_eq!(page_target_line(0, 1, 30, 0), 0);
        assert_eq!(page_target_line(0, 1, 30, 1), 0);
    }

    #[test]
    fn page_target_extreme_values_do_not_overflow() {
        // `cur + dir * lines` overflowed i32, and a line count above
        // i32::MAX cast to a negative bound so `clamp(0, neg)` panicked.
        assert_eq!(page_target_line(5, 1, i32::MAX, 10), 9);
        assert_eq!(page_target_line(5, -1, i32::MAX, 10), 0);
        assert_eq!(page_target_line(0, 1, 30, 3_000_000_000), 30);
        assert_eq!(page_target_line(2_999_999_990, 1, 30, 3_000_000_000), 2_999_999_999);
    }
}