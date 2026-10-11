use gpui::{ClipboardItem, Context, Pixels, Point, Window, px};
use std::time::Duration;

use crate::actions::{
    EditBackspace, EditBlockComment, EditCopy, EditCut, EditDelete, EditDeleteLine,
    EditDuplicateLine, EditFillRight, EditFindNext, EditFindPrev, EditInsertBlankLineAbove,
    EditInsertBlankLineBelow, EditInsertNewline, EditInsertNowTime, EditInsertTab,
    EditInsertTodayDate, EditLowercase, EditMoveLineDown, EditMoveLineUp,
    EditPaste, EditRedo, EditSelectAll, EditStreamComment, EditTrimTrailingWs, EditUndo,
    EditUppercase, ViewCsvPreview, ViewCsvSource, ViewCsvToggleFilterRow,
    ViewMarkdownEdit, ViewMarkdownPreview, ViewPreview, ViewShowWhitespace, ViewSource,
    ViewToggleFolderPanel, ViewToggleFullScreen, ViewToggleTheme, ViewWordWrap,
};
use crate::style_model::style::{format_hex_rgb, parse_hex_rgb};
use crate::tab::TabViewMode;
use crate::syntax::comments::comment_syntax;
use crate::tab::InputFocus;
use crate::text::utf16::{next_char_byte_in_str, prev_char_byte_in_str};
use crate::workspace::Workspace;
use super::history::{HistoryDirection, HistoryTarget, apply_rich_history, history_target};

#[cfg(test)]
#[path = "rich_edit_tests.rs"]
mod rich_edit_tests;

