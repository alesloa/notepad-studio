// PowerEditor/src/NppCommands.cpp:2059–2087 — the Edit menu's line indentation
// and duplicate commands. Scintilla's keyboard Tab and selection duplicate are
// separate commands (Editor.cxx:4168–4209). Behaviour studied; Rust is original.

use super::util::floor_char_boundary;
use crate::actions::{EditDecreaseLineIndent, EditDuplicateCurrentLine, EditIncreaseLineIndent};
use crate::buffer::{Selection, ContentRevision, prepared::{PrefixEdit, PreparedGroup}};
use crate::macros::{self, MacroEvent};
use crate::tab::{InputFocus, TabViewMode};
use crate::workspace::Workspace;
use gpui::{Context, Window};
use ropey::Rope;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_PREFIX_BYTES: usize = 1024 * 1024;
const MAX_EDIT_BYTES: usize = 32 * 1024 * 1024;
// Small commands keep the synchronous macro path; larger manual commands
// prepare a sealed history entry and metadata on the background executor.
const MAX_LINE_EDITS: usize = 4096;
const MAX_FOREGROUND_ANNOTATION_VISITS: usize = 16_384;
const MAX_FOREGROUND_LINE_BYTES: usize = 256 * 1024;
const MAX_DUPLICATE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(crate) enum MenuLineCommand {
    Increase,
    Decrease,
    Duplicate,
}

impl MenuLineCommand {
    fn event(self) -> MacroEvent {
        match self {
            Self::Increase => MacroEvent::IncreaseLineIndent,
            Self::Decrease => MacroEvent::DecreaseLineIndent,
            Self::Duplicate => MacroEvent::DuplicateCurrentLine,
        }
    }
}

type Replacement = PrefixEdit;
struct Plan {
    edits: Vec<Replacement>,
    selection: Selection,
}
struct Line {
    start: usize,
    body_end: usize,
    end: usize,
}

// Ropey also splits at Unicode line separators. These commands recognise only
// CR, LF and CRLF, so widen its cheap initial lookup to the surrounding line.
fn line_at(rope: &Rope, at: usize) -> Line {
    let at = floor_char_boundary(rope, at);
    let mut start = rope.line_to_byte(rope.byte_to_line(at));
    while start > 0 {
        let previous = rope.get_byte(start - 1);
        if previous == Some(b'\n')
            || (previous == Some(b'\r') && rope.get_byte(start) != Some(b'\n'))
        {
            break;
        }
        start -= 1;
    }
    let index = rope.byte_to_line(at);
    let mut end = if index.saturating_add(1) < rope.len_lines() {
        rope.line_to_byte(index + 1)
    } else {
        rope.len_bytes()
    };
    if !matches!(
        end.checked_sub(1).and_then(|i| rope.get_byte(i)),
        Some(b'\r' | b'\n')
    ) {
        if let Some(mut bytes) = rope.get_bytes_at(end) {
            while let Some(byte) = bytes.next() {
                end = end.saturating_add(1);
                if byte == b'\n' {
                    break;
                }
                if byte == b'\r' {
                    if bytes.next() == Some(b'\n') {
                        end = end.saturating_add(1);
                    }
                    break;
                }
            }
        }
    }
    let mut body_end = end;
    if body_end > start
        && matches!(
            body_end.checked_sub(1).and_then(|i| rope.get_byte(i)),
            Some(b'\r' | b'\n')
        )
    {
        body_end -= 1;
        if rope.get_byte(body_end) == Some(b'\n')
            && body_end > start
            && rope.get_byte(body_end - 1) == Some(b'\r')
        {
            body_end -= 1;
        }
    }
    Line {
        start,
        body_end,
        end,
    }
}

fn shift_after(position: usize, removed: usize, inserted: usize) -> usize {
    if inserted >= removed {
        position.saturating_add(inserted - removed)
    } else {
        position.saturating_sub(removed - inserted)
    }
}

