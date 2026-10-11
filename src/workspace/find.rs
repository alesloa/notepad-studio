// Find / Replace / Goto Line / Mark / Dialog-tab action methods on
// Workspace. Driven by FindState, GotoState, and MarkState. The Find dialog
// (in `ui::find_dialog`) calls these via cx.listener(...) on its own
// dispatch tree; menu items / keybindings dispatch them on the editor's
// dispatch tree, which still owns the actions even though the inline find
// bar is gone.

use std::ops::Range;

use gpui::{ClipboardItem, Context, Window};

use crate::actions::{
    EditFind, EditFindAllCurrent, EditFindAllOpened, EditFindClose, EditFindCount, EditFindNext,
    EditFindPrev, EditFindToggleCase, EditFindToggleExtended, EditFindToggleRegex,
    EditFindToggleReplace, EditFindToggleWord, EditFindToggleWrap, EditGotoLine, EditReplace,
    EditReplaceAll, EditReplaceAllOpened, EditReplaceCurrent, MarkAll, MarkClearAll, MarkCopy,
    MarkToggleBookmarkLine, MarkToggleCase, MarkToggleExtended, MarkTogglePurge, MarkToggleRegex,
    MarkToggleWord, MarkToggleWrap, SearchMark,
};
use crate::find_in_files::FindResult;
use crate::find_state::DialogTab;
use crate::tab::InputFocus;
use crate::text::extended_escape::interpret_extended;
use crate::ui::find_dialog::{close_find_dialog, open_or_focus_find_dialog};
use crate::workspace::Workspace;

/// True for bytes that participate in a word identifier — alphanumeric or
/// underscore. Multi-byte UTF-8 trailing bytes (`0x80..=0xBF`) are also
/// treated as word bytes so a multibyte identifier (e.g. Cyrillic) doesn't
/// get sliced mid-codepoint by the cursor-word walk in
/// `refresh_smart_highlight`.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || (0x80..=0xff).contains(&b)
}

/// Search options shared by the Find, Replace and Mark tabs.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct SearchFlags {
    regex: bool,
    extended: bool,
    whole_word: bool,
    match_case: bool,
    dot_matches_newline: bool,
}

/// Compile `query` the way the Find / Replace / Mark tabs search: Extended
/// decodes escapes first (ignored in regex mode), Normal and Extended match
/// literally, whole word wraps the literal in `\b`. Look-around and
/// backreferences are unsupported by the `regex` crate and come back as
/// `Err`, which callers report as "Invalid regex".
pub(super) fn search_regex(query: &str, flags: &SearchFlags) -> Result<regex::Regex, regex::Error> {
    let raw_query = if flags.extended && !flags.regex {
        interpret_extended(query)
    } else {
        query.to_string()
    };
    let pattern = if flags.regex {
        raw_query
    } else if flags.whole_word {
        format!(r"\b{}\b", regex::escape(&raw_query))
    } else {
        regex::escape(&raw_query)
    };
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(!flags.match_case)
        .multi_line(true)
        // `. matches newline` only matters in regex mode; literal patterns
        // have their dots escaped.
        .dot_matches_new_line(flags.regex && flags.dot_matches_newline)
        .build()
}

/// Every non-empty match of `re` in `text`, optionally restricted to the
/// byte range `scope` ("In selection"). Zero-length matches (`^`, `\b`,
/// `a*` on "b") are skipped so "current" never cycles on an empty range.
///
/// The scope is captured when "In selection" is toggled and is not shifted
/// by later edits or tab switches, so it can point past the end or into the
/// middle of a multi-byte char. Clamp and floor both ends to a char boundary.
fn collect_matches(text: &str, re: &regex::Regex, scope: Option<&Range<usize>>) -> Vec<Range<usize>> {
    let mut matches: Vec<Range<usize>> = Vec::new();
    if let Some(s) = scope {
        let lo = text.floor_char_boundary(s.start.min(text.len()));
        let hi = text.floor_char_boundary(s.end.min(text.len()));
        if lo < hi {
            let Some(slice) = text.get(lo..hi) else {
                return matches;
            };
            for m in re.find_iter(slice) {
                if m.start() == m.end() {
                    continue;
                }
                matches.push((lo + m.start())..(lo + m.end()));
            }
        }
    } else {
        for m in re.find_iter(text) {
            if m.start() == m.end() {
                continue;
            }
            matches.push(m.start()..m.end());
        }
    }
    matches
}

/// Index Find Next / Find Previous lands on. With a `current` match, step
/// one match forward or back. Without one, start at `caret`: the first
/// match at or after it going forward, the last match ending at or before
/// it going backward. `None` when the step would have to wrap and Wrap
/// around is off.
fn step_target(
    matches: &[Range<usize>],
    current: Option<usize>,
    caret: usize,
    forward: bool,
    wrap: bool,
) -> Option<usize> {
    let last = matches.len().checked_sub(1)?;
    let wrapped = |to: usize| if wrap { Some(to) } else { None };
    match (current.filter(|&c| c <= last), forward) {
        (Some(c), true) if c < last => Some(c + 1),
        (Some(_), true) => wrapped(0),
        (Some(c), false) if c > 0 => Some(c - 1),
        (Some(_), false) => wrapped(last),
        (None, true) => match matches.iter().position(|r| r.start >= caret) {
            Some(i) => Some(i),
            None => wrapped(0),
        },
        (None, false) => match matches.iter().rposition(|r| r.end <= caret) {
            Some(i) => Some(i),
            None => wrapped(last),
        },
    }
}

impl Workspace {
    /// Recompute `find_state.matches` from the current query + flags against
    /// the active buffer. Sets `error` on regex failure and clears `matches`.
    /// Picks the first match at or after the current cursor as `current`.
    pub(crate) fn refresh_matches(&mut self) {
        self.cancel_regex_replacement();
        if self.find_state.query.is_empty() {
            self.find_state.matches.clear();
            self.find_state.rich_doc_block_offsets = None;
            self.find_state.current = None;
            self.find_state.error = None;
            return;
        }

        let re = match search_regex(&self.find_state.query, &self.find_flags()) {
            Ok(r) => r,
            Err(e) => {
                self.find_state.matches.clear();
                self.find_state.rich_doc_block_offsets = None;
                self.find_state.current = None;
                self.find_state.error = Some(format!("Invalid regex: {e}"));
                self.find_state.status = None;
                return;
            }
        };

        // Rich-doc tabs search the visible joined text of paragraphs/headings
        // instead of the raw rope (which holds the XML markup the user never
        // sees). Match ranges become byte offsets into the flat text; the
        // per-block offset map is kept on find_state so navigation/replace/
        // highlight can translate them back into TextPos pairs.
        if let Some(payload) = &self.tabs[self.active].buffer.rich_doc {
            let (flat_text, block_offsets) = payload.flat_search_text();
            // In-selection scoping mirrors the source-view branch — when the
            // flag is on and a scope was captured at toggle time, restrict
            // the regex scan to that flat-text byte range. Scope bytes are
            // recorded in `toggle_find_in_selection` using the rich-doc
            // payload's `global_flat_for_pos`, so they align with the same
            // coordinate system the matches use.
            let scope = if self.find_state.in_selection {
                self.find_state.in_selection_scope.clone()
            } else {
                None
            };
            let matches = collect_matches(&flat_text, &re, scope.as_ref());
            // Pick the first match at or after the current cursor's global
            // flat position. Falls back to the first match when no cursor or
            // every match precedes the cursor.
            let cursor_flat = payload
                .selection
                .map(|s| payload.global_flat_for_pos(&block_offsets, s.cursor))
                .unwrap_or(0);
            let current = if matches.is_empty() {
                None
            } else {
                Some(matches.iter().position(|r| r.start >= cursor_flat).unwrap_or(0))
            };
            self.find_state.matches = matches;
            self.find_state.rich_doc_block_offsets = Some(block_offsets);
            self.find_state.current = current;
            self.find_state.error = None;
            self.find_state.status = None;
            return;
        }

        self.find_state.rich_doc_block_offsets = None;
        let head = self.tabs[self.active].selection.head;

        // "In selection" scoping. Block selection has its own column-aware
        // semantics — fall back to full-buffer scan if the active tab is in
        // block mode regardless of the flag. If the flag is on but no scope
        // was captured (empty selection at toggle time), also fall back.
        let scope = if self.find_state.in_selection
            && self.tabs[self.active].block_selection.is_none()
        {
            self.find_state.in_selection_scope.clone()
        } else {
            None
        };

        // Scope the &str borrow tight — we have to mutate `find_state` after.
        // `flat_text()` reuses the cached snapshot until the next rope edit,
        // so find-as-you-type is O(haystack-scan) per keystroke instead of
        // O(rope-flatten + haystack-scan).
        let matches: Vec<Range<usize>> = {
            let buffer_text = self.tabs[self.active].buffer.flat_text();
            collect_matches(buffer_text, &re, scope.as_ref())
        };

        let current = if matches.is_empty() {
            None
        } else {
            Some(matches.iter().position(|r| r.start >= head).unwrap_or(0))
        };

        self.find_state.matches = matches;
        self.find_state.current = current;
        self.find_state.error = None;
        self.find_state.status = None;
    }

    /// Maximum buffer size at which Smart Highlighting will scan. Larger
    /// buffers fall back to no smart band so the per-cursor-move scan never
    /// stalls the editor on huge files. Mirrors NPP's own defensive cutoff.
    pub(crate) const SMART_HILITE_BUFFER_LIMIT_BYTES: usize = 1024 * 1024;