/// True if `line` (with leading-whitespace allowed) looks like a CommonMark
/// list item marker: `- `, `* `, `+ ` (bullet) or `1. `, `2) ` (ordered).
/// Used by `insert_tab` to decide whether Tab in markdown-preview mode
/// should nest the line (prepend two spaces at line start) or fall
/// through to the regular cursor-position indent insert.
fn is_markdown_list_line(line: &str) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if matches!(first, '-' | '*' | '+') {
        return matches!(chars.next(), Some(' ') | Some('\t'));
    }
    if first.is_ascii_digit() {
        for c in chars.by_ref() {
            if c.is_ascii_digit() {
                continue;
            }
            if c == '.' || c == ')' {
                return matches!(chars.next(), Some(' ') | Some('\t'));
            }
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tab_list_tests {
    use super::is_markdown_list_line;

    #[test]
    fn matches_bullet_markers() {
        assert!(is_markdown_list_line("- foo"));
        assert!(is_markdown_list_line("* foo"));
        assert!(is_markdown_list_line("+ foo"));
        assert!(is_markdown_list_line("  - nested"));
        assert!(is_markdown_list_line("\t- tabbed"));
    }

    #[test]
    fn matches_ordered_markers() {
        assert!(is_markdown_list_line("1. foo"));
        assert!(is_markdown_list_line("42. foo"));
        assert!(is_markdown_list_line("1) foo"));
        assert!(is_markdown_list_line("  10. nested"));
    }

    #[test]
    fn rejects_non_lists() {
        assert!(!is_markdown_list_line(""));
        assert!(!is_markdown_list_line("just text"));
        assert!(!is_markdown_list_line("-no-space"));
        assert!(!is_markdown_list_line("1."));      // no space
        assert!(!is_markdown_list_line("1.no"));
        assert!(!is_markdown_list_line("# heading"));
        assert!(!is_markdown_list_line("> quote"));
    }
}

/// Notepad++ Advanced auto-indent decision. Returns true when the new line
/// should get one extra indent level on top of the inherited leading
/// whitespace. `before_caret` is the substring of the current line up to (but
/// not including) the byte where Enter was pressed — the trailing-trimmed
/// view of it determines the trigger.
///
/// Triggers:
/// * Universal: trailing `{`, `[`, or `(` — for any language. Matches NPP's
///   bracket-only Advanced behavior.
/// * Python: trailing `:` (e.g. `for x in y:`, `if cond:`, `def f():`).
///   Skipped when the colon is inside a slice expression — best-effort
///   heuristic: a `:` immediately preceded by `[` doesn't count, since
///   `a[:5]` shouldn't trigger indent.
///
/// Ruby / Lua / Bash keyword-driven indent (e.g. `do`, `then`, `function`)
/// is intentionally NOT implemented — line-prefix keyword detection has too
/// many false positives without a real syntax tree (e.g. `do_something()`
/// would wrongly trigger). Tracked in `UNTESTED.md` as a deferral.
fn needs_extra_indent(before_caret: &str, lang: Option<crate::syntax::LanguageId>) -> bool {
    let trimmed = before_caret.trim_end();
    let Some(last) = trimmed.chars().last() else {
        return false;
    };
    if matches!(last, '{' | '[' | '(') {
        return true;
    }
    if last == ':' && matches!(lang, Some(crate::syntax::LanguageId::Python)) {
        let prev = trimmed[..trimmed.len() - 1].chars().last();
        if !matches!(prev, Some('[')) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod auto_indent_tests {
    use super::needs_extra_indent;
    use crate::syntax::LanguageId;

    #[test]
    fn bracket_triggers_regardless_of_language() {
        assert!(needs_extra_indent("function() {", Some(LanguageId::JavaScript)));
        assert!(needs_extra_indent("xs = [", Some(LanguageId::Python)));
        assert!(needs_extra_indent("call(", None));
    }

    #[test]
    fn python_colon_triggers() {
        assert!(needs_extra_indent("for x in y:", Some(LanguageId::Python)));
        assert!(needs_extra_indent("if cond:  ", Some(LanguageId::Python)));
        assert!(needs_extra_indent("def f(x):", Some(LanguageId::Python)));
    }

    #[test]
    fn python_slice_colon_does_not_trigger() {
        assert!(!needs_extra_indent("a[:", Some(LanguageId::Python)));
    }

    #[test]
    fn non_python_colon_does_not_trigger() {
        assert!(!needs_extra_indent("label:", Some(LanguageId::JavaScript)));
        assert!(!needs_extra_indent("foo:", None));
    }

    #[test]
    fn closing_bracket_does_not_trigger() {
        assert!(!needs_extra_indent("function foo()", Some(LanguageId::Lua)));
        assert!(!needs_extra_indent("xs.map(|x| { x })", Some(LanguageId::Rust)));
    }

    #[test]
    fn empty_before_caret_does_not_trigger() {
        assert!(!needs_extra_indent("", Some(LanguageId::Python)));
        assert!(!needs_extra_indent("   ", Some(LanguageId::Python)));
    }
}

#[derive(Debug, Clone, Copy)]
enum RichDocStyleAxis {
    Bold,
    Italic,
    Underline,
    Strike,
}

#[derive(Debug, Clone, Copy)]
enum HeadingShift {
    Promote,
    Demote,
}

/// Direction selector for the comment helpers. `Toggle` matches the Notepad++
/// "Toggle Block / Toggle Stream" semantics; `ForceSet` and `ForceUncomment`
/// drive the directional Block Comment Set / Block Uncomment / Stream
/// Uncomment menu entries.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum CommentDir {
    Toggle,
    ForceSet,
    ForceUncomment,
}

impl Workspace {
    // ---- Edit actions ----------------------------------------------------------------------

    pub(crate) fn select_all(&mut self, _: &EditSelectAll, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_select_all_document(window, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::select_all_rows(self, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_select_all(window);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::SelectAll);
        // Cmd+A returns to linear semantics — drop any block selection.
        self.clear_block_selection();
        let total = self.buffer().rope.len_bytes();
        let tab = self.active_tab_mut();
        tab.selection.anchor = 0;
        tab.selection.head = total;
        tab.selection.is_dragging = false;
        cx.notify();
    }

    pub(crate) fn copy(&mut self, _: &EditCopy, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_copy_selection(cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::copy_grid_selection(self, cx);
            return;
        }
        if focus != InputFocus::Editor {
            if let Some(text) = self.input_selection_text(window) {
                if !text.is_empty() {
                    crate::clipboard_history::record_copy(&text);
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::Copy);
        if self.has_block_selection() {
            self.block_copy(cx);
            return;
        }
        if self.selection().is_empty() {
            if !crate::settings::snapshot().copy_cut_line_without_selection {
                return;
            }
            // Notepad++ parity: empty selection copies the current line in
            // full (including its terminator) so users can grab a line with
            // Ctrl+C and paste it as a new line without first selecting it.
            let head = self.selection().head;
            let rope = &self.buffer().rope;
            let (_, span) = line_span_at(rope, head);
            let text = rope.byte_slice(span).to_string();
            if !text.is_empty() {
                self.write_editor_clipboard(&text, cx);
            }
            return;
        }
        let text = self.buffer().slice_bytes(self.selection().ordered());
        if !text.is_empty() {
            self.write_editor_clipboard(&text, cx);
        }
    }

    /// Put text copied out of the editor on the clipboard with the
    /// document's line ending, as Scintilla copies the document's bytes.
    fn write_editor_clipboard(&self, text: &str, cx: &mut Context<Self>) {
        let text = crate::text::eol_convert::to_doc_eols(text, self.buffer().line_ending);
        crate::clipboard_history::record_copy(&text);
        cx.write_to_clipboard(ClipboardItem::new_string(text.into_owned()));
    }

    pub(crate) fn cut(&mut self, _: &EditCut, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_cut_selection(cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::cut_grid_selection(self, cx);
            return;
        }
        if focus != InputFocus::Editor {
            if let Some(text) = self.input_selection_text(window) {
                if !text.is_empty() {
                    crate::clipboard_history::record_copy(&text);
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    self.input_replace_selection(window, "");
                    cx.notify();
                }
            }
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::Cut);
        if self.has_block_selection() {
            self.block_cut(cx);
            return;
        }
        if self.selection().is_empty() {
            if !crate::settings::snapshot().copy_cut_line_without_selection {
                return;
            }
            // Notepad++ parity: empty selection cuts the current line in
            // full. When the line is the file's last and has no trailing
            // terminator we also swallow the preceding newline so the cut
            // doesn't leave a phantom empty trailing line.
            let head = self.selection().head;
            let rope = &self.buffer().rope;
            let (_, span) = line_span_at(rope, head);
            let text = rope.byte_slice(span).to_string();
            if text.is_empty() {
                return;
            }
            let delete = cut_line_range(rope, head);
            self.write_editor_clipboard(&text, cx);
            self.replace_byte_range(delete, "", cx);
            return;
        }
        let r = self.selection().ordered();
        let text = self.buffer().slice_bytes(r.clone());
        if text.is_empty() {
            return;
        }
        self.write_editor_clipboard(&text, cx);
        self.replace_byte_range(r, "", cx);
    }

    pub(crate) fn paste(&mut self, _: &EditPaste, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::paste_into_grid(self, cx);
            return;
        }
        // For the rich-doc body we look at NSPasteboard's HTML/RTF flavors
        // too — paste-with-formatting (Phase 2g.K) routes there when the
        // clipboard carries more than just plain text.
        if focus == InputFocus::RichDocBody {
            let rich = crate::clipboard_rich::read();
            let plain = rich
                .plain
                .clone()
                .or_else(|| cx.read_from_clipboard().and_then(|item| item.text()))
                .unwrap_or_default();
            if plain.is_empty() && !rich.has_formatting() {
                return;
            }
            if rich.has_formatting() {
                if let Some(html) = rich.html.as_deref() {
                    let blocks = crate::rich_doc::import_html::html_to_blocks(html);
                    if !blocks.is_empty() {
                        self.show_paste_format_dialog(
                            crate::workspace::PasteFormatPending {
                                plain: plain.clone(),
                                blocks,
                            },
                            cx,
                        );
                        return;
                    }
                }
                // RTF-only (no HTML) — Phase 2g.K is HTML-first; fall
                // through to plain paste until an RTF converter lands.
            }
            self.rich_doc_paste(&plain, cx);
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if focus != InputFocus::Editor {
            // Single-line inputs strip embedded newlines so a multi-line
            // clipboard payload doesn't break the dialog layout.
            let single_line: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
            self.input_replace_selection(window, &single_line);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::Paste);
        // The rope holds LF only; a CR LF or CR from the clipboard would be
        // written back doubled on save.
        let text = crate::text::eol_convert::to_rope_eols(&text);
        // Notepad++ column paste: clipboard payloads written by a prior
        // block Cut/Copy carry a sentinel prefix. When detected, route
        // through the rectangular paste path.
        if self.try_block_paste(&text, cx) {
            return;
        }
        let r = self.selection().ordered();
        self.replace_byte_range(r, &text, cx);
    }

    /// Drop `text` into the active editor at the current selection, the same
    /// way `paste` would but sourcing from an arbitrary string instead of
    /// the OS clipboard. Used by the Clipboard History panel to insert a
    /// previously-copied entry without disturbing the live clipboard. Returns
    /// `true` if the insertion happened (active tab is a regular editor and
    /// the payload was non-empty).
    pub(crate) fn paste_string_into_editor(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if text.is_empty() {
            return false;
        }
        // Skip if the active tab isn't a plain editor surface — Clipboard
        // History is plain-text only, and routing a plain string through a
        // markdown-preview or rich-doc surface would silently drop styling
        // context the user might expect.
        if !matches!(
            self.tabs[self.active].view_mode,
            crate::tab::TabViewMode::Edit
        ) {
            return false;
        }
        let text = crate::text::eol_convert::to_rope_eols(text);
        let r = self.selection().ordered();
        self.replace_byte_range(r, &text, cx);
        cx.notify();
        true
    }

    pub(crate) fn undo(&mut self, _: &EditUndo, window: &mut Window, cx: &mut Context<Self>) {
        match history_target(self.input_focus(window), self.buffer().read_only, self.csv_undo_active()) {
            HistoryTarget::RichDocument => {
                if apply_rich_history(&mut self.active_tab_mut().buffer, HistoryDirection::Undo) {
                    self.after_rich_doc_edit(cx);
                }
                return;
            }
            HistoryTarget::Csv => {
                self.csv_undo_active_tab(cx);
                return;
            }
            HistoryTarget::ReadOnly => {
                self.set_status("Buffer is read-only");
                cx.notify();
                return;
            }
            HistoryTarget::Ignored => return,
            HistoryTarget::Editor => {}
        }
        crate::macros::record(crate::macros::MacroEvent::Undo);
        let Some(entry) = self.active_tab_mut().undo_stack.pop() else {
            return;
        };
        if !self.replay_undo_entry(&entry, cx) { return; }
        self.active_tab_mut().redo_stack.push(entry);
        if self.find_state.open && !self.find_state.query.is_empty() {
            self.refresh_matches();
        }
        cx.notify();
    }

    pub(crate) fn redo(&mut self, _: &EditRedo, window: &mut Window, cx: &mut Context<Self>) {
        match history_target(self.input_focus(window), self.buffer().read_only, self.csv_undo_active()) {
            HistoryTarget::RichDocument => {
                if apply_rich_history(&mut self.active_tab_mut().buffer, HistoryDirection::Redo) {
                    self.after_rich_doc_edit(cx);
                }
                return;
            }
            HistoryTarget::Csv => {
                self.csv_redo_active_tab(cx);
                return;
            }
            HistoryTarget::ReadOnly => {
                self.set_status("Buffer is read-only");
                cx.notify();
                return;
            }
            HistoryTarget::Ignored => return,
            HistoryTarget::Editor => {}
        }
        crate::macros::record(crate::macros::MacroEvent::Redo);
        let Some(entry) = self.active_tab_mut().redo_stack.pop() else {
            return;
        };
        if !self.replay_redo_entry(&entry, cx) { return; }
        self.active_tab_mut().undo_stack.push(entry);
        if self.find_state.open && !self.find_state.query.is_empty() {
            self.refresh_matches();
        }
    }

    /// True when the active tab is in CSV preview mode AND has a model
    /// wired up — i.e. undo/redo should target the grid not the rope.
    fn csv_undo_active(&self) -> bool {
        if !matches!(
            self.tabs[self.active].view_mode,
            TabViewMode::CsvPreview
        ) {
            return false;
        }
        let tab_id = self.tabs[self.active].session_id;
        self.csv_models.contains_key(&tab_id)
    }

    /// Pop one undo grain off the active tab's CSV model and re-emit
    /// the result back to the rope so the saved file mirrors the
    /// restored state.
    fn csv_undo_active_tab(&mut self, cx: &mut Context<Self>) {
        let tab_id = self.tabs[self.active].session_id;
        // Commit any pending cell-edit text before unwinding history
        // so the in-flight value doesn't get silently lost.
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        let did = self
            .csv_models
            .get_mut(&tab_id)
            .map(|m| m.undo())
            .unwrap_or(false);
        if did {
            crate::ui::csv_doc::actions::emit_to_source(self, cx);
            cx.notify();
        }
    }

    fn csv_redo_active_tab(&mut self, cx: &mut Context<Self>) {
        let tab_id = self.tabs[self.active].session_id;
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        let did = self
            .csv_models
            .get_mut(&tab_id)
            .map(|m| m.redo())
            .unwrap_or(false);
        if did {
            crate::ui::csv_doc::actions::emit_to_source(self, cx);
            cx.notify();
        }
    }

    pub(crate) fn backspace(&mut self, _: &EditBackspace, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_delete(false, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::handle_grid_delete_key(self, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_backspace(window);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::Backspace);
        if self.has_block_selection() {
            // Backspace on a column selection deletes the rectangle when
            // it has area; on a thin (zero-width) block, deletes one
            // char from min_col on every spanned line.
            if let Some(block) = self.block_selection() {
                if block.min_col() == block.max_col() && block.min_col() > 0 {
                    let new_block = crate::buffer::BlockSelection {
                        anchor_line: block.min_line(),
                        anchor_col: block.min_col() - 1,
                        head_line: block.max_line(),
                        head_col: block.max_col(),
                        is_dragging: false,
                    };
                    self.set_block_selection(Some(new_block));
                }
            }
            self.block_replace("", cx);
            return;
        }
        if self.selection().is_empty() {
            let head = self.selection().head;
            let Some(range) = backspace_range(&self.buffer().rope, head) else {
                return;
            };
            self.replace_byte_range(range, "", cx);
        } else {
            let r = self.selection().ordered();
            self.replace_byte_range(r, "", cx);
        }
    }

    pub(crate) fn delete(&mut self, _: &EditDelete, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_delete(true, cx);
            return;
        }
        if focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::handle_grid_delete_key(self, cx);
            return;
        }
        if focus != InputFocus::Editor {
            self.input_delete(window);
            cx.notify();
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::Delete);
        if self.has_block_selection() {
            if let Some(block) = self.block_selection() {
                if block.min_col() == block.max_col() {
                    // Thin column → expand by one to forward-delete one
                    // char on every spanned line.
                    let new_block = crate::buffer::BlockSelection {
                        anchor_line: block.min_line(),
                        anchor_col: block.min_col(),
                        head_line: block.max_line(),
                        head_col: block.max_col() + 1,
                        is_dragging: false,
                    };
                    self.set_block_selection(Some(new_block));
                }
            }
            self.block_replace("", cx);
            return;
        }
        if self.selection().is_empty() {
            let head = self.selection().head;
            let Some(range) = delete_forward_range(&self.buffer().rope, head) else {
                return;
            };
            self.replace_byte_range(range, "", cx);
        } else {
            let r = self.selection().ordered();
            self.replace_byte_range(r, "", cx);
        }
    }

    pub(crate) fn insert_newline(&mut self, _: &EditInsertNewline, window: &mut Window, cx: &mut Context<Self>) {
        // Auto-Completion popup: NPP `Insert Selection ▸ ENTER` setting.
        // Same gating as Tab — popup + pref means Enter accepts the
        // candidate; otherwise the normal Enter handling below runs.
        if self.autocomplete.is_some()
            && crate::settings::snapshot().autocompletion_insert_with_enter
            && self.accept_autocomplete(window, cx)
        {
            return;
        }
        let focus = self.input_focus(window);
        // Shift+Enter shares this action with Enter (Notepad++ binds both
        // to SCI_NEWLINE). In the find bar and the CSV grid it runs the
        // reverse step instead; `find_prev` routes each of those.
        if window.modifiers().shift
            && matches!(
                focus,
                InputFocus::FindQuery
                    | InputFocus::Replacement
                    | InputFocus::CsvCell
                    | InputFocus::CsvGrid
                    | InputFocus::CsvFindBar
                    | InputFocus::CsvReplaceBar
            )
        {
            self.find_prev(&EditFindPrev, window, cx);
            return;
        }
        match focus {
            InputFocus::FindQuery | InputFocus::Replacement => {
                self.find_next(&EditFindNext, window, cx);
            }
            InputFocus::Goto => {
                self.commit_goto_line(cx);
                if !self.goto_state.open { window.focus(&self.focus_handle, cx); }
            }
            InputFocus::FifQuery
            | InputFocus::FifReplacement
            | InputFocus::FifFilter
            | InputFocus::FifDirectory => {
                // The FIF and FIP tabs share input focus handles since
                // they share state. Dispatch on which tab the user is
                // looking at so Enter runs the correct search.
                if matches!(self.dialog_tab, crate::find_state::DialogTab::FindInProject) {
                    self.find_in_projects_run_action(
                        &crate::actions::FindInProjectsRun,
                        window,
                        cx,
                    );
                } else {
                    self.find_in_files.open = true;
                    self.run_find_in_files(window, cx);
                }
            }
            InputFocus::MarkQuery => {
                self.mark_all(&crate::actions::MarkAll, window, cx);
            }
            InputFocus::MacroPrompt => {
                self.commit_macro_prompt(window, cx);
            }
            InputFocus::TabRename => {
                self.commit_tab_rename(cx);
                window.focus(&self.focus_handle, cx);
            }
            InputFocus::FolderInlineCreate => {
                self.commit_folder_inline_create(cx);
                // Only return focus to the editor on successful commit —
                // a validation failure leaves the draft mounted so the user
                // can keep editing without retyping.
                if self.folder_inline_create.is_none() {
                    window.focus(&self.focus_handle, cx);
                }
            }
            InputFocus::RichDocLinkUrl => {
                self.commit_rich_doc_link_bar(cx);
                window.focus(&self.focus_handle, cx);
            }
            InputFocus::RichDocColorHex => {
                // Enter commits whatever's typed and closes the bar. Live-
                // apply already pushed the value through on every keystroke,
                // so commit just dismisses.
                self.close_rich_doc_color_bar(cx);
                window.focus(&self.focus_handle, cx);
            }
            InputFocus::RichDocBody => {
                // Phase 2g.B — Enter splits the current paragraph at the
                // cursor (after collapsing any active selection). Routing
                // through the action handler instead of the on_key_down
                // listener ensures Enter works regardless of which div in
                // the rich-doc pane the keystroke originated from.
                // scintilla/src/Editor.cxx:3347 — delete + newline is one Undo.
                self.with_rich_doc_edit_group(cx, |w, cx| w.rich_doc_insert_paragraph_break(cx));
            }
            InputFocus::CsvCell => {
                // Excel Enter — commit any in-progress edit and step the
                // active cell down one row. move_active_cell_enter
                // handles the commit so the value lands before the
                // highlight moves.
                crate::ui::csv_doc::actions::move_active_cell_enter(self, true, cx);
            }
            InputFocus::CsvGrid => {
                // Grid has focus but no cell editor. Enter navigates
                // down — matches Excel: pressing Enter on a selected
                // (non-editing) cell moves the selection. F2 opens the
                // editor; typing a printable char seeds an edit on the
                // active cell.
                crate::ui::csv_doc::actions::move_active_cell_enter(self, true, cx);
            }
            InputFocus::CsvFindBar => {
                // Find bar: Enter advances to the next match. Shift+Enter
                // steps backward through `find_prev` (routed above).
                crate::ui::csv_doc::actions::find_step(self, true, window, cx);
            }
            InputFocus::CsvReplaceBar => {
                // Replace bar: Enter replaces current match + advances.
                crate::ui::csv_doc::actions::replace_current_action(self, cx);
            }
            InputFocus::Editor => {
                // Markdown preview WYSIWYG: pressing Enter inside the
                // rendered preview splits the current paragraph / heading.
                // pulldown-cmark needs a blank line (`\n\n`) between
                // blocks; a single `\n` is rendered as a soft break inside
                // the same paragraph and we'd never escape the heading.
                // Auto-indent doesn't apply in preview mode — markdown
                // paragraphs don't carry leading whitespace, and any
                // inherited indent would just create stray indented-code
                // blocks on the next line.
                let in_md_preview = {
                    let tab = &self.tabs[self.active];
                    matches!(tab.view_mode, crate::tab::TabViewMode::MarkdownPreview)
                        && crate::ui::markdown_view::is_markdown_path(
                            tab.buffer.file_path.as_deref(),
                        )
                };
                if in_md_preview {
                    crate::macros::record(crate::macros::MacroEvent::InsertNewline);
                    let r = self.selection().ordered();
                    let ending = "\n".to_string();
                    // Two line breaks end the paragraph. The rope holds LF
                    // only; saving writes the document's line ending.
                    let insert = format!("{ending}{ending}");
                    self.replace_byte_range(r, &insert, cx);
                    return;
                }
                crate::macros::record(crate::macros::MacroEvent::InsertNewline);
                // Auto-indent mode (Indentation prefs panel) gates inheritance:
                //   None     → just the line ending, no indent copy.
                //   Basic    → copy leading whitespace of the broken line.
                //   Advanced → Basic + one extra indent unit when the caret
                //              follows `{`, `[`, or `(`, or when the trimmed
                //              body before the caret ends with a language-
                //              specific opener keyword (Python `:`, Ruby/Lua
                //              `do`/`then`, Bash `then`/`do`, etc.).
                let r = self.selection().ordered();
                let ending = "\n".to_string();
                let prefs = crate::settings::snapshot();
                let mode = prefs.auto_indent_mode;
                let lang = self.buffer().language;
                let mut insert = ending;
                if !matches!(mode, crate::settings::AutoIndentMode::None) {
                    let (inherit, before_caret) =
                        auto_indent_context(&self.buffer().rope, r.start);
                    insert.push_str(&inherit);
                    if matches!(mode, crate::settings::AutoIndentMode::Advanced) {
                        if needs_extra_indent(&before_caret, lang) {
                            let lang_key = lang.map(|l| l.key());
                            let (uses_tabs, width) =
                                crate::settings::indent_for_lang(lang_key);
                            if uses_tabs {
                                insert.push('\t');
                            } else {
                                insert.push_str(&" ".repeat(width));
                            }
                        }
                    }
                }
                self.replace_byte_range(r, &insert, cx);
            }
        }
    }

    pub(crate) fn insert_tab(&mut self, _: &EditInsertTab, window: &mut Window, cx: &mut Context<Self>) {
        // Auto-Completion popup: NPP `Insert Selection ▸ TAB` setting.
        // When the popup is up and the user has the TAB-insert pref on, Tab
        // accepts the selected candidate instead of inserting an indent.
        // Off-pref or no popup falls through to normal Tab handling.
        if self.autocomplete.is_some()
            && crate::settings::snapshot().autocompletion_insert_with_tab
            && self.accept_autocomplete(window, cx)
        {
            return;
        }
        let focus = self.input_focus(window);
        if focus == InputFocus::RichDocBody {
            self.rich_doc_insert_tab(cx);
            return;
        }
        // CSV grid Tab nav — moves the active cell one column right with
        // row wrap. Same path whether the cell editor is open or the
        // grid container itself has focus (move_active_cell_tab commits
        // any in-flight edit first).
        if focus == InputFocus::CsvCell || focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::move_active_cell_tab(self, true, cx);
            return;
        }
        // CSV find bar Tab → focus replace input when replace row is up.
        if focus == InputFocus::CsvFindBar {
            crate::ui::csv_doc::actions::focus_replace_input(self, window, cx);
            return;
        }
        // CSV replace bar Tab → focus find input (cycle).
        if focus == InputFocus::CsvReplaceBar {
            crate::ui::csv_doc::actions::focus_find_input(self, window, cx);
            return;
        }
        if focus != InputFocus::Editor {
            return;
        }
        // Markdown preview: Tab on a list-item line prepends 2 spaces at
        // the line start so the item nests under its predecessor (the
        // CommonMark minimum nest indent for bullet lists). Inserting a
        // literal \t at the cursor position would create an indented-code
        // block under pulldown-cmark. Multi-line selections and non-list
        // lines fall through to the regular cursor-position insert below.
        if self.is_in_markdown_preview() {
            let sel = self.selection();
            if sel.is_empty() {
                let rope = &self.buffer().rope;
                let head = floor_rope_char_boundary(rope, sel.head);
                let (_, span) = line_span_at(rope, head);
                let line_start = span.start;
                let line_text: String = rope.byte_slice(span).to_string();
                let line_body = line_text.trim_end_matches(['\n', '\r']);
                if is_markdown_list_line(line_body) {
                    crate::macros::record(crate::macros::MacroEvent::InsertTab);
                    let indent = "  ";
                    self.replace_byte_range(line_start..line_start, indent, cx);
                    let new_head = head + indent.len();
                    self.active_tab_mut().selection.collapse_to(new_head);
                    return;
                }
            }
        }
        crate::macros::record(crate::macros::MacroEvent::InsertTab);
        let r = self.selection().ordered();
        // Indent character is preference-driven: literal tab when the
        // resolved `indent_uses_tabs` is on, otherwise the resolved width
        // in spaces. Per-language override in `language_indent_overrides`
        // (NPP `Indentation ▸ language listbox`) wins over the global
        // `indent_uses_tabs` + `tab_width` for the active buffer's
        // detected language.
        let lang_key = self.buffer().language.map(|l| l.key());
        let (uses_tabs, width) = crate::settings::indent_for_lang(lang_key);
        let insert: String = if uses_tabs {
            "\t".to_string()
        } else {
            " ".repeat(width)
        };
        self.replace_byte_range(r, &insert, cx);
    }

    // ---- Input field editing helpers (find / replace / goto) -------------------------------
    //
    // Each input is a `String + cursor + anchor` tuple. When `anchor != cursor`,
    // the bytes between them are the active selection — drives copy / cut /
    // paste / type-to-replace and renders a highlight rect under the text.
    //
    // The four "delete-or-collapse" verbs (backspace / delete / type / paste)
    // first delete a non-empty selection if one is present, then perform their
    // single-char move on the now-collapsed cursor. Movement verbs collapse
    // the selection unless `extend = true`.

    pub(crate) fn input_backspace(&mut self, window: &Window) {
        let focus = self.input_focus(window);
        let Some((target, cursor, anchor)) = self.input_field_mut(focus) else {
            return;
        };
        if !input_field_backspace(target, cursor, anchor) {
            return;
        }
        if focus == InputFocus::FindQuery {
            self.refresh_matches();
        }
    }

    pub(crate) fn input_delete(&mut self, window: &Window) {
        let focus = self.input_focus(window);
        let Some((target, cursor, anchor)) = self.input_field_mut(focus) else {
            return;
        };
        if !input_field_delete(target, cursor, anchor) {
            return;
        }
        if focus == InputFocus::FindQuery {
            self.refresh_matches();
        }
    }

    pub(crate) fn input_move(&mut self, window: &Window, delta: i32, extend: bool) {
        let focus = self.input_focus(window);
        let Some((target, cursor, anchor)) = self.input_field_mut(focus) else {
            return;
        };
        input_field_step(target, cursor, anchor, delta, extend);
    }

    pub(crate) fn input_jump(&mut self, window: &Window, to_end: bool, extend: bool) {
        let focus = self.input_focus(window);
        let Some((target, cursor, anchor)) = self.input_field_mut(focus) else {
            return;
        };
        *cursor = if to_end { target.len() } else { 0 };
        if !extend {
            *anchor = *cursor;
        }
    }

    /// Word-boundary jump inside an input. Mirrors the editor's
    /// `prev_word_boundary` / `next_word_boundary` walk: skip whitespace, then
    /// skip the run of word characters, landing at the next gap.
    pub(crate) fn input_word_move(&mut self, window: &Window, delta: i32, extend: bool) {
        let focus = self.input_focus(window);
        let Some((target, cursor, anchor)) = self.input_field_mut(focus) else {
            return;
        };
        let s: &str = target;
        snap_input_caret(s, cursor, anchor);
        let new = if delta < 0 {
            input_prev_word_boundary(s, *cursor)
        } else {
            input_next_word_boundary(s, *cursor)
        };
        *cursor = new;
        if !extend {
            *anchor = *cursor;
        }
    }

    /// Cmd+A inside an input — anchor at 0, cursor at end.
    pub(crate) fn input_select_all(&mut self, window: &Window) {
        let focus = self.input_focus(window);
        let Some((target, cursor, anchor)) = self.input_field_mut(focus) else {
            return;
        };
        *anchor = 0;
        *cursor = target.len();
    }

    /// Selection text (or empty string when collapsed). Borrows immutably so
    /// callers can pass the value to the clipboard without juggling lifetimes.
    pub(crate) fn input_selection_text(&self, window: &Window) -> Option<String> {
        let focus = self.input_focus(window);
        let (target, cursor, anchor) = self.input_field_ref(focus)?;
        Some(input_field_selection(target, cursor, anchor))
    }

    /// Replace the current selection (or insert at cursor when collapsed)
    /// with `new_text`. Same path the IME / paste handlers use.
    pub(crate) fn input_replace_selection(&mut self, window: &Window, new_text: &str) {
        let focus = self.input_focus(window);
        let Some((target, cursor, anchor)) = self.input_field_mut(focus) else {
            return;
        };
        input_field_replace(target, cursor, anchor, new_text);
        if focus == InputFocus::FindQuery {
            self.refresh_matches();
        }
    }

    /// Set both cursor and anchor on the given input — used by mouse-down
    /// hit-testing inside `text_input_with_chevron` to seat the caret. When
    /// `extend` is true, only `cursor` moves (Shift+click extends the
    /// existing selection); otherwise both collapse to `byte`.
    pub(crate) fn input_set_caret_byte(&mut self, focus: InputFocus, byte: usize, extend: bool) {
        let Some((target, cursor, anchor)) = self.input_field_mut(focus) else {
            return;
        };
        let clamped = floor_char_boundary_in(target, byte);
        *cursor = clamped;
        if !extend {
            *anchor = clamped;
        }
    }

    /// Read-only helper: resolve the focused field and return refs to its
    /// string + cursor + anchor. Used by `input_selection_text` (which can't
    /// borrow the workspace mutably while also handing the slice out).
    fn input_field_ref(&self, focus: InputFocus) -> Option<(&str, usize, usize)> {
        match focus {
            InputFocus::FindQuery => Some((
                &self.find_state.query,
                self.find_state.query_cursor,
                self.find_state.query_anchor,
            )),
            InputFocus::Replacement => Some((
                &self.find_state.replacement,
                self.find_state.replacement_cursor,
                self.find_state.replacement_anchor,
            )),
            InputFocus::Goto => Some((
                &self.goto_state.text,
                self.goto_state.text_cursor,
                self.goto_state.text_anchor,
            )),
            InputFocus::FifQuery => Some((
                &self.find_in_files.query,
                self.find_in_files.query_cursor,
                self.find_in_files.query_anchor,
            )),
            InputFocus::FifReplacement => Some((
                &self.find_in_files.replacement,
                self.find_in_files.replacement_cursor,
                self.find_in_files.replacement_anchor,
            )),
            InputFocus::FifFilter => Some((
                &self.find_in_files.filter,
                self.find_in_files.filter_cursor,
                self.find_in_files.filter_anchor,
            )),
            InputFocus::FifDirectory => Some((
                &self.find_in_files.directory_text,
                self.find_in_files.directory_text_cursor,
                self.find_in_files.directory_text_anchor,
            )),
            InputFocus::MarkQuery => Some((
                &self.mark_state.query,
                self.mark_state.query_cursor,
                self.mark_state.query_anchor,
            )),
            InputFocus::MacroPrompt => self
                .macro_prompt
                .as_ref()
                .map(|s| (s.text.as_str(), s.text_cursor, s.text_anchor)),
            InputFocus::TabRename => self
                .tab_rename
                .as_ref()
                .map(|s| (s.text.as_str(), s.text_cursor, s.text_anchor)),
            InputFocus::FolderInlineCreate => self
                .folder_inline_create
                .as_ref()
                .map(|s| (s.text.as_str(), s.text_cursor, s.text_anchor)),
            InputFocus::RichDocLinkUrl => self
                .rich_doc_link_bar
                .as_ref()
                .map(|s| (s.text.as_str(), s.text_cursor, s.text_anchor)),
            InputFocus::RichDocColorHex => self
                .rich_doc_color_bar
                .as_ref()
                .map(|s| (s.text.as_str(), s.text_cursor, s.text_anchor)),
            InputFocus::CsvCell => {
                let tab_id = self.tabs[self.active].session_id;
                self.csv_models
                    .get(&tab_id)
                    .and_then(|m| m.cell_input.as_ref())
                    .map(|i| (i.buffer.as_str(), i.cursor, i.anchor))
            }
            InputFocus::CsvFindBar => {
                let tab_id = self.tabs[self.active].session_id;
                self.csv_models
                    .get(&tab_id)
                    .and_then(|m| m.find_input.as_ref())
                    .map(|i| (i.buffer.as_str(), i.cursor, i.anchor))
            }
            InputFocus::CsvReplaceBar => {
                let tab_id = self.tabs[self.active].session_id;
                self.csv_models
                    .get(&tab_id)
                    .and_then(|m| m.replace_input.as_ref())
                    .map(|i| (i.buffer.as_str(), i.cursor, i.anchor))
            }
            InputFocus::Editor | InputFocus::RichDocBody | InputFocus::CsvGrid => None,
        }
    }

    /// Goto Line input — Enter parses the text as a 1-based line number, jumps
    /// the active editor cursor to that line's start, then closes the bar.
    /// Out-of-range values clamp to the last line; non-numeric input is a
    /// no-op (the bar stays open so the user can correct it).
    pub(crate) fn commit_goto_line(&mut self, cx: &mut Context<Self>) {
        let trimmed = self.goto_state.text.trim();
        let Some(line_one_based) = trimmed.parse::<usize>().ok().filter(|line| *line > 0) else {
            self.set_status("Go to Line needs a positive line number");
            cx.notify();
            return;
        };
        let total = self.buffer().rope.len_lines().max(1);
        let line = (line_one_based - 1).min(total - 1);
        let target = self.buffer().rope.line_to_byte(line);
        self.move_to_byte(target, false, true);
        self.goto_state.open = false;
        self.goto_state.text.clear();
        self.goto_state.text_cursor = 0;
        self.goto_state.text_anchor = 0;
        self.commit_caret(cx);
    }

    pub(crate) fn toggle_theme(&mut self, _: &ViewToggleTheme, _w: &mut Window, cx: &mut Context<Self>) {
        let next = self.theme_mode.flipped();
        // Settings is the cross-window source of truth — write it first so
        // the Preferences ▸ Dark Mode radio reflects the new value, then
        // mirror it to every open Workspace so all windows flip in lockstep.
        let mut s = crate::settings::snapshot();
        s.theme_mode = next;
        crate::settings::replace_and_save(s);
        crate::settings::propagate_theme_to_all_workspaces(next, cx);
        // `propagate_theme_to_all_workspaces` updates `self.theme_mode` via
        // its workspace iteration, but the active workspace's session also
        // needs the manifest write so a restart restores the same theme
        // before Settings is consulted (manifest takes precedence).
        self.save_session(cx);
    }

    /// Toggle the host window between windowed and full-screen state. Uses
    /// gpui's cross-platform `Window::toggle_fullscreen` (NSWindow
    /// `toggleFullScreen:` on macOS, equivalent path on Linux/Windows). NPP
    /// binds F11 to this; we keep the same binding cross-platform. Ephemeral
    /// — full-screen state is not persisted across launches.
    pub(crate) fn toggle_full_screen(
        &mut self,
        _: &ViewToggleFullScreen,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }

    /// Toggle the "show whitespace" overlay. Spaces render as a small dot,
    /// tabs as a faint arrow extending across the tab's shaped width.
    /// Persisted in the session manifest so the choice survives relaunch.
    pub(crate) fn toggle_show_whitespace(
        &mut self,
        _: &ViewShowWhitespace,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_whitespace = !self.show_whitespace;
        self.save_session(cx);
        cx.notify();
    }

    /// Toggle word wrap. When on, lines that exceed the editor pane's width
    /// break at the last whitespace before the right edge (hard-break for
    /// unbreakable runs). Wrap state and the per-tab visual-row cache live
    /// together — flipping the toggle invalidates every tab's cache so the
    /// next paint re-derives the right vertical extent. When wrap turns on
    /// we also zero `scroll_left` so an offscreen horizontal offset doesn't
    /// leak into the wrapped layout. Persisted via session manifest so the
    /// preference survives relaunch.
    /// Set the active tab's render mode. `preview = true` swaps in the
    /// rendered markdown view; `false` returns to the editor. No-ops when
    /// the active tab isn't a markdown file (toolbar disables the buttons
    /// in that case, but defend against keyboard shortcut firing anyway).
    pub(crate) fn set_markdown_view_mode(&mut self, preview: bool, cx: &mut Context<Self>) {
        let tab = self.active_tab_mut();
        if tab.buffer.file_path.is_none()
            || !crate::ui::markdown_view::is_markdown_path(tab.buffer.file_path.as_deref())
        {
            return;
        }
        let next = if preview {
            TabViewMode::MarkdownPreview
        } else {
            TabViewMode::Edit
        };
        if tab.view_mode != next {
            tab.view_mode = next;
            self.close_autocomplete(cx);
            cx.notify();
        }
    }

    /// Action wrapper for the View ▸ Markdown ▸ Source toolbar toggle.
    pub(crate) fn view_markdown_edit(
        &mut self,
        _: &ViewMarkdownEdit,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_markdown_view_mode(false, cx);
    }

    /// Action wrapper for the View ▸ Markdown ▸ Preview toolbar toggle.
    pub(crate) fn view_markdown_preview(
        &mut self,
        _: &ViewMarkdownPreview,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_markdown_view_mode(true, cx);
    }

    /// Set the active tab's render mode for CSV/TSV files. `preview = true`
    /// swaps in the spreadsheet grid; `false` returns to the raw text
    /// editor (the rope already holds the delimited text). No-ops when the
    /// active tab isn't a CSV file.
    pub(crate) fn set_csv_view_mode(&mut self, preview: bool, cx: &mut Context<Self>) {
        let path_is_csv = {
            let tab = &self.tabs[self.active];
            crate::ui::csv_doc::is_csv_path(tab.buffer.file_path.as_deref())
        };
        if !path_is_csv {
            return;
        }
        // Commit any pending cell edit before switching modes so the
        // pending value doesn't get lost when the cell input goes away.
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        let next = if preview {
            TabViewMode::CsvPreview
        } else {
            TabViewMode::Edit
        };
        let tab = self.active_tab_mut();
        if tab.view_mode != next {
            tab.view_mode = next;
            self.close_autocomplete(cx);
            cx.notify();
        }
    }

    pub(crate) fn view_csv_source(
        &mut self,
        _: &ViewCsvSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self.input_focus(window);
        self.set_csv_view_mode(false, cx);
        self.focus_source_editor(focus, window, cx);
    }

    pub(crate) fn view_csv_preview(
        &mut self,
        _: &ViewCsvPreview,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_csv_view_mode(true, cx);
    }

    /// Cmd+Shift+L on a CSV preview tab — flips the global filter-row
    /// visibility. On non-CSV tabs the keymap still fires but the
    /// handler is a no-op (the CSV model lookup misses).
    pub(crate) fn view_csv_toggle_filter_row(
        &mut self,
        _: &ViewCsvToggleFilterRow,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::ui::csv_doc::actions::toggle_csv_filter_row(self, cx);
    }

    /// Universal "View Source" toolbar action — dispatches by file type so a
    /// single button works for markdown, rich-doc, and CSV. No-op on plain
    /// text (the toolbar disables the button in that case but defend against
    /// keyboard fall-through).
    pub(crate) fn view_source(
        &mut self,
        _: &ViewSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self.input_focus(window);
        let tab = &self.tabs[self.active];
        let path = tab.buffer.file_path.as_deref();
        if tab.buffer.is_rich_doc() {
            self.set_rich_doc_view_mode(false, cx);
        } else if crate::ui::csv_doc::is_csv_path(path) {
            self.set_csv_view_mode(false, cx);
        } else if crate::ui::markdown_view::is_markdown_path(path) {
            self.set_markdown_view_mode(false, cx);
        }
        self.focus_source_editor(focus, window, cx);
    }

    fn focus_source_editor(&self, previous: InputFocus, window: &mut Window, cx: &mut Context<Self>) {
        // The rendered body's focus node disappears on Source/Raw. Keep
        // unrelated visible inputs (such as Go to Line) focused.
        if self.tabs.get(self.active).is_some_and(|tab| tab.view_mode == TabViewMode::Edit)
            && matches!(previous, InputFocus::RichDocBody | InputFocus::CsvCell | InputFocus::CsvGrid)
        {
            window.focus(&self.focus_handle, cx);
        }
    }

    /// Universal "View Preview" toolbar action — counterpart to `view_source`.
    pub(crate) fn view_preview(
        &mut self,
        _: &ViewPreview,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = &self.tabs[self.active];
        let path = tab.buffer.file_path.as_deref();
        if tab.buffer.is_rich_doc() {
            self.set_rich_doc_view_mode(true, cx);
        } else if crate::ui::csv_doc::is_csv_path(path) {
            self.set_csv_view_mode(true, cx);
        } else if crate::ui::markdown_view::is_markdown_path(path) {
            self.set_markdown_view_mode(true, cx);
        }
    }

    /// Flip the active tab between Doc View (rendered AST) and Edit (raw
    /// document.xml in the rope). No-ops when the active tab isn't a rich-doc
    /// — toolbar disables the buttons in that case, but defend against
    /// keyboard-shortcut firing anyway.
    pub(crate) fn set_rich_doc_view_mode(&mut self, doc_view: bool, cx: &mut Context<Self>) {
        let tab = self.active_tab_mut();
        if !tab.buffer.is_rich_doc() {
            return;
        }
        let next = if doc_view {
            TabViewMode::RichDocView
        } else {
            TabViewMode::Edit
        };
        if tab.view_mode != next {
            tab.view_mode = next;
            self.close_autocomplete(cx);
            cx.notify();
        }
    }

    pub(crate) fn view_rich_doc(
        &mut self,
        _: &crate::actions::rich_doc::RichDocView,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_rich_doc_view_mode(true, cx);
    }

    pub(crate) fn view_raw(
        &mut self,
        _: &crate::actions::rich_doc::RichDocRaw,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self.input_focus(window);
        self.set_rich_doc_view_mode(false, cx);
        self.focus_source_editor(focus, window, cx);
    }

    /// Set the active rich-doc block (the target for Bold / Italic / Underline
    /// toolbar buttons). Called by `rich_doc_view` when the user clicks into
    /// a rendered paragraph or heading.
    pub(crate) fn set_rich_doc_active_block(&mut self, idx: usize, cx: &mut Context<Self>) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        if payload.active_block != Some(idx) {
            payload.active_block = Some(idx);
            cx.notify();
        }
    }

    /// Toggle bold on every run in the active block. Mutates the payload's AST
    /// and appends to `edit_log` so save knows to re-serialize. Marks the
    /// buffer dirty so Cmd+S persists.
    pub(crate) fn rich_doc_bold(
        &mut self,
        _: &crate::actions::rich_doc::RichDocBold,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_toggle_style(RichDocStyleAxis::Bold, cx);
    }

    pub(crate) fn rich_doc_italic(
        &mut self,
        _: &crate::actions::rich_doc::RichDocItalic,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_toggle_style(RichDocStyleAxis::Italic, cx);
    }

    pub(crate) fn rich_doc_underline(
        &mut self,
        _: &crate::actions::rich_doc::RichDocUnderline,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_toggle_style(RichDocStyleAxis::Underline, cx);
    }

    pub(crate) fn rich_doc_strikethrough(
        &mut self,
        _: &crate::actions::rich_doc::RichDocStrikethrough,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_toggle_style(RichDocStyleAxis::Strike, cx);
    }

    pub(crate) fn rich_doc_align_left(&mut self, _w: &mut gpui::Window, cx: &mut Context<Self>) {
        self.rich_doc_set_alignment(crate::rich_doc::ast::Alignment::Left, cx);
    }
    pub(crate) fn rich_doc_align_center(&mut self, _w: &mut gpui::Window, cx: &mut Context<Self>) {
        self.rich_doc_set_alignment(crate::rich_doc::ast::Alignment::Center, cx);
    }
    pub(crate) fn rich_doc_align_right(&mut self, _w: &mut gpui::Window, cx: &mut Context<Self>) {
        self.rich_doc_set_alignment(crate::rich_doc::ast::Alignment::Right, cx);
    }
    pub(crate) fn rich_doc_align_justify(&mut self, _w: &mut gpui::Window, cx: &mut Context<Self>) {
        self.rich_doc_set_alignment(crate::rich_doc::ast::Alignment::Justify, cx);
    }
    pub(crate) fn rich_doc_indent_increase(
        &mut self,
        _w: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_apply_indent_delta(1, cx);
    }
    pub(crate) fn rich_doc_indent_decrease(
        &mut self,
        _w: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_apply_indent_delta(-1, cx);
    }

    pub(crate) fn rich_doc_promote_heading(
        &mut self,
        _: &crate::actions::rich_doc::RichDocPromoteHeading,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_shift_heading(HeadingShift::Promote, cx);
    }

    pub(crate) fn rich_doc_demote_heading(
        &mut self,
        _: &crate::actions::rich_doc::RichDocDemoteHeading,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_shift_heading(HeadingShift::Demote, cx);
    }

    /// Apply Promote or Demote across every block touched by the selection
    /// (or just `active_block` when no selection). Promote shifts toward
    /// larger visual weight: Paragraph → H3 → H2 → H1, clamps at H1. Demote
    /// shifts the other way: H1 → H2 → … → H6 → Paragraph, clamps at
    /// Paragraph. Tables and Images are skipped; mixed-kind selections each
    /// shift independently from their own current level.
    fn rich_doc_shift_heading(&mut self, shift: HeadingShift, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let block_range: (usize, usize) = if let Some(sel) = payload.selection {
            if !sel.is_collapsed() {
                let (lo, hi) = sel.range_full();
                (lo.block, hi.block)
            } else {
                let Some(b) = payload.active_block else {
                    return;
                };
                (b, b)
            }
        } else {
            let Some(b) = payload.active_block else {
                return;
            };
            (b, b)
        };
        let mut dirty = false;
        for block in block_range.0..=block_range.1 {
            let Some(b) = payload.ast.blocks.get(block) else {
                continue;
            };
            let current_level = match b {
                crate::rich_doc::ast::Block::Heading { level, .. } => Some(*level),
                crate::rich_doc::ast::Block::Paragraph { .. } => None,
                _ => continue,
            };
            let next_level = match (shift, current_level) {
                (HeadingShift::Promote, None) => Some(3),
                (HeadingShift::Promote, Some(1)) => continue,
                (HeadingShift::Promote, Some(n)) => Some(n - 1),
                (HeadingShift::Demote, Some(6)) => None,
                (HeadingShift::Demote, Some(n)) => Some(n + 1),
                (HeadingShift::Demote, None) => continue,
            };
            payload.apply_edit(crate::rich_doc::edits::Edit::SetHeadingLevel {
                block,
                level: next_level,
            });
            dirty = true;
        }
        if dirty {
            self.after_rich_doc_edit(cx);
        }
    }

    pub(crate) fn rich_doc_toggle_list_bullet(
        &mut self,
        _w: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_toggle_list(false, cx);
    }

    pub(crate) fn rich_doc_toggle_list_number(
        &mut self,
        _w: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_toggle_list(true, cx);
    }

    fn rich_doc_toggle_list(&mut self, ordered: bool, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(block) = payload.active_block else {
            return;
        };
        payload.apply_edit(crate::rich_doc::edits::Edit::ToggleBlockList { block, ordered });
        self.after_rich_doc_edit(cx);
    }

    /// Ensure the rich-doc body focus handle exists. Lazy-init so cold paths
    /// (read-only tabs, no rich-doc) don't allocate a FocusHandle for nothing.
    pub(crate) fn ensure_rich_doc_body_focus(
        &mut self,
        cx: &mut Context<Self>,
    ) -> gpui::FocusHandle {
        self.rich_doc_body_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }

    /// Set the rich-doc selection cursor + anchor inside the active payload.
    /// `block` becomes the active block. Used by paragraph-click handlers
    /// when the user single-clicks to position the caret.
    pub(crate) fn set_rich_doc_caret(
        &mut self,
        block: usize,
        flat_byte: usize,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.ensure_rich_doc_body_focus(cx);
        window.focus(&handle, cx);
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        payload.active_block = Some(block);
        let pos = payload.pos_from_flat_byte(block, flat_byte);
        payload.selection = Some(crate::rich_doc::TextSelection { anchor: pos, cursor: pos });
        payload.drag_active = true;
        cx.notify();
    }

    /// Extend the cursor (anchor unchanged) to `flat_byte` inside `block`.
    /// Used by mouse-move during click-drag. Phase 2g.D: drag may now cross
    /// paragraph boundaries — when the cursor lands in a different block than
    /// the drag origin, `active_block` follows so the toolbar / arrow-key
    /// state stays anchored to the current cursor block.
    pub(crate) fn extend_rich_doc_caret(
        &mut self,
        block: usize,
        flat_byte: usize,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        if !payload.drag_active {
            return;
        }
        let new_cursor = payload.pos_from_flat_byte(block, flat_byte);
        let mut changed = false;
        if payload.active_block != Some(block) {
            payload.active_block = Some(block);
            changed = true;
        }
        if let Some(sel) = payload.selection.as_mut() {
            if sel.cursor != new_cursor {
                sel.cursor = new_cursor;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// Select the word at `flat_byte` inside `block`. Used by double-click.
    /// Phase 2g.A: "word" = a run of `is_alphanumeric() || '_'` chars
    /// concatenated across the block's full text.
    pub(crate) fn select_rich_doc_word(
        &mut self,
        block: usize,
        flat_byte: usize,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.ensure_rich_doc_body_focus(cx);
        window.focus(&handle, cx);
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        payload.active_block = Some(block);
        let text = payload.block_text(block);
        let (lo, hi) = word_bounds_at(&text, flat_byte);
        let anchor = payload.pos_from_flat_byte(block, lo);
        let cursor = payload.pos_from_flat_byte(block, hi);
        payload.selection = Some(crate::rich_doc::TextSelection { anchor, cursor });
        payload.drag_active = false;
        cx.notify();
    }

    /// Triggered on mouse-up — terminate any in-progress drag and discard any
    /// armed-but-uncommitted pending anchor (pure click → release: caret
    /// stays where it was, selection unchanged).
    pub(crate) fn end_rich_doc_drag(&mut self, cx: &mut Context<Self>) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let mut changed = false;
        if payload.drag_active {
            payload.drag_active = false;
            changed = true;
        }
        if payload.pending_drag_anchor.is_some() {
            payload.pending_drag_anchor = None;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// While a rich-doc drag is in progress, nudge the rich-doc pane's
    /// scroll offset when the cursor moves past the top or bottom of the
    /// viewport. Mirrors the per-event scroll logic already in
    /// `src/ui/rich_doc_view.rs` so the timer-driven path and the
    /// MouseMoveEvent path apply the same edge-zone (24px) and per-step
    /// cap (16px) — keeping the scroll feel identical whether the user is
    /// moving the mouse or holding it still past the edge. Returns the
    /// applied delta in pixels (0 when no scroll was needed).
    ///
    /// Mirrors `preview_edge_auto_scroll` in shape; constants differ from
    /// the preview path because rich-doc's per-block listener was tuned
    /// to its own values before this method existed.
    pub(crate) fn rich_doc_edge_auto_scroll(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> Pixels {
        let viewport = self.rich_doc_scroll_handle.bounds();
        let viewport_h = viewport.size.height;
        if viewport_h <= px(0.) {
            return px(0.);
        }
        let edge_zone = px(24.);
        let max_step = px(16.);
        let top_thresh = viewport.top() + edge_zone;
        let bot_thresh = viewport.bottom() - edge_zone;
        let y = position.y;
        let mut dy = px(0.);
        if y < top_thresh {
            let depth = (top_thresh - y).max(px(0.));
            let factor =
                (f32::from(depth.min(edge_zone)) / f32::from(edge_zone)).clamp(0., 1.);
            dy = max_step * factor;
        } else if y > bot_thresh {
            let depth = (y - bot_thresh).max(px(0.));
            let factor =
                (f32::from(depth.min(edge_zone)) / f32::from(edge_zone)).clamp(0., 1.);
            dy = max_step * -factor;
        }
        if dy == px(0.) {
            return px(0.);
        }
        let max = self.rich_doc_scroll_handle.max_offset();
        let mut off = self.rich_doc_scroll_handle.offset();
        let new_y = (off.y + dy).min(px(0.)).max(-max.height);
        let delta = new_y - off.y;
        if delta != px(0.) {
            off.y = new_y;
            self.rich_doc_scroll_handle.set_offset(off);
            cx.notify();
        }
        delta
    }

    /// Kick off (or no-op if already running) a repeating timer that keeps
    /// re-firing edge auto-scroll while the user holds the drag cursor past
    /// the top/bottom edge of the rich-doc viewport. Without this, scrolling
    /// stalls the instant mouse movement stops — NSTextView keeps going.
    ///
    /// Stores the captured cursor position in `rich_doc_last_drag_pos` so the
    /// tick loop has something to re-feed into the edge check. The spawned
    /// task self-cancels when the drag ends, the cursor moves back inside
    /// the viewport, or the captured position clears.
    ///
    /// Tick interval is 30ms — matches preview's continuous-autoscroll.
    pub(crate) fn rich_doc_start_continuous_autoscroll(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_last_drag_pos = Some(position);
        if self.rich_doc_autoscroll_task.is_some() {
            return;
        }
        let viewport = self.rich_doc_scroll_handle.bounds();
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
                        // `drag_active` lives on the rich-doc payload, not on
                        // tab.selection like the preview path. Pull it through
                        // the active tab; bail if the tab has no rich-doc or
                        // the drag flag has cleared.
                        let drag_active = w
                            .tabs
                            .get(w.active)
                            .and_then(|t| t.buffer.rich_doc.as_ref())
                            .map(|p| p.drag_active)
                            .unwrap_or(false);
                        if !drag_active {
                            w.rich_doc_autoscroll_task = None;
                            w.rich_doc_last_drag_pos = None;
                            return false;
                        }
                        let Some(pos) = w.rich_doc_last_drag_pos else {
                            w.rich_doc_autoscroll_task = None;
                            return false;
                        };
                        let viewport = w.rich_doc_scroll_handle.bounds();
                        let viewport_h = viewport.size.height;
                        if viewport_h <= px(0.) {
                            w.rich_doc_autoscroll_task = None;
                            return false;
                        }
                        let top = viewport.origin.y;
                        let bottom = top + viewport_h;
                        if pos.y >= top && pos.y <= bottom {
                            // Cursor walked back inside the viewport — stop
                            // ticking. Next per-block MouseMoveEvent past an
                            // edge will restart us.
                            w.rich_doc_autoscroll_task = None;
                            return false;
                        }
                        w.rich_doc_edge_auto_scroll(pos, cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        });
        self.rich_doc_autoscroll_task = Some(task);
    }

    /// Cancel the rich-doc continuous-autoscroll timer (if any) and clear the
    /// captured drag position. Called from the per-block mouse-up handler so
    /// the timer doesn't outlive the gesture. Dropping the `Task` cancels
    /// the spawned future.
    pub(crate) fn rich_doc_stop_continuous_autoscroll(&mut self, _cx: &mut Context<Self>) {
        self.rich_doc_autoscroll_task = None;
        self.rich_doc_last_drag_pos = None;
    }

    /// Promote a `pending_drag_anchor` armed by a blank-area mouse-down to a
    /// real drag (first MouseMove with the button held). Sets the selection
    /// anchor + cursor to the pending position, flips `drag_active = true`,
    /// updates `active_block`, and clears `pending_drag_anchor`. No-op when
    /// no pending anchor exists. Per-block mouse-move handlers then take
    /// over for cross-block extension.
    pub(crate) fn commit_pending_rich_doc_drag(&mut self, cx: &mut Context<Self>) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(anchor) = payload.pending_drag_anchor.take() else {
            return;
        };
        payload.active_block = Some(anchor.block);
        payload.selection = Some(crate::rich_doc::TextSelection {
            anchor,
            cursor: anchor,
        });
        payload.drag_active = true;
        cx.notify();
    }

    /// Insert `text` at the active rich-doc cursor / replacing the active
    /// selection. Wired from `EntityInputHandler::replace_text_in_range`
    /// when the focus is RichDocBody.
    pub(crate) fn rich_doc_insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        if text.is_empty() {
            return;
        }
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(sel) = payload.selection else {
            return;
        };
        // Delete any existing selection first (single-block or cross-block);
        // then insert at the (now collapsed) cursor.
        let insert_pos = if !sel.is_collapsed() {
            let (lo, hi) = sel.range_full();
            if lo.block == hi.block {
                let block = lo.block;
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteRange {
                    block,
                    run_lo: lo.run,
                    byte_lo: lo.byte,
                    run_hi: hi.run,
                    byte_hi: hi.byte,
                });
                crate::rich_doc::TextPos { block, run: lo.run, byte: lo.byte }
            } else {
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteAcrossBlocks {
                    from_block: lo.block,
                    from_run: lo.run,
                    from_byte: lo.byte,
                    to_block: hi.block,
                    to_run: hi.run,
                    to_byte: hi.byte,
                });
                crate::rich_doc::TextPos { block: lo.block, run: lo.run, byte: lo.byte }
            }
        } else {
            sel.cursor
        };
        payload.apply_edit(crate::rich_doc::edits::Edit::InsertText {
            block: insert_pos.block,
            run: insert_pos.run,
            byte: insert_pos.byte,
            text: text.to_string(),
        });
        // Advance cursor by text.len() bytes inside the same run.
        let new_byte = insert_pos.byte + text.len();
        let new_pos = crate::rich_doc::TextPos {
            block: insert_pos.block,
            run: insert_pos.run,
            byte: new_byte,
        };
        payload.active_block = Some(insert_pos.block);
        payload.selection = Some(crate::rich_doc::TextSelection {
            anchor: new_pos,
            cursor: new_pos,
        });
        self.after_rich_doc_edit(cx);
    }

    /// Phase 2g.B — Enter inside the rich-doc body splits the current
    /// paragraph at the cursor. Selection (if any) is deleted first; the
    /// remaining single-position cursor then splits, leaving the right half
    /// as a new Paragraph at block+1 and dropping the cursor at its start.
    pub(crate) fn rich_doc_insert_paragraph_break(&mut self, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(sel) = payload.selection else {
            return;
        };
        // 1) Collapse the selection if it's non-empty.
        let pos = if !sel.is_collapsed() {
            let (lo, hi) = sel.range_full();
            if lo.block == hi.block {
                let block = lo.block;
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteRange {
                    block,
                    run_lo: lo.run,
                    byte_lo: lo.byte,
                    run_hi: hi.run,
                    byte_hi: hi.byte,
                });
                crate::rich_doc::TextPos { block, run: lo.run, byte: lo.byte }
            } else {
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteAcrossBlocks {
                    from_block: lo.block,
                    from_run: lo.run,
                    from_byte: lo.byte,
                    to_block: hi.block,
                    to_run: hi.run,
                    to_byte: hi.byte,
                });
                crate::rich_doc::TextPos { block: lo.block, run: lo.run, byte: lo.byte }
            }
        } else {
            sel.cursor
        };
        // 2) Split at pos. After split the right half lives at block+1.
        payload.apply_edit(crate::rich_doc::edits::Edit::SplitBlock {
            block: pos.block,
            run: pos.run,
            byte: pos.byte,
        });
        let new_block = pos.block + 1;
        let new_pos = crate::rich_doc::TextPos { block: new_block, run: 0, byte: 0 };
        payload.active_block = Some(new_block);
        payload.selection = Some(crate::rich_doc::TextSelection {
            anchor: new_pos,
            cursor: new_pos,
        });
        self.after_rich_doc_edit(cx);
    }

    /// Delete the active selection if non-empty, else delete one grapheme
    /// before the cursor (Backspace) or after (forward Delete).
    /// Phase 2g.B: Backspace at byte 0 of a block merges into the previous
    /// paragraph; forward Delete at the end of a block merges in the next.
    /// Cross-block selections delete the entire span and collapse the blocks
    /// in between.
    pub(crate) fn rich_doc_delete(
        &mut self,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(sel) = payload.selection else {
            return;
        };
        if !sel.is_collapsed() {
            let (lo, hi) = sel.range_full();
            if lo.block == hi.block {
                let block = lo.block;
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteRange {
                    block,
                    run_lo: lo.run,
                    byte_lo: lo.byte,
                    run_hi: hi.run,
                    byte_hi: hi.byte,
                });
                let collapsed = crate::rich_doc::TextPos {
                    block,
                    run: lo.run,
                    byte: lo.byte,
                };
                payload.active_block = Some(block);
                payload.selection = Some(crate::rich_doc::TextSelection {
                    anchor: collapsed,
                    cursor: collapsed,
                });
            } else {
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteAcrossBlocks {
                    from_block: lo.block,
                    from_run: lo.run,
                    from_byte: lo.byte,
                    to_block: hi.block,
                    to_run: hi.run,
                    to_byte: hi.byte,
                });
                let collapsed = crate::rich_doc::TextPos {
                    block: lo.block,
                    run: lo.run,
                    byte: lo.byte,
                };
                payload.active_block = Some(lo.block);
                payload.selection = Some(crate::rich_doc::TextSelection {
                    anchor: collapsed,
                    cursor: collapsed,
                });
            }
            self.after_rich_doc_edit(cx);
            return;
        }
        // Collapsed — delete one grapheme, or merge with neighbor at boundary.
        let block = sel.cursor.block;
        let flat = payload.flat_byte_for(sel.cursor);
        let text = payload.block_text(block);
        if forward {
            if flat < text.len() {
                let mut hi = flat + 1;
                while hi < text.len() && !text.is_char_boundary(hi) {
                    hi += 1;
                }
                let lo_pos = payload.pos_from_flat_byte(block, flat);
                let hi_pos = payload.pos_from_flat_byte(block, hi);
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteRange {
                    block,
                    run_lo: lo_pos.run,
                    byte_lo: lo_pos.byte,
                    run_hi: hi_pos.run,
                    byte_hi: hi_pos.byte,
                });
                payload.selection = Some(crate::rich_doc::TextSelection {
                    anchor: lo_pos,
                    cursor: lo_pos,
                });
                self.after_rich_doc_edit(cx);
                return;
            }
            // At end of block — try merging with next paragraph/heading.
            if block + 1 >= payload.ast.blocks.len() {
                return;
            }
            let next_is_textual = matches!(
                payload.ast.blocks.get(block + 1),
                Some(crate::rich_doc::ast::Block::Paragraph { .. })
                    | Some(crate::rich_doc::ast::Block::Heading { .. })
            );
            if !next_is_textual {
                return;
            }
            // Cursor stays in `block` at its current end.
            let keep_pos = payload.block_end(block);
            payload.apply_edit(crate::rich_doc::edits::Edit::MergeWithNext { block });
            payload.active_block = Some(block);
            payload.selection = Some(crate::rich_doc::TextSelection {
                anchor: keep_pos,
                cursor: keep_pos,
            });
            self.after_rich_doc_edit(cx);
            return;
        }
        // Backspace branch.
        if flat > 0 {
            let mut lo = flat - 1;
            while lo > 0 && !text.is_char_boundary(lo) {
                lo -= 1;
            }
            let lo_pos = payload.pos_from_flat_byte(block, lo);
            let hi_pos = payload.pos_from_flat_byte(block, flat);
            payload.apply_edit(crate::rich_doc::edits::Edit::DeleteRange {
                block,
                run_lo: lo_pos.run,
                byte_lo: lo_pos.byte,
                run_hi: hi_pos.run,
                byte_hi: hi_pos.byte,
            });
            payload.selection = Some(crate::rich_doc::TextSelection {
                anchor: lo_pos,
                cursor: lo_pos,
            });
            self.after_rich_doc_edit(cx);
            return;
        }
        // Backspace at byte 0 — merge with previous paragraph/heading.
        if block == 0 {
            return;
        }
        let prev = block - 1;
        let prev_is_textual = matches!(
            payload.ast.blocks.get(prev),
            Some(crate::rich_doc::ast::Block::Paragraph { .. })
                | Some(crate::rich_doc::ast::Block::Heading { .. })
        );
        if !prev_is_textual {
            return;
        }
        // Cursor lands at the previous block's old end (boundary point).
        let join_pos = payload.block_end(prev);
        payload.apply_edit(crate::rich_doc::edits::Edit::MergeWithNext { block: prev });
        payload.active_block = Some(prev);
        payload.selection = Some(crate::rich_doc::TextSelection {
            anchor: join_pos,
            cursor: join_pos,
        });
        self.after_rich_doc_edit(cx);
    }

    /// Move the rich-doc cursor left / right one grapheme. Extends the
    /// selection (anchor unchanged) when `shift` is held.
    /// Phase 2g.B: stepping past the start / end of a block crosses into
    /// the previous / next paragraph/heading.
    pub(crate) fn rich_doc_move_cursor(
        &mut self,
        delta: i32,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(sel) = payload.selection else {
            return;
        };
        let block = sel.cursor.block;
        let text = payload.block_text(block);
        let flat = payload.flat_byte_for(sel.cursor);
        let new_cursor = if delta < 0 {
            if flat == 0 {
                // Cross to end of previous textual block, if any.
                let prev = find_prev_textual_block(payload, block);
                match prev {
                    Some(p) => payload.block_end(p),
                    None => sel.cursor,
                }
            } else {
                let mut lo = flat - 1;
                while lo > 0 && !text.is_char_boundary(lo) {
                    lo -= 1;
                }
                payload.pos_from_flat_byte(block, lo)
            }
        } else if flat >= text.len() {
            let next = find_next_textual_block(payload, block);
            match next {
                Some(n) => payload.block_start(n),
                None => sel.cursor,
            }
        } else {
            let mut hi = flat + 1;
            while hi < text.len() && !text.is_char_boundary(hi) {
                hi += 1;
            }
            payload.pos_from_flat_byte(block, hi)
        };
        let new_sel = if shift {
            crate::rich_doc::TextSelection { anchor: sel.anchor, cursor: new_cursor }
        } else {
            crate::rich_doc::TextSelection { anchor: new_cursor, cursor: new_cursor }
        };
        if !shift {
            payload.active_block = Some(new_cursor.block);
        } else {
            payload.active_block = Some(new_cursor.block);
        }
        payload.selection = Some(new_sel);
        cx.notify();
    }

    /// Phase 2g.B — Up/Down arrow moves the rich-doc cursor to the previous
    /// or next paragraph/heading. There's no shaped-line cache exposed to
    /// figure out exact column, so we snap to the end of the neighbor when
    /// going up (caret slides to "as far right as the prev line allows")
    /// and start when going down. Shift extends the selection.
    pub(crate) fn rich_doc_move_vertical(
        &mut self,
        delta: i32,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(sel) = payload.selection else {
            return;
        };
        let block = sel.cursor.block;
        let flat = payload.flat_byte_for(sel.cursor);
        let target_block = if delta < 0 {
            find_prev_textual_block(payload, block)
        } else {
            find_next_textual_block(payload, block)
        };
        let new_cursor = match target_block {
            Some(b) => {
                let dest_len = payload.block_total_bytes(b);
                payload.pos_from_flat_byte(b, flat.min(dest_len))
            }
            None => {
                if delta < 0 {
                    payload.block_start(block)
                } else {
                    payload.block_end(block)
                }
            }
        };
        let new_sel = if shift {
            crate::rich_doc::TextSelection { anchor: sel.anchor, cursor: new_cursor }
        } else {
            crate::rich_doc::TextSelection { anchor: new_cursor, cursor: new_cursor }
        };
        payload.active_block = Some(new_cursor.block);
        payload.selection = Some(new_sel);
        cx.notify();
    }

    /// Triple-click — select the entire active block (paragraph).
    pub(crate) fn rich_doc_select_paragraph(
        &mut self,
        block: usize,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.ensure_rich_doc_body_focus(cx);
        window.focus(&handle, cx);
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        if block >= payload.ast.blocks.len() {
            return;
        }
        let anchor = payload.block_start(block);
        let cursor = payload.block_end(block);
        payload.active_block = Some(block);
        payload.selection = Some(crate::rich_doc::TextSelection { anchor, cursor });
        payload.drag_active = false;
        cx.notify();
    }

    /// Quad-click / Cmd+A — select every textual block in the document.
    pub(crate) fn rich_doc_select_all_document(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.ensure_rich_doc_body_focus(cx);
        window.focus(&handle, cx);
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let first = (0..payload.ast.blocks.len())
            .find(|&b| payload.block_runs(b).is_some());
        let last = (0..payload.ast.blocks.len())
            .rev()
            .find(|&b| payload.block_runs(b).is_some());
        let (Some(first), Some(last)) = (first, last) else {
            return;
        };
        let anchor = payload.block_start(first);
        let cursor = payload.block_end(last);
        payload.active_block = Some(last);
        payload.selection = Some(crate::rich_doc::TextSelection { anchor, cursor });
        payload.drag_active = false;
        cx.notify();
    }

    /// Shift+click — extend the existing selection's cursor to (block, flat_byte),
    /// keeping anchor unchanged. Used by shift+click into a different block.
    pub(crate) fn extend_rich_doc_selection_to(
        &mut self,
        block: usize,
        flat_byte: usize,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.ensure_rich_doc_body_focus(cx);
        window.focus(&handle, cx);
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let new_cursor = payload.pos_from_flat_byte(block, flat_byte);
        let anchor = payload
            .selection
            .map(|s| s.anchor)
            .unwrap_or(new_cursor);
        payload.active_block = Some(block);
        payload.selection = Some(crate::rich_doc::TextSelection {
            anchor,
            cursor: new_cursor,
        });
        payload.drag_active = false;
        cx.notify();
    }

    /// Move cursor to start / end of the active block.
    pub(crate) fn rich_doc_move_to_edge(
        &mut self,
        to_end: bool,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(sel) = payload.selection else {
            return;
        };
        let block = sel.cursor.block;
        let new_cursor = if to_end {
            payload.block_end(block)
        } else {
            payload.block_start(block)
        };
        let new_sel = if shift {
            crate::rich_doc::TextSelection { anchor: sel.anchor, cursor: new_cursor }
        } else {
            crate::rich_doc::TextSelection { anchor: new_cursor, cursor: new_cursor }
        };
        payload.selection = Some(new_sel);
        cx.notify();
    }

    /// Select the entire active block (Cmd/Ctrl+A inside rich-doc body).
    pub(crate) fn rich_doc_select_block(&mut self, cx: &mut Context<Self>) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(block) = payload.active_block else {
            return;
        };
        let anchor = payload.block_start(block);
        let cursor = payload.block_end(block);
        payload.selection = Some(crate::rich_doc::TextSelection { anchor, cursor });
        cx.notify();
    }

    /// Copy the current rich-doc selection to the system clipboard. No-op
    /// when nothing is selected.
    pub(crate) fn rich_doc_copy_selection(&self, cx: &mut Context<Self>) {
        let Some(payload) = self.tabs[self.active].buffer.rich_doc.as_ref() else {
            return;
        };
        let text = payload.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }

    /// Cut: copy + delete in one call.
    pub(crate) fn rich_doc_cut_selection(&mut self, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        self.rich_doc_copy_selection(cx);
        let has_sel = self.tabs[self.active]
            .buffer
            .rich_doc
            .as_ref()
            .and_then(|p| p.selection)
            .map(|s| !s.is_collapsed())
            .unwrap_or(false);
        if has_sel {
            self.rich_doc_delete(false, cx);
        }
    }

    /// Click in non-text whitespace (block row padding or empty pane area) —
    /// grab focus for the rich-doc body and collapse any active selection
    /// to the caret (so the click "deselects everything" without moving the
    /// caret to a new spot). Matches Word / NPP behavior: clicking outside
    /// text deselects in place; the caret only relocates when the click
    /// lands on actual text glyphs. When `prospective_anchor` is `Some`,
    /// also arms a potential drag from that position — see
    /// `pending_drag_anchor` on `RichDocPayload`. Pure click → release
    /// leaves the anchor un-promoted (caret stays put); the first mouse-move
    /// with button held promotes it to a real selection drag via
    /// `commit_pending_rich_doc_drag` in the per-block mouse-move handler.
    pub(crate) fn focus_rich_doc_body_with_anchor(
        &mut self,
        prospective_anchor: Option<crate::rich_doc::TextPos>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.ensure_rich_doc_body_focus(cx);
        window.focus(&handle, cx);
        let tab = self.active_tab_mut();
        if let Some(payload) = tab.buffer.rich_doc.as_mut() {
            if let Some(sel) = payload.selection.as_mut() {
                if sel.anchor != sel.cursor {
                    sel.anchor = sel.cursor;
                }
            }
            payload.drag_active = false;
            payload.pending_drag_anchor = prospective_anchor;
        }
        cx.notify();
    }

    /// Cmd+V into the rich-doc body — normalize line endings, split on `\n`,
    /// and interleave paragraph breaks between segments. Mirrors the IME
    /// `replace_text_in_range` path so Cmd+V and middle-click-paste produce
    /// the same block layout for multi-line clipboard payloads.
    pub(crate) fn rich_doc_paste(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        if text.is_empty() {
            return;
        }
        // Strip NPP's C0-control set (0x00..=0x08, 0x0E..=0x1F, 0x7F) — keep
        // \t (0x09), \n (0x0A), \r (0x0D) so multi-line payloads still split
        // into paragraphs below.
        let cleaned: String = text
            .chars()
            .filter(|c| {
                let cp = *c as u32;
                !matches!(cp, 0x00..=0x08 | 0x0E..=0x1F | 0x7F)
            })
            .collect();
        if cleaned.is_empty() {
            return;
        }
        let normalized = cleaned.replace("\r\n", "\n").replace('\r', "\n");
        // scintilla/win32/ScintillaWin.cxx:3171-3175 — paste replaces the
        // selection and inserts every line as one Undo action.
        self.with_rich_doc_edit_group(cx, |w, cx| {
            let mut first = true;
            for segment in normalized.split('\n') {
                if !first {
                    w.rich_doc_insert_paragraph_break(cx);
                }
                if !segment.is_empty() {
                    w.rich_doc_insert_text(segment, cx);
                }
                first = false;
            }
        });
    }

    /// Phase 2g.K — splice pre-decoded formatted blocks at the caret. Driven
    /// by the paste-format chooser modal's "Keep Formatting" button. Deletes
    /// any active selection first (single-block or cross-block), then runs
    /// the InsertBlocks edit so the change participates in the undo stack
    /// like any other rich-doc edit. Caret lands at the end of the spliced
    /// content so a follow-up keystroke continues typing where the paste
    /// finished — same behavior as plain `rich_doc_paste`.
    pub(crate) fn rich_doc_paste_formatted(
        &mut self,
        blocks: Vec<crate::rich_doc::ast::Block>,
        cx: &mut Context<Self>,
    ) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        if blocks.is_empty() {
            return;
        }
        self.with_rich_doc_edit_group(cx, |w, cx| w.rich_doc_paste_formatted_inner(blocks, cx));
    }

    fn rich_doc_paste_formatted_inner(
        &mut self,
        blocks: Vec<crate::rich_doc::ast::Block>,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(sel) = payload.selection else {
            return;
        };
        // Collapse any selection — InsertBlocks needs a single caret position.
        let insert_pos = if !sel.is_collapsed() {
            let (lo, hi) = sel.range_full();
            if lo.block == hi.block {
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteRange {
                    block: lo.block,
                    run_lo: lo.run,
                    byte_lo: lo.byte,
                    run_hi: hi.run,
                    byte_hi: hi.byte,
                });
                crate::rich_doc::TextPos { block: lo.block, run: lo.run, byte: lo.byte }
            } else {
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteAcrossBlocks {
                    from_block: lo.block,
                    from_run: lo.run,
                    from_byte: lo.byte,
                    to_block: hi.block,
                    to_run: hi.run,
                    to_byte: hi.byte,
                });
                crate::rich_doc::TextPos { block: lo.block, run: lo.run, byte: lo.byte }
            }
        } else {
            sel.cursor
        };
        let blocks_count = blocks.len();
        // Total text bytes contributed by the last incoming block — needed to
        // place the caret at the seam between the pasted content and any
        // suffix that merged in after it.
        let last_block_text_len = blocks
            .last()
            .and_then(|b| match b {
                crate::rich_doc::ast::Block::Paragraph { runs, .. }
                | crate::rich_doc::ast::Block::Heading { runs, .. } => Some(
                    runs.iter().map(|r| r.text.len()).sum::<usize>(),
                ),
                _ => None,
            })
            .unwrap_or(0);
        // Flat byte offset of the cursor inside its host block, captured
        // before the edit reshapes the run structure. After a single-block
        // paste the host gains `last_block_text_len` bytes, so the caret's
        // new flat offset is the original + the pasted run length.
        let flat_at_insert = payload.flat_byte_for(insert_pos);
        payload.apply_edit(crate::rich_doc::edits::Edit::InsertBlocks {
            at_block: insert_pos.block,
            at_run: insert_pos.run,
            at_byte: insert_pos.byte,
            blocks,
        });
        // For a single-block paste the host block keeps its index but its
        // run structure changes — translate the desired flat byte through
        // `pos_from_flat_byte` so the caret lands on a real run boundary.
        // Multi-block pastes leave the last incoming block at
        // `insert_pos.block + (blocks_count - 1)` (the suffix half, possibly
        // merged with the trailing incoming paragraph).
        let new_pos = if blocks_count == 1 {
            let target_flat = flat_at_insert + last_block_text_len;
            payload.pos_from_flat_byte(insert_pos.block, target_flat)
        } else {
            let target_block = insert_pos.block + blocks_count.saturating_sub(1);
            payload.pos_from_flat_byte(target_block, last_block_text_len)
        };
        payload.active_block = Some(new_pos.block);
        payload.selection = Some(crate::rich_doc::TextSelection {
            anchor: new_pos,
            cursor: new_pos,
        });
        self.after_rich_doc_edit(cx);
    }

    /// Insert a tab character at the cursor. Phase 2g.B does not implement
    /// indent-by-selection; this just types `\t` into the active run.
    pub(crate) fn rich_doc_insert_tab(&mut self, cx: &mut Context<Self>) {
        // scintilla/src/Editor.cxx:2094 — selected text replacement is one Undo.
        self.with_rich_doc_edit_group(cx, |w, cx| w.rich_doc_insert_text("\t", cx));
    }

    /// Option+Arrow word jump inside the rich-doc body. `dir < 0` walks left,
    /// `dir > 0` walks right. Word boundaries follow `word_bounds_at` rules
    /// (alphanumeric + `_`). When the cursor is at the block edge, crosses
    /// into the adjacent textual block (end-of-prev on left, start-of-next on
    /// right) so word-jump traverses paragraph boundaries like a real editor.
    pub(crate) fn rich_doc_move_word(
        &mut self,
        dir: i32,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(sel) = payload.selection else {
            return;
        };
        let block = sel.cursor.block;
        let text = payload.block_text(block);
        let flat = payload.flat_byte_for(sel.cursor);
        let new_cursor = if dir < 0 {
            if flat == 0 {
                match find_prev_textual_block(payload, block) {
                    Some(p) => payload.block_end(p),
                    None => sel.cursor,
                }
            } else {
                let target = prev_word_boundary_in(&text, flat);
                payload.pos_from_flat_byte(block, target)
            }
        } else if flat >= text.len() {
            match find_next_textual_block(payload, block) {
                Some(n) => payload.block_start(n),
                None => sel.cursor,
            }
        } else {
            let target = next_word_boundary_in(&text, flat);
            payload.pos_from_flat_byte(block, target)
        };
        let new_sel = if shift {
            crate::rich_doc::TextSelection { anchor: sel.anchor, cursor: new_cursor }
        } else {
            crate::rich_doc::TextSelection { anchor: new_cursor, cursor: new_cursor }
        };
        payload.active_block = Some(new_cursor.block);
        payload.selection = Some(new_sel);
        cx.notify();
    }

    /// Cmd+Up / Cmd+Down / Cmd+Home / Cmd+End — jump to the start or end of
    /// the entire rich-doc document. `to_end=true` parks the cursor past the
    /// last byte of the final textual block; `false` parks at (first, 0, 0).
    pub(crate) fn rich_doc_move_doc_edge(
        &mut self,
        to_end: bool,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(sel) = payload.selection else {
            return;
        };
        let target_block = if to_end {
            (0..payload.ast.blocks.len()).rev().find(|&b| payload.block_runs(b).is_some())
        } else {
            (0..payload.ast.blocks.len()).find(|&b| payload.block_runs(b).is_some())
        };
        let Some(b) = target_block else { return };
        let new_cursor = if to_end {
            payload.block_end(b)
        } else {
            payload.block_start(b)
        };
        let new_sel = if shift {
            crate::rich_doc::TextSelection { anchor: sel.anchor, cursor: new_cursor }
        } else {
            crate::rich_doc::TextSelection { anchor: new_cursor, cursor: new_cursor }
        };
        payload.active_block = Some(new_cursor.block);
        payload.selection = Some(new_sel);
        cx.notify();
    }

    /// PageUp / PageDown — approximation: walk N textual blocks at once.
    /// `lines` is the viewport's page size; we treat each block as ~1 line
    /// because there's no shaped-line cache to do true wrap-aware paging.
    /// Better than nothing — at minimum, a press moves a page-worth of blocks.
    pub(crate) fn rich_doc_move_page(
        &mut self,
        dir: i32,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let lines = self.viewport_page_lines();
        for _ in 0..lines {
            self.rich_doc_move_vertical(dir, shift, cx);
        }
    }

    /// Viewport page size in lines (clamped to at least 1). Mirrors the
    /// plain-text `page_size_lines` helper but lives here so the rich-doc
    /// helpers above can reach it without crossing module boundaries.
    fn viewport_page_lines(&self) -> usize {
        let lh = self.viewport.line_height;
        if lh <= gpui::px(0.) {
            return 1;
        }
        super::movement::lines_per_page(self.viewport.bounds.size.height / lh).unsigned_abs()
            as usize
    }

    fn rich_doc_set_alignment(
        &mut self,
        alignment: crate::rich_doc::ast::Alignment,
        cx: &mut Context<Self>,
    ) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(block) = payload.active_block else {
            return;
        };
        payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockAlignment { block, alignment });
        self.after_rich_doc_edit(cx);
    }

    fn rich_doc_apply_indent_delta(&mut self, delta: i8, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        let Some(block) = payload.active_block else {
            return;
        };
        payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockIndent { block, delta });
        self.after_rich_doc_edit(cx);
    }

    /// Open the inline rich-doc color-picker bar over the active block.
    /// Closes any other rich-doc bar first so the two can't co-exist. The
    /// bar lives until Esc, the × close button, or another open call;
    /// every keystroke / swatch click in the bar live-applies through
    /// `apply_rich_doc_color_buffer`.
    pub(crate) fn open_rich_doc_color_bar(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.tabs[self.active]
            .buffer
            .rich_doc
            .as_ref()
            .and_then(|p| p.active_block)
        else {
            return;
        };
        let original = self.tabs[self.active]
            .buffer
            .rich_doc
            .as_ref()
            .and_then(|p| p.block_color(block));
        let hex = original.map(format_hex_rgb).unwrap_or_default();
        self.close_rich_doc_link_bar(cx);
        if self.rich_doc_color_focus.is_none() {
            self.rich_doc_color_focus = Some(cx.focus_handle());
        }
        self.rich_doc_color_bar = Some(crate::find_state::RichDocColorBarState::new(
            block, &hex, original,
        ));
        self.focus_input(InputFocus::RichDocColorHex, window, cx);
    }

    pub(crate) fn close_rich_doc_color_bar(&mut self, cx: &mut Context<Self>) {
        if self.rich_doc_color_bar.take().is_some() {
            cx.notify();
        }
    }

    /// Apply the active color buffer to the bar's target block. Called
    /// after every hex keystroke and swatch click. Empty buffer means
    /// "no color set yet" — skip until the user types something or clicks
    /// a swatch; partial buffers (1-5 chars) likewise skip. Six-char
    /// buffers parse and commit.
    pub(crate) fn apply_rich_doc_color_buffer(&mut self, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let Some(state) = self.rich_doc_color_bar.as_ref() else {
            return;
        };
        let target_block = state.target_block;
        if state.text.len() != 6 {
            return;
        }
        let Some(color) = parse_hex_rgb(&state.text) else {
            return;
        };
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        if payload.ast.blocks.get(target_block).is_none() {
            return;
        }
        payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockColor {
            block: target_block,
            color: Some(color),
        });
        self.after_rich_doc_edit(cx);
    }

    /// Click handler for a swatch tile in the color bar — overwrites the
    /// hex buffer with the swatch's value and live-applies. Also syncs the
    /// HSV picker thumbs so the More-colours popover (if open) tracks.
    pub(crate) fn rich_doc_color_pick_swatch(&mut self, rgb_val: u32, cx: &mut Context<Self>) {
        if let Some(state) = self.rich_doc_color_bar.as_mut() {
            let hex = format_hex_rgb(rgb_val);
            state.text_cursor = hex.len();
            state.text_anchor = hex.len();
            state.text = hex;
        }
        crate::ui::style_configurator::hsv_surface::sync_from_buffer(self);
        self.apply_rich_doc_color_buffer(cx);
    }

    /// Toggle the "More colors…" popover next to the color bar's quick-pick
    /// swatches. No-op when the bar isn't open. When transitioning closed →
    /// open, syncs the HSV picker thumbs from the current hex buffer so the
    /// SV / hue thumbs land on the live colour right away.
    pub(crate) fn toggle_rich_doc_more_colors(&mut self, cx: &mut Context<Self>) {
        let mut opened = false;
        if let Some(state) = self.rich_doc_color_bar.as_mut() {
            state.more_colors_open = !state.more_colors_open;
            opened = state.more_colors_open;
            cx.notify();
        }
        if opened {
            crate::ui::style_configurator::hsv_surface::sync_from_buffer(self);
        }
    }

    /// Close the "More colors…" popover without dismissing the bar itself.
    /// Used by the scrim's click-out handler.
    pub(crate) fn close_rich_doc_more_colors(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.rich_doc_color_bar.as_mut() {
            if state.more_colors_open {
                state.more_colors_open = false;
                cx.notify();
            }
        }
    }

    /// Popover-swatch click: apply the colour via the same path as the inline
    /// quick swatches, then close the popover. The bar itself stays open so
    /// the user can keep fine-tuning (clicking another popover swatch
    /// reopens it via the More button).
    pub(crate) fn rich_doc_color_pick_more_swatch(
        &mut self,
        rgb_val: u32,
        cx: &mut Context<Self>,
    ) {
        self.rich_doc_color_pick_swatch(rgb_val, cx);
        self.close_rich_doc_more_colors(cx);
    }

    /// Clear button: strip the color from the active block and dismiss
    /// the bar.
    pub(crate) fn rich_doc_color_clear(&mut self, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let Some(state) = self.rich_doc_color_bar.as_ref() else {
            return;
        };
        let target_block = state.target_block;
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        if payload.ast.blocks.get(target_block).is_none() {
            return;
        }
        payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockColor {
            block: target_block,
            color: None,
        });
        self.after_rich_doc_edit(cx);
        self.close_rich_doc_color_bar(cx);
    }

    /// Open the inline rich-doc URL prompt bar. Closes any other rich-doc
    /// bar first. Seeds the field with the active block's existing URL (if
    /// any). The bar lives until Esc, ×, Apply, or Unlink.
    pub(crate) fn open_rich_doc_link_bar(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.tabs[self.active]
            .buffer
            .rich_doc
            .as_ref()
            .and_then(|p| p.active_block)
        else {
            return;
        };
        let existing = self.tabs[self.active]
            .buffer
            .rich_doc
            .as_ref()
            .and_then(|p| p.block_link(block))
            .map(|s| s.to_string());
        let had_link = existing.is_some();
        let initial = existing.unwrap_or_default();
        self.close_rich_doc_color_bar(cx);
        if self.rich_doc_link_focus.is_none() {
            self.rich_doc_link_focus = Some(cx.focus_handle());
        }
        self.rich_doc_link_bar = Some(crate::find_state::RichDocLinkBarState::new(
            block, &initial, had_link,
        ));
        self.focus_input(InputFocus::RichDocLinkUrl, window, cx);
    }

    pub(crate) fn close_rich_doc_link_bar(&mut self, cx: &mut Context<Self>) {
        if self.rich_doc_link_bar.take().is_some() {
            cx.notify();
        }
    }

    /// Apply the typed URL to the bar's target block. Empty URL is treated
    /// as Unlink (matches what a user would expect from clearing the box
    /// then hitting Apply). Trims surrounding whitespace.
    pub(crate) fn commit_rich_doc_link_bar(&mut self, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let Some(state) = self.rich_doc_link_bar.as_ref() else {
            return;
        };
        let target_block = state.target_block;
        let trimmed = state.text.trim().to_string();
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        if payload.ast.blocks.get(target_block).is_none() {
            return;
        }
        let url = if trimmed.is_empty() { None } else { Some(trimmed) };
        payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockLink {
            block: target_block,
            url,
        });
        self.after_rich_doc_edit(cx);
        self.close_rich_doc_link_bar(cx);
    }

    /// Unlink button: strip the link and dismiss the bar without
    /// committing the typed URL.
    pub(crate) fn rich_doc_link_unlink(&mut self, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let Some(state) = self.rich_doc_link_bar.as_ref() else {
            return;
        };
        let target_block = state.target_block;
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        if payload.ast.blocks.get(target_block).is_none() {
            return;
        }
        payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockLink {
            block: target_block,
            url: None,
        });
        self.after_rich_doc_edit(cx);
        self.close_rich_doc_link_bar(cx);
    }

    fn rich_doc_toggle_style(&mut self, axis: RichDocStyleAxis, cx: &mut Context<Self>) {
        if !self.rich_doc_edit_allowed(cx) {
            return;
        }
        let tab = self.active_tab_mut();
        let Some(payload) = tab.buffer.rich_doc.as_mut() else {
            return;
        };
        // Phase 2g.H — if there's a non-empty selection, apply to the
        // selected sub-range (splits runs at the edges so only the selected
        // text changes). Otherwise fall back to whole-block toggling on
        // active_block (existing behavior: user clicks somewhere and presses
        // B → bolds the paragraph, useful for headings / one-liners).
        if let Some(sel) = payload.selection.filter(|s| !s.is_collapsed()) {
            let (lo, hi) = sel.range_full();
            let predicate: fn(&crate::rich_doc::ast::InlineStyle) -> bool = match axis {
                RichDocStyleAxis::Bold => |s| s.bold,
                RichDocStyleAxis::Italic => |s| s.italic,
                RichDocStyleAxis::Underline => |s| s.underline,
                RichDocStyleAxis::Strike => |s| s.strike,
            };
            let all_have = range_all_have_style(payload, lo, hi, predicate);
            let on = !all_have;
            let edit = match axis {
                RichDocStyleAxis::Bold => crate::rich_doc::edits::Edit::SetRangeBold {
                    from_block: lo.block,
                    from_run: lo.run,
                    from_byte: lo.byte,
                    to_block: hi.block,
                    to_run: hi.run,
                    to_byte: hi.byte,
                    on,
                },
                RichDocStyleAxis::Italic => crate::rich_doc::edits::Edit::SetRangeItalic {
                    from_block: lo.block,
                    from_run: lo.run,
                    from_byte: lo.byte,
                    to_block: hi.block,
                    to_run: hi.run,
                    to_byte: hi.byte,
                    on,
                },
                RichDocStyleAxis::Underline => crate::rich_doc::edits::Edit::SetRangeUnderline {
                    from_block: lo.block,
                    from_run: lo.run,
                    from_byte: lo.byte,
                    to_block: hi.block,
                    to_run: hi.run,
                    to_byte: hi.byte,
                    on,
                },
                RichDocStyleAxis::Strike => crate::rich_doc::edits::Edit::SetRangeStrike {
                    from_block: lo.block,
                    from_run: lo.run,
                    from_byte: lo.byte,
                    to_block: hi.block,
                    to_run: hi.run,
                    to_byte: hi.byte,
                    on,
                },
            };
            payload.apply_edit(edit);
            self.after_rich_doc_edit(cx);
            return;
        }
        // No-selection path — fall back to whole-block toggle, using
        // active_block. Same as the Phase 2 / 5b behavior.
        let Some(block) = payload.active_block else {
            return;
        };
        let was_on = match axis {
            RichDocStyleAxis::Bold => payload.block_is_bold(block),
            RichDocStyleAxis::Italic => payload.block_is_italic(block),
            RichDocStyleAxis::Underline => payload.block_is_underline(block),
            RichDocStyleAxis::Strike => payload.block_is_strike(block),
        };
        let edit = match axis {
            RichDocStyleAxis::Bold => crate::rich_doc::edits::Edit::SetBlockBold {
                block,
                on: !was_on,
            },
            RichDocStyleAxis::Italic => crate::rich_doc::edits::Edit::SetBlockItalic {
                block,
                on: !was_on,
            },
            RichDocStyleAxis::Underline => crate::rich_doc::edits::Edit::SetBlockUnderline {
                block,
                on: !was_on,
            },
            RichDocStyleAxis::Strike => crate::rich_doc::edits::Edit::SetBlockStrike {
                block,
                on: !was_on,
            },
        };
        payload.apply_edit(edit);
        self.after_rich_doc_edit(cx);
    }

    /// `File ▸ Save As .docx` — only meaningful for legacy `.doc` tabs (the
    /// binary format can't be re-serialized). Pops the native save-as
    /// dialog defaulting the file name to the current document's stem with
    /// a `.docx` extension, then writes a fresh `.docx` via the AST. The
    /// original `.doc` file is never touched. On success, opens the new
    /// `.docx` in a fresh tab so the user can see the converted result.
    pub(crate) fn rich_doc_save_as_docx(
        &mut self,
        _: &crate::actions::rich_doc::RichDocSaveAsDocx,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let buf = self.buffer();
        let Some(payload) = buf.rich_doc.as_ref() else {
            return;
        };
        if payload.format != crate::rich_doc::DocFormat::Doc {
            return;
        }
        if buf.dirty {
            let raw = buf.rope.to_string();
            if let Err(err) = crate::rich_doc::validate_raw_view(payload, Some(&raw)) {
                self.set_status(format!("Save As failed: {err}"));
                cx.notify();
                return;
            }
        }
        let payload_owned = payload.clone();
        let starting_dir = buf
            .file_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(std::path::Path::to_path_buf);
        let suggested = buf
            .file_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| format!("{}.docx", s.to_string_lossy()))
            .unwrap_or_else(|| "Untitled.docx".to_string());

        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Save As .docx")
                .set_file_name(&suggested)
                .add_filter("Word Document (.docx)", &["docx"]);
            if let Some(dir) = starting_dir {
                dialog = dialog.set_directory(dir);
            }
            let Some(handle) = dialog.save_file().await else {
                return;
            };
            let target = handle.path().to_path_buf();
            crate::settings::record_last_used_directory(&target);
            let refused = this
                .update(cx, |w, cx| {
                    let active = w.active;
                    w.save_target_open_elsewhere(&target, active, cx)
                })
                .unwrap_or(false);
            if refused {
                super::show_already_open_error().await;
                return;
            }
            let save_result = crate::rich_doc::doc::save_as_docx(&payload_owned, &target);
            let _ = this.update(cx, |w, cx| match save_result {
                Ok(()) => {
                    let name = target
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "saved".to_string());
                    w.set_status(format!("Saved {name}"));
                    w.open_path(target, cx);
                    cx.notify();
                }
                Err(err) => {
                    w.set_status(format!("Save As .docx failed: {err}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// `View ▸ Folder as Workspace` toggle. Flips the persisted visibility,
    /// re-renders. The panel itself only mounts when `folder_panel_visible
    /// && !folder_panel_roots.is_empty()` — toggling on with no roots yet
    /// shows an "empty state" hint via the panel's own render.
    pub(crate) fn toggle_folder_panel(
        &mut self,
        _: &ViewToggleFolderPanel,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snap = crate::settings::snapshot();
        crate::ui::folder_panel::set_visible(!snap.folder_panel_visible);
        cx.notify();
    }

    pub(crate) fn toggle_word_wrap(
        &mut self,
        _: &ViewWordWrap,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.word_wrap = !self.word_wrap;
        // Drop every wrap cache — they're keyed implicitly by `word_wrap` so
        // turning off and on must rebuild.
        for tab in self.tabs.iter_mut() {
            tab.wrap_cache = None;
            if self.word_wrap {
                tab.scroll_left = gpui::px(0.);
            }
        }
        self.save_session(cx);
        // Re-derive the cursor's visible position under the new layout.
        self.ensure_cursor_visible(cx);
        cx.notify();
    }

    /// Resolve the currently-focused string + cursor + anchor triple so the
    /// input editing helpers above can do their work without each one
    /// repeating the eight-arm match. Returns `None` when the editor is
    /// focused.
    pub(crate) fn input_field_mut(
        &mut self,
        focus: InputFocus,
    ) -> Option<(&mut String, &mut usize, &mut usize)> {
        match focus {
            InputFocus::FindQuery => Some((
                &mut self.find_state.query,
                &mut self.find_state.query_cursor,
                &mut self.find_state.query_anchor,
            )),
            InputFocus::Replacement => Some((
                &mut self.find_state.replacement,
                &mut self.find_state.replacement_cursor,
                &mut self.find_state.replacement_anchor,
            )),
            InputFocus::Goto => Some((
                &mut self.goto_state.text,
                &mut self.goto_state.text_cursor,
                &mut self.goto_state.text_anchor,
            )),
            InputFocus::FifQuery => Some((
                &mut self.find_in_files.query,
                &mut self.find_in_files.query_cursor,
                &mut self.find_in_files.query_anchor,
            )),
            InputFocus::FifReplacement => Some((
                &mut self.find_in_files.replacement,
                &mut self.find_in_files.replacement_cursor,
                &mut self.find_in_files.replacement_anchor,
            )),
            InputFocus::FifFilter => Some((
                &mut self.find_in_files.filter,
                &mut self.find_in_files.filter_cursor,
                &mut self.find_in_files.filter_anchor,
            )),
            InputFocus::FifDirectory => Some((
                &mut self.find_in_files.directory_text,
                &mut self.find_in_files.directory_text_cursor,
                &mut self.find_in_files.directory_text_anchor,
            )),
            InputFocus::MarkQuery => Some((
                &mut self.mark_state.query,
                &mut self.mark_state.query_cursor,
                &mut self.mark_state.query_anchor,
            )),
            InputFocus::MacroPrompt => self
                .macro_prompt
                .as_mut()
                .map(|s| (&mut s.text, &mut s.text_cursor, &mut s.text_anchor)),
            InputFocus::TabRename => self
                .tab_rename
                .as_mut()
                .map(|s| (&mut s.text, &mut s.text_cursor, &mut s.text_anchor)),
            InputFocus::FolderInlineCreate => self
                .folder_inline_create
                .as_mut()
                .map(|s| (&mut s.text, &mut s.text_cursor, &mut s.text_anchor)),
            InputFocus::RichDocLinkUrl => self
                .rich_doc_link_bar
                .as_mut()
                .map(|s| (&mut s.text, &mut s.text_cursor, &mut s.text_anchor)),
            InputFocus::RichDocColorHex => self
                .rich_doc_color_bar
                .as_mut()
                .map(|s| (&mut s.text, &mut s.text_cursor, &mut s.text_anchor)),
            InputFocus::CsvCell => {
                let tab_id = self.tabs[self.active].session_id;
                self.csv_models
                    .get_mut(&tab_id)
                    .and_then(|m| m.cell_input.as_mut())
                    .map(|i| (&mut i.buffer, &mut i.cursor, &mut i.anchor))
            }
            InputFocus::CsvFindBar => {
                let tab_id = self.tabs[self.active].session_id;
                self.csv_models
                    .get_mut(&tab_id)
                    .and_then(|m| m.find_input.as_mut())
                    .map(|i| (&mut i.buffer, &mut i.cursor, &mut i.anchor))
            }
            InputFocus::CsvReplaceBar => {
                let tab_id = self.tabs[self.active].session_id;
                self.csv_models
                    .get_mut(&tab_id)
                    .and_then(|m| m.replace_input.as_mut())
                    .map(|i| (&mut i.buffer, &mut i.cursor, &mut i.anchor))
            }
            InputFocus::Editor | InputFocus::RichDocBody | InputFocus::CsvGrid => None,
        }
    }

    // ---- Notepad++ Edit ops ----------------------------------------------------------------

    pub(crate) fn duplicate_line(
        &mut self,
        _: &EditDuplicateLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::fill_down_grid(self, cx);
            return;
        }
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::DuplicateLine);
        let sel = self.selection();
        if !sel.is_empty() {
            let r = sel.ordered();
            let text = self.buffer().slice_bytes(r.clone());
            if text.is_empty() {
                return;
            }
            let anchor_was = sel.anchor;
            let head_was = sel.head;
            self.replace_byte_range(r.end..r.end, &text, cx);
            let tab = self.active_tab_mut();
            tab.selection.anchor = anchor_was;
            tab.selection.head = head_was;
            tab.desired_col_x = None;
            cx.notify();
            return;
        }
        let rope = &self.buffer().rope;
        let head = floor_rope_char_boundary(rope, sel.head);
        let (line, span) = line_span_at(rope, head);
        let line_start = span.start;
        let line_text = rope.line(line).to_string();
        let has_term = line_text.ends_with('\n') || line_text.ends_with('\r');
        let body_len = line_text.trim_end_matches(['\n', '\r']).len();
        let col_bytes = head - line_start;
        let ending = "\n".to_string();
        let (insert_at, insert_text, new_head) = if has_term {
            let insert_at = line_start + line_text.len();
            (insert_at, line_text.clone(), insert_at + col_bytes)
        } else {
            let insert_at = line_start + body_len;
            let insert_text = format!("{ending}{line_text}");
            (insert_at, insert_text, insert_at + ending.len() + col_bytes)
        };
        self.replace_byte_range(insert_at..insert_at, &insert_text, cx);
        let tab = self.active_tab_mut();
        tab.selection.collapse_to(new_head);
        tab.desired_col_x = None;
        self.commit_caret(cx);
    }

    pub(crate) fn fill_right(
        &mut self,
        _: &EditFillRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::CsvGrid {
            return;
        }
        crate::ui::csv_doc::actions::fill_right_grid(self, cx);
    }

    pub(crate) fn insert_today_date(
        &mut self,
        _: &EditInsertTodayDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::CsvGrid {
            return;
        }
        crate::ui::csv_doc::actions::insert_today_date(self, cx);
    }

    pub(crate) fn insert_now_time(
        &mut self,
        _: &EditInsertNowTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::CsvGrid {
            return;
        }
        crate::ui::csv_doc::actions::insert_now_time(self, cx);
    }

    pub(crate) fn move_line_up(
        &mut self,
        _: &EditMoveLineUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveLineUp);
        self.move_lines_by(-1, cx);
    }

    pub(crate) fn move_line_down(
        &mut self,
        _: &EditMoveLineDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::MoveLineDown);
        self.move_lines_by(1, cx);
    }

    /// Swap the selected line block with the line above (delta = -1) or below
    /// (delta = +1). Selection is preserved across the move.
    fn move_lines_by(&mut self, delta: i32, cx: &mut Context<Self>) {
        let sel = self.selection();
        let r = sel.ordered();
        let rope = &self.buffer().rope;
        let total_lines = rope.len_lines();
        if total_lines == 0 {
            return;
        }
        let (first, last, block) = selected_lines(rope, r);
        if delta < 0 && first == 0 {
            return;
        }
        if delta > 0 && last >= total_lines.saturating_sub(1) {
            return;
        }

        let (block_start, block_end) = (block.start, block.end);
        let block_text = rope.byte_slice(block_start..block_end).to_string();
        let block_has_term = block_text.ends_with('\n') || block_text.ends_with('\r');
        let ending = "\n".to_string();

        if delta < 0 {
            let above_start = rope.line_to_byte(first - 1);
            let above_end = block_start;
            let above_text = rope.byte_slice(above_start..above_end).to_string();

            let (new_text, new_block_start) = if block_has_term {
                (format!("{block_text}{above_text}"), above_start)
            } else {
                let above_no_term = above_text.trim_end_matches(['\n', '\r']).to_string();
                let new_text = format!("{block_text}{ending}{above_no_term}");
                (new_text, above_start)
            };
            let anchor_off = sel.anchor.saturating_sub(block_start);
            let head_off = sel.head.saturating_sub(block_start);
            let new_anchor = new_block_start + anchor_off;
            let new_head = new_block_start + head_off;

            self.replace_byte_range(above_start..block_end, &new_text, cx);
            let tab = self.active_tab_mut();
            tab.selection.anchor = new_anchor;
            tab.selection.head = new_head;
            tab.desired_col_x = None;
            self.commit_caret(cx);
        } else {
            let below_start = block_end;
            let below_line = last + 1;
            let below_end = if below_line + 1 < total_lines {
                rope.line_to_byte(below_line + 1)
            } else {
                rope.len_bytes()
            };
            let below_text = rope.byte_slice(below_start..below_end).to_string();
            let below_has_term = below_text.ends_with('\n') || below_text.ends_with('\r');

            let (new_text, new_block_start_in_buf) = if block_has_term && below_has_term {
                let new_text = format!("{below_text}{block_text}");
                (new_text, block_start + below_text.len())
            } else if block_has_term && !below_has_term {
                let below_with_term = format!("{below_text}{ending}");
                let block_no_term = block_text.trim_end_matches(['\n', '\r']).to_string();
                let new_text = format!("{below_with_term}{block_no_term}");
                (new_text, block_start + below_with_term.len())
            } else {
                return;
            };
            let block_no_term_len = block_text.trim_end_matches(['\n', '\r']).len();
            let block_len_new = if block_has_term && !below_has_term {
                block_no_term_len
            } else {
                block_text.len()
            };
            let clamp = |off: usize| -> usize {
                let rel = off.saturating_sub(block_start);
                rel.min(block_len_new)
            };
            let new_anchor = new_block_start_in_buf + clamp(sel.anchor);
            let new_head = new_block_start_in_buf + clamp(sel.head);

            self.replace_byte_range(block_start..below_end, &new_text, cx);
            let tab = self.active_tab_mut();
            tab.selection.anchor = new_anchor;
            tab.selection.head = new_head;
            tab.desired_col_x = None;
            self.commit_caret(cx);
        }
    }

    pub(crate) fn delete_line(
        &mut self,
        _: &EditDeleteLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::DeleteLine);
        let head = self.selection().head;
        let rope = &self.buffer().rope;
        let total_lines = rope.len_lines();
        if total_lines == 0 {
            return;
        }
        let (line, span) = line_span_at(rope, head);
        let (line_start, next_start) = (span.start, span.end);
        if line_start == next_start {
            return;
        }
        // If we're deleting the last line (no terminator after it) and there's
        // a previous line, also strip the previous line's trailing terminator
        // so we don't leave a stray blank line behind.
        let total_bytes = rope.len_bytes();
        let strip_prev_term = next_start == total_bytes && line > 0;
        let mut delete_from = line_start;
        if strip_prev_term {
            let prev_line_start = rope.line_to_byte(line - 1);
            let prev_text = rope.byte_slice(prev_line_start..line_start).to_string();
            let prev_no_term = prev_text.trim_end_matches(['\n', '\r']);
            delete_from = prev_line_start + prev_no_term.len();
        }
        self.replace_byte_range(delete_from..next_start, "", cx);
    }

    pub(crate) fn uppercase_selection(
        &mut self,
        _: &EditUppercase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::Uppercase);
        self.case_convert(true, cx);
    }

    pub(crate) fn lowercase_selection(
        &mut self,
        _: &EditLowercase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::Lowercase);
        self.case_convert(false, cx);
    }

    fn case_convert(&mut self, upper: bool, cx: &mut Context<Self>) {
        let sel = self.selection();
        let range = if !sel.is_empty() {
            sel.ordered()
        } else {
            let head = sel.head;
            let start = self.prev_word_boundary(head);
            let end = self.next_word_boundary(head);
            if start == end {
                return;
            }
            start..end
        };
        let text = self.buffer().slice_bytes(range.clone());
        if text.is_empty() {
            return;
        }
        let new_text = if upper {
            text.to_uppercase()
        } else {
            text.to_lowercase()
        };
        if new_text == text {
            return;
        }
        let anchor_was = sel.anchor;
        let head_was = sel.head;
        self.replace_byte_range(range.clone(), &new_text, cx);
        let new_end = range.start + new_text.len();
        let tab = self.active_tab_mut();
        if !sel.is_empty() {
            if anchor_was <= head_was {
                tab.selection.anchor = range.start;
                tab.selection.head = new_end;
            } else {
                tab.selection.anchor = new_end;
                tab.selection.head = range.start;
            }
        } else {
            tab.selection.collapse_to(new_end);
        }
        tab.desired_col_x = None;
        cx.notify();
    }

    pub(crate) fn block_comment(
        &mut self,
        _: &EditBlockComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::BlockComment);
        self.apply_line_comment(CommentDir::Toggle, cx);
    }

    pub(crate) fn apply_line_comment(&mut self, dir: CommentDir, cx: &mut Context<Self>) {
        let syntax = comment_syntax(self.buffer().language);
        // No line comment for this language? Fall back to wrapping the
        // line-range in a stream comment when one is defined; otherwise
        // (e.g. JSON) it's a no-op.
        let Some(line_token) = syntax.line else {
            if syntax.block.is_some() {
                self.block_comment_stream_fallback(dir, cx);
            }
            return;
        };
        let sel = self.selection();
        let r = sel.ordered();
        let rope = &self.buffer().rope;
        let total_lines = rope.len_lines();
        if total_lines == 0 {
            return;
        }
        let (_, _, block) = selected_lines(rope, r);
        let (block_start, block_end) = (block.start, block.end);
        let block_text = rope.byte_slice(block_start..block_end).to_string();

        let line_prefix_with_space = format!("{line_token} ");

        let mut all_commented = true;
        for line in block_text.split_inclusive('\n') {
            let body = line.trim_end_matches(['\n', '\r']);
            if body.is_empty() {
                continue;
            }
            if !body.trim_start().starts_with(line_token) {
                all_commented = false;
                break;
            }
        }
        if block_text.lines().all(|l| l.trim().is_empty()) {
            all_commented = false;
        }

        let strip_per_line = |text: &str| -> String {
            let mut out = String::new();
            for line in text.split_inclusive('\n') {
                let mut indent_end = 0;
                for (i, c) in line.char_indices() {
                    if c == ' ' || c == '\t' {
                        indent_end = i + c.len_utf8();
                    } else {
                        break;
                    }
                }
                let (indent, rest) = line.split_at(indent_end);
                let stripped = if let Some(s) = rest.strip_prefix(&line_prefix_with_space) {
                    s
                } else if let Some(s) = rest.strip_prefix(line_token) {
                    s
                } else {
                    rest
                };
                out.push_str(indent);
                out.push_str(stripped);
            }
            out
        };
        let add_to_non_empty = |text: &str| -> String {
            let mut out = String::new();
            for line in text.split_inclusive('\n') {
                if line.is_empty() {
                    continue;
                }
                let body = line.trim_end_matches(['\n', '\r']);
                if body.trim().is_empty() {
                    out.push_str(line);
                } else {
                    out.push_str(&line_prefix_with_space);
                    out.push_str(line);
                }
            }
            out
        };

        let new_block = match dir {
            CommentDir::Toggle => {
                if all_commented {
                    strip_per_line(&block_text)
                } else {
                    add_to_non_empty(&block_text)
                }
            }
            // Add prefix to every line that doesn't already have one — leave
            // already-commented lines untouched (no marker stacking).
            CommentDir::ForceSet => {
                let mut out = String::new();
                for line in block_text.split_inclusive('\n') {
                    if line.is_empty() {
                        continue;
                    }
                    let body = line.trim_end_matches(['\n', '\r']);
                    if body.trim().is_empty() || body.trim_start().starts_with(line_token) {
                        out.push_str(line);
                    } else {
                        out.push_str(&line_prefix_with_space);
                        out.push_str(line);
                    }
                }
                out
            }
            // Strip exactly one comment marker from every line that has one;
            // leave non-commented lines alone.
            CommentDir::ForceUncomment => strip_per_line(&block_text),
        };

        if new_block == block_text {
            return;
        }

        let anchor_was = sel.anchor;
        let head_was = sel.head;
        let new_block_len = new_block.len();
        let old_block_len = block_text.len();
        let added = new_block_len as i64 - old_block_len as i64;

        self.replace_byte_range(block_start..block_end, &new_block, cx);

        let adjust = |off: usize| -> usize {
            if off <= block_start {
                off
            } else if off >= block_end {
                ((off as i64) + added).max(block_start as i64) as usize
            } else {
                let rel = off - block_start;
                let new_end = block_start + new_block_len;
                (block_start + rel).min(new_end)
            }
        };
        let new_anchor = adjust(anchor_was);
        let new_head = adjust(head_was);
        let tab = self.active_tab_mut();
        tab.selection.anchor = new_anchor;
        tab.selection.head = new_head;
        tab.desired_col_x = None;
        cx.notify();
    }

    /// Fallback for languages with no line comment but a block comment
    /// (HTML, XML, Markdown). Wraps the selected line-range in the
    /// language's block delimiters and toggles when already wrapped. The
    /// direction selector lets Block Comment Set / Block Uncomment force
    /// wrap-only or unwrap-only behavior.
    fn block_comment_stream_fallback(&mut self, dir: CommentDir, cx: &mut Context<Self>) {
        let syntax = comment_syntax(self.buffer().language);
        let Some((open, close)) = syntax.block else {
            return;
        };
        let sel = self.selection();
        let r = sel.ordered();
        let rope = &self.buffer().rope;
        let total_lines = rope.len_lines();
        if total_lines == 0 {
            return;
        }
        let (_, _, block) = selected_lines(rope, r);
        let (block_start, block_end) = (block.start, block.end);
        let text = rope.byte_slice(block_start..block_end).to_string();
        let trailing_nl = if text.ends_with("\r\n") {
            "\r\n"
        } else if text.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let body = &text[..text.len() - trailing_nl.len()];
        let trimmed = body.trim();
        let already_wrapped = trimmed.starts_with(open)
            && trimmed.ends_with(close)
            && trimmed.len() >= open.len() + close.len();
        let new_body = if already_wrapped {
            // ForceSet wants no-op when already wrapped — don't strip.
            if dir == CommentDir::ForceSet {
                return;
            }
            let Some(open_off) = body.find(open) else {
                return;
            };
            let Some(close_off) = body.rfind(close) else {
                return;
            };
            let inner_start = open_off + open.len();
            let inner_end = close_off;
            if inner_start > inner_end {
                return;
            }
            let mut inner = body[inner_start..inner_end].to_string();
            if inner.starts_with(' ') {
                inner.remove(0);
            }
            if inner.ends_with(' ') {
                inner.pop();
            }
            inner
        } else if body.trim().is_empty() {
            return;
        } else {
            // ForceUncomment wants no-op when nothing to strip.
            if dir == CommentDir::ForceUncomment {
                return;
            }
            format!("{open} {body} {close}")
        };
        let new_block = format!("{new_body}{trailing_nl}");
        if new_block == text {
            return;
        }

        let anchor_was = sel.anchor;
        let head_was = sel.head;
        let new_block_len = new_block.len();
        let old_block_len = text.len();
        let added = new_block_len as i64 - old_block_len as i64;

        self.replace_byte_range(block_start..block_end, &new_block, cx);

        let adjust = |off: usize| -> usize {
            if off <= block_start {
                off
            } else if off >= block_end {
                ((off as i64) + added).max(block_start as i64) as usize
            } else {
                let rel = off - block_start;
                let new_end = block_start + new_block_len;
                (block_start + rel).min(new_end)
            }
        };
        let new_anchor = adjust(anchor_was);
        let new_head = adjust(head_was);
        let tab = self.active_tab_mut();
        tab.selection.anchor = new_anchor;
        tab.selection.head = new_head;
        tab.desired_col_x = None;
        cx.notify();
    }

    pub(crate) fn stream_comment(
        &mut self,
        _: &EditStreamComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::StreamComment);
        self.apply_stream_comment(CommentDir::Toggle, cx);
    }

    pub(crate) fn apply_stream_comment(&mut self, dir: CommentDir, cx: &mut Context<Self>) {
        // No-op when the active buffer's language has no block-comment
        // form (JSON, plain shell/yaml/etc.).
        let Some((open, close)) = comment_syntax(self.buffer().language).block else {
            return;
        };
        let sel = self.selection();
        if sel.is_empty() {
            return;
        }
        let r = sel.ordered();
        let text = self.buffer().slice_bytes(r.clone());
        let trimmed = text.trim();
        let already_wrapped = trimmed.starts_with(open)
            && trimmed.ends_with(close)
            && trimmed.len() >= open.len() + close.len();
        let new_text = if already_wrapped {
            // ForceSet wants no-op when already wrapped.
            if dir == CommentDir::ForceSet {
                return;
            }
            let Some(open_off) = text.find(open) else {
                return;
            };
            let Some(close_off) = text.rfind(close) else {
                return;
            };
            let inner_start = open_off + open.len();
            let inner_end = close_off;
            if inner_start > inner_end {
                return;
            }
            let mut inner = text[inner_start..inner_end].to_string();
            if inner.starts_with(' ') {
                inner.remove(0);
            }
            if inner.ends_with(' ') {
                inner.pop();
            }
            inner
        } else {
            // ForceUncomment wants no-op when nothing is wrapped.
            if dir == CommentDir::ForceUncomment {
                return;
            }
            format!("{open} {text} {close}")
        };
        let anchor_was = sel.anchor;
        let head_was = sel.head;
        self.replace_byte_range(r.clone(), &new_text, cx);
        let new_end = r.start + new_text.len();
        let tab = self.active_tab_mut();
        if anchor_was <= head_was {
            tab.selection.anchor = r.start;
            tab.selection.head = new_end;
        } else {
            tab.selection.anchor = new_end;
            tab.selection.head = r.start;
        }
        tab.desired_col_x = None;
        cx.notify();
    }

    pub(crate) fn trim_trailing_ws(
        &mut self,
        _: &EditTrimTrailingWs,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::macros::record(crate::macros::MacroEvent::TrimTrailingWs);
        let total_bytes = self.buffer().rope.len_bytes();
        if total_bytes == 0 {
            return;
        }
        let text = self.buffer().rope.slice(..).to_string();
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let (body, term) = if let Some(s) = line.strip_suffix("\r\n") {
                (s, "\r\n")
            } else if let Some(s) = line.strip_suffix('\n') {
                (s, "\n")
            } else if let Some(s) = line.strip_suffix('\r') {
                (s, "\r")
            } else {
                (line, "")
            };
            let trimmed = body.trim_end_matches([' ', '\t']);
            out.push_str(trimmed);
            out.push_str(term);
        }
        if out == text {
            return;
        }
        let head_was = self.selection().head;
        self.replace_byte_range(0..total_bytes, &out, cx);
        let clamped = head_was.min(self.buffer().rope.len_bytes());
        let tab = self.active_tab_mut();
        tab.selection.collapse_to(clamped);
        tab.desired_col_x = None;
        self.commit_caret(cx);
    }

    pub(crate) fn insert_blank_line_above(
        &mut self,
        _: &EditInsertBlankLineAbove,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::InsertBlankLineAbove);
        let head = self.selection().head;
        let rope = &self.buffer().rope;
        let (_, span) = line_span_at(rope, head);
        let line_start = span.start;
        let ending = "\n".to_string();
        self.replace_byte_range(line_start..line_start, &ending, cx);
        let tab = self.active_tab_mut();
        tab.selection.collapse_to(line_start);
        tab.desired_col_x = None;
        self.commit_caret(cx);
    }

    pub(crate) fn insert_blank_line_below(
        &mut self,
        _: &EditInsertBlankLineBelow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        crate::macros::record(crate::macros::MacroEvent::InsertBlankLineBelow);
        let head = self.selection().head;
        let rope = &self.buffer().rope;
        let (line, span) = line_span_at(rope, head);
        let line_text = rope.line(line).to_string();
        let line_has_term = line_text.ends_with('\n') || line_text.ends_with('\r');
        let next_line_start = span.end;
        let ending = "\n".to_string();
        let (insert_at, new_caret) = if line_has_term {
            (next_line_start, next_line_start)
        } else {
            (next_line_start, next_line_start + ending.len())
        };
        self.replace_byte_range(insert_at..insert_at, &ending, cx);
        let tab = self.active_tab_mut();
        tab.selection.collapse_to(new_caret);
        tab.desired_col_x = None;
        self.commit_caret(cx);
    }
}

