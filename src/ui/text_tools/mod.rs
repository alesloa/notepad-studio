// Text Tools dialog — Edit ▸ Text Tools…
//
// A floating window with a landing card grid (8 transform tools) and
// per-tool views holding the transform's parameter controls AND a
// multi-line source text area where the user pastes/types the input.
// Apply replaces the active tab's content with the result. If no active
// tab exists, falls back to opening a new untitled tab.
//
// Single-line fields use `FieldId` + `Fields` (prefix, suffix,
// delimiter, columns, pattern, join prefix, join delimiters).
// Multi-line fields use `MultiFieldId` + `MultiFields` (the source
// "Input" text, plus auxiliary surfaces: List B for Lists Comparison
// and JoinCol2..4 for Join/Merge).

mod add_prefix_suffix;
mod column_extractor;
mod eol;
mod join_merge;
mod lists_comparison;
mod remove_duplicates;
mod remove_empty_lines;
mod remove_lines_containing;
mod reverse_text;
mod widgets;

use std::cell::RefCell;
use std::ops::Range;

use gpui::{
    App, Bounds, Context, CursorStyle, Entity, EntityInputHandler, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render, SharedString, Styled,
    TitlebarOptions, UTF16Selection, WeakEntity, Window, WindowAppearance, WindowHandle,
    WindowKind, WindowOptions, div, point, prelude::*, px, rems, rgb, size,
};

use crate::actions::{
    EditCopy, EditCut, EditPaste, EditSelectAll, ViewZoomIn, ViewZoomOut, ViewZoomReset,
};
use crate::consts::{ZOOM_DEFAULT, ZOOM_REM_BASE};
use crate::edit_ops::util::floor_str_boundary;
use crate::text::utf16::{
    input_selection_range_utf16, replace_input_utf16, slice_str_chars_utf16,
};
use crate::theme::{Theme, ThemeMode};
use crate::workspace::Workspace;

use lists_comparison::CompareResult;
use widgets::{caption_text, label_text, npp_button, npp_button_primary};

thread_local! {
    static OPEN_WINDOW: RefCell<Option<WindowHandle<TextToolsDialog>>> =
        const { RefCell::new(None) };
}

// --- tool + field ids ----------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Tool {
    AddPrefixSuffix,
    JoinMerge,
    ColumnExtractor,
    RemoveDuplicates,
    RemoveEmptyLines,
    RemoveLinesContaining,
    ListsComparison,
    ReverseText,
}