    /// Recompute `smart_match_ranges` against the active buffer based on the
    /// word currently under the cursor. Cheap and self-skipping when the
    /// cached `smart_match_word` still covers the cursor — flip-flopping the
    /// caret inside one identifier does not trigger a re-scan.
    pub(crate) fn refresh_smart_highlight(&mut self) {
        let snap = crate::settings::snapshot();
        if !snap.smart_highlight_enabled {
            self.smart_match_ranges.clear();
            self.smart_match_word = None;
            return;
        }

        let head = self.tabs[self.active].selection.head;
        let buffer = &self.tabs[self.active].buffer;
        let total = buffer.rope.len_bytes();
        // Apply the buffer limit before copying a physical line or accepting
        // a cached word. One line can span the whole document.
        if total > Self::SMART_HILITE_BUFFER_LIMIT_BYTES || head > total {
            self.smart_match_ranges.clear();
            self.smart_match_word = None;
            return;
        }

        // Pull the word at cursor by walking the local line both directions
        // until we hit a non-word byte. Word = unicode word char per the
        // \w regex class (alphanumeric or underscore).
        let line_idx = buffer.rope.byte_to_line(head);
        let line_start = buffer.rope.line_to_byte(line_idx);
        let line_str = buffer.rope.line(line_idx).to_string();
        let line_str = line_str.trim_end_matches(['\n', '\r']);
        let local = head.saturating_sub(line_start).min(line_str.len());

        let bytes = line_str.as_bytes();
        let mut start = local;
        while start > 0 && is_word_byte(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = local;
        while end < bytes.len() && is_word_byte(bytes[end]) {
            end += 1;
        }
        if end <= start {
            self.smart_match_ranges.clear();
            self.smart_match_word = None;
            return;
        }
        let word = &line_str[start..end];
        if word.chars().count() < 2 {
            // Avoid lighting up every `i` / `x` in the file — NPP uses a
            // similar 2-char floor.
            self.smart_match_ranges.clear();
            self.smart_match_word = None;
            return;
        }

        // Cache hit — caret moved within the same word; existing ranges still
        // apply, no work needed.
        if self.smart_match_word.as_deref() == Some(word) {
            return;
        }

        let (case_sensitive, whole_word) = if snap.smart_highlight_use_find_settings
            && self.find_state.open
        {
            (self.find_state.match_case, self.find_state.whole_word)
        } else {
            (
                snap.smart_highlight_case_sensitive,
                snap.smart_highlight_whole_word,
            )
        };

        let escaped = regex::escape(word);
        let pattern = if whole_word {
            format!(r"\b{escaped}\b")
        } else {
            escaped
        };
        let re = match regex::RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .multi_line(true)
            .build()
        {
            Ok(r) => r,
            Err(_) => {
                self.smart_match_ranges.clear();
                self.smart_match_word = None;
                return;
            }
        };

        let cursor_word_range = (line_start + start)..(line_start + end);
        let word_owned = word.to_string();
        // Drop the immutable `buffer` borrow before reaching for `flat_text`,
        // which requires `&mut buffer` to (re)build the cache.
        let _ = buffer;
        let ranges: Vec<Range<usize>> = {
            let buffer_text = self.tabs[self.active].buffer.flat_text();
            let mut ranges: Vec<Range<usize>> = Vec::new();
            for m in re.find_iter(buffer_text) {
                if m.start() == m.end() {
                    continue;
                }
                // Skip the occurrence the cursor itself sits on — the caret's
                // visual presence already marks it.
                if m.start() == cursor_word_range.start && m.end() == cursor_word_range.end {
                    continue;
                }
                ranges.push(m.start()..m.end());
            }
            ranges
        };

        self.smart_match_ranges = ranges;
        self.smart_match_word = Some(word_owned);
    }

    /// Recompute `tag_match_pair` (and, when their sub-toggles are on,
    /// `tag_match_attrs` + `tag_match_zone`) against the active buffer based
    /// on the caret position. Gated on:
    ///   * `settings::match_tags_enabled`, and
    ///   * an HTML- or XML-language buffer (other languages get no overlay).
    /// Bails on buffers larger than [`Self::SMART_HILITE_BUFFER_LIMIT_BYTES`]
    /// so the per-caret-move scan can't stall the editor on huge documents.
    /// Attribute + zone passes piggyback on the same buffer snapshot so each
    /// caret move only rebuilds the source string once.
    pub(crate) fn refresh_tag_match(&mut self) {
        let snap = crate::settings::snapshot();
        if !snap.match_tags_enabled {
            self.tag_match_pair = None;
            self.tag_match_attrs.clear();
            self.tag_match_zone = None;
            return;
        }
        let buffer = &self.tabs[self.active].buffer;
        let html = matches!(buffer.language, Some(crate::syntax::LanguageId::Html));
        let xml = matches!(buffer.language, Some(crate::syntax::LanguageId::Xml));
        if !html && !xml {
            self.tag_match_pair = None;
            self.tag_match_attrs.clear();
            self.tag_match_zone = None;
            return;
        }
        let total = buffer.rope.len_bytes();
        if total > Self::SMART_HILITE_BUFFER_LIMIT_BYTES {
            self.tag_match_pair = None;
            self.tag_match_attrs.clear();
            self.tag_match_zone = None;
            return;
        }
        let head = self.tabs[self.active].selection.head.min(total);
        let _ = buffer;
        let (pair, attrs, zone) = {
            let source = self.tabs[self.active].buffer.flat_text();
            (
                super::tag_match::find_tag_pair(source, head, html),
                if snap.match_tag_attrs_enabled {
                    super::tag_match::find_attribute_ranges(source, head)
                } else {
                    Vec::new()
                },
                if snap.match_tags_in_comment_zone {
                    super::tag_match::find_zone_pair(source, head)
                } else {
                    None
                },
            )
        };
        self.tag_match_pair = pair;
        self.tag_match_attrs = attrs;
        self.tag_match_zone = zone;
    }

    pub(crate) fn open_find(&mut self, _: &EditFind, window: &mut Window, cx: &mut Context<Self>) {
        // When the active tab is a CSV in Preview mode, route Cmd+F to
        // the inline CSV find bar instead of the rope-text find dialog
        // — searching cells from a text dialog wouldn't make sense
        // (cell coords, not rope offsets, drive the match list).
        if csv_preview_active(self) {
            // Cmd+F always opens in find-only mode (drops the replace
            // row if it was up from a prior Cmd+H). Toggling between
            // Cmd+F and Cmd+H feels natural this way.
            let tab_id = self.tabs[self.active].session_id;
            if let Some(model) = self.csv_models.get_mut(&tab_id) {
                model.find_show_replace = false;
                model.replace_input = None;
            }
            crate::ui::csv_doc::actions::open_find_bar(self, window, cx);
            return;
        }
        // Pre-fill the query from the current selection if it's a single
        // line and not too long — matches Notepad++ and most modern editors.
        if !self.selection().is_empty() {
            let r = self.selection().ordered();
            let text = self.buffer().slice_bytes(r);
            if !text.is_empty() && !text.contains('\n') && text.len() < 256 {
                self.find_state.query = text;
                self.find_state.query_cursor = self.find_state.query.len();
            }
        }
        self.refresh_matches();
        open_or_focus_find_dialog(self, DialogTab::Find, window, cx);
    }

    /// Esc handler — closes the dialog if it's up. Editor focus is restored
    /// once the dialog window is removed because focus naturally returns to
    /// whichever window grabs activation next (the editor, which is still
    /// rendered behind the dialog).
    pub(crate) fn close_find(
        &mut self,
        _: &EditFindClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Auto-Completion popup — highest priority. The popup is mounted as
        // a deferred overlay but logically owns the keyboard while up, so
        // Esc must dismiss it before any dialog / context-menu Esc handler
        // runs. Cheap to check and idempotent when no popup is active.
        if self.autocomplete.is_some() {
            self.close_autocomplete(cx);
            self.close_function_hint(cx);
            return;
        }
        if self.function_hint_active() {
            self.close_function_hint(cx);
            return;
        }
        // Paste-format chooser modal — next priority. It's a true modal
        // mounted above every other overlay, so Esc must dismiss it before
        // falling through to less-important Esc-consumers.
        if self.paste_format_pending.is_some() {
            self.hide_paste_format_dialog(cx);
            return;
        }
        // Help ▸ About modal — also modal; ranks just below paste-format
        // because the two never co-mount in practice but the order matters
        // if we ever stack them.
        if self.about_dialog_open {
            self.hide_about_dialog(cx);
            return;
        }
        // Right-click context menu dismisses on Esc too — first thing checked
        // so a popped menu always wins over later overlays.
        if self.context_menu.is_some() {
            self.hide_context_menu(cx);
            return;
        }
        // Tab right-click menu — same priority as the editor context menu.
        if self.tab_context_menu.is_some() {
            self.hide_tab_context_menu(cx);
            return;
        }
        // CSV grid right-click menu — Esc dismisses without applying.
        if self.csv_context_menu.is_some() {
            self.hide_csv_context_menu(cx);
            return;
        }
        // CSV inline find bar — Esc closes the bar and returns focus
        // to the grid. Checked here so Esc inside the find input
        // doesn't fall through to the grid's drag / selection tiers.
        if self.input_focus(window) == InputFocus::CsvFindBar
            || self.input_focus(window) == InputFocus::CsvReplaceBar
        {
            crate::ui::csv_doc::actions::close_find_bar(self, window, cx);
            return;
        }
        if self.input_focus(window) == InputFocus::CsvCell {
            crate::ui::csv_doc::actions::cancel_cell_edit(self, cx);
            if let Some(handle) = self.csv_models.get(&self.tabs[self.active].session_id).and_then(|model| model.grid_focus.as_ref()) {
                window.focus(handle, cx);
            }
            return;
        }
        // CSV grid focus: cancel in-flight drag / resize first, then
        // clear axis selection, then the active-cell highlight. Each
        // tier consumes the Esc on its own so the user can dismiss the
        // grid state one step at a time without falling through to the
        // editor / find / dialog Esc tiers below.
        if self.input_focus(window) == InputFocus::CsvGrid
            && crate::ui::csv_doc::actions::handle_grid_escape(self, cx)
        {
            return;
        }
        // Inline tab-rename bar — Esc cancels.
        if self.tab_rename.is_some() {
            self.close_tab_rename(cx);
            window.focus(&self.focus_handle, cx);
            return;
        }
        // Folder-panel inline-create draft — Esc cancels without touching disk.
        if self.folder_inline_create.is_some() {
            self.cancel_folder_inline_create(cx);
            window.focus(&self.focus_handle, cx);
            return;
        }
        // Rich-doc inline link prompt and color picker — Esc dismisses.
        if self.rich_doc_link_bar.is_some() {
            self.close_rich_doc_link_bar(cx);
            window.focus(&self.focus_handle, cx);
            return;
        }
        if self.rich_doc_color_bar.is_some() {
            self.close_rich_doc_color_bar(cx);
            window.focus(&self.focus_handle, cx);
            return;
        }
        // Notepad++ column-mode block selection collapses on Esc, same
        // as a linear selection-clear in NPP. Returns the caret to the
        // block's head corner.
        if self.has_block_selection() {
            self.clear_block_selection();
            cx.notify();
            return;
        }
        // Inline macro prompt — Esc cancels with no replay / save.
        if self.macro_prompt.is_some() {
            self.close_macro_prompt(cx);
            window.focus(&self.focus_handle, cx);
            return;
        }
        // Inline goto bar still close-on-Esc — keep that path alive.
        if self.goto_state.open {
            self.goto_state.open = false;
            window.focus(&self.focus_handle, cx);
            cx.notify();
            return;
        }
        if self.find_dialog_window.is_some() {
            close_find_dialog(self, window, cx);
            window.focus(&self.focus_handle, cx);
            return;
        }
        if self.find_in_files.open {
            self.find_in_files.open = false;
            window.focus(&self.focus_handle, cx);
            cx.notify();
            return;
        }
        if self.pending_prepared_edit.is_some()
            && window.is_window_active()
            && self.focus_handle.is_focused(window)
            && self.input_focus(window) == InputFocus::Editor
            && self.tabs.get(self.active).is_some_and(|tab| tab.view_mode == crate::tab::TabViewMode::Edit)
        {
            self.cancel_prepared_edit(cx);
            self.set_status("Line indentation cancelled; no changes applied");
            cx.notify();
        }
    }

    /// Open the dialog on the Replace tab. Pre-fills query from selection
    /// the same way Find does — Notepad++ behaves this way.
    pub(crate) fn toggle_replace_bar(
        &mut self,
        _: &EditReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // CSV preview: route Cmd+H to the inline find bar with replace
        // row visible. Mirrors the Cmd+F intercept in `open_find` and
        // keeps the rope find-dialog from popping over a CSV file.
        if csv_preview_active(self) {
            crate::ui::csv_doc::actions::open_replace_bar(self, window, cx);
            return;
        }
        if self.find_state.query.is_empty() && !self.selection().is_empty() {
            let r = self.selection().ordered();
            let text = self.buffer().slice_bytes(r);
            if !text.is_empty() && !text.contains('\n') && text.len() < 256 {
                self.find_state.query = text;
                self.find_state.query_cursor = self.find_state.query.len();
            }
        }
        self.refresh_matches();
        open_or_focus_find_dialog(self, DialogTab::Replace, window, cx);
    }

    pub(crate) fn toggle_find_case(
        &mut self,
        _: &EditFindToggleCase,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_state.match_case = !self.find_state.match_case;
        self.refresh_matches();
        cx.notify();
    }

    pub(crate) fn toggle_find_word(
        &mut self,
        _: &EditFindToggleWord,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_state.whole_word = !self.find_state.whole_word;
        self.refresh_matches();
        cx.notify();
    }

    pub(crate) fn toggle_find_regex(
        &mut self,
        _: &EditFindToggleRegex,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_state.regex = !self.find_state.regex;
        // Notepad++ disables Extended when Regex turns on — treat them as
        // mutually exclusive so the user doesn't get a confusing pre-pass
        // happening before regex compilation.
        if self.find_state.regex {
            self.find_state.extended = false;
        }
        self.refresh_matches();
        cx.notify();
    }

    pub(crate) fn toggle_find_extended(
        &mut self,
        _: &EditFindToggleExtended,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_state.extended = !self.find_state.extended;
        if self.find_state.extended {
            self.find_state.regex = false;
        }
        self.refresh_matches();
        cx.notify();
    }

    pub(crate) fn toggle_find_wrap(
        &mut self,
        _: &EditFindToggleWrap,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_state.wrap_around = !self.find_state.wrap_around;
        cx.notify();
    }

    /// Flip the Backward direction flag. No match-list refresh needed —
    /// the flag only affects step direction in find_next / find_prev.
    pub(crate) fn toggle_find_backward(&mut self, cx: &mut Context<Self>) {
        self.find_state.backward = !self.find_state.backward;
        cx.notify();
    }

    /// Flip the In selection flag. When turning ON, capture the current
    /// linear selection's byte range as the scope; when turning OFF, drop
    /// the scope. Refreshes matches so the dialog footer / counter chip
    /// reflect the scoped count immediately.
    pub(crate) fn toggle_find_in_selection(&mut self, cx: &mut Context<Self>) {
        let now_on = !self.find_state.in_selection;
        self.find_state.in_selection = now_on;
        if now_on {
            // Rich-doc tabs capture the scope in flat-search-text byte
            // coordinates — `refresh_matches`'s rich-doc branch consumes the
            // scope in the same coordinate system. Empty / collapsed
            // selections capture nothing so the branch falls back to a full
            // scan rather than producing zero matches.
            if let Some(payload) = self.tabs[self.active].buffer.rich_doc.as_ref() {
                self.find_state.in_selection_scope = payload.selection.and_then(|sel| {
                    if sel.is_collapsed() {
                        return None;
                    }
                    let (_, offsets) = payload.flat_search_text();
                    let (lo_pos, hi_pos) = sel.range_full();
                    let lo = payload.global_flat_for_pos(&offsets, lo_pos);
                    let hi = payload.global_flat_for_pos(&offsets, hi_pos);
                    if hi > lo { Some(lo..hi) } else { None }
                });
            } else {
                // Source-view: block selection has its own column-aware
                // semantics — don't capture a scope from it; refresh_matches
                // falls back to full-buffer scan in that case anyway.
                let in_block = self.tabs[self.active].block_selection.is_some();
                let sel = self.tabs[self.active].selection;
                self.find_state.in_selection_scope = if !in_block && !sel.is_empty() {
                    Some(sel.ordered())
                } else {
                    None
                };
            }
        } else {
            self.find_state.in_selection_scope = None;
        }
        self.refresh_matches();
        cx.notify();
    }

    /// Re-implements the "toggle the replace row" action from the inline bar
    /// era as "switch the dialog to the Replace tab". Lets the menu item
    /// keep working without renaming the action type.
    pub(crate) fn toggle_replace_visibility(
        &mut self,
        _: &EditFindToggleReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find_dialog_window.is_some() {
            self.set_dialog_tab(DialogTab::Replace, window, cx);
        }
    }

    pub(crate) fn open_goto_line(
        &mut self,
        _: &EditGotoLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.goto_state.open = true;
        self.goto_state.text.clear();
        self.goto_state.text_cursor = 0;
        window.focus(&self.goto_focus, cx);
        cx.notify();
    }

    /// Move keyboard focus into one of the find/replace/goto inputs. Used by
    /// click handlers on the input boxes — the input field doesn't grow its
    /// own focus on click otherwise.
    pub(crate) fn focus_input(
        &mut self,
        target: InputFocus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if target != InputFocus::Editor { self.close_autocomplete(cx); self.close_function_hint(cx); }
        self.ensure_dialog_focus_handles(cx);
        match target {
            InputFocus::FindQuery => window.focus(&self.find_focus, cx),
            InputFocus::Replacement => window.focus(&self.replace_focus, cx),
            InputFocus::Goto => window.focus(&self.goto_focus, cx),
            InputFocus::FifQuery => window.focus(&self.fif_query_focus, cx),
            InputFocus::FifReplacement => {
                if let Some(h) = self.fif_replacement_focus.clone() {
                    window.focus(&h, cx);
                }
            }
            InputFocus::FifFilter => window.focus(&self.fif_filter_focus, cx),
            InputFocus::FifDirectory => {
                if let Some(h) = self.fif_directory_focus.clone() {
                    window.focus(&h, cx);
                }
            }
            InputFocus::MarkQuery => {
                if let Some(h) = self.mark_focus.clone() {
                    window.focus(&h, cx);
                }
            }
            InputFocus::MacroPrompt => {
                if let Some(h) = self.macro_focus.clone() {
                    window.focus(&h, cx);
                }
            }
            InputFocus::TabRename => {
                if let Some(h) = self.tab_rename_focus.clone() {
                    window.focus(&h, cx);
                }
            }
            InputFocus::FolderInlineCreate => {
                if let Some(h) = self.folder_inline_create_focus.clone() {
                    window.focus(&h, cx);
                }
            }
            InputFocus::RichDocLinkUrl => {
                if let Some(h) = self.rich_doc_link_focus.clone() {
                    window.focus(&h, cx);
                }
            }
            InputFocus::RichDocColorHex => {
                if let Some(h) = self.rich_doc_color_focus.clone() {
                    window.focus(&h, cx);
                }
            }
            InputFocus::RichDocBody => {
                if let Some(h) = self.rich_doc_body_focus.clone() {
                    window.focus(&h, cx);
                }
            }
            InputFocus::CsvCell => {
                let tab_id = self.tabs[self.active].session_id;
                if let Some(model) = self.csv_models.get(&tab_id) {
                    if let Some(input) = model.cell_input.as_ref() {
                        window.focus(&input.focus_handle, cx);
                    }
                }
            }
            InputFocus::CsvGrid => {
                let tab_id = self.tabs[self.active].session_id;
                if let Some(model) = self.csv_models.get(&tab_id) {
                    if let Some(h) = model.grid_focus.as_ref() {
                        window.focus(h, cx);
                    }
                }
            }
            InputFocus::CsvFindBar => {
                let tab_id = self.tabs[self.active].session_id;
                if let Some(model) = self.csv_models.get(&tab_id) {
                    if let Some(h) = model.find_input_handle.as_ref() {
                        window.focus(h, cx);
                    }
                }
            }
            InputFocus::CsvReplaceBar => {
                let tab_id = self.tabs[self.active].session_id;
                if let Some(model) = self.csv_models.get(&tab_id) {
                    if let Some(h) = model.replace_input_handle.as_ref() {
                        window.focus(h, cx);
                    }
                }
            }
            InputFocus::Editor => window.focus(&self.focus_handle, cx),
        }
        cx.notify();
    }

    /// User picked an entry from the chevron history dropdown — overwrite
    /// the corresponding input's text + selection (caret to end), close the
    /// dropdown, refresh matches when the slot is the active-tab search
    /// query. Matches NPP's "pick from history" behaviour: the dialog
    /// behaves as if the user retyped the entry then paused — no automatic
    /// Find Next.
    pub(crate) fn apply_find_history_pick(
        &mut self,
        slot: crate::settings::FindHistorySlot,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::settings::FindHistorySlot;
        let end = value.len();
        match slot {
            FindHistorySlot::FindQuery => {
                self.find_state.query = value;
                self.find_state.query_cursor = end;
                self.find_state.query_anchor = end;
                self.refresh_matches();
                self.focus_input(InputFocus::FindQuery, window, cx);
            }
            FindHistorySlot::Replacement => {
                self.find_state.replacement = value;
                self.find_state.replacement_cursor = end;
                self.find_state.replacement_anchor = end;
                self.focus_input(InputFocus::Replacement, window, cx);
            }
            FindHistorySlot::FifQuery => {
                self.find_in_files.query = value;
                self.find_in_files.query_cursor = end;
                self.find_in_files.query_anchor = end;
                self.focus_input(InputFocus::FifQuery, window, cx);
            }
            FindHistorySlot::FifReplacement => {
                self.find_in_files.replacement = value;
                self.find_in_files.replacement_cursor = end;
                self.find_in_files.replacement_anchor = end;
                self.focus_input(InputFocus::FifReplacement, window, cx);
            }
            FindHistorySlot::FifFilter => {
                self.find_in_files.filter = value;
                self.find_in_files.filter_cursor = end;
                self.find_in_files.filter_anchor = end;
                self.focus_input(InputFocus::FifFilter, window, cx);
            }
            FindHistorySlot::FifDirectory => {
                self.find_in_files.directory_text = value;
                self.find_in_files.directory_text_cursor = end;
                self.find_in_files.directory_text_anchor = end;
                self.focus_input(InputFocus::FifDirectory, window, cx);
            }
            FindHistorySlot::MarkQuery => {
                self.mark_state.query = value;
                self.mark_state.query_cursor = end;
                self.mark_state.query_anchor = end;
                self.focus_input(InputFocus::MarkQuery, window, cx);
            }
        }
        self.find_history_open = None;
        cx.notify();
    }

    /// Place the editor cursor on `matches[idx]`, selecting the match so the
    /// user can see exactly what they're on. Notepad++ behaves the same way
    /// — Find Next selects the next occurrence. Branches on whether matches
    /// were sourced from the rope or from the rich-doc flat text.
    fn jump_to_match(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(range) = self.find_state.matches.get(idx).cloned() else {
            return;
        };
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        if let Some(offsets) = self.find_state.rich_doc_block_offsets.clone() {
            let payload_opt = self.tabs[self.active].buffer.rich_doc.as_ref();
            let (Some(anchor_pos), Some(cursor_pos)) = (
                payload_opt.and_then(|p| p.pos_from_global_flat(&offsets, range.start, true)),
                payload_opt.and_then(|p| p.pos_from_global_flat(&offsets, range.end, true)),
            ) else {
                return;
            };
            if let Some(payload) = self.tabs[self.active].buffer.rich_doc.as_mut() {
                payload.selection = Some(crate::rich_doc::TextSelection {
                    anchor: anchor_pos,
                    cursor: cursor_pos,
                });
                payload.active_block = Some(cursor_pos.block);
                payload.drag_active = false;
                payload.pending_drag_anchor = None;
            }
            self.find_state.current = Some(idx);
            self.caret_visible = true;
            // Multi-block matches: scroll to the FIRST block of the span so
            // the user sees the start of the match. Mirrors NPP / VS Code.
            self.rich_doc_scroll_match_into_view(anchor_pos.block, cx);
            cx.notify();
            return;
        }
        // Matches come from a scan of this rope, so they sit on char
        // boundaries inside it. Check anyway: a range that doesn't would put
        // the selection mid-char, and the next `byte_slice` on it panics.
        if self.buffer().rope.get_byte_slice(range.clone()).is_none() {
            return;
        }
        self.settle_source_input_edit_caret();
        let tab = self.active_tab_mut();
        tab.selection.anchor = range.start;
        tab.selection.head = range.end;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        self.sync_source_input_caret();
        self.find_state.current = Some(idx);
        self.last_lines.clear();
        self.caret_visible = true;
        self.ensure_cursor_visible(cx);
        cx.notify();
    }

    pub(crate) fn find_next(&mut self, _: &EditFindNext, _w: &mut Window, cx: &mut Context<Self>) {
        // Backward direction radio inverts both Find Next and Find Prev so
        // the persistent toggle behaves like NPP's Up/Down direction radio.
        let forward = !self.find_state.backward;
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FindQuery,
            &self.find_state.query,
        );
        self.step_match(forward, cx);
    }