/// Clamp `byte` to `s.len()` and move it back to the start of the char it
/// falls in, so it is always safe to slice `s` at the result.
pub(crate) fn floor_char_boundary_in(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}

/// Pull an input's cursor and anchor back inside `target`, onto char starts.
/// Some openers replace the text and reset only the cursor (Ctrl+F prefill,
/// Go To reopen), so the anchor can be left past the end or inside a
/// multi-byte char.
fn snap_input_caret(target: &str, cursor: &mut usize, anchor: &mut usize) {
    *cursor = floor_char_boundary_in(target, *cursor);
    *anchor = floor_char_boundary_in(target, *anchor);
}

/// Backspace inside a single-line input: delete the selection, or the char
/// before the caret. Returns false when there was nothing to delete.
fn input_field_backspace(target: &mut String, cursor: &mut usize, anchor: &mut usize) -> bool {
    snap_input_caret(target, cursor, anchor);
    if *anchor != *cursor {
        let lo = (*cursor).min(*anchor);
        let hi = (*cursor).max(*anchor);
        target.replace_range(lo..hi, "");
        *cursor = lo;
        *anchor = lo;
    } else {
        if *cursor == 0 {
            return false;
        }
        let prev = prev_char_byte_in_str(target, *cursor);
        target.replace_range(prev..*cursor, "");
        *cursor = prev;
        *anchor = prev;
    }
    true
}