impl Tool {
    fn title(self) -> &'static str {
        match self {
            Tool::AddPrefixSuffix => "Add Prefix/Suffix",
            Tool::JoinMerge => "Join/Merge Text",
            Tool::ColumnExtractor => "Column Extractor",
            Tool::RemoveDuplicates => "Remove Duplicates",
            Tool::RemoveEmptyLines => "Remove Empty Lines",
            Tool::RemoveLinesContaining => "Remove Lines Containing",
            Tool::ListsComparison => "Lists Comparison",
            Tool::ReverseText => "Reverse Lines",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Tool::AddPrefixSuffix => "Wrap each line.",
            Tool::JoinMerge => "Row-merge columns.",
            Tool::ColumnExtractor => "Pick delimited cols.",
            Tool::RemoveDuplicates => "Drop dup lines.",
            Tool::RemoveEmptyLines => "Drop blank lines.",
            Tool::RemoveLinesContaining => "Drop matching lines.",
            Tool::ListsComparison => "Diff/intersect lists.",
            Tool::ReverseText => "Flip line order.",
        }
    }

    fn all() -> [Tool; 8] {
        [
            Tool::AddPrefixSuffix,
            Tool::JoinMerge,
            Tool::ColumnExtractor,
            Tool::RemoveDuplicates,
            Tool::RemoveEmptyLines,
            Tool::RemoveLinesContaining,
            Tool::ListsComparison,
            Tool::ReverseText,
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FieldId {
    Prefix,
    Suffix,
    Delimiter,
    Columns,
    Pattern,
    JoinPrefix,
    JoinDelim1,
    JoinDelim2,
    JoinDelim3,
}

impl FieldId {
    fn all() -> [FieldId; 9] {
        [
            FieldId::Prefix,
            FieldId::Suffix,
            FieldId::Delimiter,
            FieldId::Columns,
            FieldId::Pattern,
            FieldId::JoinPrefix,
            FieldId::JoinDelim1,
            FieldId::JoinDelim2,
            FieldId::JoinDelim3,
        ]
    }

    pub(super) fn container_id(self) -> &'static str {
        match self {
            FieldId::Prefix => "tt-input-prefix",
            FieldId::Suffix => "tt-input-suffix",
            FieldId::Delimiter => "tt-input-delimiter",
            FieldId::Columns => "tt-input-columns",
            FieldId::Pattern => "tt-input-pattern",
            FieldId::JoinPrefix => "tt-input-join-prefix",
            FieldId::JoinDelim1 => "tt-input-join-delim1",
            FieldId::JoinDelim2 => "tt-input-join-delim2",
            FieldId::JoinDelim3 => "tt-input-join-delim3",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum MultiFieldId {
    InputText,
    ListB,
    JoinCol2,
    JoinCol3,
    JoinCol4,
}

impl MultiFieldId {
    fn all() -> [MultiFieldId; 5] {
        [
            MultiFieldId::InputText,
            MultiFieldId::ListB,
            MultiFieldId::JoinCol2,
            MultiFieldId::JoinCol3,
            MultiFieldId::JoinCol4,
        ]
    }

    pub(super) fn container_id(self) -> &'static str {
        match self {
            MultiFieldId::InputText => "tt-multi-input-text",
            MultiFieldId::ListB => "tt-multi-list-b",
            MultiFieldId::JoinCol2 => "tt-multi-join-col2",
            MultiFieldId::JoinCol3 => "tt-multi-join-col3",
            MultiFieldId::JoinCol4 => "tt-multi-join-col4",
        }
    }
}

/// Text, caret and anchor of one input. The caret and anchor are byte
/// offsets into `text`. Edits keep them on char boundaries, and every read
/// clamps and floors them again, since a slice or `replace_range` off a
/// boundary panics.
#[derive(Default, Clone)]
pub(super) struct FieldState {
    pub(super) text: String,
    pub(super) cursor: usize,
    pub(super) anchor: usize,
}

impl FieldState {
    pub(super) fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    pub(super) fn collapse_to(&mut self, byte: usize) {
        let b = floor_str_boundary(&self.text, byte);
        self.cursor = b;
        self.anchor = b;
    }

    /// The caret, clamped and floored to a char boundary.
    fn caret(&self) -> usize {
        floor_str_boundary(&self.text, self.cursor)
    }

    /// The selection, ordered, clamped and floored to char boundaries.
    fn ordered(&self) -> Range<usize> {
        let a = self.caret();
        let b = floor_str_boundary(&self.text, self.anchor);
        a.min(b)..a.max(b)
    }

    fn selected(&self) -> &str {
        self.text.get(self.ordered()).unwrap_or("")
    }

    /// Replace the selection with `new` and put the caret after it.
    fn replace_selection(&mut self, new: &str) {
        let r = self.ordered();
        self.text.replace_range(r.clone(), new);
        self.collapse_to(r.start.saturating_add(new.len()));
    }

    /// Cut: remove the selection and return it; `None` when it is empty.
    fn take_selection(&mut self) -> Option<String> {
        let sel = self.selected().to_string();
        if sel.is_empty() {
            return None;
        }
        self.replace_selection("");
        Some(sel)
    }

    /// Backspace: delete the selection, or the char before the caret.
    fn delete_backward(&mut self) {
        let r = self.ordered();
        if r.is_empty() {
            self.anchor = prev_grapheme(&self.text, r.start);
            self.cursor = r.start;
        }
        self.replace_selection("");
    }

    /// Delete: delete the selection, or the char after the caret.
    fn delete_forward(&mut self) {
        let r = self.ordered();
        if r.is_empty() {
            self.anchor = next_grapheme(&self.text, r.start);
            self.cursor = r.start;
        }
        self.replace_selection("");
    }
}

/// Tagged focus target — either a single-line field or a multi-line textarea.
/// Used by `focus_order()` to express each tool's Tab cycle as one flat list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FocusTarget {
    Single(FieldId),
    Multi(MultiFieldId),
}

#[derive(Default, Clone)]
pub(super) struct Fields {
    pub(super) prefix: FieldState,
    pub(super) suffix: FieldState,
    pub(super) delimiter: FieldState,
    pub(super) columns: FieldState,
    pub(super) pattern: FieldState,
    pub(super) join_prefix: FieldState,
    pub(super) join_delim1: FieldState,
    pub(super) join_delim2: FieldState,
    pub(super) join_delim3: FieldState,
}

impl Fields {
    pub(super) fn get(&self, id: FieldId) -> &FieldState {
        match id {
            FieldId::Prefix => &self.prefix,
            FieldId::Suffix => &self.suffix,
            FieldId::Delimiter => &self.delimiter,
            FieldId::Columns => &self.columns,
            FieldId::Pattern => &self.pattern,
            FieldId::JoinPrefix => &self.join_prefix,
            FieldId::JoinDelim1 => &self.join_delim1,
            FieldId::JoinDelim2 => &self.join_delim2,
            FieldId::JoinDelim3 => &self.join_delim3,
        }
    }

    pub(super) fn get_mut(&mut self, id: FieldId) -> &mut FieldState {
        match id {
            FieldId::Prefix => &mut self.prefix,
            FieldId::Suffix => &mut self.suffix,
            FieldId::Delimiter => &mut self.delimiter,
            FieldId::Columns => &mut self.columns,
            FieldId::Pattern => &mut self.pattern,
            FieldId::JoinPrefix => &mut self.join_prefix,
            FieldId::JoinDelim1 => &mut self.join_delim1,
            FieldId::JoinDelim2 => &mut self.join_delim2,
            FieldId::JoinDelim3 => &mut self.join_delim3,
        }
    }
}

/// A multi-line input holds the same state as a single-line one; its text
/// is LF-separated.
pub(super) type MultiFieldState = FieldState;

#[derive(Default, Clone)]
pub(super) struct MultiFields {
    pub(super) input_text: MultiFieldState,
    pub(super) list_b: MultiFieldState,
    pub(super) join_col2: MultiFieldState,
    pub(super) join_col3: MultiFieldState,
    pub(super) join_col4: MultiFieldState,
}

impl MultiFields {
    pub(super) fn get(&self, id: MultiFieldId) -> &MultiFieldState {
        match id {
            MultiFieldId::InputText => &self.input_text,
            MultiFieldId::ListB => &self.list_b,
            MultiFieldId::JoinCol2 => &self.join_col2,
            MultiFieldId::JoinCol3 => &self.join_col3,
            MultiFieldId::JoinCol4 => &self.join_col4,
        }
    }

    pub(super) fn get_mut(&mut self, id: MultiFieldId) -> &mut MultiFieldState {
        match id {
            MultiFieldId::InputText => &mut self.input_text,
            MultiFieldId::ListB => &mut self.list_b,
            MultiFieldId::JoinCol2 => &mut self.join_col2,
            MultiFieldId::JoinCol3 => &mut self.join_col3,
            MultiFieldId::JoinCol4 => &mut self.join_col4,
        }
    }
}

// --- dialog entity -------------------------------------------------------

pub(crate) struct TextToolsDialog {
    focus: FocusHandle,
    prefix_focus: FocusHandle,
    suffix_focus: FocusHandle,
    delimiter_focus: FocusHandle,
    columns_focus: FocusHandle,
    pattern_focus: FocusHandle,
    join_prefix_focus: FocusHandle,
    join_delim1_focus: FocusHandle,
    join_delim2_focus: FocusHandle,
    join_delim3_focus: FocusHandle,
    input_text_focus: FocusHandle,
    list_b_focus: FocusHandle,
    join_col2_focus: FocusHandle,
    join_col3_focus: FocusHandle,
    join_col4_focus: FocusHandle,

    pub(super) fields: Fields,
    pub(super) multi_fields: MultiFields,
    pub(super) join_column_count: usize,
    pub(super) compare_target: CompareResult,
    pub(super) active_tool: Option<Tool>,
    pub(super) status: Option<SharedString>,

    theme_mode: ThemeMode,
    system_appearance: WindowAppearance,
    pub(super) workspace: Option<Entity<Workspace>>,
}

impl TextToolsDialog {
    fn new(
        theme_mode: ThemeMode,
        system_appearance: WindowAppearance,
        input_text: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut multi_fields = MultiFields::default();
        multi_fields.input_text.text = input_text;
        Self {
            focus: cx.focus_handle(),
            prefix_focus: cx.focus_handle(),
            suffix_focus: cx.focus_handle(),
            delimiter_focus: cx.focus_handle(),
            columns_focus: cx.focus_handle(),
            pattern_focus: cx.focus_handle(),
            join_prefix_focus: cx.focus_handle(),
            join_delim1_focus: cx.focus_handle(),
            join_delim2_focus: cx.focus_handle(),
            join_delim3_focus: cx.focus_handle(),
            input_text_focus: cx.focus_handle(),
            list_b_focus: cx.focus_handle(),
            join_col2_focus: cx.focus_handle(),
            join_col3_focus: cx.focus_handle(),
            join_col4_focus: cx.focus_handle(),
            fields: Fields::default(),
            multi_fields,
            join_column_count: 2,
            compare_target: CompareResult::OnlyInA,
            active_tool: None,
            status: None,
            theme_mode,
            system_appearance,
            workspace: None,
        }
    }

    pub(super) fn theme(&self) -> Theme {
        let snap = crate::settings::snapshot();
        Theme::resolve(
            self.theme_mode,
            self.system_appearance,
            snap.tone_palette,
            &snap.customized_slots_array(),
        )
    }

    pub(super) fn focus_for(&self, id: FieldId) -> FocusHandle {
        match id {
            FieldId::Prefix => self.prefix_focus.clone(),
            FieldId::Suffix => self.suffix_focus.clone(),
            FieldId::Delimiter => self.delimiter_focus.clone(),
            FieldId::Columns => self.columns_focus.clone(),
            FieldId::Pattern => self.pattern_focus.clone(),
            FieldId::JoinPrefix => self.join_prefix_focus.clone(),
            FieldId::JoinDelim1 => self.join_delim1_focus.clone(),
            FieldId::JoinDelim2 => self.join_delim2_focus.clone(),
            FieldId::JoinDelim3 => self.join_delim3_focus.clone(),
        }
    }

    pub(super) fn focus_for_multi(&self, id: MultiFieldId) -> FocusHandle {
        match id {
            MultiFieldId::InputText => self.input_text_focus.clone(),
            MultiFieldId::ListB => self.list_b_focus.clone(),
            MultiFieldId::JoinCol2 => self.join_col2_focus.clone(),
            MultiFieldId::JoinCol3 => self.join_col3_focus.clone(),
            MultiFieldId::JoinCol4 => self.join_col4_focus.clone(),
        }
    }

    fn focused_single(&self, window: &Window) -> Option<FieldId> {
        FieldId::all()
            .iter()
            .copied()
            .find(|id| self.focus_for(*id).is_focused(window))
    }

    fn focused_multi(&self, window: &Window) -> Option<MultiFieldId> {
        MultiFieldId::all()
            .iter()
            .copied()
            .find(|id| self.focus_for_multi(*id).is_focused(window))
    }

    /// Per-tool visual focus order — matches the on-screen top-to-bottom,
    /// left-to-right reading order each tool's render() emits. Used by
    /// Tab / Shift+Tab to cycle through inputs without forcing the user
    /// to click each field.
    fn focus_order(&self) -> Vec<FocusTarget> {
        let n = self.join_column_count;
        match self.active_tool {
            Some(Tool::AddPrefixSuffix) => vec![
                FocusTarget::Single(FieldId::Prefix),
                FocusTarget::Single(FieldId::Suffix),
                FocusTarget::Multi(MultiFieldId::InputText),
            ],
            Some(Tool::JoinMerge) => {
                let mut order = vec![FocusTarget::Single(FieldId::JoinPrefix)];
                if n >= 2 {
                    order.push(FocusTarget::Single(FieldId::JoinDelim1));
                }
                if n >= 3 {
                    order.push(FocusTarget::Single(FieldId::JoinDelim2));
                }
                if n >= 4 {
                    order.push(FocusTarget::Single(FieldId::JoinDelim3));
                }
                order.push(FocusTarget::Multi(MultiFieldId::InputText));
                if n >= 2 {
                    order.push(FocusTarget::Multi(MultiFieldId::JoinCol2));
                }
                if n >= 3 {
                    order.push(FocusTarget::Multi(MultiFieldId::JoinCol3));
                }
                if n >= 4 {
                    order.push(FocusTarget::Multi(MultiFieldId::JoinCol4));
                }
                order
            }
            Some(Tool::ColumnExtractor) => vec![
                FocusTarget::Single(FieldId::Delimiter),
                FocusTarget::Single(FieldId::Columns),
                FocusTarget::Multi(MultiFieldId::InputText),
            ],
            Some(Tool::ListsComparison) => vec![
                FocusTarget::Multi(MultiFieldId::InputText),
                FocusTarget::Multi(MultiFieldId::ListB),
            ],
            Some(Tool::RemoveLinesContaining) => vec![
                FocusTarget::Single(FieldId::Pattern),
                FocusTarget::Multi(MultiFieldId::InputText),
            ],
            Some(Tool::RemoveDuplicates)
            | Some(Tool::RemoveEmptyLines)
            | Some(Tool::ReverseText) => vec![FocusTarget::Multi(MultiFieldId::InputText)],
            None => Vec::new(),
        }
    }

    fn current_focus_index(&self, window: &Window) -> Option<(Vec<FocusTarget>, usize)> {
        let order = self.focus_order();
        if order.is_empty() {
            return None;
        }
        let here = if let Some(id) = self.focused_single(window) {
            FocusTarget::Single(id)
        } else if let Some(id) = self.focused_multi(window) {
            FocusTarget::Multi(id)
        } else {
            return Some((order, 0));
        };
        let idx = order.iter().position(|t| *t == here)?;
        Some((order, idx))
    }

    fn focus_target(&self, target: FocusTarget, window: &mut Window, cx: &mut Context<Self>) {
        let handle = match target {
            FocusTarget::Single(id) => self.focus_for(id),
            FocusTarget::Multi(id) => self.focus_for_multi(id),
        };
        window.focus(&handle, cx);
        cx.notify();
    }

    fn cycle_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((order, idx)) = self.current_focus_index(window) else {
            return;
        };
        let n = order.len();
        if n <= 1 {
            return;
        }
        let next = if forward {
            (idx + 1) % n
        } else {
            (idx + n - 1) % n
        };
        self.focus_target(order[next], window, cx);
    }

    pub(super) fn input_text_str(&self) -> &str {
        &self.multi_fields.input_text.text
    }

    pub(super) fn join_columns_for_preview(&self) -> Vec<&str> {
        let mut cols: Vec<&str> = vec![self.multi_fields.input_text.text.as_str()];
        if self.join_column_count >= 2 {
            cols.push(self.multi_fields.join_col2.text.as_str());
        }
        if self.join_column_count >= 3 {
            cols.push(self.multi_fields.join_col3.text.as_str());
        }
        if self.join_column_count >= 4 {
            cols.push(self.multi_fields.join_col4.text.as_str());
        }
        cols
    }

    /// Compute the transformed text for the current tool from the
    /// in-modal multi-line input(s) plus single-line control fields. `Err`
    /// holds the status to show when there is no safe result.
    pub(super) fn compute(&self, target: Option<CompareResult>) -> Result<String, &'static str> {
        let out = match self.active_tool {
            Some(Tool::AddPrefixSuffix) => add_prefix_suffix::transform(
                &self.fields.prefix.text,
                &self.fields.suffix.text,
                self.input_text_str(),
            ),
            Some(Tool::JoinMerge) => {
                let cols = self.join_columns_for_preview();
                let delims: [&str; 3] = [
                    self.fields.join_delim1.text.as_str(),
                    self.fields.join_delim2.text.as_str(),
                    self.fields.join_delim3.text.as_str(),
                ];
                let n = cols.len();
                let active_delims: Vec<&str> = (0..n.saturating_sub(1))
                    .map(|i| delims.get(i).copied().unwrap_or(""))
                    .collect();
                join_merge::transform(&self.fields.join_prefix.text, &cols, &active_delims)
            }
            Some(Tool::ColumnExtractor) => column_extractor::transform(
                &self.fields.delimiter.text,
                &self.fields.columns.text,
                self.input_text_str(),
            ),
            Some(Tool::RemoveDuplicates) => {
                remove_duplicates::transform(self.input_text_str()).map(|(s, _)| s)
            }
            Some(Tool::RemoveEmptyLines) => {
                remove_empty_lines::transform(self.input_text_str()).map(|(s, _)| s)
            }
            Some(Tool::RemoveLinesContaining) => {
                remove_lines_containing::transform(
                    &self.fields.pattern.text,
                    self.input_text_str(),
                )
                .map(|(s, _)| s)
            }
            Some(Tool::ListsComparison) => {
                let (a, b, both) = lists_comparison::transform(
                    self.input_text_str(),
                    &self.multi_fields.list_b.text,
                );
                let pick = target.unwrap_or(self.compare_target);
                match pick {
                    CompareResult::OnlyInA => a,
                    CompareResult::OnlyInB => b,
                    CompareResult::InBoth => both,
                }
            }
            Some(Tool::ReverseText) => reverse_text::transform(self.input_text_str()),
            None => Some(String::new()),
        };
        out.ok_or(eol::MERGED_LINES)
    }

    pub(super) fn enter_tool(&mut self, tool: Tool, _cx: &mut Context<Self>) {
        self.active_tool = Some(tool);
        self.status = None;
        // Reset secondary multi-line areas when switching tools so stale
        // List B / column data doesn't carry over.
        self.multi_fields.list_b = MultiFieldState::default();
        self.multi_fields.join_col2 = MultiFieldState::default();
        self.multi_fields.join_col3 = MultiFieldState::default();
        self.multi_fields.join_col4 = MultiFieldState::default();
        if tool != Tool::JoinMerge {
            self.join_column_count = 2;
        }
    }

    pub(super) fn exit_tool(&mut self, _cx: &mut Context<Self>) {
        self.active_tool = None;
        self.status = None;
    }

    pub(super) fn grow_join_columns(&mut self, _cx: &mut Context<Self>) {
        if self.active_tool != Some(Tool::JoinMerge) {
            return;
        }
        if self.join_column_count >= 4 {
            return;
        }
        self.join_column_count += 1;
    }

    pub(super) fn shrink_join_columns(&mut self, _cx: &mut Context<Self>) {
        if self.active_tool != Some(Tool::JoinMerge) {
            return;
        }
        if self.join_column_count <= 2 {
            return;
        }
        self.join_column_count -= 1;
    }

    fn apply_result(&mut self, target: Option<CompareResult>, cx: &mut Context<Self>) {
        let new_text = match self.compute(target) {
            Ok(text) => text,
            Err(why) => {
                self.status = Some(why.into());
                cx.notify();
                return;
            }
        };
        let Some(workspace) = self.workspace.clone() else {
            self.status = Some("No workspace — open Notepad Studio first.".into());
            cx.notify();
            return;
        };
        let mut applied_in_place = false;
        workspace.update(cx, |ws, cx| {
            if ws.tabs.is_empty() {
                ws.new_tab(cx);
                ws.replace_byte_range(0..0, &new_text, cx);
            } else {
                let len = ws.buffer().rope.len_bytes();
                ws.replace_byte_range(0..len, &new_text, cx);
                applied_in_place = true;
            }
        });
        self.status = Some(if applied_in_place {
            "Result applied to active tab. Cmd+Z to undo.".into()
        } else {
            "No active tab — opened result in a new tab.".into()
        });
        cx.notify();
    }

    fn apply_active(&mut self, cx: &mut Context<Self>) {
        self.apply_result(None, cx);
    }

    fn apply_lists_only_a(&mut self, cx: &mut Context<Self>) {
        self.apply_result(Some(CompareResult::OnlyInA), cx);
    }

    fn apply_lists_only_b(&mut self, cx: &mut Context<Self>) {
        self.apply_result(Some(CompareResult::OnlyInB), cx);
    }

    fn apply_lists_both(&mut self, cx: &mut Context<Self>) {
        self.apply_result(Some(CompareResult::InBoth), cx);
    }

    fn copy_to_clipboard(&mut self, cx: &mut Context<Self>) {
        self.status = Some(match self.compute(None) {
            Ok(text) => {
                crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(text));
                "Result copied to clipboard.".into()
            }
            Err(why) => why.into(),
        });
        cx.notify();
    }

    // --- focused-field clipboard ops --------------------------------------
    //
    // These run from `EditCopy` / `EditCut` / `EditPaste` / `EditSelectAll`
    // action handlers wired on the dialog root in `render`. Routing through
    // actions (rather than `on_key_down`) is mandatory: the macOS Edit menu
    // owns `cmd-c` / `cmd-v` / `cmd-x` / `cmd-a` as menu key-equivalents and
    // dispatches them as those GPUI actions before any `keyDown` reaches the
    // window. Without action handlers, the keystroke is swallowed and our
    // own `handle_key_down` matchers never fire.

    fn copy_focused_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let f = if let Some(id) = self.focused_single(window) {
            self.fields.get(id)
        } else if let Some(id) = self.focused_multi(window) {
            self.multi_fields.get(id)
        } else {
            return;
        };
        let sel = f.selected();
        if !sel.is_empty() {
            crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(sel.to_string()));
        }
    }

    fn cut_focused_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let f = if let Some(id) = self.focused_single(window) {
            self.fields.get_mut(id)
        } else if let Some(id) = self.focused_multi(window) {
            self.multi_fields.get_mut(id)
        } else {
            return;
        };
        if let Some(sel) = f.take_selection() {
            crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(sel));
            cx.notify();
        }
    }

    fn paste_at_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let Some(text) = item.text() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if let Some(id) = self.focused_single(window) {
            // Single-line: collapse newlines so paste matches a real
            // single-line input — most OS controls strip newlines on paste.
            let cleaned: String = text.replace(['\r', '\n'], " ");
            self.fields.get_mut(id).replace_selection(&cleaned);
            cx.notify();
            return;
        }
        if let Some(id) = self.focused_multi(window) {
            self.multi_fields.get_mut(id).replace_selection(&lf_line_ends(&text));
            cx.notify();
        }
    }

    fn select_all_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.focused_single(window) {
            self.fields.get_mut(id).select_all();
            cx.notify();
            return;
        }
        if let Some(id) = self.focused_multi(window) {
            self.multi_fields.get_mut(id).select_all();
            cx.notify();
        }
    }
}