fn map_prefix(mut position: usize, edits: &[Replacement]) -> usize {
    // Original ranges remain valid when visited from the document's end.
    for edit in edits.iter().rev() {
        if position >= edit.range.end {
            position = shift_after(position, edit.range.len(), edit.text.len());
        } else if position > edit.range.start && edit.text.len() < edit.range.len() {
            position = position.min(edit.range.start.saturating_add(edit.text.len()));
        }
    }
    position
}

fn indent_plan(
    rope: &Rope,
    selection: Selection,
    tabs: bool,
    width: usize,
    increase: bool,
) -> Result<Plan, &'static str> {
    indent_plan_bounded(rope, selection, tabs, width, increase, MAX_LINE_EDITS, None)
}

fn selected_rows(rope: &Rope, selection: Selection) -> usize {
    let ordered = selection.clamped(rope).ordered();
    let first = rope.byte_to_line(ordered.start);
    let last = rope.byte_to_line(ordered.end);
    let rows = last.saturating_sub(first).saturating_add(1);
    if ordered.end > ordered.start && rope.line_to_byte(last) == ordered.end { rows.saturating_sub(1) }
    else { rows }
}

fn physical_line_start(rope: &Rope, at: usize) -> bool {
    at == 0 || at.checked_sub(1).and_then(|previous| rope.get_byte(previous)) == Some(b'\n')
        || (at.checked_sub(1).and_then(|previous| rope.get_byte(previous)) == Some(b'\r')
            && rope.get_byte(at) != Some(b'\n'))
}

// Inspect Rope row boundaries without scanning or copying their contents.
// A Unicode-only boundary can widen line_at far beyond this cheap row span.
fn foreground_line_work_bounded(rope: &Rope, selection: Selection) -> bool {
    let ordered = selection.clamped(rope).ordered();
    let Ok(first) = rope.try_byte_to_line(ordered.start) else { return false; };
    let Ok(mut last) = rope.try_byte_to_line(ordered.end) else { return false; };
    let Ok(start) = rope.try_line_to_byte(first) else { return false; };
    let Ok(last_start) = rope.try_line_to_byte(last) else { return false; };
    if ordered.end > ordered.start && last_start == ordered.end && physical_line_start(rope, last_start) {
        last = last.saturating_sub(1);
    }
    let end = if last.saturating_add(1) < rope.len_lines() {
        let Ok(end) = rope.try_line_to_byte(last.saturating_add(1)) else { return false; };
        end
    } else { rope.len_bytes() };
    physical_line_start(rope, start)
        && (end == rope.len_bytes() || physical_line_start(rope, end))
        && end.saturating_sub(start) <= MAX_FOREGROUND_LINE_BYTES
}