/// Forward delete inside a single-line input: delete the selection, or the
/// char after the caret. Returns false when there was nothing to delete.
fn input_field_delete(target: &mut String, cursor: &mut usize, anchor: &mut usize) -> bool {
    snap_input_caret(target, cursor, anchor);
    if *anchor != *cursor {
        let lo = (*cursor).min(*anchor);
        let hi = (*cursor).max(*anchor);
        target.replace_range(lo..hi, "");
        *cursor = lo;
        *anchor = lo;
    } else {
        if *cursor >= target.len() {
            return false;
        }
        let next = next_char_byte_in_str(target, *cursor);
        target.replace_range(*cursor..next, "");
    }
    true
}

/// Left/Right arrow inside a single-line input.
fn input_field_step(target: &str, cursor: &mut usize, anchor: &mut usize, delta: i32, extend: bool) {
    snap_input_caret(target, cursor, anchor);
    if !extend && *anchor != *cursor {
        // Collapsing arrow over a selection: jump to the appropriate
        // endpoint instead of moving past it.
        *cursor = if delta < 0 {
            (*cursor).min(*anchor)
        } else {
            (*cursor).max(*anchor)
        };
        *anchor = *cursor;
        return;
    }
    if delta < 0 {
        *cursor = prev_char_byte_in_str(target, *cursor);
    } else {
        *cursor = next_char_byte_in_str(target, *cursor);
    }
    if !extend {
        *anchor = *cursor;
    }
}