impl Focusable for TextToolsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

// --- IME / typed input dispatch ------------------------------------------

impl EntityInputHandler for TextToolsDialog {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        if let Some(id) = self.focused_single(window) {
            let s = &self.fields.get(id).text;
            return Some(slice_str_chars_utf16(s, range_utf16, actual_range));
        }
        if let Some(id) = self.focused_multi(window) {
            let s = &self.multi_fields.get(id).text;
            return Some(slice_str_chars_utf16(s, range_utf16, actual_range));
        }
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if let Some(id) = self.focused_single(window) {
            let f = self.fields.get(id);
            return Some(input_selection_range_utf16(&f.text, f.cursor, f.anchor));
        }
        if let Some(id) = self.focused_multi(window) {
            let f = self.multi_fields.get(id);
            return Some(input_selection_range_utf16(&f.text, f.cursor, f.anchor));
        }
        None
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Single-line inputs drop line ends, multi-line ones store LF only.
        let (f, text) = if let Some(id) = self.focused_single(window) {
            let text: String = new_text.chars().filter(|c| !matches!(c, '\r' | '\n')).collect();
            (self.fields.get_mut(id), text)
        } else if let Some(id) = self.focused_multi(window) {
            (self.multi_fields.get_mut(id), lf_line_ends(new_text))
        } else {
            return;
        };
        // `replace_input_utf16` slices with the live selection when the OS
        // sends no range; hand it one that is on char boundaries.
        let sel = f.ordered();
        replace_input_utf16(
            &mut f.text,
            &mut f.cursor,
            &mut f.anchor,
            sel.start,
            sel.end,
            range_utf16,
            &text,
        );
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

/// `s` with CRLF and lone CR turned into LF. The multi-line inputs keep LF
/// only, so their line and caret maths sees one kind of line end.
fn lf_line_ends(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

// --- keyboard ------------------------------------------------------------

/// Start of the char before `byte` (floored to a boundary first).
fn prev_grapheme(s: &str, byte: usize) -> usize {
    let byte = floor_str_boundary(s, byte);
    s.get(..byte)
        .and_then(|head| head.char_indices().next_back())
        .map_or(0, |(i, _)| i)
}

/// End of the char at `byte` (floored to a boundary first).
fn next_grapheme(s: &str, byte: usize) -> usize {
    let byte = floor_str_boundary(s, byte);
    s.get(byte..)
        .and_then(|tail| tail.chars().next())
        .map_or(s.len(), |c| byte + c.len_utf8())
}

/// `(start, end)` of the line holding `byte`, excluding its `\n`.
fn line_bounds(s: &str, byte: usize) -> (usize, usize) {
    let byte = floor_str_boundary(s, byte);
    let Some((head, tail)) = s.split_at_checked(byte) else {
        return (0, s.len());
    };
    let start = head.rfind('\n').map_or(0, |i| i + 1);
    let end = tail.find('\n').map_or(s.len(), |i| byte + i);
    (start, end)
}

fn move_line_vertical(s: &str, byte: usize, down: bool) -> usize {
    let byte = floor_str_boundary(s, byte);
    let (cur_start, cur_end) = line_bounds(s, byte);
    let col = byte - cur_start;
    if down {
        if cur_end >= s.len() {
            return s.len();
        }
        let next_start = cur_end + 1;
        let (_, next_end) = line_bounds(s, next_start);
        let target = next_start + col;
        let mut tgt = target.min(next_end);
        while !s.is_char_boundary(tgt) && tgt > next_start {
            tgt -= 1;
        }
        tgt
    } else {
        if cur_start == 0 {
            return 0;
        }
        let prev_end = cur_start - 1;
        let (prev_start, _) = line_bounds(s, prev_end);
        let target = prev_start + col;
        let mut tgt = target.min(prev_end);
        while !s.is_char_boundary(tgt) && tgt > prev_start {
            tgt -= 1;
        }
        tgt
    }
}

impl TextToolsDialog {
    fn handle_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let mods = &ev.keystroke.modifiers;
        let key = ev.keystroke.key.as_str();
        let cmd_or_ctrl = mods.platform || mods.control;
        let shift = mods.shift;

        if key == "escape" && !cmd_or_ctrl && !shift {
            close_current_window(window);
            return;
        }

        // Tab / Shift+Tab cycles focus through the active tool's visible
        // inputs. Must run before per-field handlers so a focused textarea
        // doesn't try to insert a literal tab character (it currently
        // wouldn't anyway, but matches NPP's dialog convention).
        if key == "tab" && !cmd_or_ctrl {
            self.cycle_focus(!shift, window, cx);
            return;
        }

        if let Some(id) = self.focused_single(window) {
            self.handle_single_key(id, key, cmd_or_ctrl, shift, cx);
            return;
        }
        if let Some(id) = self.focused_multi(window) {
            self.handle_multi_key(id, key, cmd_or_ctrl, shift, cx);
        }
    }

    fn handle_single_key(
        &mut self,
        id: FieldId,
        key: &str,
        cmd_or_ctrl: bool,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let f = self.fields.get_mut(id);
        match key {
            "left" => {
                let new = prev_grapheme(&f.text, f.cursor);
                f.cursor = new;
                if !shift {
                    f.anchor = new;
                }
            }
            "right" => {
                let new = next_grapheme(&f.text, f.cursor);
                f.cursor = new;
                if !shift {
                    f.anchor = new;
                }
            }
            "home" => {
                f.cursor = 0;
                if !shift {
                    f.anchor = 0;
                }
            }
            "end" => {
                let n = f.text.len();
                f.cursor = n;
                if !shift {
                    f.anchor = n;
                }
            }
            "backspace" => f.delete_backward(),
            "delete" => f.delete_forward(),
            "a" if cmd_or_ctrl => f.select_all(),
            "c" if cmd_or_ctrl => {
                let sel = f.selected();
                if !sel.is_empty() {
                    crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(sel.to_string()));
                }
                return;
            }
            "x" if cmd_or_ctrl => {
                if let Some(sel) = f.take_selection() {
                    crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(sel));
                }
            }
            _ => return,
        }
        cx.notify();
    }

    fn handle_multi_key(
        &mut self,
        id: MultiFieldId,
        key: &str,
        cmd_or_ctrl: bool,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let f = self.multi_fields.get_mut(id);
        match key {
            "left" => {
                let new = prev_grapheme(&f.text, f.cursor);
                f.cursor = new;
                if !shift {
                    f.anchor = new;
                }
            }
            "right" => {
                let new = next_grapheme(&f.text, f.cursor);
                f.cursor = new;
                if !shift {
                    f.anchor = new;
                }
            }
            "up" => {
                let new = move_line_vertical(&f.text, f.cursor, false);
                f.cursor = new;
                if !shift {
                    f.anchor = new;
                }
            }
            "down" => {
                let new = move_line_vertical(&f.text, f.cursor, true);
                f.cursor = new;
                if !shift {
                    f.anchor = new;
                }
            }
            "home" => {
                let (start, _) = line_bounds(&f.text, f.cursor);
                f.cursor = start;
                if !shift {
                    f.anchor = start;
                }
            }
            "end" => {
                let (_, end) = line_bounds(&f.text, f.cursor);
                f.cursor = end;
                if !shift {
                    f.anchor = end;
                }
            }
            "enter" => f.replace_selection("\n"),
            "backspace" => f.delete_backward(),
            "delete" => f.delete_forward(),
            "a" if cmd_or_ctrl => f.select_all(),
            "c" if cmd_or_ctrl => {
                let sel = f.selected();
                if !sel.is_empty() {
                    crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(sel.to_string()));
                }
                return;
            }
            "x" if cmd_or_ctrl => {
                if let Some(sel) = f.take_selection() {
                    crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(sel));
                }
            }
            _ => return,
        }
        cx.notify();
    }
}

