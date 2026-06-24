// Render — Workspace's top-level layout.

use gpui::{
    Context, ExternalPaths, Focusable, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::*, rems, rgb,
};

use crate::ui::about_dialog::about_dialog;
use crate::ui::autocomplete_popup::autocomplete_popup;
use crate::ui::chrome::{LoadingChip, status_bar};
use crate::ui::context_menu::{ContextMenuFlags, context_menu};
use crate::ui::tab_context_menu::{flags_for as tab_context_flags_for, tab_context_menu};
use crate::ui::rich_doc_color_bar::rich_doc_color_bar;
use crate::ui::rich_doc_link_bar::rich_doc_link_bar;
use crate::ui::tab_rename_bar::tab_rename_bar;
use crate::ui::toolbar::{ToolbarFlags, markdown_format_bar, rich_doc_format_bar, toolbar};
use crate::ui::editor_pane::editor_pane;
use crate::ui::markdown_view::{is_markdown_path, markdown_preview_pane};
use crate::ui::csv_doc::{
    CsvGridSnapshot, actions as csv_actions, csv_format_bar, csv_preview_pane, is_csv_path,
};
use crate::ui::find_in_files_panel::{FifPanelResizeHandle, find_in_files_panel};
use crate::ui::folder_panel::context_menu::folder_context_menu;
use crate::ui::folder_panel::folder_panel;
use crate::ui::goto_bar::goto_bar;
use crate::ui::macro_prompt_bar::macro_prompt_bar;
use crate::ui::tab_strip::{TabSummary, tab_strip, tab_strip_vertical};
use crate::ui::text_tools_preview_panel::{PreviewPaneResizeHandle, text_tools_preview_panel};
use crate::workspace::Workspace;

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let prefs = crate::settings::snapshot();
        // Sync the OS title bar with the active tab's path (or "new N" label).
        // NPP format: `*<path or label> - Notepad Studio`, asterisk only when dirty.
        // `MISC. ▸ Show only filename in title bar` swaps the full-path body
        // for the basename / "new N" label.
        {
            let active = &self.tabs[self.active];
            let prefix = if active.buffer.dirty { "*" } else { "" };
            let body = if prefs.title_bar_filename_only {
                active.buffer.display_short_name().to_string()
            } else {
                active.buffer.title_bar_label()
            };
            let next = format!("{prefix}{body} - {}", crate::consts::APP_NAME);
            if self.last_window_title.as_deref() != Some(next.as_str()) {
                window.set_window_title(&next);
                self.last_window_title = Some(next);
            }
        }
        let (line, col) = self.cursor_position();
        let buffer = self.buffer();
        let active_tab = &self.tabs[self.active];
        let active_is_markdown = is_markdown_path(active_tab.buffer.file_path.as_deref());
        let active_in_preview =
            matches!(active_tab.view_mode, crate::tab::TabViewMode::MarkdownPreview);
        let active_is_rich_doc = active_tab.buffer.is_rich_doc();
        let active_in_rich_doc_view =
            matches!(active_tab.view_mode, crate::tab::TabViewMode::RichDocView);
        let active_is_csv = is_csv_path(active_tab.buffer.file_path.as_deref());
        let active_in_csv_preview =
            matches!(active_tab.view_mode, crate::tab::TabViewMode::CsvPreview);
        let (
            rd_active_block,
            rd_bold,
            rd_italic,
            rd_underline,
            rd_strike,
            rd_color,
            rd_link,
            rd_alignment,
            rd_indent,
            rd_list,
            rd_heading_level,
        ) = active_tab
            .buffer
            .rich_doc
            .as_ref()
            .and_then(|p| {
                p.active_block.map(|b| {
                    let (alignment, indent) = p.block_props(b);
                    (
                        Some(b),
                        p.block_is_bold(b),
                        p.block_is_italic(b),
                        p.block_is_underline(b),
                        p.block_is_strike(b),
                        p.block_color(b),
                        p.block_link(b).is_some(),
                        alignment,
                        indent,
                        p.block_list(b),
                        p.block_heading_level(b),
                    )
                })
            })
            .unwrap_or((
                None,
                false,
                false,
                false,
                false,
                None,
                false,
                crate::rich_doc::ast::Alignment::Left,
                0,
                None,
                None,
            ));
        let toolbar_flags = ToolbarFlags {
            can_undo: !active_tab.undo_stack.is_empty(),
            can_redo: !active_tab.redo_stack.is_empty(),
            has_selection: !self.selection().is_empty(),
            active_dirty: active_tab.buffer.dirty,
            any_dirty: self.tabs.iter().any(|t| t.buffer.dirty),
            is_recording: crate::macros::is_recording(),
            has_recorded_macro: !crate::macros::current().is_empty(),
            word_wrap: self.word_wrap,
            show_whitespace: self.show_whitespace,
            is_markdown: active_is_markdown,
            markdown_preview_active: active_in_preview,
            is_csv: active_is_csv,
            csv_preview_active: active_in_csv_preview,
            is_rich_doc: active_is_rich_doc,
            rich_doc_view_active: active_in_rich_doc_view,
            rich_doc_has_active_block: rd_active_block.is_some(),
            rich_doc_block_bold: rd_bold,
            rich_doc_block_italic: rd_italic,
            rich_doc_block_underline: rd_underline,
            rich_doc_block_strike: rd_strike,
            rich_doc_block_color: rd_color,
            rich_doc_block_link: rd_link,
            rich_doc_block_alignment: rd_alignment,
            rich_doc_block_indent_level: rd_indent,
            rich_doc_block_list: rd_list,
            rich_doc_block_heading_level: rd_heading_level,
        };
        let encoding = buffer.encoding_label();
        let line_ending = buffer.line_ending_status_label();
        let syntax = buffer.syntax_label();
        let total_lines = buffer.rope.len_lines().max(1);

        let active = self.active;
        let tab_summaries: Vec<TabSummary> = self
            .tabs
            .iter()
            .map(|t| TabSummary {
                name: t.buffer.display_short_name(),
                dirty: t.buffer.dirty,
                color: t.buffer.tab_color,
            })
            .collect();
        let status_message = self.status_message.clone();
        let loading_chip = buffer.loading.as_ref().map(|p| LoadingChip {
            progress_text: crate::io_stream::format_progress(p),
            cancel: p.cancel.clone(),
        });

        div()
            .key_context("Workspace")
            .track_focus(&self.focus_handle(cx))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(Self::toggle_full_screen))
            .on_action(cx.listener(Self::toggle_show_whitespace))
            .on_action(cx.listener(Self::toggle_word_wrap))
            .on_action(cx.listener(Self::view_markdown_edit))
            .on_action(cx.listener(Self::view_markdown_preview))
            .on_action(cx.listener(Self::view_csv_source))
            .on_action(cx.listener(Self::view_csv_preview))
            .on_action(cx.listener(Self::view_csv_toggle_filter_row))
            .on_action(cx.listener(Self::view_source))
            .on_action(cx.listener(Self::view_preview))
            .on_action(cx.listener(Self::view_rich_doc))
            .on_action(cx.listener(Self::view_raw))
            .on_action(cx.listener(Self::rich_doc_bold))
            .on_action(cx.listener(Self::rich_doc_italic))
            .on_action(cx.listener(Self::rich_doc_underline))
            .on_action(cx.listener(Self::rich_doc_strikethrough))
            .on_action(cx.listener(Self::rich_doc_promote_heading))
            .on_action(cx.listener(Self::rich_doc_demote_heading))
            .on_action(cx.listener(Self::rich_doc_save_as_docx))
            .on_action(cx.listener(Self::md_bold))
            .on_action(cx.listener(Self::md_italic))
            .on_action(cx.listener(Self::md_strikethrough))
            .on_action(cx.listener(Self::md_inline_code))
            .on_action(cx.listener(Self::md_heading_1))
            .on_action(cx.listener(Self::md_heading_2))
            .on_action(cx.listener(Self::md_heading_3))
            .on_action(cx.listener(Self::md_bullet_list))
            .on_action(cx.listener(Self::md_ordered_list_action))
            .on_action(cx.listener(Self::md_task_list))
            .on_action(cx.listener(Self::md_block_quote))
            .on_action(cx.listener(Self::md_code_block_action))
            .on_action(cx.listener(Self::md_link))
            .on_action(cx.listener(Self::md_image))
            .on_action(cx.listener(Self::md_horizontal_rule_action))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::insert_newline))
            .on_action(cx.listener(Self::insert_tab))
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::move_line_start))
            .on_action(cx.listener(Self::move_line_end))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::move_doc_start))
            .on_action(cx.listener(Self::move_doc_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::move_word_left))
            .on_action(cx.listener(Self::move_word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::block_extend_up))
            .on_action(cx.listener(Self::block_extend_down))
            .on_action(cx.listener(Self::block_extend_left))
            .on_action(cx.listener(Self::block_extend_right))
            .on_action(cx.listener(Self::open_new_tab))
            .on_action(cx.listener(Self::open_dialog))
            .on_action(cx.listener(Self::open_folder_as_workspace))
            .on_action(cx.listener(Self::toggle_folder_panel))
            .on_action(cx.listener(Self::save_active))
            .on_action(cx.listener(Self::save_active_as))
            .on_action(cx.listener(Self::save_all))
            .on_action(cx.listener(Self::close_active_tab))
            .on_action(cx.listener(Self::close_all_tabs))
            .on_action(cx.listener(Self::restore_closed_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::next_tab_sequential))
            .on_action(cx.listener(Self::prev_tab_sequential))
            .on_action(cx.listener(Self::goto_tab_1))
            .on_action(cx.listener(Self::goto_tab_2))
            .on_action(cx.listener(Self::goto_tab_3))
            .on_action(cx.listener(Self::goto_tab_4))
            .on_action(cx.listener(Self::goto_tab_5))
            .on_action(cx.listener(Self::goto_tab_6))
            .on_action(cx.listener(Self::goto_tab_7))
            .on_action(cx.listener(Self::goto_tab_8))
            .on_action(cx.listener(Self::goto_tab_9))
            // Open the find dialog from the editor side. Each chord lands on
            // the matching dialog tab.
            .on_action(cx.listener(Self::open_find))
            .on_action(cx.listener(Self::close_find))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_prev))
            .on_action(cx.listener(Self::find_count))
            .on_action(cx.listener(Self::find_all_current))
            .on_action(cx.listener(Self::toggle_replace_bar))
            .on_action(cx.listener(Self::toggle_find_case))
            .on_action(cx.listener(Self::toggle_find_word))
            .on_action(cx.listener(Self::toggle_find_regex))
            .on_action(cx.listener(Self::toggle_find_extended))
            .on_action(cx.listener(Self::toggle_find_wrap))
            .on_action(cx.listener(Self::toggle_replace_visibility))
            .on_action(cx.listener(Self::replace_current_match))
            .on_action(cx.listener(Self::replace_all_matches))
            .on_action(cx.listener(Self::open_goto_line))
            .on_action(cx.listener(Self::duplicate_line))
            .on_action(cx.listener(Self::fill_right))
            .on_action(cx.listener(Self::insert_today_date))
            .on_action(cx.listener(Self::insert_now_time))
            .on_action(cx.listener(Self::move_line_up))
            .on_action(cx.listener(Self::move_line_down))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(Self::uppercase_selection))
            .on_action(cx.listener(Self::lowercase_selection))
            .on_action(cx.listener(Self::block_comment))
            .on_action(cx.listener(Self::stream_comment))
            .on_action(cx.listener(Self::trim_trailing_ws))
            .on_action(cx.listener(Self::eol_windows))
            .on_action(cx.listener(Self::eol_unix))
            .on_action(cx.listener(Self::eol_macintosh))
            .on_action(cx.listener(Self::eol_cycle))
            .on_action(cx.listener(Self::encoding_utf8))
            .on_action(cx.listener(Self::encoding_utf8_bom))
            .on_action(cx.listener(Self::encoding_utf16_le))
            .on_action(cx.listener(Self::encoding_utf16_be))
            .on_action(cx.listener(Self::encoding_latin1))
            .on_action(cx.listener(Self::encoding_cycle))
            .on_action(cx.listener(Self::insert_blank_line_above))
            .on_action(cx.listener(Self::insert_blank_line_below))
            // Auto-Completion menu actions. Word does the buffer scan;
            // Function / Function-and-Word delegate to Word; Function
            // Parameters Hint surfaces the registry-not-loaded status.
            .on_action(cx.listener(Self::autocomplete_word))
            .on_action(cx.listener(Self::autocomplete_function))
            .on_action(cx.listener(Self::autocomplete_function_and_word))
            .on_action(cx.listener(Self::autocomplete_function_params))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::toggle_bookmark))
            .on_action(cx.listener(Self::goto_next_bookmark))
            .on_action(cx.listener(Self::goto_prev_bookmark))
            .on_action(cx.listener(Self::clear_all_bookmarks))
            .on_action(cx.listener(Self::open_find_in_files))
            .on_action(cx.listener(Self::close_find_in_files))
            .on_action(cx.listener(Self::find_in_files_run_action))
            .on_action(cx.listener(Self::find_in_files_replace))
            .on_action(cx.listener(Self::find_in_files_pick_dir))
            .on_action(cx.listener(Self::find_in_files_toggle_case))
            .on_action(cx.listener(Self::find_in_files_toggle_word))
            .on_action(cx.listener(Self::find_in_files_toggle_regex))
            .on_action(cx.listener(Self::find_in_files_toggle_extended))
            .on_action(cx.listener(Self::find_in_files_toggle_follow))
            .on_action(cx.listener(Self::find_in_files_toggle_subfolders))
            .on_action(cx.listener(Self::find_in_files_toggle_hidden))
            .on_action(cx.listener(Self::find_in_files_swap))
            .on_action(cx.listener(Self::find_in_files_toggle_transparency))
            .on_action(cx.listener(Self::find_in_files_transparency_on_focus_loss))
            .on_action(cx.listener(Self::find_in_files_transparency_always))
            .on_action(cx.listener(Self::find_in_files_transparency_dec))
            .on_action(cx.listener(Self::find_in_files_transparency_inc))
            .on_action(cx.listener(Self::open_find_in_projects))
            .on_action(cx.listener(Self::find_in_projects_run_action))
            .on_action(cx.listener(Self::find_in_projects_replace))
            .on_action(cx.listener(Self::find_in_projects_swap))
            .on_action(cx.listener(Self::find_in_projects_toggle_panel_1))
            .on_action(cx.listener(Self::find_in_projects_toggle_panel_2))
            .on_action(cx.listener(Self::find_in_projects_toggle_panel_3))
            // Drop-to-open: drag files from Finder / Explorer onto the
            // window and they land as tabs. gpui translates the platform's
            // FileDropEvent into an ExternalPaths active-drag, so this
            // listener fires when the user releases over any non-occluding
            // surface in the workspace tree.
            .on_drop(cx.listener(
                |this, paths: &ExternalPaths, window, cx| {
                    let paths = paths.paths().to_vec();
                    this.handle_external_paths_drop(paths, window, cx);
                },
            ))
            // Mark dialog tab.
            .on_action(cx.listener(Self::open_mark))
            .on_action(cx.listener(Self::mark_all))
            .on_action(cx.listener(Self::mark_clear_all))
            .on_action(cx.listener(Self::mark_copy))
            .on_action(cx.listener(Self::mark_toggle_case))
            .on_action(cx.listener(Self::mark_toggle_word))
            .on_action(cx.listener(Self::mark_toggle_regex))
            .on_action(cx.listener(Self::mark_toggle_extended))
            .on_action(cx.listener(Self::mark_toggle_wrap))
            .on_action(cx.listener(Self::mark_toggle_purge))
            .on_action(cx.listener(Self::mark_toggle_bookmark_line))
            // Notepad++ Edit submenus — most are still stubs, wired so the
            // menu items fire and emit a stub log line. See
            // `src/edit_ops/*.rs` for the per-submenu impl files. Read-Only
            // toggle / clear are fully implemented and route to the existing
            // tab-level plumbing.
            .on_action(cx.listener(Self::insert_datetime_short))
            .on_action(cx.listener(Self::insert_datetime_long))
            .on_action(cx.listener(Self::insert_datetime_customized))
            .on_action(cx.listener(Self::copy_full_path))
            .on_action(cx.listener(Self::copy_filename))
            .on_action(cx.listener(Self::copy_dir_path))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::proper_case_force))
            .on_action(cx.listener(Self::proper_case_blend))
            .on_action(cx.listener(Self::sentence_case_force))
            .on_action(cx.listener(Self::sentence_case_blend))
            .on_action(cx.listener(Self::invert_case))
            .on_action(cx.listener(Self::random_case))
            .on_action(cx.listener(Self::remove_duplicate_lines))
            .on_action(cx.listener(Self::remove_consecutive_duplicate_lines))
            .on_action(cx.listener(Self::split_lines))
            .on_action(cx.listener(Self::join_lines))
            .on_action(cx.listener(Self::remove_empty_lines))
            .on_action(cx.listener(Self::remove_empty_lines_containing_blank))
            .on_action(cx.listener(Self::sort_lines_lex_asc))
            .on_action(cx.listener(Self::sort_lines_lex_desc))
            .on_action(cx.listener(Self::sort_lines_int_asc))
            .on_action(cx.listener(Self::sort_lines_int_desc))
            .on_action(cx.listener(Self::sort_lines_dec_comma_asc))
            .on_action(cx.listener(Self::sort_lines_dec_comma_desc))
            .on_action(cx.listener(Self::sort_lines_dec_dot_asc))
            .on_action(cx.listener(Self::sort_lines_dec_dot_desc))
            .on_action(cx.listener(Self::reverse_line_order))
            .on_action(cx.listener(Self::randomize_line_order))
            .on_action(cx.listener(Self::trim_leading_ws))
            .on_action(cx.listener(Self::trim_leading_and_trailing_ws))
            .on_action(cx.listener(Self::eol_to_space))
            .on_action(cx.listener(Self::remove_unnecessary_blank_and_eol))
            .on_action(cx.listener(Self::tab_to_space))
            .on_action(cx.listener(Self::space_to_tab_all))
            .on_action(cx.listener(Self::space_to_tab_leading))
            .on_action(cx.listener(Self::open_selected_file))
            .on_action(cx.listener(Self::open_selected_in_new_instance))
            .on_action(cx.listener(Self::open_containing_folder))
            .on_action(cx.listener(Self::open_containing_folder_in_terminal))
            .on_action(cx.listener(Self::search_selection_on_internet))
            .on_action(cx.listener(Self::send_selection_via_email))
            .on_action(cx.listener(Self::begin_end_select))
            .on_action(cx.listener(Self::begin_end_select_column_mode))
            .on_action(cx.listener(Self::edit_toggle_read_only))
            .on_action(cx.listener(Self::edit_clear_read_only_flag))
            // NPP Comment/Uncomment block + stream uncomment / set variants —
            // toggle versions exist in `workspace::edits`; the directional
            // (set / uncomment) ops are stubs in this round.
            .on_action(cx.listener(Self::block_comment_set))
            .on_action(cx.listener(Self::block_uncomment))
            .on_action(cx.listener(Self::stream_uncomment))
            // Macros.
            .on_action(cx.listener(Self::macro_start_stop_recording))
            .on_action(cx.listener(Self::macro_start_recording))
            .on_action(cx.listener(Self::macro_stop_recording))
            .on_action(cx.listener(Self::macro_playback))
            .on_action(cx.listener(Self::macro_run_multiple_times))
            .on_action(cx.listener(Self::macro_save_current))
            .on_action(cx.listener(Self::macro_run_saved))
            // Plugins.
            .on_action(cx.listener(Self::plugin_run_command))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.text))
            .font_family("Menlo")
            // Editor body sizes are in rems so window-level rem_size changes
            // (zoom) flow through automatically.
            .text_size(rems(13.5 / 16.))
            .line_height(rems(20. / 16.))
            .when(!prefs.toolbar_hidden, |this| {
                this.child(toolbar(&theme, toolbar_flags, cx.entity().clone()))
            })
            // Secondary markdown formatting bar — only when the active tab
            // is a markdown file viewed in preview. Lives below the main
            // toolbar so the formatting controls float as their own row.
            .when(active_is_markdown && active_in_preview, |this| {
                this.child(markdown_format_bar(&theme, cx.entity().clone()))
            })
            // Secondary CSV format bar — header toggle, delimiter buttons,
            // add row/column. Only when the active tab is a CSV in
            // preview mode.
            .when(active_is_csv && active_in_csv_preview, |this| {
                let delimiter = self
                    .csv_models
                    .get(&self.tabs[self.active].session_id)
                    .map(|m| m.delimiter)
                    .unwrap_or(b',');
                this.child(csv_format_bar(
                    &theme,
                    cx.entity().clone(),
                    delimiter,
                ))
            })
            // Secondary rich-doc formatting bar — Doc View only. Buttons
            // gate on active-block selection so they grey out until the
            // user clicks into a paragraph.
            .when(active_is_rich_doc && active_in_rich_doc_view, |this| {
                this.child(rich_doc_format_bar(
                    &theme,
                    toolbar_flags,
                    cx.entity().clone(),
                ))
            })
            // Horizontal tab strip — skipped when the tab bar is hidden OR
            // when `tab_vertical` is on (vertical strip is mounted inside
            // the body row below).
            .when(!prefs.tab_bar_hidden && !prefs.tab_vertical, |this| {
                this.child(tab_strip(&theme, &tab_summaries, active, cx.entity().clone()))
            })
            // Goto-line stays inline above the editor — the spec explicitly
            // allows keeping Goto inline rather than folding it into the
            // dialog. Find / Replace / Find-in-Files / Mark all moved to the
            // dialog window in `ui::find_dialog`; the inline goto bar is a
            // tiny single-purpose surface that's clearer where it is.
            .when(self.goto_state.open, |this| {
                this.child(goto_bar(&theme, &self.goto_state, cx.entity().clone()))
            })
            .when_some(self.macro_prompt.as_ref(), |this, state| {
                this.child(macro_prompt_bar(&theme, state, cx.entity().clone()))
            })
            .when_some(self.tab_rename.as_ref(), |this, state| {
                this.child(tab_rename_bar(&theme, state, cx.entity().clone()))
            })
            .when_some(self.rich_doc_link_bar.as_ref(), |this, state| {
                this.child(rich_doc_link_bar(&theme, state, cx.entity().clone()))
            })
            .when_some(self.rich_doc_color_bar.as_ref(), |this, state| {
                this.child(rich_doc_color_bar(&theme, state, cx.entity().clone()))
            })
            // Body row — horizontal split: [folder_panel?] | editor.
            // `flex_grow + min_h(0)` so the row consumes the remaining
            // vertical space between the bars above and the preview /
            // status bar below; `min_w(0)` on the editor wrapper so it
            // shrinks instead of inflating to its longest line and pushing
            // the panel off-screen on narrow windows.
            .child({
                let body = if active_is_rich_doc && active_in_rich_doc_view {
                    let body_focus = self.ensure_rich_doc_body_focus(cx);
                    // Per-block find-match highlights — empty unless the find
                    // dialog is open with matches sourced from the rich-doc
                    // flat text. Computed here so the rich_doc_view module
                    // doesn't reach into find_state directly.
                    let mut match_highlights: std::collections::HashMap<usize, Vec<(usize, usize, bool)>> =
                        std::collections::HashMap::new();
                    if self.find_state.open {
                        if let (Some(offsets), Some(payload)) = (
                            self.find_state.rich_doc_block_offsets.as_ref(),
                            self.tabs[self.active].buffer.rich_doc.as_ref(),
                        ) {
                            let per_block = payload.matches_per_block(offsets, &self.find_state.matches);
                            let current = self.find_state.current;
                            for (block_idx, lo, hi, mi) in per_block {
                                let is_current = current == Some(mi);
                                match_highlights
                                    .entry(block_idx)
                                    .or_insert_with(Vec::new)
                                    .push((lo, hi, is_current));
                            }
                        }
                    }
                    crate::ui::rich_doc_view::rich_doc_pane(
                        &theme,
                        &self.tabs[self.active],
                        cx.entity().clone(),
                        body_focus,
                        match_highlights,
                        self.rich_doc_scroll_handle.clone(),
                    )
                    .into_any_element()
                } else if active_in_csv_preview && active_is_csv {
                    csv_actions::ensure_model(self, window, cx);
                    let tab_id = self.tabs[self.active].session_id;
                    if let Some(model) = self.csv_models.get(&tab_id) {
                        let snapshot = CsvGridSnapshot::from_model(model);
                        csv_preview_pane(&theme, &snapshot, cx.entity().clone())
                            .into_any_element()
                    } else {
                        editor_pane(&theme, total_lines, cx.entity().clone())
                            .into_any_element()
                    }
                } else if active_in_preview && active_is_markdown {
                    let source = self.tabs[self.active].buffer.rope.to_string();
                    let sel = self.selection();
                    let sel_range = if sel.is_empty() {
                        None
                    } else {
                        Some(sel.ordered())
                    };
                    // Caret position is always passed — even when the
                    // selection is empty — so the inline marker-hiding
                    // logic in markdown_view can reveal `**` / `*` /
                    // `~~` / `` ` `` markers as the caret enters their
                    // styled region.
                    let caret = Some(sel.head);
                    // Reset the per-block hit-target registry; each block's
                    // CaretOverlay::paint will push itself back in as the
                    // tree paints. The window-level mouse listeners read
                    // this list to extend drags across block gaps and start
                    // drags from dead space (right margin, between blocks,
                    // scroll-past-end spacer).
                    self.preview_block_targets.clear();
                    let doc_dir = active_tab
                        .buffer
                        .file_path
                        .as_deref()
                        .and_then(|p| p.parent())
                        .map(|p| p.to_path_buf());
                    markdown_preview_pane(
                        &theme,
                        &source,
                        cx.entity().clone(),
                        sel_range,
                        caret,
                        self.md_preview_scroll_handle.clone(),
                        doc_dir,
                    )
                    .into_any_element()
                } else {
                    editor_pane(&theme, total_lines, cx.entity().clone()).into_any_element()
                };
                let mut row = div().flex().flex_row().flex_grow().min_h(rems(0.));
                if prefs.folder_panel_visible {
                    let selected = self.folder_panel.selected.clone();
                    let inline_create = self
                        .folder_inline_create
                        .as_ref()
                        .zip(self.folder_inline_create_focus.clone())
                        .map(|(draft, focus)| {
                            crate::ui::folder_panel::tree::InlineCreateSnapshot {
                                parent_dir: draft.parent_dir.clone(),
                                kind: draft.kind,
                                focus,
                                error: draft.error.clone(),
                            }
                        });
                    row = row.child(folder_panel(
                        &theme,
                        cx.entity().clone(),
                        prefs.folder_panel_width,
                        selected,
                        inline_create,
                    ));
                }
                // Vertical tab strip sits between the folder panel and the
                // editor pane. Same gate as the horizontal strip — hidden
                // when `tab_bar_hidden` is set, regardless of orientation.
                if prefs.tab_vertical && !prefs.tab_bar_hidden {
                    row = row.child(tab_strip_vertical(
                        &theme,
                        &tab_summaries,
                        active,
                        cx.entity().clone(),
                    ));
                }
                row.child(
                    div()
                        .flex_grow()
                        .min_w(rems(0.))
                        .min_h(rems(0.))
                        .flex()
                        .flex_col()
                        .child(body),
                )
            })
            .when_some(self.preview_pane.as_ref(), |this, state| {
                let pane_height = self.preview_pane_height;
                this.child(PreviewPaneResizeHandle {
                    workspace: cx.entity().downgrade(),
                    theme_chrome: theme.chrome,
                    theme_border: theme.border,
                })
                .child(text_tools_preview_panel(
                    &theme,
                    state,
                    cx.entity().clone(),
                    pane_height,
                ))
            })
            .when(self.find_in_files.open, |this| {
                let panel_height = self.fif_panel_height;
                this.child(FifPanelResizeHandle {
                    workspace: cx.entity().downgrade(),
                    theme_chrome: theme.chrome,
                    theme_border: theme.border,
                })
                .child(find_in_files_panel(
                    &theme,
                    &self.find_in_files,
                    cx.entity().clone(),
                    panel_height,
                ))
            })
            // NPP `General ▸ Status Bar ▸ Hide` — when set, the status bar
            // is omitted entirely. Settings live behind a process-global
            // mutex; the Preferences dialog calls `nudge_workspaces` after
            // toggling so this branch re-evaluates next paint.
            .when(!prefs.status_bar_hidden, |this| {
                this.child(status_bar(
                    &theme,
                    line,
                    col,
                    encoding,
                    line_ending,
                    syntax,
                    status_message.as_deref(),
                    loading_chip,
                    cx.entity().clone(),
                ))
            })
            .when_some(self.context_menu, |this, position| {
                let active_tab = &self.tabs[self.active];
                let flags = ContextMenuFlags {
                    can_undo: !active_tab.undo_stack.is_empty(),
                    can_redo: !active_tab.redo_stack.is_empty(),
                    has_selection: !self.selection().is_empty(),
                };
                this.child(context_menu(&theme, position, flags, cx.entity().clone()))
            })
            .when_some(self.tab_context_menu, |this, state| {
                // Pre-compute flags from `self` so the menu doesn't double-
                // lease the entity. If the target tab vanished (race with
                // close), drop the menu silently — next click dismisses.
                match tab_context_flags_for(self, state.target_tab_idx) {
                    Some(flags) => this.child(tab_context_menu(
                        &theme,
                        state,
                        flags,
                        cx.entity().clone(),
                    )),
                    None => this,
                }
            })
            .when(self.about_dialog_open, |this| {
                this.child(about_dialog(&theme, cx.entity().clone()))
            })
            .when(self.paste_format_pending.is_some(), |this| {
                this.child(crate::ui::paste_format_dialog::paste_format_dialog(
                    &theme,
                    cx.entity().clone(),
                ))
            })
            .when_some(self.folder_panel.context_menu.clone(), |this, state| {
                this.child(folder_context_menu(&theme, &state, cx.entity().clone()))
            })
            .when_some(self.csv_context_menu, |this, state| {
                this.child(crate::ui::csv_doc::csv_context_menu(
                    &theme,
                    state,
                    cx.entity().clone(),
                ))
            })
            .when_some(self.csv_delimiter_menu, |this, pos| {
                let delimiter = self
                    .csv_models
                    .get(&self.tabs[self.active].session_id)
                    .map(|m| m.delimiter)
                    .unwrap_or(b',');
                this.child(crate::ui::csv_doc::csv_delimiter_menu(
                    &theme,
                    pos,
                    delimiter,
                    cx.entity().clone(),
                ))
            })
            .when_some(
                {
                    let tab_id = self
                        .tabs
                        .get(self.active)
                        .map(|t| t.session_id);
                    tab_id.and_then(|id| self.csv_models.get(&id)).and_then(
                        crate::ui::csv_doc::FilterDropdownSnapshot::from_model,
                    )
                },
                |this, snap| {
                    this.child(crate::ui::csv_doc::csv_filter_dropdown(
                        &theme,
                        snap,
                        cx.entity().clone(),
                    ))
                },
            )
            .when_some(self.autocomplete.as_ref().cloned(), |this, state| {
                // Auto-Completion popup mounts last so it sits above the
                // context menus / modals — its backdrop captures the next
                // click anywhere outside the candidate list and dismisses.
                this.child(autocomplete_popup(
                    &theme,
                    state.anchor,
                    &state.candidates,
                    state.selected,
                    cx.entity().clone(),
                ))
            })
    }
}