/// Selected text of a single-line input (empty when collapsed).
fn input_field_selection(target: &str, cursor: usize, anchor: usize) -> String {
    let (mut cursor, mut anchor) = (cursor, anchor);
    snap_input_caret(target, &mut cursor, &mut anchor);
    let lo = cursor.min(anchor);
    let hi = cursor.max(anchor);
    target.get(lo..hi).unwrap_or_default().to_string()
}

/// Replace the selection of a single-line input (or insert at the caret
/// when collapsed) and leave a collapsed caret after the new text.
fn input_field_replace(target: &mut String, cursor: &mut usize, anchor: &mut usize, new_text: &str) {
    snap_input_caret(target, cursor, anchor);
    let lo = (*cursor).min(*anchor);
    let hi = (*cursor).max(*anchor);
    target.replace_range(lo..hi, new_text);
    *cursor = lo + new_text.len();
    *anchor = *cursor;
}

/// Rope twin of [`floor_char_boundary_in`]: clamp `byte` to the buffer and
/// move it back to the start of the char it falls in. Every ropey call that
/// takes a byte offset (`byte_slice`, `byte_to_line`, `byte_to_char`) is
/// safe on the result.
pub(crate) fn floor_rope_char_boundary(rope: &ropey::Rope, byte: usize) -> usize {
    let b = byte.min(rope.len_bytes());
    rope.char_to_byte(rope.byte_to_char(b))
}