// --- window lifecycle ----------------------------------------------------

fn close_current_window(window: &mut Window) {
    OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
    window.remove_window();
}

pub(crate) fn open_or_focus(cx: &mut App) {
    let cached = OPEN_WINDOW.with(|cell| cell.borrow().clone());
    if let Some(handle) = cached {
        let result = handle.update(cx, |_, window, cx| {
            window.activate_window();
            cx.notify();
        });
        if result.is_ok() {
            return;
        }
        OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
    }

    let theme_mode = current_theme_mode(cx);
    let initial_zoom = active_workspace_zoom(cx).unwrap_or(ZOOM_DEFAULT);
    let input_text = active_workspace_buffer_text(cx).unwrap_or_default();
    let appearance = cx
        .active_window()
        .map(|h| {
            cx.update_window(h, |_, window, _| window.appearance())
                .unwrap_or(WindowAppearance::Dark)
        })
        .unwrap_or(WindowAppearance::Dark);

    let bounds = crate::app::window::modal_bounds_centered_on_workspace(
        cx,
        size(px(960.), px(720.)),
    );
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
            kind: WindowKind::Floating,
            is_resizable: true,
            window_min_size: Some(size(px(760.), px(520.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Text Tools".into()),
                appears_transparent: false,
                traffic_light_position: Some(point(px(10.), px(10.))),
            }),
            ..Default::default()
        },
        |window, cx| {
            window.set_rem_size(px(ZOOM_REM_BASE * initial_zoom));
            cx.new(|cx| TextToolsDialog::new(theme_mode, appearance, input_text, cx))
        },
    );

    let handle = match result {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to open Text Tools: {err}");
            return;
        }
    };

    OPEN_WINDOW.with(|cell| *cell.borrow_mut() = Some(handle));

    cx.defer(move |cx| {
        if let Ok(dialog_entity) = handle.entity(cx) {
            cx.observe_release(&dialog_entity, move |_, _| {
                OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
            })
            .detach();
        }
        let workspace_entity = active_workspace_entity(cx);
        let _ = handle.update(cx, |dialog, _window, cx| {
            dialog.workspace = workspace_entity;
            cx.notify();
        });
    });
}