    pub(crate) fn find_prev(&mut self, _: &EditFindPrev, _w: &mut Window, cx: &mut Context<Self>) {
        // Shift+Enter inside the CSV grid maps to Excel's "step up one
        // row" — the same dispatch as Shift+Enter while editing. Route
        // before find_prev's actual work so the shortcut doesn't
        // double as a stealth find step. Shift+Enter reaches this
        // handler through `insert_newline`, which owns the chord.
        let focus = self.input_focus(_w);
        if focus == InputFocus::CsvCell || focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::move_active_cell_enter(self, false, cx);
            return;
        }
        // Shift+Enter inside the find bar steps to the previous match.
        if focus == InputFocus::CsvFindBar {
            crate::ui::csv_doc::actions::find_step(self, false, _w, cx);
            return;
        }
        // Shift+Enter inside the replace bar runs Replace All.
        if focus == InputFocus::CsvReplaceBar {
            crate::ui::csv_doc::actions::replace_all_action(self, cx);
            return;
        }
        let forward = self.find_state.backward;
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FindQuery,
            &self.find_state.query,
        );
        self.step_match(forward, cx);
    }

    /// Apply a replace to a rich-doc match. `range` is a half-open byte
    /// range into the joined flat-search text (matching the units in
    /// `find_state.matches`). Translates to TextPos coordinates, dispatches
    /// `DeleteRange` or `DeleteAcrossBlocks`, then inserts the replacement
    /// at the start position. Sets the caret to the end of the inserted
    /// text so subsequent edits stay anchored where the user expects.
    fn replace_rich_doc_range(&mut self, range: Range<usize>, replacement: &str) -> bool {
        if self.buffer().read_only || (range.is_empty() && replacement.is_empty()) {
            return false;
        }
        let Some(offsets) = self.find_state.rich_doc_block_offsets.clone() else {
            return false;
        };
        let payload_opt = self.tabs[self.active].buffer.rich_doc.as_ref();
        let (Some(lo), Some(hi)) = (
            payload_opt.and_then(|p| p.pos_from_global_flat(&offsets, range.start, true)),
            payload_opt.and_then(|p| p.pos_from_global_flat(&offsets, range.end, true)),
        ) else {
            return false;
        };
        let Some(payload) = self.tabs[self.active].buffer.rich_doc.as_mut() else {
            return false;
        };
        // Delete the matched span first.
        if !(lo.block == hi.block && lo.run == hi.run && lo.byte == hi.byte) {
            if lo.block == hi.block {
                payload.apply_edit(crate::rich_doc::edits::Edit::DeleteRange {
                    block: lo.block,
                    run_lo: lo.run,
                    byte_lo: lo.byte,
                    run_hi: hi.run,
                    byte_hi: hi.byte,
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
            }
        }
        // After the delete, the insertion point is at (lo.block, lo.run, lo.byte).
        // For an empty replacement (a delete), there's nothing more to do.
        if !replacement.is_empty() {
            payload.apply_edit(crate::rich_doc::edits::Edit::InsertText {
                block: lo.block,
                run: lo.run,
                byte: lo.byte,
                text: replacement.to_string(),
            });
        }
        // Place the caret at the end of the replacement (or the deletion
        // point when replacement is empty) so the user sees where the edit
        // landed and so a follow-up Find Next picks the right next match.
        let caret_flat = payload.flat_byte_for(lo).saturating_add(replacement.len());
        let caret = payload.pos_from_flat_byte(lo.block, caret_flat);
        payload.selection = Some(crate::rich_doc::TextSelection {
            anchor: caret,
            cursor: caret,
        });
        payload.active_block = Some(lo.block);
        payload.drag_active = false;
        payload.pending_drag_anchor = None;
        true
    }

    pub(super) fn replace_rich_doc_ranges(
        &mut self,
        ranges: &[Range<usize>],
        replacement: &str,
        cx: &mut Context<Self>,
    ) -> usize {
        if !self.rich_doc_edit_allowed(cx) {
            return 0;
        }
        self.with_rich_doc_edit_group(cx, |w, cx| {
            let mut replaced = 0usize;
            for range in ranges {
                if w.replace_rich_doc_range(range.clone(), replacement) {
                    replaced = replaced.saturating_add(1);
                }
            }
            if replaced > 0 {
                w.after_rich_doc_edit(cx);
            }
            replaced
        })
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let kept = self.resync_matches();
        if self.find_state.matches.is_empty() {
            return;
        }
        let current = if kept { self.find_state.current } else { None };
        let caret = self.find_caret(forward);
        let Some(target) = step_target(
            &self.find_state.matches,
            current,
            caret,
            forward,
            self.find_state.wrap_around,
        ) else {
            return;
        };
        self.jump_to_match(target, cx);
    }

    /// Re-run the search against the active buffer before acting on
    /// `find_state.matches`. The cache only follows edits while the dialog
    /// is open and never follows a tab switch, so it can hold ranges from
    /// another buffer: past the end, inside a multi-byte char, or over
    /// unrelated text that Replace would then overwrite. Returns true when
    /// the match the user was on is still there (same byte range) and keeps
    /// it as `current`, so stepping is unchanged when nothing moved.
    fn resync_matches(&mut self) -> bool {
        let prev = self
            .find_state
            .current
            .and_then(|i| self.find_state.matches.get(i).cloned());
        let status = self.find_state.status.take();
        self.refresh_matches();
        if self.find_state.error.is_none() {
            self.find_state.status = status;
        }
        let Some(prev) = prev else {
            return false;
        };
        match self.find_state.matches.iter().position(|r| *r == prev) {
            Some(i) => {
                self.find_state.current = Some(i);
                true
            }
            None => false,
        }
    }

    pub(super) fn find_flags(&self) -> SearchFlags {
        SearchFlags {
            regex: self.find_state.regex,
            extended: self.find_state.extended,
            whole_word: self.find_state.whole_word,
            match_case: self.find_state.match_case,
            dot_matches_newline: self.find_state.dot_matches_newline,
        }
    }

    /// Byte offset a Find Next / Find Previous with no current match starts
    /// from: the selection's end going forward, its start going backward.
    /// Rich-doc tabs use flat-search-text offsets, like their matches.
    fn find_caret(&self, forward: bool) -> usize {
        let Some(tab) = self.tabs.get(self.active) else {
            return 0;
        };
        if let Some(offsets) = self.find_state.rich_doc_block_offsets.as_ref() {
            let Some(payload) = tab.buffer.rich_doc.as_ref() else {
                return 0;
            };
            let Some(sel) = payload.selection else {
                return 0;
            };
            let (lo, hi) = sel.range_full();
            let pos = if forward { hi } else { lo };
            return payload.global_flat_for_pos(offsets, pos);
        }
        let sel = tab.selection.ordered();
        if forward { sel.end } else { sel.start }
    }

    /// "Count" — populate `find_state.status` with a one-line tally. The
    /// existing matches snapshot already has the count; just surface it.
    pub(crate) fn find_count(
        &mut self,
        _: &EditFindCount,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find_state.query.is_empty() {
            self.find_state.status = Some("Enter a query first".to_string());
        } else {
            self.resync_matches();
            let n = self.find_state.matches.len();
            self.find_state.status = Some(format!(
                "{n} match{}",
                if n == 1 { "" } else { "es" }
            ));
        }
        cx.notify();
    }

    /// "Find All in Current Document" — collects every match in the active
    /// buffer, populates the Find-in-Files results panel grouped by file, and
    /// opens that panel so the user can click each hit to jump to it.
    /// Mirrors Notepad++'s `Search ▸ Find All in Current Document` which
    /// pops up the same `Search results` panel as Find in Files / Find All
    /// in All Opened Documents.
    ///
    /// Also keeps the legacy behavior of marking every match (so the editor
    /// stays highlighted after the dialog closes) — the panel is purely
    /// additive.
    pub(crate) fn find_all_current(
        &mut self,
        _: &EditFindAllCurrent,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find_state.query.is_empty() {
            self.find_state.status = Some("Enter a query first".to_string());
            cx.notify();
            return;
        }
        let Some(re) = self.build_find_regex() else {
            cx.notify();
            return;
        };

        let tab_idx = self.active;
        let buffer = &self.tabs[tab_idx].buffer;
        let content = buffer.rope.to_string();

        let display_label = buffer
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                buffer
                    .new_label
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("new {}", tab_idx + 1))
            });
        let path = buffer.file_path.clone().unwrap_or_default();

        let mut results: Vec<FindResult> = Vec::new();
        crate::find_in_files::collect_results(
            &content,
            &re,
            &path,
            Some(tab_idx),
            Some(&display_label),
            &mut results,
        );

        let total = results.len();
        let truncated = total >= crate::find_in_files::FIND_IN_FILES_RESULT_CAP;

        // Keep the legacy Mark-All side effect.
        self.refresh_matches();
        self.mark_state.marks = self.find_state.matches.clone();

        self.find_in_files.results = results;
        self.find_in_files.truncated = truncated;
        self.find_in_files.error = None;
        self.find_in_files.searching = false;
        self.find_in_files.last_search_query = self.find_state.query.clone();
        self.find_in_files.last_search_mode = crate::workspace::find_in_files::describe_search_mode(
            self.find_state.regex,
            self.find_state.extended,
        );
        self.find_in_files.last_search_files_searched = 1;
        self.find_in_files.status = Some(if total == 0 {
            "No matches in current document".to_string()
        } else {
            format!(
                "{total} match{} in current document{}",
                if total == 1 { "" } else { "es" },
                if truncated { " (truncated)" } else { "" },
            )
        });
        self.find_state.status = self.find_in_files.status.clone();
        self.find_in_files.open = true;
        cx.notify();
    }

    /// Replace the current match with `find_state.replacement` (which may be
    /// the empty string — that's a delete). Match list is recomputed against
    /// the post-replace buffer so subsequent Find Next / Replace lands on the
    /// match immediately after the cursor.
    pub(crate) fn replace_current_match(
        &mut self,
        _: &EditReplaceCurrent,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_function_hint(cx);
        if self.find_state.regex {
            self.start_regex_replacement(super::find_replacement::Mode::Current, cx);
            return;
        }
        if self.buffer().read_only {
            self.set_status("Buffer is read-only");
            cx.notify();
            return;
        }
        self.resync_matches();
        let Some(idx) = self.find_state.current else {
            return;
        };
        let Some(range) = self.find_state.matches.get(idx).cloned() else {
            return;
        };
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FindQuery,
            &self.find_state.query,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::Replacement,
            &self.find_state.replacement,
        );
        let replacement = if self.find_state.extended && !self.find_state.regex {
            interpret_extended(&self.find_state.replacement)
        } else {
            self.find_state.replacement.clone()
        };
        // Rich-doc path — translate flat range into AST coordinates, delete
        // (single-block or cross-block), then insert replacement at the start
        // position. Refresh recomputes from the post-edit flat text.
        if self.find_state.rich_doc_block_offsets.is_some() {
            // Pre-compute the byte delta so we can slide the in-selection
            // scope's end edge by the same amount the flat text grew/shrank.
            // Without this the next refresh either over-scans (text grew) or
            // drops matches (text shrank) inside the captured scope.
            let delta = replacement.len() as isize - (range.end - range.start) as isize;
            if self.replace_rich_doc_ranges(std::slice::from_ref(&range), &replacement, cx) == 0 {
                return;
            }
            if let Some(scope) = self.find_state.in_selection_scope.as_mut() {
                scope.end = (scope.end as isize + delta).max(scope.start as isize) as usize;
            }
            self.refresh_matches();
            if crate::settings::snapshot().replace_dont_move_to_next
                && !self.find_state.matches.is_empty()
            {
                let clamped = idx.min(self.find_state.matches.len() - 1);
                self.find_state.current = Some(clamped);
            }
            self.find_state.status = Some("Replaced 1 occurrence".to_string());
            cx.notify();
            return;
        }
        let delta = replacement.len() as isize - (range.end - range.start) as isize;
        self.replace_byte_range(range, &replacement, cx);
        // Keep the in-selection scope alive across replacements by sliding
        // its end edge by the byte delta. Without this the next refresh
        // would either over-scan (if scope.end stayed put while text grew)
        // or miss matches (if text shrank).
        if let Some(scope) = self.find_state.in_selection_scope.as_mut() {
            scope.end = (scope.end as isize + delta).max(scope.start as isize) as usize;
        }
        self.refresh_matches();
        // NPP `Searching ▸ Replace: Don't move to following occurrence`.
        // `refresh_matches` defaults to selecting the first match at-or-after
        // the cursor (i.e. the next match). When the user has opted out of
        // that advance, retarget the highlight at the same logical index —
        // clamped because a Replace can shrink the match list (a replacement
        // text that itself doesn't match, etc.).
        if crate::settings::snapshot().replace_dont_move_to_next
            && !self.find_state.matches.is_empty()
        {
            let clamped = idx.min(self.find_state.matches.len() - 1);
            self.find_state.current = Some(clamped);
        }
        self.find_state.status = Some("Replaced 1 occurrence".to_string());
        cx.notify();
    }

    /// Replace every cached match with `find_state.replacement`. Walks back-
    /// to-front so earlier byte ranges stay valid as the rope mutates. Wraps
    /// the entire run in a `begin_edit_group` / `end_edit_group` transaction
    /// so a single Undo press reverses every replacement and restores the
    /// caret to where it sat before Replace All ran.
    pub(crate) fn replace_all_matches(
        &mut self,
        _: &EditReplaceAll,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_function_hint(cx);
        if self.find_state.regex {
            self.start_regex_replacement(super::find_replacement::Mode::All, cx);
            return;
        }
        if self.buffer().read_only {
            self.set_status("Buffer is read-only");
            cx.notify();
            return;
        }
        self.resync_matches();
        if self.find_state.matches.is_empty() {
            return;
        }
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FindQuery,
            &self.find_state.query,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::Replacement,
            &self.find_state.replacement,
        );
        let count = self.find_state.matches.len();
        let replacement = if self.find_state.extended && !self.find_state.regex {
            interpret_extended(&self.find_state.replacement)
        } else {
            self.find_state.replacement.clone()
        };
        // Rich-doc path — back-to-front delete/insert per match so earlier
        // offsets stay valid as the AST mutates. Each edit goes through
        // RichDocPayload::apply_edit (logged for save replay).
        if self.find_state.rich_doc_block_offsets.is_some() {
            let ranges: Vec<Range<usize>> = self
                .find_state
                .matches
                .iter()
                .rev()
                .cloned()
                .collect();
            // Sum byte-delta across all replacements so we can slide the
            // in-selection scope's end edge after the run finishes — same
            // pattern as the source-view branch below.
            let mut total_delta: isize = 0;
            for r in &ranges {
                total_delta += replacement.len() as isize - (r.end - r.start) as isize;
            }
            let count = self.replace_rich_doc_ranges(&ranges, &replacement, cx);
            if count == 0 {
                return;
            }
            if let Some(scope) = self.find_state.in_selection_scope.as_mut() {
                scope.end = (scope.end as isize + total_delta).max(scope.start as isize) as usize;
            }
            self.refresh_matches();
            let status = format!(
                "Replaced {count} occurrence{}",
                if count == 1 { "" } else { "s" }
            );
            self.find_state.status = Some(status.clone());
            self.set_status(status);
            cx.notify();
            return;
        }
        let ranges: Vec<Range<usize>> = self
            .find_state
            .matches
            .iter()
            .rev()
            .cloned()
            .collect();
        // Sum byte-delta across all replacements so we can slide the
        // in-selection scope's end edge after the run finishes. Doing it
        // once at the end is fine since ranges are processed back-to-front
        // and `replace_byte_range` doesn't read the scope itself.
        let mut total_delta: isize = 0;
        self.begin_edit_group();
        for r in ranges {
            total_delta += replacement.len() as isize - (r.end - r.start) as isize;
            self.replace_byte_range(r, &replacement, cx);
        }
        self.end_edit_group(cx);
        if let Some(scope) = self.find_state.in_selection_scope.as_mut() {
            scope.end = (scope.end as isize + total_delta).max(scope.start as isize) as usize;
        }
        self.refresh_matches();
        let status = format!(
            "Replaced {count} occurrence{}",
            if count == 1 { "" } else { "s" }
        );
        self.find_state.status = Some(status.clone());
        self.set_status(status);
        cx.notify();
    }

    /// Build a `regex::Regex` from the live `find_state` flags. Centralised
    /// here so the all-opened paths can't drift from `refresh_matches`. Returns
    /// `None` and surfaces `find_state.error` if the user's pattern is empty
    /// or invalid — caller bails on `None`.
    fn build_find_regex(&mut self) -> Option<regex::Regex> {
        if self.find_state.query.is_empty() {
            self.find_state.matches.clear();
            self.find_state.current = None;
            self.find_state.error = None;
            self.find_state.status = None;
            return None;
        }
        match search_regex(&self.find_state.query, &self.find_flags()) {
            Ok(r) => {
                self.find_state.error = None;
                Some(r)
            }
            Err(e) => {
                self.find_state.error = Some(format!("Invalid regex: {e}"));
                self.find_state.status = None;
                None
            }
        }
    }

    /// Notepad++'s "Find All in All Opened Documents" — runs the active Find
    /// pattern against every open buffer in this window and pushes the results
    /// into the Find-in-Files panel so the user can click a row to jump.
    ///
    /// Reuses the FIF results panel rather than building a parallel sink:
    /// the result row already does file-name + line + snippet + click-to-jump,
    /// which is exactly what Notepad++ shows for this command. Each result is
    /// tagged with the originating tab index so jump-to-result skips the
    /// path-based tab lookup (which would fail for Untitled buffers).
    pub(crate) fn find_all_in_all_opened(
        &mut self,
        _: &EditFindAllOpened,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find_state.query.is_empty() {
            self.find_state.status = Some("Enter a query first".to_string());
            cx.notify();
            return;
        }
        let Some(re) = self.build_find_regex() else {
            cx.notify();
            return;
        };

        let mut results: Vec<FindResult> = Vec::new();
        let mut docs_with_hits: usize = 0;

        for (tab_idx, tab) in self.tabs.iter().enumerate() {
            let buffer = &tab.buffer;
            let content = buffer.rope.to_string();

            let display_label = buffer
                .file_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| {
                    // Untitled buffers carry their own "new N" label so each
                    // row stays unambiguous in the results panel.
                    buffer
                        .new_label
                        .as_ref()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("new {}", tab_idx + 1))
                });
            let path = buffer.file_path.clone().unwrap_or_default();

            let added = crate::find_in_files::collect_results(
                &content,
                &re,
                &path,
                Some(tab_idx),
                Some(&display_label),
                &mut results,
            );
            if added > 0 {
                docs_with_hits += 1;
            }
            if results.len() >= crate::find_in_files::FIND_IN_FILES_RESULT_CAP {
                break;
            }
        }

        let total = results.len();
        let truncated = total >= crate::find_in_files::FIND_IN_FILES_RESULT_CAP;
        let files_searched = self.tabs.len();
        self.find_in_files.results = results;
        self.find_in_files.truncated = truncated;
        self.find_in_files.error = None;
        self.find_in_files.searching = false;
        self.find_in_files.last_search_query = self.find_state.query.clone();
        self.find_in_files.last_search_mode = crate::workspace::find_in_files::describe_search_mode(
            self.find_state.regex,
            self.find_state.extended,
        );
        self.find_in_files.last_search_files_searched = files_searched;
        self.find_in_files.status = Some(if total == 0 {
            "No matches in any open document".to_string()
        } else {
            format!(
                "{total} match{} across {docs_with_hits} document{}{}",
                if total == 1 { "" } else { "es" },
                if docs_with_hits == 1 { "" } else { "s" },
                if truncated { " (truncated)" } else { "" },
            )
        });
        // Mirror the same status into the find-dialog chip area so the user
        // sees feedback even before glancing at the panel.
        self.find_state.status = self.find_in_files.status.clone();
        self.find_in_files.open = true;
        cx.notify();
    }

    /// Notepad++'s "Replace All in All Opened Documents" — applies the active
    /// Find pattern + Replace string to every open buffer through the same
    /// `apply_edit` funnel that powers per-buffer Replace All.
    ///
    /// Each buffer's edits are an independent undo group: the function
    /// switches `self.active` to that buffer's index, runs the back-to-front
    /// rewrite, then restores the original active tab so the user's view
    /// doesn't jump. No cross-buffer transaction — undo on the active tab
    /// only walks back its own buffer's history.
    pub(crate) fn replace_all_in_all_opened(
        &mut self,
        _: &EditReplaceAllOpened,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_regex_replacement();
        if self.find_state.query.is_empty() {
            self.find_state.status = Some("Enter a query first".to_string());
            cx.notify();
            return;
        }
        if !self.find_state.regex && self.build_find_regex().is_none() {
            cx.notify();
            return;
        }

        // NPP `Searching ▸ Confirm Replace All in All Opened Documents`.
        // When enabled, fire an OS-native Yes/No before rewriting every
        // open buffer. The actual loop lives in `run_replace_all_in_all`
        // so the immediate path and the post-confirm path call the same
        // code.
        if crate::settings::snapshot().confirm_replace_all_in_all_docs {
            let Some(confirmation) = self.begin_replace_all_confirmation(cx) else { return };
            let query = self.find_state.query.clone();
            let replacement_preview = self.find_state.replacement.clone();
            let doc_count = self.tabs.len();
            cx.spawn_in(window, async move |this, cx| {
                let confirm = rfd::AsyncMessageDialog::new()
                    .set_level(rfd::MessageLevel::Warning)
                    .set_title("Replace All in All Opened")
                    .set_description(format!(
                        "Replace every match of\n  {query}\nwith\n  {replacement_preview}\nacross {doc_count} open document(s)?"
                    ))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show()
                    .await;
                if !matches!(confirm, rfd::MessageDialogResult::Yes) {
                    let _ = this.update(cx, |w, cx| {
                        if !w.replacement_confirmation_valid(&confirmation) {
                            w.retire_replace_all_confirmation(&confirmation, cx);
                            return;
                        }
                        w.cancel_regex_replacement();
                        w.find_state.status = Some("Replace All in All Opened cancelled".to_string());
                        cx.notify();
                    });
                    return;
                }
                let _ = this.update(cx, |w, cx| {
                    w.resume_replace_all_confirmation(confirmation, cx);
                });
            })
            .detach();
            return;
        }

        self.run_replace_all_in_all(cx);
    }

    /// The actual rewrite loop, extracted so both the immediate and the
    /// post-confirm path share semantics.
    pub(super) fn run_replace_all_in_all(&mut self, cx: &mut Context<Self>) {
        if self.find_state.regex {
            self.start_regex_replacement(super::find_replacement::Mode::AllOpened, cx);
            return;
        }
        let Some(re) = self.build_find_regex() else {
            cx.notify();
            return;
        };

        let replacement = if self.find_state.extended && !self.find_state.regex {
            interpret_extended(&self.find_state.replacement)
        } else {
            self.find_state.replacement.clone()
        };

        let original_active = self.active;
        let mut total_replacements: usize = 0;
        let mut docs_changed: usize = 0;

        for tab_idx in 0..self.tabs.len() {
            // Compute matches against this buffer's current rope. Each
            // replacement walks back-to-front so earlier byte ranges stay
            // valid.
            let content = self.tabs[tab_idx].buffer.rope.to_string();
            let ranges: Vec<Range<usize>> = re
                .find_iter(&content)
                .filter(|m| m.start() != m.end())
                .map(|m| m.start()..m.end())
                .collect();
            if ranges.is_empty() {
                continue;
            }
            let count_in_buffer = ranges.len();

            // `apply_edit` / `replace_byte_range` operates on `self.active`.
            // Switch the index directly (without touching session/last_lines)
            // so the user's viewport doesn't twitch through every buffer.
            // We restore `self.active` at the end.
            //
            // Each per-buffer rewrite opens its own undo transaction so a
            // single Undo press on that buffer reverses every replacement
            // there at once. There is no cross-buffer composite — undo on
            // tab N only walks tab N's history.
            self.active = tab_idx;
            self.begin_edit_group();
            for r in ranges.into_iter().rev() {
                self.replace_byte_range(r, &replacement, cx);
            }
            self.end_edit_group(cx);
            total_replacements += count_in_buffer;
            docs_changed += 1;
        }

        // Restore active tab without going through `set_active_tab` (which
        // early-returns when idx == active). Manual reset of the cached
        // line geometry so the editor repaint picks up the right buffer.
        if self.active != original_active {
            self.active = original_active;
            self.last_lines.clear();
        }

        // Refresh the active buffer's match snapshot so subsequent Find Next
        // / Replace land on the post-edit state of *this* buffer.
        self.refresh_matches();

        let status = if total_replacements == 0 {
            "No matches in any open document".to_string()
        } else {
            format!(
                "Replaced {total_replacements} occurrence{} across {docs_changed} document{}",
                if total_replacements == 1 { "" } else { "s" },
                if docs_changed == 1 { "" } else { "s" },
            )
        };
        self.find_state.status = Some(status.clone());
        self.set_status(status);
        cx.notify();
    }

    // -- Dialog tab switching --------------------------------------------------------------

    /// Switch the dialog to `tab`. Refreshes find matches when arriving on
    /// Find or Replace so the highlight overlay tracks the active tab.
    pub(crate) fn set_dialog_tab(
        &mut self,
        tab: DialogTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_dialog_focus_handles(cx);
        // Switching tabs invalidates the open history dropdown — its anchor
        // input is no longer on screen.
        self.find_history_open = None;
        self.dialog_tab = tab;
        self.find_state.open = matches!(tab, DialogTab::Find | DialogTab::Replace);
        match tab {
            DialogTab::Find | DialogTab::Replace => {
                self.refresh_matches();
                window.focus(&self.find_focus, cx);
            }
            DialogTab::FindInFiles => {
                if self.find_in_files.follow_current_doc {
                    if let Some(p) = self
                        .buffer()
                        .file_path
                        .as_ref()
                        .and_then(|p| p.parent())
                        .map(|p| p.to_path_buf())
                    {
                        self.find_in_files.directory = Some(p.clone());
                        self.find_in_files.directory_text = p.display().to_string();
                        self.find_in_files.directory_text_cursor =
                            self.find_in_files.directory_text.len();
                    }
                }
                window.focus(&self.fif_query_focus, cx);
            }
            DialogTab::FindInProject => {
                window.focus(&self.fif_query_focus, cx);
            }
            DialogTab::Mark => {
                if let Some(h) = self.mark_focus.clone() {
                    window.focus(&h, cx);
                }
            }
        }
        cx.notify();
    }

    // -- Mark tab actions ------------------------------------------------------------------

    pub(crate) fn open_mark(
        &mut self,
        _: &SearchMark,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Pre-fill from selection like the other tabs do.
        if self.mark_state.query.is_empty() && !self.selection().is_empty() {
            let r = self.selection().ordered();
            let text = self.buffer().slice_bytes(r);
            if !text.is_empty() && !text.contains('\n') && text.len() < 256 {
                self.mark_state.query = text;
                self.mark_state.query_cursor = self.mark_state.query.len();
            }
        }
        open_or_focus_find_dialog(self, DialogTab::Mark, window, cx);
    }

    pub(crate) fn mark_toggle_case(
        &mut self,
        _: &MarkToggleCase,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mark_state.match_case = !self.mark_state.match_case;
        cx.notify();
    }

    pub(crate) fn mark_toggle_word(
        &mut self,
        _: &MarkToggleWord,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mark_state.whole_word = !self.mark_state.whole_word;
        cx.notify();
    }

    pub(crate) fn mark_toggle_regex(
        &mut self,
        _: &MarkToggleRegex,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mark_state.regex = !self.mark_state.regex;
        // Mirror Notepad++'s mutual exclusion — turning regex on clears
        // extended.
        if self.mark_state.regex {
            self.mark_state.extended = false;
        }
        cx.notify();
    }

    /// Toggle Mark tab's "Extended" radio. Mutually-exclusive with `regex`.
    pub(crate) fn mark_toggle_extended(
        &mut self,
        _: &MarkToggleExtended,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mark_state.extended = !self.mark_state.extended;
        if self.mark_state.extended {
            self.mark_state.regex = false;
        }
        cx.notify();
    }

    /// Toggle "Wrap around". When on, Mark All re-orders matches so the
    /// first hit returned is the next one after the caret line, wrapping
    /// to the buffer top.
    pub(crate) fn mark_toggle_wrap(
        &mut self,
        _: &MarkToggleWrap,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mark_state.wrap_around = !self.mark_state.wrap_around;
        cx.notify();
    }

    /// Toggle "Purge for each search". When on, Mark All clears the existing
    /// mark layer before adding the new run's hits. When off, new hits append
    /// to the existing marks.
    pub(crate) fn mark_toggle_purge(
        &mut self,
        _: &MarkTogglePurge,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mark_state.purge_for_each_search = !self.mark_state.purge_for_each_search;
        cx.notify();
    }

    pub(crate) fn mark_toggle_bookmark_line(
        &mut self,
        _: &MarkToggleBookmarkLine,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mark_state.bookmark_line = !self.mark_state.bookmark_line;
        cx.notify();
    }

    pub(crate) fn mark_all(&mut self, _: &MarkAll, _w: &mut Window, cx: &mut Context<Self>) {
        if self.mark_state.query.is_empty() {
            self.mark_state.status = Some("Enter a query first".to_string());
            self.mark_state.error = None;
            cx.notify();
            return;
        }
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::MarkQuery,
            &self.mark_state.query,
        );
        let flags = SearchFlags {
            regex: self.mark_state.regex,
            extended: self.mark_state.extended,
            whole_word: self.mark_state.whole_word,
            match_case: self.mark_state.match_case,
            dot_matches_newline: self.mark_state.dot_matches_newline,
        };
        let re = match search_regex(&self.mark_state.query, &flags) {
            Ok(r) => r,
            Err(e) => {
                self.mark_state.marks.clear();
                self.tabs[self.active].buffer.clear_marks();
                self.mark_state.error = Some(format!("Invalid regex: {e}"));
                self.mark_state.status = None;
                cx.notify();
                return;
            }
        };

        let buffer_text = self.tabs[self.active].buffer.rope.to_string();
        let mut hits: Vec<Range<usize>> = Vec::new();
        let mut bookmark_lines: Vec<usize> = Vec::new();
        for m in re.find_iter(&buffer_text) {
            if m.start() == m.end() {
                continue;
            }
            hits.push(m.start()..m.end());
            if self.mark_state.bookmark_line {
                let line = self.tabs[self.active].buffer.rope.byte_to_line(m.start());
                if !bookmark_lines.last().is_some_and(|&l| l == line) {
                    bookmark_lines.push(line);
                }
            }
        }
        let count = hits.len();

        // Apply "Wrap around" — re-order so the first hit listed is the next
        // match at-or-after the caret position, wrapping to the buffer start.
        // Notepad++'s wrap-around lights up the after-caret matches in the
        // same yellow on the same Mark All, so this is just visual ordering;
        // every match still gets a highlight.
        let ordered: Vec<Range<usize>> = if self.mark_state.wrap_around {
            let head = self.tabs[self.active].selection.head;
            let pivot = hits.iter().position(|r| r.start >= head).unwrap_or(0);
            hits.iter()
                .skip(pivot)
                .chain(hits.iter().take(pivot))
                .cloned()
                .collect()
        } else {
            hits
        };

        // Apply "Purge for each search" — when off, append to the existing
        // mark layer (deduped); when on, replace the layer entirely.
        if self.mark_state.purge_for_each_search {
            self.tabs[self.active].buffer.set_marks(ordered.clone());
        } else {
            for r in &ordered {
                self.tabs[self.active].buffer.add_mark(r.clone());
            }
        }
        // Mirror the buffer's mark layer into MarkState so Copy Marked Text
        // and the dialog status code path don't have to re-borrow the active
        // tab. Buffer.marks is the source of truth — this is a UI cache.
        self.mark_state.marks = self.tabs[self.active].buffer.marks.clone();
        self.mark_state.error = None;
        self.mark_state.status = Some(format!(
            "Marked {count} occurrence{}",
            if count == 1 { "" } else { "s" }
        ));
        if self.mark_state.bookmark_line {
            for line in bookmark_lines {
                let line_byte = self.tabs[self.active].buffer.rope.line_to_byte(line);
                let already = self.tabs[self.active].buffer.is_line_bookmarked(line);
                if !already {
                    self.tabs[self.active]
                        .buffer
                        .toggle_bookmark_at_byte(line_byte);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn mark_clear_all(
        &mut self,
        _: &MarkClearAll,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let removed = self
            .tabs[self.active]
            .buffer
            .marks
            .len()
            .max(self.mark_state.marks.len());
        self.tabs[self.active].buffer.clear_marks();
        self.mark_state.marks.clear();
        self.mark_state.status = Some(format!(
            "Cleared {removed} mark{}",
            if removed == 1 { "" } else { "s" }
        ));
        self.mark_state.error = None;
        cx.notify();
    }

    pub(crate) fn mark_copy(&mut self, _: &MarkCopy, _w: &mut Window, cx: &mut Context<Self>) {
        // Source-of-truth is the active buffer's `marks` layer — that's what
        // the editor paints. `mark_state.marks` is a cache that may go stale
        // after edits (the buffer's `shift_marks_for_edit` keeps Buffer.marks
        // in sync; MarkState mirrors only on the next Mark All run).
        let ranges: Vec<Range<usize>> = self.tabs[self.active].buffer.marks.clone();
        if ranges.is_empty() {
            self.mark_state.status = Some("No marks to copy".to_string());
            cx.notify();
            return;
        }
        let mut out = String::new();
        for r in &ranges {
            let text = self.buffer().slice_bytes(r.clone());
            out.push_str(&text);
            out.push('\n');
        }
        cx.write_to_clipboard(ClipboardItem::new_string(out));
        let count = ranges.len();
        self.mark_state.status = Some(format!(
            "Copied {count} marked text{} to clipboard",
            if count == 1 { "" } else { "s" }
        ));
        cx.notify();
    }
}

/// True when the active tab is a CSV file in Preview mode — used by
/// the global Cmd+F dispatch to choose between the inline CSV find
/// bar and the rope-text find dialog.
fn csv_preview_active(w: &Workspace) -> bool {
    let Some(tab) = w.tabs.get(w.active) else {
        return false;
    };
    matches!(tab.view_mode, crate::tab::TabViewMode::CsvPreview)
        && crate::ui::csv_doc::is_csv_path(tab.buffer.file_path.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(q: &str) -> regex::Regex {
        regex::Regex::new(&regex::escape(q)).unwrap()
    }

    #[test]
    fn scope_ending_mid_char_does_not_panic() {
        // Scope captured over "ab" (0..2), then "中" (3 bytes) was typed at
        // the start: the stale scope now ends inside the CJK char.
        let text = "中ab";
        let re = literal("a");
        let got = collect_matches(text, &re, Some(&(0..2)));
        assert!(got.iter().all(|r| text.is_char_boundary(r.start) && text.is_char_boundary(r.end)));
    }

    #[test]
    fn scope_starting_mid_char_does_not_panic() {
        let text = "a中b";
        let re = literal("b");
        let got = collect_matches(text, &re, Some(&(2..text.len())));
        assert_eq!(got, vec![4..5]);
    }

    #[test]
    fn step_target_steps_from_current() {
        let m = vec![0..1, 4..5, 8..9];
        assert_eq!(step_target(&m, Some(0), 0, true, false), Some(1));
        assert_eq!(step_target(&m, Some(2), 0, true, false), None);
        assert_eq!(step_target(&m, Some(2), 0, true, true), Some(0));
        assert_eq!(step_target(&m, Some(1), 0, false, false), Some(0));
        assert_eq!(step_target(&m, Some(0), 0, false, false), None);
        assert_eq!(step_target(&m, Some(0), 0, false, true), Some(2));
    }

    #[test]
    fn step_target_without_current_starts_at_caret() {
        let m = vec![0..1, 4..5, 8..9];
        assert_eq!(step_target(&m, None, 4, true, false), Some(1));
        assert_eq!(step_target(&m, None, 9, true, false), None);
        assert_eq!(step_target(&m, None, 9, true, true), Some(0));
        assert_eq!(step_target(&m, None, 5, false, false), Some(1));
        // Backward from document start: nothing before, wrap decides.
        assert_eq!(step_target(&m, None, 0, false, false), None);
        assert_eq!(step_target(&m, None, 0, false, true), Some(2));
    }

    #[test]
    fn step_target_ignores_out_of_range_current_and_empty_list() {
        let m = vec![0..1, 4..5];
        // A `current` left over from a longer list counts as no current.
        assert_eq!(step_target(&m, Some(7), 2, true, false), Some(1));
        assert_eq!(step_target(&[], Some(0), 0, true, true), None);
        assert_eq!(step_target(&[], None, 0, false, true), None);
    }

    #[test]
    fn stale_range_is_rejected_by_rope_guard() {
        // `jump_to_match` refuses ranges `get_byte_slice` rejects: a match
        // cached from another buffer ending inside "中" or past the end.
        let rope = ropey::Rope::from_str("abcd中");
        assert!(rope.get_byte_slice(4..6).is_none());
        assert!(rope.get_byte_slice(4..20).is_none());
        assert!(rope.get_byte_slice(4..7).is_some());
    }

    fn flags(regex: bool) -> SearchFlags {
        SearchFlags {
            regex,
            extended: false,
            whole_word: false,
            match_case: true,
            dot_matches_newline: false,
        }
    }

    fn assert_on_boundaries(text: &str, got: &[Range<usize>]) {
        for r in got {
            assert!(r.start < r.end && r.end <= text.len(), "{r:?} in {text:?}");
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end), "{r:?} in {text:?}");
        }
    }

    const EDGE_TEXTS: [&str; 6] = ["a\r\nb", "a\nb\n", "\n\n", "", "é中😀👨‍👩‍👧", "aé\r\n中a"];

    #[test]
    fn zero_width_patterns_yield_no_matches() {
        for pat in ["^", "$", r"\b", "", "(?m)^$", "x*"] {
            let re = search_regex(pat, &flags(true)).unwrap();
            for text in EDGE_TEXTS {
                assert!(collect_matches(text, &re, None).is_empty(), "{pat:?} on {text:?}");
            }
        }
    }

    #[test]
    fn regex_matches_land_on_char_boundaries() {
        for pat in ["a*", ".", ".+", "$\n", r"\w+", r"[^a]", r"\r?\n"] {
            let re = search_regex(pat, &flags(true)).unwrap();
            for text in EDGE_TEXTS {
                let got = collect_matches(text, &re, None);
                assert_on_boundaries(text, &got);
            }
        }
        // `.` matches one char at a time, the ZWJ family's parts included.
        let text = "é中😀👨‍👩‍👧";
        let re = search_regex(".", &flags(true)).unwrap();
        assert_eq!(collect_matches(text, &re, None).len(), text.chars().count());
        // `a*` on "a\r\nb" finds the "a" and skips the empty hits.
        let re = search_regex("a*", &flags(true)).unwrap();
        assert_eq!(collect_matches("a\r\nb", &re, None), vec![0..1]);
    }

    #[test]
    fn unsupported_regex_features_are_errors_not_panics() {
        for pat in ["(?=x)", "(?<=a)b", "(?!a)", r"(a)\1", "(", "[z-a]", "a{2,1}"] {
            assert!(search_regex(pat, &flags(true)).is_err(), "{pat:?}");
            // The same text is a plain literal outside regex mode.
            assert!(search_regex(pat, &flags(false)).is_ok(), "{pat:?}");
        }
    }

    #[test]
    fn oversized_regex_is_rejected_quickly() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(search_regex("(?:a{1000}){1000}", &flags(true)).is_err());
        });
        let rejected = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("timed out");
        assert!(rejected);
    }

    #[test]
    fn whole_word_respects_multibyte_word_chars() {
        let f = SearchFlags { whole_word: true, ..flags(false) };
        let re = search_regex("é", &f).unwrap();
        assert_eq!(collect_matches("café é", &re, None), vec![6..8]);
    }

    #[test]
    fn match_case_off_folds_accented_letters() {
        let off = SearchFlags { match_case: false, ..flags(false) };
        let re = search_regex("É", &off).unwrap();
        assert_eq!(collect_matches("é", &re, None), vec![0..2]);
        let re = search_regex("É", &flags(false)).unwrap();
        assert!(collect_matches("é", &re, None).is_empty());
    }

    #[test]
    fn malformed_extended_escapes_compile_and_search() {
        let ext = SearchFlags { extended: true, ..flags(false) };
        for q in [r"\x", r"\xZZ", r"\u12", r"\o999", r"\d999", r"\b2", "a\\"] {
            let re = search_regex(q, &ext).unwrap();
            for text in EDGE_TEXTS {
                assert_on_boundaries(text, &collect_matches(text, &re, None));
            }
        }
    }

    #[test]
    fn degenerate_scopes_and_empty_documents() {
        let re = literal("a");
        // Empty, reversed and past-the-end scopes.
        assert!(collect_matches("aaa", &re, Some(&(1..1))).is_empty());
        let reversed = Range { start: 3, end: 1 };
        assert!(collect_matches("aaa", &re, Some(&reversed)).is_empty());
        assert_eq!(collect_matches("aaa", &re, Some(&(2..100))), vec![2..3]);
        assert!(collect_matches("aaa", &re, Some(&(50..100))).is_empty());
        // Empty document.
        assert!(collect_matches("", &re, None).is_empty());
        assert!(collect_matches("", &re, Some(&(0..5))).is_empty());
        assert_eq!(step_target(&[], None, 0, true, true), None);
        // An empty query compiles to an empty pattern: no matches, no loop.
        let re = search_regex("", &flags(false)).unwrap();
        assert!(collect_matches("abc", &re, None).is_empty());
    }
}