/// Walk a `&str` left-to-right looking for the previous word boundary, mirror
/// of `Workspace::prev_word_boundary` on a single-line input. Whitespace before
/// the cursor is skipped, then the run of word characters is consumed; the
/// landing offset is the last byte index that started a word.
fn input_prev_word_boundary(s: &str, byte: usize) -> usize {
    let byte = floor_char_boundary_in(s, byte);
    if byte == 0 {
        return 0;
    }
    let mut entered_word = false;
    let mut last_idx = 0usize;
    for (i, c) in s[..byte].char_indices().rev() {
        if c.is_whitespace() {
            if entered_word {
                last_idx = i + c.len_utf8();
                return last_idx;
            }
            last_idx = i;
        } else {
            entered_word = true;
            last_idx = i;
        }
    }
    last_idx
}

/// Walk a `&str` rightward looking for the next word boundary. Skip the run of
/// word characters, then any whitespace gap, landing at the next word start.
fn input_next_word_boundary(s: &str, byte: usize) -> usize {
    if byte >= s.len() {
        return s.len();
    }
    let byte = floor_char_boundary_in(s, byte);
    let mut entered_word = false;
    for (i, c) in s[byte..].char_indices() {
        if c.is_whitespace() {
            if entered_word {
                return byte + i;
            }
        } else {
            entered_word = true;
        }
    }
    s.len()
}