fn current_theme_mode(cx: &mut App) -> ThemeMode {
    let handle = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| cx.windows().into_iter().find_map(|h| h.downcast::<Workspace>()));
    let Some(handle) = handle else {
        return ThemeMode::Dark;
    };
    handle.read(cx).map(|w| w.theme_mode).unwrap_or(ThemeMode::Dark)
}

fn active_workspace_entity(cx: &mut App) -> Option<Entity<Workspace>> {
    let handle = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| cx.windows().into_iter().find_map(|h| h.downcast::<Workspace>()))?;
    handle.entity(cx).ok()
}

fn active_workspace_zoom(cx: &mut App) -> Option<f32> {
    let handle = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| cx.windows().into_iter().find_map(|h| h.downcast::<Workspace>()))?;
    handle.read(cx).map(|w| w.zoom_level).ok()
}

fn active_workspace_buffer_text(cx: &mut App) -> Option<String> {
    let handle = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| cx.windows().into_iter().find_map(|h| h.downcast::<Workspace>()))?;
    handle
        .read(cx)
        .map(|w| w.buffer().rope.to_string())
        .ok()
}

// --- render --------------------------------------------------------------

impl Render for TextToolsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let entity = cx.entity().clone();
        let weak = entity.downgrade();

        let body: gpui::AnyElement = match self.active_tool {
            None => render_landing(&theme, &weak),
            Some(tool) => render_tool_body(&theme, &weak, self, tool),
        };

        let footer = render_footer(&theme, &weak, self);

        let root = div()
            .key_context("TextToolsDialog")
            .track_focus(&self.focus)
            .cursor(CursorStyle::Arrow)
            .on_action(cx.listener(|_, _: &ViewZoomIn, _, _| {}))
            .on_action(cx.listener(|_, _: &ViewZoomOut, _, _| {}))
            .on_action(cx.listener(|_, _: &ViewZoomReset, _, _| {}))
            // macOS Edit menu owns cmd-c/v/x/a as menu key-equivalents and
            // dispatches them as these actions before any keystroke reaches
            // `on_key_down`. Catch them here so the focused TextTools input
            // gets normal clipboard / select-all behaviour.
            .on_action(cx.listener(|this, _: &EditCopy, window, cx| {
                this.copy_focused_selection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &EditCut, window, cx| {
                this.cut_focused_selection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &EditPaste, window, cx| {
                this.paste_at_focused(window, cx);
            }))
            .on_action(cx.listener(|this, _: &EditSelectAll, window, cx| {
                this.select_all_focused(window, cx);
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_key_down(ev, window, cx);
            }))
            .size_full()
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.text))
            .font_family(crate::fonts::mono())
            .text_size(rems(13. / 16.))
            .flex()
            .flex_col();

        root.child(
            div()
                .id("tt-body-scroll")
                .flex_grow()
                .min_h(px(0.))
                .overflow_y_scroll()
                .px(rems(20. / 16.))
                .py(rems(1.))
                .child(body),
        )
        .child(footer)
    }
}

