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
        // Auto-Completion popup steals Up/Down for candidate selection.
        // Checked before any focus / mode dispatch so the popup behaves the
        // same whether it was triggered from the editor or another surface.
        if self.autocomplete.is_some() {
            self.autocomplete_nav(-1, cx);
            return;
        }
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_vertical(-1, false, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, -1, 0, false, cx);
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveUp);
        if self.is_in_markdown_preview()
            && self.preview_move_vertical(-1, false, window, cx)
        {
            return;
        }
        self.move_vertical(-1, false, cx);
    }

    pub(crate) fn move_down(&mut self, _: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.autocomplete.is_some() {
            self.autocomplete_nav(1, cx);
            return;
        }
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_move_vertical(1, false, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::nudge_active_cell(self, 1, 0, false, cx);
            return;
        }
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
        let raw = (self.viewport.bounds.size.height / lh).floor() as i32 - 1;
        raw.max(1)
    }

    fn move_page(&mut self, dir: i32, extend: bool, cx: &mut Context<Self>) {
        self.clear_block_selection();
        let lines = self.page_size_lines();
        let head = self.selection().head;
        let rope = &self.buffer().rope;
        let total_lines = rope.len_lines().max(1) as i32;
        let cur_line = rope.byte_to_line(head) as i32;
        let target_line = (cur_line + dir * lines).clamp(0, total_lines - 1);
        let target_byte = rope.line_to_byte(target_line as usize);
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
    // Bookmarks are per-buffer line markers. F2 toggles, Ctrl-F2 / Shift-F2
    // jump to the next / previous one (wrapping), and Cmd/Ctrl-Shift-F2 clears
    // all bookmarks in the active buffer. The buffer holds the offset state;
    // these handlers translate cursor line ↔ bookmark line and notify so the
    // gutter repaints.

    pub(crate) fn toggle_bookmark(
        &mut self,
        _: &BookmarkToggle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // F2 inside the CSV grid (no cell editor) opens the cell editor
        // on the active cell — Excel convention. Bookmarks are not
        // meaningful for a grid view so hijacking F2 here is safe.
        if self.input_focus(window) == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::open_active_cell_editor(self, window, cx);
            return;
        }
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
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let head = self.selection().head;
        let buffer = self.buffer();
        let cur_line = buffer.rope.byte_to_line(head);
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
        let cur_line = buffer.rope.byte_to_line(head);
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