/// Find the word boundary around `byte` in `text`. Word chars are
/// `is_alphanumeric() || '_'`. Returns the (lo, hi) byte range. If `byte`
/// doesn't sit on a word char, returns (byte, byte) (collapsed — no word
/// found). Used by the rich-doc double-click handler.
/// Find the nearest paragraph/heading block strictly before `block`.
fn find_prev_textual_block(
    payload: &crate::rich_doc::RichDocPayload,
    block: usize,
) -> Option<usize> {
    if block == 0 {
        return None;
    }
    (0..block).rev().find(|&b| payload.block_runs(b).is_some())
}

/// Find the nearest paragraph/heading block strictly after `block`.
fn find_next_textual_block(
    payload: &crate::rich_doc::RichDocPayload,
    block: usize,
) -> Option<usize> {
    ((block + 1)..payload.ast.blocks.len()).find(|&b| payload.block_runs(b).is_some())
}

/// Phase 2g.H — does every byte in the rich-doc range [lo, hi) belong to a
/// run whose style satisfies `predicate`? Used by `rich_doc_toggle_style` to
/// decide toggle direction (all-on → turn off, mixed/all-off → turn on).
/// Considers only paragraph / heading runs that the range actually
/// intersects. Empty / zero-byte ranges return false so toggling on an empty
/// selection (shouldn't happen — caller checks is_collapsed) still does the
/// right thing. Non-text blocks in the span are treated as "have the style"
/// (skipped) — a tables-only range would otherwise force "turn off" with
/// nothing to turn off.
fn range_all_have_style(
    payload: &crate::rich_doc::RichDocPayload,
    lo: crate::rich_doc::TextPos,
    hi: crate::rich_doc::TextPos,
    predicate: fn(&crate::rich_doc::ast::InlineStyle) -> bool,
) -> bool {
    if (lo.block, lo.run, lo.byte) >= (hi.block, hi.run, hi.byte) {
        return false;
    }
    let mut saw_any_byte = false;
    for block_idx in lo.block..=hi.block {
        let Some(runs) = payload.block_runs(block_idx) else {
            continue;
        };
        if runs.is_empty() {
            continue;
        }
        let (start_run, start_byte) = if block_idx == lo.block {
            (lo.run.min(runs.len() - 1), lo.byte)
        } else {
            (0, 0)
        };
        let (end_run, end_byte) = if block_idx == hi.block {
            (hi.run.min(runs.len() - 1), hi.byte)
        } else {
            let last = runs.len() - 1;
            (last, runs[last].text.len())
        };
        for ri in start_run..=end_run {
            let Some(r) = runs.get(ri) else {
                continue;
            };
            let run_lo = if ri == start_run { start_byte } else { 0 };
            let run_hi = if ri == end_run { end_byte } else { r.text.len() };
            if run_hi <= run_lo {
                continue;
            }
            saw_any_byte = true;
            if !predicate(&r.style) {
                return false;
            }
        }
    }
    saw_any_byte
}

/// Walk left from `byte` to the previous word boundary inside `text`.
/// Mirrors `Workspace::prev_word_boundary` (plain-text editor): skip
/// trailing whitespace, then walk back through the run of non-whitespace
/// characters, stop at the first whitespace before the word.
fn prev_word_boundary_in(text: &str, byte: usize) -> usize {
    if byte == 0 || text.is_empty() {
        return 0;
    }
    let mut b = byte.min(text.len());
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    let mut entered_word = false;
    while b > 0 {
        let mut prev = b - 1;
        while prev > 0 && !text.is_char_boundary(prev) {
            prev -= 1;
        }
        let Some(c) = text[prev..b].chars().next() else { break };
        if c.is_whitespace() {
            if entered_word {
                break;
            }
            b = prev;
        } else {
            entered_word = true;
            b = prev;
        }
    }
    b
}

/// Walk right from `byte` to the next word boundary inside `text`. Mirrors
/// `Workspace::next_word_boundary` — walks through the current run of
/// non-whitespace, then stops at the first whitespace after.
fn next_word_boundary_in(text: &str, byte: usize) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut b = byte.min(text.len());
    while b < text.len() && !text.is_char_boundary(b) {
        b += 1;
    }
    let mut entered_word = false;
    while b < text.len() {
        let Some(c) = text[b..].chars().next() else { break };
        if c.is_whitespace() {
            if entered_word {
                break;
            }
            b += c.len_utf8();
        } else {
            entered_word = true;
            b += c.len_utf8();
        }
    }
    b
}

fn word_bounds_at(text: &str, byte: usize) -> (usize, usize) {
    if text.is_empty() || byte > text.len() {
        return (text.len(), text.len());
    }
    // The pivot must start a char: at the end of the text it is the last
    // char's first byte, and a mid-char offset floors to its char start.
    let byte = floor_char_boundary_in(text, byte);
    let pivot = if byte == text.len() {
        floor_char_boundary_in(text, byte.saturating_sub(1))
    } else {
        byte
    };
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let pivot_char = text[pivot..].chars().next();
    if !pivot_char.map(is_word).unwrap_or(false) {
        return (byte, byte);
    }
    let mut lo = pivot;
    while lo > 0 {
        let mut prev = lo - 1;
        while prev > 0 && !text.is_char_boundary(prev) {
            prev -= 1;
        }
        let Some(c) = text[prev..lo].chars().next() else { break };
        if is_word(c) {
            lo = prev;
        } else {
            break;
        }
    }
    let mut hi = pivot;
    while hi < text.len() {
        let Some(c) = text[hi..].chars().next() else { break };
        if is_word(c) {
            hi += c.len_utf8();
        } else {
            break;
        }
    }
    (lo, hi)
}

#[cfg(test)]
mod rich_doc_word_tests {
    use super::{next_word_boundary_in, prev_word_boundary_in, word_bounds_at};

    #[test]
    fn word_bounds_finds_simple_word() {
        let text = "hello world";
        assert_eq!(word_bounds_at(text, 2), (0, 5));
        assert_eq!(word_bounds_at(text, 8), (6, 11));
    }

    #[test]
    fn word_bounds_on_whitespace_returns_collapsed() {
        let text = "hello world";
        assert_eq!(word_bounds_at(text, 5), (5, 5));
    }

    #[test]
    fn word_bounds_includes_underscore() {
        let text = "foo_bar baz";
        assert_eq!(word_bounds_at(text, 4), (0, 7));
    }

    #[test]
    fn word_bounds_handles_empty() {
        assert_eq!(word_bounds_at("", 0), (0, 0));
    }