fn render_landing(theme: &Theme, weak: &WeakEntity<TextToolsDialog>) -> gpui::AnyElement {
    let mut grid = div().flex().flex_col().gap(rems(10. / 16.));

    let tools = Tool::all();
    for chunk_idx in 0..2 {
        let mut row = div().flex().flex_row().gap(rems(10. / 16.));
        for col_idx in 0..4 {
            let i = chunk_idx * 4 + col_idx;
            let tool = tools[i];
            let weak_card = weak.clone();
            let card_id: gpui::ElementId = SharedString::from(format!("tt-card-{i}")).into();
            let card = div()
                .id(card_id)
                .flex_1()
                .flex_basis(rems(0.))
                .min_w(rems(140. / 16.))
                .min_h(rems(72. / 16.))
                .px(rems(12. / 16.))
                .py(rems(10. / 16.))
                .bg(rgb(theme.button_bg))
                .border_1()
                .border_color(rgb(theme.border))
                .cursor(gpui::CursorStyle::PointingHand)
                .hover(|s| s.bg(rgb(theme.button_hover_bg)))
                .active(|s| s.bg(rgb(theme.button_pressed_bg)))
                .flex()
                .flex_col()
                .gap(rems(6. / 16.))
                .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                    let _ = weak_card.update(cx, |this, cx| {
                        this.enter_tool(tool, cx);
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .text_color(rgb(theme.text))
                        .text_size(rems(13. / 16.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(SharedString::from(tool.title())),
                )
                .child(
                    div()
                        .text_color(rgb(theme.chrome_text))
                        .text_size(rems(11. / 16.))
                        .child(SharedString::from(tool.description())),
                );
            row = row.child(card);
        }
        grid = grid.child(row);
    }

    div()
        .flex()
        .flex_col()
        .gap(rems(14. / 16.))
        .child(
            div()
                .text_color(rgb(theme.text))
                .text_size(rems(13. / 16.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(SharedString::from("Text Tools")),
        )
        .child(label_text(
            theme,
            "Pick a tool. Paste or type your text, set the parameters, then Apply replaces the active tab.",
        ))
        .child(grid)
        .child(div().h(rems(8. / 16.)))
        .child(caption_text(
            theme,
            "The active tab seeds the Input area on open. Apply replaces the active tab with the result — Cmd+Z undoes it. Use Copy to Clipboard to keep the original untouched.",
        ))
        .into_any_element()
}

fn render_tool_body(
    theme: &Theme,
    weak: &WeakEntity<TextToolsDialog>,
    dialog: &TextToolsDialog,
    tool: Tool,
) -> gpui::AnyElement {
    let weak_back = weak.clone();
    let back_btn = div()
        .id("tt-back")
        .px(rems(10. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.text))
        .cursor(gpui::CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child("← Back")
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_back.update(cx, |this, cx| {
                this.exit_tool(cx);
                cx.notify();
            });
        });

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(10. / 16.))
        .child(back_btn)
        .child(
            div()
                .text_color(rgb(theme.text))
                .text_size(rems(15. / 16.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(SharedString::from(tool.title())),
        );

    let inner = match tool {
        Tool::AddPrefixSuffix => add_prefix_suffix::render(theme, weak, dialog),
        Tool::JoinMerge => join_merge::render(theme, weak, dialog),
        Tool::ColumnExtractor => column_extractor::render(theme, weak, dialog),
        Tool::RemoveDuplicates => remove_duplicates::render(theme, weak, dialog),
        Tool::RemoveEmptyLines => remove_empty_lines::render(theme, weak, dialog),
        Tool::RemoveLinesContaining => remove_lines_containing::render(theme, weak, dialog),
        Tool::ListsComparison => lists_comparison::render(theme, weak, dialog),
        Tool::ReverseText => reverse_text::render(theme, weak, dialog),
    };

    div()
        .flex()
        .flex_col()
        .gap(rems(12. / 16.))
        .child(header)
        .child(inner)
        .into_any_element()
}

fn render_footer(
    theme: &Theme,
    weak: &WeakEntity<TextToolsDialog>,
    dialog: &TextToolsDialog,
) -> gpui::AnyElement {
    let weak_apply = weak.clone();
    let weak_apply_a = weak.clone();
    let weak_apply_b = weak.clone();
    let weak_apply_both = weak.clone();
    let weak_copy = weak.clone();

    // Lists Comparison gets three apply buttons (Only A / Only B / Both).
    // Other tools get a single Apply.
    let is_lists = matches!(dialog.active_tool, Some(Tool::ListsComparison));
    let mut buttons: Vec<gpui::AnyElement> = Vec::new();

    if is_lists {
        buttons.push(npp_button_primary(
            theme,
            "tt-apply-a",
            "Apply: Only A",
            move |cx| {
                let _ = weak_apply_a.update(cx, |this, cx| this.apply_lists_only_a(cx));
            },
        ));
        buttons.push(npp_button_primary(
            theme,
            "tt-apply-b",
            "Apply: Only B",
            move |cx| {
                let _ = weak_apply_b.update(cx, |this, cx| this.apply_lists_only_b(cx));
            },
        ));
        buttons.push(npp_button_primary(
            theme,
            "tt-apply-both",
            "Apply: Both",
            move |cx| {
                let _ = weak_apply_both.update(cx, |this, cx| this.apply_lists_both(cx));
            },
        ));
    } else if dialog.active_tool.is_some() {
        buttons.push(npp_button_primary(
            theme,
            "tt-apply",
            "Apply",
            move |cx| {
                let _ = weak_apply.update(cx, |this, cx| {
                    this.apply_active(cx);
                });
            },
        ));
    } else {
        buttons.push(widgets::npp_button_disabled(theme, "Apply"));
    }

    let copy_btn = if dialog.active_tool.is_some() {
        npp_button(theme, "tt-copy", "Copy to Clipboard", move |cx| {
            let _ = weak_copy.update(cx, |this, cx| {
                this.copy_to_clipboard(cx);
            });
        })
    } else {
        widgets::npp_button_disabled(theme, "Copy to Clipboard")
    };
    let cancel_btn = div()
        .id("tt-cancel")
        .px(rems(12. / 16.))
        .py(rems(4. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(12. / 16.))
        .text_color(rgb(theme.text))
        .cursor(gpui::CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child("Close")
        .on_mouse_down(MouseButton::Left, move |_e, window, _cx| {
            close_current_window(window);
        })
        .into_any_element();

    let status: gpui::AnyElement = match &dialog.status {
        Some(s) => div()
            .text_color(rgb(theme.chrome_text))
            .text_size(rems(11. / 16.))
            .child(s.clone())
            .into_any_element(),
        None => div().into_any_element(),
    };

    let mut row = div()
        .h(rems(48. / 16.))
        .border_t_1()
        .border_color(rgb(theme.border))
        .px(rems(20. / 16.))
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(10. / 16.))
        .child(status)
        .child(div().flex_grow());
    for b in buttons {
        row = row.child(b);
    }
    row.child(copy_btn).child(cancel_btn).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str, cursor: usize, anchor: usize) -> FieldState {
        FieldState { text: text.to_string(), cursor, anchor }
    }

    #[test]
    fn stale_or_mid_char_caret_slices_on_char_boundaries() {
        // a=0, é=1..3, 中=3..6
        let cases: &[(usize, usize, &str)] = &[
            (2, 9, "é中"),
            (9, 2, "é中"),
            (4, 2, "é"),
            (6, 6, ""),
            (99, 99, ""),
        ];
        for &(cursor, anchor, want) in cases {
            let f = field("aé中", cursor, anchor);
            assert_eq!(&f.text[f.ordered()], want, "cursor {cursor} anchor {anchor}");
        }
    }

    #[test]
    fn line_bounds_and_vertical_moves_floor_mid_char_offsets() {
        // é=0..2, \n=2, x=3, \n=4, 中=5..8
        let s = "é\nx\n中";
        assert_eq!(line_bounds(s, 1), (0, 2));
        assert_eq!(line_bounds(s, 6), (5, 8));
        assert_eq!(line_bounds(s, 99), (5, 8));
        assert_eq!(move_line_vertical(s, 1, true), 3);
        assert_eq!(move_line_vertical(s, 7, false), 3);
        assert_eq!(move_line_vertical(s, 99, false), 4);
    }

    #[test]
    fn field_edits_with_stale_or_mid_char_carets_stay_valid() {
        // a=0, é=1..3, 中=3..6, 👨‍👩‍👧=6..24
        let text = "aé中👨‍👩‍👧";
        let backspace: &[(usize, usize, &str, usize)] = &[
            (2, 2, "é中👨‍👩‍👧", 0),
            (4, 4, "a中👨‍👩‍👧", 1),
            (99, 99, "aé中👨‍👩‍", 20),
            (0, 0, text, 0),
            (2, 99, "a", 1),
        ];
        for &(cursor, anchor, want, caret) in backspace {
            let mut f = field(text, cursor, anchor);
            f.delete_backward();
            assert_eq!((f.text.as_str(), f.cursor, f.anchor), (want, caret, caret), "backspace {cursor}/{anchor}");
        }
        let delete: &[(usize, usize, &str, usize)] = &[
            (2, 2, "a中👨‍👩‍👧", 1),
            (5, 5, "aé👨‍👩‍👧", 3),
            (99, 99, text, 24),
            (7, 0, "👨‍👩‍👧", 0),
        ];
        for &(cursor, anchor, want, caret) in delete {
            let mut f = field(text, cursor, anchor);
            f.delete_forward();
            assert_eq!((f.text.as_str(), f.cursor, f.anchor), (want, caret, caret), "delete {cursor}/{anchor}");
        }

        let mut f = field(text, 4, 2);
        assert_eq!(f.selected(), "é");
        f.replace_selection("\n");
        assert_eq!((f.text.as_str(), f.cursor), ("a\n中👨‍👩‍👧", 2));

        let mut f = field(text, 99, 5);
        assert_eq!(f.take_selection().as_deref(), Some("中👨‍👩‍👧"));
        assert_eq!((f.text.as_str(), f.cursor, f.anchor), ("aé", 3, 3));
        assert_eq!(f.take_selection(), None);

        let mut f = field("", 7, 3);
        f.delete_backward();
        f.delete_forward();
        f.replace_selection("x");
        assert_eq!((f.text.as_str(), f.cursor), ("x", 1));
    }

    #[test]
    fn multi_line_input_stores_lf_only() {
        let cases: &[(&str, &str)] = &[
            ("", ""),
            ("a\r\nb\rc\nd", "a\nb\nc\nd"),
            ("\r\r\n\n", "\n\n\n"),
            ("é\r\n中", "é\n中"),
        ];
        for (input, want) in cases {
            assert_eq!(lf_line_ends(input), *want, "input {input:?}");
        }
    }
}