fn indent_plan_bounded(rope: &Rope, selection: Selection, tabs: bool, width: usize,
    increase: bool, max_edits: usize, cancel: Option<&AtomicBool>) -> Result<Plan, &'static str>
{
    if !(1..=32).contains(&width) {
        return Err("Choose a tab width between 1 and 32 before changing line indent");
    }
    let selection = selection.clamped(rope);
    let ordered = selection.ordered();
    let rows = selected_rows(rope, selection);
    if rows > max_edits {
        return Err("Select at most 4096 lines to change their indentation");
    }
    let first = line_at(rope, ordered.start);
    // An excluded next-line start needs no lookup of that unselected line.
    // In particular, its Unicode separators must not trigger a large scan.
    let last = if ordered.end > ordered.start && physical_line_start(rope, ordered.end) {
        Line { start: ordered.end, body_end: ordered.end, end: ordered.end }
    } else { line_at(rope, ordered.end) };
    let multiline = first.start != last.start;
    let end = if multiline && last.start == ordered.end {
        ordered.end
    } else {
        last.end
    };
    let mut at = first.start;
    let mut edits = Vec::new();
    let mut allocated = 0usize;
    loop {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) { return Err("Line indentation cancelled"); }
        let line = line_at(rope, at);
        let mut prefix_end = line.start;
        let mut columns = 0usize;
        if let Some(bytes) = rope.get_bytes_at(line.start) {
            for byte in bytes {
                columns = match byte {
                    b' ' => columns.saturating_add(1),
                    b'\t' => columns.saturating_add(width - columns % width),
                    _ => break,
                };
                prefix_end = prefix_end.saturating_add(1);
                if columns > MAX_PREFIX_BYTES {
                    return Err("Line indentation is too large to change safely");
                }
            }
        }
        // Scintilla skips completely empty lines during a multi-line indent.
        if !increase || !multiline || line.body_end != line.start {
            let next = if increase {
                columns.saturating_add(width)
            } else {
                columns.saturating_sub(width)
            };
            if next > MAX_PREFIX_BYTES {
                return Err("Line indentation is too large to change safely");
            }
            let (tab_count, spaces) = if tabs {
                (next / width, next % width)
            } else {
                (0, next)
            };
            let length = tab_count.saturating_add(spaces);
            allocated = allocated.saturating_add(length);
            if allocated > MAX_EDIT_BYTES || edits.len() >= max_edits {
                return Err("Select fewer lines to change their indentation");
            }
            // The worker's legacy history and both offset maps also cost
            // memory. Bound the plan before allocating millions of records.
            if cancel.is_some() {
                crate::buffer::prepared::history_record_budget(edits.len().saturating_add(1), edits.capacity())?;
            }
            let mut text = String::new();
            text.try_reserve_exact(length)
                .map_err(|_| "Not enough memory to change line indentation")?;
            text.extend(std::iter::repeat_n('\t', tab_count));
            text.extend(std::iter::repeat_n(' ', spaces));
            let unchanged = rope
                .get_byte_slice(line.start..prefix_end)
                .is_some_and(|prefix| prefix.bytes().eq(text.bytes()));
            if !unchanged {
                if cancel.is_some() && edits.len() == edits.capacity() {
                    let capacity = edits.capacity().checked_mul(2).ok_or("Not enough memory to change line indentation")?.max(4);
                    crate::buffer::prepared::history_record_budget(edits.len().saturating_add(1), capacity)?;
                    edits.try_reserve_exact(capacity.saturating_sub(edits.len()))
                        .map_err(|_| "Not enough memory to change line indentation")?;
                } else {
                    edits.try_reserve(1).map_err(|_| "Not enough memory to change line indentation")?;
                }
                edits.push(Replacement {
                    range: line.start..prefix_end,
                    text,
                });
            }
        }
        if !multiline || line.end >= end || line.end <= at {
            break;
        }
        at = line.end;
    }
    let next_selection = if multiline && !edits.is_empty() {
        let start = first.start;
        let end = map_prefix(end, &edits);
        if selection.anchor <= selection.head {
            Selection {
                anchor: start,
                head: end,
                is_dragging: false,
            }
        } else {
            Selection {
                anchor: end,
                head: start,
                is_dragging: false,
            }
        }
    } else {
        Selection {
            anchor: map_prefix(selection.anchor, &edits),
            head: map_prefix(selection.head, &edits),
            is_dragging: false,
        }
    };
    Ok(Plan {
        edits,
        selection: next_selection,
    })
}

pub(crate) fn prepare_indentation(rope: Rope, revision: ContentRevision, selection: Selection,
    tabs: bool, width: usize, increase: bool, cancel: &AtomicBool) -> Result<PreparedGroup, &'static str>
{
    let plan = indent_plan_bounded(&rope, selection, tabs, width, increase, usize::MAX, Some(cancel))?;
    PreparedGroup::build(rope, revision, selection, plan.selection, plan.edits, cancel)
}

fn duplicate_plan(rope: &Rope, selection: Selection, eol: &str) -> Result<Plan, &'static str> {
    let selection = selection.clamped(rope);
    let line = line_at(rope, selection.head);
    let length = line
        .body_end
        .saturating_sub(line.start)
        .saturating_add(eol.len());
    if length > MAX_DUPLICATE_BYTES {
        return Err("Current line is too large to duplicate safely");
    }
    let mut text = String::new();
    text.try_reserve_exact(length)
        .map_err(|_| "Not enough memory to duplicate the current line")?;
    text.push_str(eol);
    if let Some(body) = rope.get_byte_slice(line.start..line.body_end) {
        for chunk in body.chunks() {
            text.push_str(chunk);
        }
    } else {
        return Err("The current line changed; try duplicating it again");
    }
    // Document insertion keeps a caret at the original line. A selection's
    // start at the insertion point follows its selected text into the copy.
    let selection_start = selection.anchor.min(selection.head);
    let move_endpoint = |position: usize| {
        if position > line.body_end
            || (position == line.body_end && !selection.is_empty() && position == selection_start)
        {
            position.saturating_add(text.len())
        } else {
            position
        }
    };
    let next_selection = Selection {
        anchor: move_endpoint(selection.anchor),
        head: move_endpoint(selection.head),
        is_dragging: false,
    };
    Ok(Plan {
        edits: vec![Replacement {
            range: line.body_end..line.body_end,
            text,
        }],
        selection: next_selection,
    })
}