    #[test]
    fn prev_word_from_end_of_second_word_lands_at_start_of_second() {
        let text = "hello world";
        assert_eq!(prev_word_boundary_in(text, 11), 6);
    }

    #[test]
    fn prev_word_from_start_of_second_word_skips_space_into_first() {
        let text = "hello world";
        assert_eq!(prev_word_boundary_in(text, 6), 0);
    }

    #[test]
    fn prev_word_at_zero_stays_zero() {
        assert_eq!(prev_word_boundary_in("hello", 0), 0);
    }

    #[test]
    fn next_word_from_start_of_first_word_lands_at_end_of_first() {
        let text = "hello world";
        assert_eq!(next_word_boundary_in(text, 0), 5);
    }

    #[test]
    fn next_word_from_end_of_first_word_skips_space_to_end_of_second() {
        let text = "hello world";
        assert_eq!(next_word_boundary_in(text, 5), 11);
    }

    #[test]
    fn next_word_handles_punctuation_run() {
        // is_whitespace separator only — punctuation walks as part of "word".
        let text = "foo,bar baz";
        assert_eq!(next_word_boundary_in(text, 0), 7);
    }

    #[test]
    fn prev_word_handles_utf8_multibyte() {
        // "héllo wörld" — é = 0xC3 0xA9 (2 bytes), ö = 0xC3 0xB6 (2 bytes).
        let text = "héllo wörld";
        // From end (12) back to start of "wörld" at byte 7 ("h é l l o   w ö ...").
        // Bytes: h(0) é(1-2) l(3) l(4) o(5) ' '(6) w(7) ö(8-9) r(10) l(11) d(12)
        assert_eq!(prev_word_boundary_in(text, 12), 7);
    }

    #[test]
    fn word_bounds_at_end_after_multibyte_char() {
        // Double-click past the last char of a block whose final char is
        // multi-byte: the pivot used to land inside that char.
        let cases: &[(&str, usize, (usize, usize))] = &[
            ("é", 2, (0, 2)),
            ("aé", 3, (0, 3)),
            ("中文", 6, (0, 6)),
            ("ab 👍", 7, (7, 7)),
            ("👨‍👩‍👧", 18, (18, 18)),
            ("🇫🇷", 8, (8, 8)),
        ];
        for &(text, byte, want) in cases {
            assert_eq!(word_bounds_at(text, byte), want, "{text:?} @ {byte}");
        }
    }

    #[test]
    fn word_bounds_mid_char_snaps_to_char_start() {
        let cases: &[(&str, usize, (usize, usize))] = &[
            ("héllo", 2, (0, 6)),
            ("x 中 y", 3, (2, 5)),
            ("a👍b", 3, (1, 1)),
        ];
        for &(text, byte, want) in cases {
            assert_eq!(word_bounds_at(text, byte), want, "{text:?} @ {byte}");
        }
    }

    #[test]
    fn word_bounds_past_end_is_collapsed_at_end() {
        assert_eq!(word_bounds_at("abc", 99), (3, 3));
    }
}

#[cfg(test)]
mod input_word_tests {
    use super::{input_next_word_boundary, input_prev_word_boundary};

    #[test]
    fn input_word_boundaries_ascii() {
        assert_eq!(input_prev_word_boundary("foo bar", 7), 4);
        assert_eq!(input_prev_word_boundary("foo bar", 4), 0);
        assert_eq!(input_next_word_boundary("foo bar", 0), 3);
        assert_eq!(input_next_word_boundary("foo bar", 3), 7);
    }

    #[test]
    fn input_word_boundaries_clamp_past_end() {
        assert_eq!(input_prev_word_boundary("foo", 99), 0);
        assert_eq!(input_next_word_boundary("foo", 99), 3);
        assert_eq!(input_prev_word_boundary("", 5), 0);
        assert_eq!(input_next_word_boundary("", 5), 0);
    }

    #[test]
    fn input_word_boundaries_mid_char_do_not_panic() {
        // A stale or mid-char cursor must snap, never slice inside a char.
        let texts = ["héllo wörld", "中文 字", "a👍 b", "👨‍👩‍👧 x", "🇫🇷🇩🇪 y"];
        for text in texts {
            for byte in 0..=text.len() + 2 {
                let p = input_prev_word_boundary(text, byte);
                let n = input_next_word_boundary(text, byte);
                assert!(text.is_char_boundary(p), "{text:?} prev @ {byte} -> {p}");
                assert!(text.is_char_boundary(n), "{text:?} next @ {byte} -> {n}");
                assert!(p <= byte.min(text.len()));
            }
        }
        assert_eq!(input_prev_word_boundary("héllo", 2), 0);
        assert_eq!(input_next_word_boundary("héllo", 2), 6);
    }
}

#[cfg(test)]
mod input_field_tests {
    use super::{
        input_field_backspace, input_field_delete, input_field_replace, input_field_selection,
        input_field_step,
    };

    const TEXTS: &[&str] = &["", "abc", "é", "héllo", "中文", "a👍b", "👨‍👩‍👧", "🇫🇷x"];

    fn assert_caret_ok(text: &str, cursor: usize, anchor: usize, what: &str) {
        assert!(cursor <= text.len() && text.is_char_boundary(cursor), "{what}: cursor {cursor} in {text:?}");
        assert!(anchor <= text.len() && text.is_char_boundary(anchor), "{what}: anchor {anchor} in {text:?}");
    }

    #[test]
    fn goto_reopen_stale_anchor_does_not_panic() {
        // Go To Line clears its text and resets only the cursor, so the
        // anchor from the previous entry ("12") points past the end.
        let mut text = String::new();
        let (mut cursor, mut anchor) = (0usize, 2usize);
        assert_eq!(input_field_selection(&text, cursor, anchor), "");
        assert!(!input_field_backspace(&mut text, &mut cursor, &mut anchor));
        input_field_replace(&mut text, &mut cursor, &mut anchor, "5");
        assert_eq!((text.as_str(), cursor, anchor), ("5", 1, 1));
    }

    #[test]
    fn find_prefill_stale_anchor_mid_char_does_not_panic() {
        // Ctrl+F prefills the query from the selection ("é") and moves only
        // the cursor; an anchor of 1 left over from a previous query sits
        // inside the 2-byte char.
        let mut text = String::from("é");
        let (mut cursor, mut anchor) = (2usize, 1usize);
        let _ = input_field_selection(&text, cursor, anchor);
        input_field_replace(&mut text, &mut cursor, &mut anchor, "x");
        assert_caret_ok(&text, cursor, anchor, "replace");
    }

    #[test]
    fn every_input_op_survives_any_cursor_and_anchor() {
        for &base in TEXTS {
            for c in 0..=base.len() + 2 {
                for a in 0..=base.len() + 2 {
                    let ctx = format!("{base:?} cursor={c} anchor={a}");
                    let _ = input_field_selection(base, c, a);

                    let (mut t, mut cc, mut aa) = (base.to_string(), c, a);
                    input_field_backspace(&mut t, &mut cc, &mut aa);
                    assert_caret_ok(&t, cc, aa, &format!("backspace {ctx}"));

                    let (mut t, mut cc, mut aa) = (base.to_string(), c, a);
                    input_field_delete(&mut t, &mut cc, &mut aa);
                    assert_caret_ok(&t, cc, aa, &format!("delete {ctx}"));

                    let (mut t, mut cc, mut aa) = (base.to_string(), c, a);
                    input_field_replace(&mut t, &mut cc, &mut aa, "中");
                    assert_caret_ok(&t, cc, aa, &format!("replace {ctx}"));

                    for (delta, extend) in [(-1, false), (1, false), (-1, true), (1, true)] {
                        let (mut cc, mut aa) = (c, a);
                        input_field_step(base, &mut cc, &mut aa, delta, extend);
                        assert_caret_ok(base, cc, aa, &format!("step {delta} {extend} {ctx}"));
                    }
                }
            }
        }
    }

    #[test]
    fn input_ops_on_valid_carets_keep_behaviour() {
        let mut t = String::from("héllo");
        let (mut c, mut a) = (3usize, 3usize);
        assert!(input_field_backspace(&mut t, &mut c, &mut a));
        assert_eq!((t.as_str(), c, a), ("hllo", 1, 1));
        assert!(input_field_delete(&mut t, &mut c, &mut a));
        assert_eq!((t.as_str(), c, a), ("hlo", 1, 1));
        let (mut c, mut a) = (0usize, 3usize);
        assert_eq!(input_field_selection(&t, c, a), "hlo");
        input_field_replace(&mut t, &mut c, &mut a, "é");
        assert_eq!((t.as_str(), c, a), ("é", 2, 2));
        input_field_step(&t, &mut c, &mut a, -1, false);
        assert_eq!((c, a), (0, 0));
    }
}

/// Line holding `byte` (clamped to the rope) and its byte span, terminator
/// included.
fn line_span_at(rope: &ropey::Rope, byte: usize) -> (usize, std::ops::Range<usize>) {
    let line = rope.byte_to_line(byte.min(rope.len_bytes()));
    let start = rope.line_to_byte(line);
    let end = if line + 1 < rope.len_lines() {
        rope.line_to_byte(line + 1)
    } else {
        rope.len_bytes()
    };
    (line, start..end)
}

/// Whole lines the ordered selection `r` touches: first and last line, and
/// their byte span with the last line's terminator. A selection ending
/// right at a line start does not take that line.
fn selected_lines(
    rope: &ropey::Rope,
    r: std::ops::Range<usize>,
) -> (usize, usize, std::ops::Range<usize>) {
    let len = rope.len_bytes();
    let (start, end) = (r.start.min(len), r.end.min(len));
    let first = rope.byte_to_line(start);
    let last_raw = rope.byte_to_line(end);
    let last = if end > start && last_raw > first && rope.line_to_byte(last_raw) == end {
        last_raw - 1
    } else {
        last_raw
    };
    let block_start = rope.line_to_byte(first);
    let block_end = if last + 1 < rope.len_lines() {
        rope.line_to_byte(last + 1)
    } else {
        rope.len_bytes()
    };
    (first, last, block_start..block_end)
}

/// Bytes an empty-selection Cut removes: the caret's line with its
/// terminator. The last line has no terminator, so it takes the line
/// break before it instead and leaves no empty trailing line.
fn cut_line_range(rope: &ropey::Rope, head: usize) -> std::ops::Range<usize> {
    let (line, span) = line_span_at(rope, head);
    // Ropey's last line never holds a line break; every other line does.
    let is_last = line + 1 >= rope.len_lines();
    if !is_last || span.start == 0 {
        return span;
    }
    let start_char = rope.byte_to_char(span.start);
    let Some(prev) = start_char.checked_sub(1) else {
        return span;
    };
    let crlf = rope.get_char(prev) == Some('\n')
        && prev
            .checked_sub(1)
            .and_then(|i| rope.get_char(i))
            .is_some_and(|c| c == '\r');
    let break_start = if crlf { prev - 1 } else { prev };
    rope.char_to_byte(break_start)..span.end
}

/// Bytes Backspace removes with an empty selection at `head`: the char
/// before the caret, or the whole CRLF pair as Scintilla does
/// (`Document::DelCharBack`). A head inside a char or past EOF is snapped
/// first.
fn backspace_range(rope: &ropey::Rope, head: usize) -> Option<std::ops::Range<usize>> {
    let head = floor_rope_char_boundary(rope, head);
    let mut prev = rope.byte_to_char(head).checked_sub(1)?;
    if rope.get_char(prev) == Some('\n')
        && prev.checked_sub(1).and_then(|i| rope.get_char(i)) == Some('\r')
    {
        prev -= 1;
    }
    Some(rope.char_to_byte(prev)..head)
}

/// Bytes Delete removes with an empty selection at `head`: the char after
/// the caret, or the whole CRLF pair as Scintilla does
/// (`Document::LenChar`). A head inside a char or past EOF is snapped
/// first.
fn delete_forward_range(rope: &ropey::Rope, head: usize) -> Option<std::ops::Range<usize>> {
    let head = floor_rope_char_boundary(rope, head);
    let char_idx = rope.byte_to_char(head);
    if char_idx >= rope.len_chars() {
        return None;
    }
    let crlf = rope.get_char(char_idx) == Some('\r') && rope.get_char(char_idx + 1) == Some('\n');
    let end = if crlf { char_idx + 2 } else { char_idx + 1 };
    Some(head..rope.char_to_byte(end))
}

/// Auto-indent inputs for Enter at `caret`: the leading whitespace the new
/// line inherits, capped at the caret so Enter inside the indent doesn't
/// carry indent past it (as Notepad++ does), and the line text before the
/// caret for the Advanced extra-indent check.
fn auto_indent_context(rope: &ropey::Rope, caret: usize) -> (String, String) {
    let caret = floor_rope_char_boundary(rope, caret);
    let line = rope.byte_to_line(caret);
    let line_start = rope.line_to_byte(line);
    let line_text = rope.line(line).to_string();
    let body = line_text.trim_end_matches(['\n', '\r']);
    let local_offset = caret.saturating_sub(line_start);
    let indent_end = body
        .char_indices()
        .find(|(_, c)| !matches!(c, ' ' | '\t'))
        .map(|(i, _)| i)
        .unwrap_or(body.len());
    let inherit_end = indent_end.min(local_offset);
    (
        body.get(..inherit_end).unwrap_or("").to_string(),
        body.get(..local_offset).unwrap_or(body).to_string(),
    )
}

#[cfg(test)]
mod line_command_tests {
    use super::{
        auto_indent_context, backspace_range, cut_line_range, delete_forward_range,
        line_span_at, selected_lines,
    };
    use ropey::Rope;

    #[test]
    fn line_span_covers_line_and_terminator() {
        let rope = Rope::from_str("ab\r\ncd\ref");
        assert_eq!(line_span_at(&rope, 0), (0, 0..4));
        assert_eq!(line_span_at(&rope, 3), (0, 0..4));
        assert_eq!(line_span_at(&rope, 4), (1, 4..7));
        assert_eq!(line_span_at(&rope, 9), (2, 7..9));
        assert_eq!(line_span_at(&Rope::new(), 0), (0, 0..0));
        assert_eq!(line_span_at(&Rope::from_str("a\n"), 2), (1, 2..2));
    }

    #[test]
    fn line_commands_with_head_past_end_do_not_panic() {
        // A head past EOF used to panic in `byte_to_line`.
        let rope = Rope::from_str("ab\r\ncd\ref");
        assert_eq!(line_span_at(&rope, 99), (2, 7..9));
        assert_eq!(selected_lines(&rope, 5..99), (1, 2, 4..9));
        assert_eq!(selected_lines(&rope, 98..99), (2, 2, 7..9));
        assert_eq!(cut_line_range(&rope, 99), 6..9);
        assert_eq!(auto_indent_context(&Rope::from_str("  ab"), 99), ("  ".into(), "  ab".into()));
        assert_eq!(backspace_range(&Rope::from_str("aé"), 99), Some(1..3));
        assert_eq!(delete_forward_range(&Rope::from_str("aé"), 99), None);
    }

    #[test]
    fn selected_lines_excludes_line_the_selection_ends_on() {
        let rope = Rope::from_str("a\nb\nc");
        assert_eq!(selected_lines(&rope, 0..0), (0, 0, 0..2));
        assert_eq!(selected_lines(&rope, 0..2), (0, 0, 0..2));
        assert_eq!(selected_lines(&rope, 0..3), (0, 1, 0..4));
        assert_eq!(selected_lines(&rope, 4..5), (2, 2, 4..5));
        assert_eq!(selected_lines(&Rope::new(), 0..0), (0, 0, 0..0));
    }

    #[test]
    fn cut_last_line_after_multibyte_text_does_not_panic() {
        // The last line takes the break before it. Probing two bytes back
        // for "\r\n" sliced into the middle of `é` / U+2028 and panicked.
        assert_eq!(cut_line_range(&Rope::from_str("é\nabc"), 4), 2..6);
        assert_eq!(cut_line_range(&Rope::from_str("中\nx"), 4), 3..5);
        assert_eq!(cut_line_range(&Rope::from_str("a\u{2028}b"), 4), 1..5);
        assert_eq!(cut_line_range(&Rope::from_str("a\u{85}b"), 3), 1..4);
    }

    #[test]
    fn cut_line_ranges_on_each_line_ending() {
        assert_eq!(cut_line_range(&Rope::from_str("abc"), 1), 0..3);
        assert_eq!(cut_line_range(&Rope::from_str("a\r\nb"), 3), 1..4);
        assert_eq!(cut_line_range(&Rope::from_str("a\nb"), 2), 1..3);
        assert_eq!(cut_line_range(&Rope::from_str("a\rb"), 2), 1..3);
        assert_eq!(cut_line_range(&Rope::from_str("a\r\nb\r\n"), 0), 0..3);
        assert_eq!(cut_line_range(&Rope::from_str("a\nb\n"), 2), 2..4);
        // A middle line ending in U+2028 is terminated: cut only that line,
        // not the break before it as well.
        assert_eq!(cut_line_range(&Rope::from_str("a\nb\u{2028}c"), 2), 2..6);
    }

    #[test]
    fn backspace_and_delete_land_on_char_boundaries() {
        let rope = Rope::from_str("aé👍");
        assert_eq!(backspace_range(&rope, 0), None);
        assert_eq!(backspace_range(&rope, 1), Some(0..1));
        assert_eq!(backspace_range(&rope, 3), Some(1..3));
        assert_eq!(backspace_range(&rope, 7), Some(3..7));
        assert_eq!(delete_forward_range(&rope, 0), Some(0..1));
        assert_eq!(delete_forward_range(&rope, 3), Some(3..7));
        assert_eq!(delete_forward_range(&rope, 7), None);
        assert_eq!(delete_forward_range(&Rope::new(), 0), None);
        // A caret inside a char snaps to its start first. Before, the range
        // ended or started mid-char and the rope edit panicked.
        assert_eq!(backspace_range(&rope, 2), Some(0..1));
        assert_eq!(delete_forward_range(&rope, 2), Some(1..3));
        assert_eq!(delete_forward_range(&rope, 5), Some(3..7));
    }

    #[test]
    fn backspace_and_delete_treat_crlf_as_one_char() {
        // Scintilla deletes a CRLF pair as one unit (Document::DelCharBack,
        // Document::LenChar); deleting half left a lone CR or LF behind.
        let rope = Rope::from_str("a\r\nb");
        assert_eq!(backspace_range(&rope, 3), Some(1..3));
        assert_eq!(delete_forward_range(&rope, 1), Some(1..3));
        // Caret between CR and LF: only the neighbouring code unit goes.
        assert_eq!(backspace_range(&rope, 2), Some(1..2));
        assert_eq!(delete_forward_range(&rope, 2), Some(2..3));
        let rope = Rope::from_str("\r\n\r\n");
        assert_eq!(backspace_range(&rope, 4), Some(2..4));
        assert_eq!(delete_forward_range(&rope, 0), Some(0..2));
        // Lone CR and LF stay single.
        let rope = Rope::from_str("\n\r");
        assert_eq!(backspace_range(&rope, 2), Some(1..2));
        assert_eq!(delete_forward_range(&rope, 0), Some(0..1));
    }
    #[test]
    fn auto_indent_inherits_leading_whitespace_up_to_caret() {
        let rope = Rope::from_str("\tfoo {\r\n  bar");
        assert_eq!(auto_indent_context(&rope, 6), ("\t".into(), "\tfoo {".into()));
        assert_eq!(auto_indent_context(&rope, 0), ("".into(), "".into()));
        assert_eq!(auto_indent_context(&rope, 9), (" ".into(), " ".into()));
        assert_eq!(auto_indent_context(&rope, 13), ("  ".into(), "  bar".into()));
        // A caret inside `é` snaps to its start.
        assert_eq!(auto_indent_context(&Rope::from_str("  é"), 3), ("  ".into(), "  ".into()));
    }
}