#[cfg(test)]
mod smart_highlight_tests {
    use super::Workspace;
    use crate::tab::Tab;
    use gpui::{AppContext, TestAppContext, VisualContext, WindowAppearance};

    fn with_workspace(text: &str, test: impl FnOnce(&mut Workspace)) {
        let _settings = crate::settings::override_for_test(crate::settings::Settings {
            smart_highlight_enabled: true,
            ..crate::settings::Settings::default()
        });
        let mut cx = TestAppContext::single();
        let workspace = cx.new(|cx| {
            let mut w = Workspace::new(
                cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
                cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
                1., WindowAppearance::Dark,
            );
            let mut tab = Tab::empty();
            tab.buffer.rope = ropey::Rope::from_str(text);
            tab.buffer.dirty = false;
            w.tabs = vec![tab];
            w.active = 0;
            w.window_closed = true;
            w
        });
        cx.add_empty_window().update_window_entity(&workspace, |w, _, _| test(w));
        cx.quit();
    }

    #[test]
    fn cached_word_cannot_bypass_the_large_buffer_cutoff() {
        with_workspace("anchor anchor\r\ncafé 中😀", |w| {
            w.tabs[0].selection.collapse_to(2);
            w.refresh_smart_highlight();
            assert_eq!(w.smart_match_word.as_deref(), Some("anchor"));
            assert_eq!(w.smart_match_ranges.len(), 1);
            assert_eq!(w.smart_match_ranges.first(), Some(&(7..13)));
            // Keep the caret on the cached word while this physical line
            // grows beyond the limit. Even a cache hit must clear the band.
            w.tabs[0].buffer.rope.insert(0,
                &" ".repeat(Workspace::SMART_HILITE_BUFFER_LIMIT_BYTES));
            w.tabs[0].selection.collapse_to(Workspace::SMART_HILITE_BUFFER_LIMIT_BYTES + 2);
            w.refresh_smart_highlight();
            assert!(w.smart_match_ranges.is_empty());
            assert!(w.smart_match_word.is_none());
            assert!(w.tabs[0].undo_stack.is_empty());
            assert!(w.tabs[0].redo_stack.is_empty());
            assert!(!w.buffer().dirty);
        });
    }

    #[test]
    fn cutoff_includes_the_exact_limit_and_preserves_unicode_source() {
        let suffix = "café 中😀\r\nanchor anchor";
        let text = format!("{}{}",
            " ".repeat(Workspace::SMART_HILITE_BUFFER_LIMIT_BYTES - suffix.len()), suffix);
        with_workspace(&text, |w| {
            w.tabs[0].selection.collapse_to(text.len() - "anchor anchor".len() + 2);
            w.refresh_smart_highlight();
            assert_eq!(w.smart_match_word.as_deref(), Some("anchor"));
            assert_eq!(w.smart_match_ranges.len(), 1);
            assert_eq!(w.smart_match_ranges.first(), Some(&(text.len() - 6..text.len())));
            assert_eq!(w.buffer().rope.to_string(), text);
            assert!(!w.buffer().dirty);
            assert!(w.tabs[0].undo_stack.is_empty());
            assert!(w.tabs[0].redo_stack.is_empty());
        });
    }
}