impl Workspace {
    pub(crate) fn increase_line_indent(
        &mut self,
        _: &EditIncreaseLineIndent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_menu_line_command(MenuLineCommand::Increase, window, cx);
    }
    pub(crate) fn decrease_line_indent(
        &mut self,
        _: &EditDecreaseLineIndent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_menu_line_command(MenuLineCommand::Decrease, window, cx);
    }
    pub(crate) fn duplicate_current_line(
        &mut self,
        _: &EditDuplicateCurrentLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_menu_line_command(MenuLineCommand::Duplicate, window, cx);
    }
    pub(crate) fn run_menu_line_command(
        &mut self,
        command: MenuLineCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let refusal = if tab.view_mode != TabViewMode::Edit {
            Some("Switch to Source or Raw View to use line commands")
        } else if !window.is_window_active()
            || !self.focus_handle.is_focused(window)
            || self.input_focus(window) != InputFocus::Editor
        {
            Some("Focus the active Source or Raw View editor to use line commands")
        } else if tab.buffer.loading.is_some() {
            Some("Wait for the file to finish loading before using line commands")
        } else if tab.buffer.read_only {
            Some("Buffer is read-only")
        } else if tab.block_selection.is_some() {
            Some("Use a single caret or a normal text selection for these line commands")
        } else {
            None
        };
        if let Some(message) = refusal {
            self.set_status(message);
            cx.notify();
            return false;
        }
        let selection = self.selection();
        if matches!(command, MenuLineCommand::Increase | MenuLineCommand::Decrease)
            && {
                let rows = selected_rows(&tab.buffer.rope, selection);
                let annotations = tab.buffer.bookmarks.len().saturating_add(tab.buffer.marks.len());
                rows > MAX_LINE_EDITS
                    || rows.saturating_mul(annotations) > MAX_FOREGROUND_ANNOTATION_VISITS
                    || !foreground_line_work_bounded(&tab.buffer.rope, selection)
            }
        {
            if macros::execution_active() || tab.edit_transaction.is_some() {
                self.set_status("Large line indentation is unavailable during macro recording, playback or an open edit group");
                cx.notify();
                return false;
            }
            return self.start_prepared_indent(matches!(command, MenuLineCommand::Increase), window, cx);
        }
        let result = match command {
            MenuLineCommand::Duplicate => {
                duplicate_plan(&tab.buffer.rope, selection, tab.buffer.line_ending.as_str())
            }
            MenuLineCommand::Increase | MenuLineCommand::Decrease => {
                let (tabs, width) = crate::settings::indent_for_lang(
                    tab.buffer.language.map(|language| language.key()),
                );
                indent_plan(
                    &tab.buffer.rope,
                    selection,
                    tabs,
                    width,
                    matches!(command, MenuLineCommand::Increase),
                )
            }
        };
        let plan = match result {
            Ok(plan) => plan,
            Err(message) => {
                self.set_status(message);
                cx.notify();
                return false;
            }
        };
        if plan.edits.is_empty() {
            return true;
        }
        self.begin_edit_group();
        for edit in plan.edits.iter().rev() {
            self.replace_byte_range(edit.range.clone(), &edit.text, cx);
        }
        self.active_tab_mut().selection = plan.selection;
        self.end_edit_group(cx);
        self.commit_caret(cx);
        macros::record(command.event());
        true
    }
}

#[cfg(test)]
#[path = "menu_line_commands_tests.rs"]
mod tests;
