// PowerEditor/src/NppCommands.cpp:933–1078, MISC/Common/Sorters.h:130–176,
// 660–706, MISC/Common/SortLocale.cpp:28–332 — the six additional line sorts.
// Behaviour studied; this Rust implementation is original.

use std::{cmp::Ordering, ops::Range, sync::atomic::{AtomicBool, Ordering as AtomicOrdering}};
use gpui::{Context, Window};
use ropey::Rope;
use crate::{actions::*, buffer::{BlockSelection, ContentRevision, Selection, prepared::{PrefixEdit, PreparedGroup}},
    macros::MacroEvent, tab::{InputFocus, TabViewMode}, workspace::Workspace};
use crate::text::encoding::Encoding;
use super::line_ops::line_span_in;

pub(crate) const MAX_SORT_BYTES: usize = 32 * 1024 * 1024;
const MAX_SORT_LINES: usize = 1_048_576;
const MEMORY_ERROR: &str = "Not enough memory to sort lines";
const CANCELLED: &str = "Line sorting cancelled; no changes applied";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SortCommand {
    IgnoreCaseAsc, IgnoreCaseDesc, LocaleAsc, LocaleDesc, LengthAsc, LengthDesc,
}

impl SortCommand {
    fn descending(self) -> bool {
        matches!(self, Self::IgnoreCaseDesc | Self::LocaleDesc | Self::LengthDesc)
    }
    pub(crate) fn locale(self) -> bool { matches!(self, Self::LocaleAsc | Self::LocaleDesc) }
    pub(crate) fn event(self) -> MacroEvent {
        match self {
            Self::IgnoreCaseAsc => MacroEvent::SortLinesLexIgnoreCaseAsc,
            Self::IgnoreCaseDesc => MacroEvent::SortLinesLexIgnoreCaseDesc,
            Self::LocaleAsc => MacroEvent::SortLinesLocaleAsc,
            Self::LocaleDesc => MacroEvent::SortLinesLocaleDesc,
            Self::LengthAsc => MacroEvent::SortLinesLengthAsc,
            Self::LengthDesc => MacroEvent::SortLinesLengthDesc,
        }
    }
}

pub(super) struct Line<'a> { pub(super) body: &'a str, pub(super) eol: &'a str, pub(super) start: usize }
pub(crate) struct SortPlan { pub(crate) range: Range<usize>, pub(crate) text: String, pub(crate) selection: Selection, changed: bool }
pub(crate) type LocaleKey = fn(&str) -> Result<Vec<u8>, &'static str>;
#[derive(Clone, Copy)]
pub(crate) struct SortFormat<'a> { pub(crate) eol: &'a str, pub(crate) encoding: Encoding }

pub(super) fn check_cancel(cancel: &AtomicBool) -> Result<(), &'static str> {
    if cancel.load(AtomicOrdering::Relaxed) { Err(CANCELLED) } else { Ok(()) }
}

pub(super) fn lines_in<'a>(text: &'a str, keep_final: bool, cancel: &AtomicBool) -> Result<Vec<Line<'a>>, &'static str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let (mut start, mut at) = (0usize, 0usize);
    let mut next_poll = 0usize;
    loop {
        if at >= next_poll { check_cancel(cancel)?; next_poll = at.saturating_add(8192); }
        let byte = bytes.get(at).copied();
        if byte.is_none() || matches!(byte, Some(b'\r' | b'\n')) {
            let mut end = at;
            if byte.is_some() {
                end = end.saturating_add(1);
                if byte == Some(b'\r') && bytes.get(end) == Some(&b'\n') { end = end.saturating_add(1); }
            }
            if byte.is_some() || start < at || keep_final {
                if lines.len() >= MAX_SORT_LINES { return Err("Select fewer than 1,048,576 lines to sort"); }
                if lines.len() == lines.capacity() { lines.try_reserve(256).map_err(|_| MEMORY_ERROR)?; }
                let body = text.get(start..at).ok_or("Invalid line sorting range")?;
                let eol = text.get(at..end).ok_or("Invalid line sorting range")?;
                lines.push(Line { body, eol, start });
            }
            if byte.is_none() { break; }
            at = end;
            start = end;
        } else { at = at.saturating_add(1); }
    }
    Ok(lines)
}

// Notepad++ does not change the CRT's C character locale: _wcsicmp folds
// ASCII A–Z and compares UTF-16 units, stopping at the first NUL.
fn ignore_case_cmp(left: &[u16], right: &[u16], cancel: &AtomicBool) -> Result<Ordering, &'static str> {
    let fold = |unit: u16| if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) { unit + 32 } else { unit };
    let mut left = left.iter().copied().take_while(|&unit| unit != 0).map(fold);
    let mut right = right.iter().copied().take_while(|&unit| unit != 0).map(fold);
    let mut count = 0usize;
    loop {
        if count.is_multiple_of(8192) { check_cancel(cancel)?; }
        count = count.saturating_add(1);
        let a = left.next(); let b = right.next();
        let order = a.cmp(&b);
        if order != Ordering::Equal || a.is_none() { return Ok(order); }
    }
}

pub(super) fn merge_order(order: &mut Vec<usize>, mut compare: impl FnMut(usize, usize) -> Result<Ordering, &'static str>,
    cancel: &AtomicBool) -> Result<(), &'static str>
{
    let mut scratch = Vec::new();
    scratch.try_reserve_exact(order.len()).map_err(|_| MEMORY_ERROR)?;
    scratch.extend(order.iter().copied());
    let mut width = 1usize;
    while width < order.len() {
        check_cancel(cancel)?;
        let mut start = 0usize;
        while start < order.len() {
            let middle = start.saturating_add(width).min(order.len());
            let end = middle.saturating_add(width).min(order.len());
            let (mut left, mut right, mut out) = (start, middle, start);
            while left < middle || right < end {
                if out.is_multiple_of(1024) { check_cancel(cancel)?; }
                let take_left = if right >= end { true } else if left >= middle { false }
                    else { compare(*order.get(left).ok_or("Invalid line sort order")?,
                        *order.get(right).ok_or("Invalid line sort order")?)? != Ordering::Greater };
                let index = if take_left { let value = left; left += 1; value } else { let value = right; right += 1; value };
                let value = *order.get(index).ok_or("Invalid line sort order")?;
                let slot = scratch.get_mut(out).ok_or("Invalid line sort order")?;
                *slot = value; out += 1;
            }
            start = end;
        }
        std::mem::swap(order, &mut scratch);
        width = width.saturating_mul(2);
    }
    Ok(())
}

fn sort_key<'a>(body: &'a str, block: Option<BlockSelection>, cancel: &AtomicBool) -> Result<&'a str, &'static str> {
    let Some(block) = block else { return Ok(body); };
    let from = block.min_col();
    let to = block.max_col();
    if to == 0 { return Ok(body); }
    let (mut start, mut end) = (body.len(), body.len());
    for (column, (byte, _)) in body.char_indices().enumerate() {
        if column.is_multiple_of(8192) { check_cancel(cancel)?; }
        if column == from { start = byte; }
        if from != to && column == to { end = byte; break; }
    }
    Ok(body.get(start..end).unwrap_or_default())
}

fn corner_offset(rope: &Rope, line: usize, column: usize, encoding: Encoding, cancel: &AtomicBool) -> Result<usize, &'static str> {
    let row = rope.get_line(line.min(rope.len_lines().saturating_sub(1))).ok_or("Invalid rectangular sort row")?;
    let mut bytes = 0usize;
    let mut chars = 0usize;
    for c in row.chars() {
        if chars == column || matches!(c, '\r' | '\n') { break; }
        if chars.is_multiple_of(8192) { check_cancel(cancel)?; }
        bytes = bytes.saturating_add(if encoding == Encoding::Latin1 { 1 } else { c.len_utf8() });
        chars = chars.saturating_add(1);
    }
    Ok(bytes.saturating_add(column.saturating_sub(chars)))
}

pub(super) fn basic_columns(rope: &Rope, block: Option<BlockSelection>, encoding: Encoding, cancel: &AtomicBool) -> Result<(usize, usize), &'static str> {
    let Some(block) = block else { return Ok((0, 0)); };
    // The older Notepad++ sorters use Scintilla byte offsets (plus virtual
    // space) as UTF16 indices, even when the corners contain multibyte text.
    let anchor = corner_offset(rope, block.anchor_line, block.anchor_col, encoding, cancel)?;
    let head = corner_offset(rope, block.head_line, block.head_col, encoding, cancel)?;
    Ok((anchor.min(head), anchor.max(head)))
}

pub(super) fn selected_span(rope: &Rope, selection: Selection, block: Option<BlockSelection>) -> Option<super::line_ops::LineSpan> {
    let Some(block) = block else { return line_span_in(rope, (!selection.is_empty()).then(|| selection.ordered())); };
    let last = rope.len_lines().saturating_sub(1);
    let top = block.min_line().min(last);
    let bottom = block.max_line().min(last);
    let start = rope.line_to_byte(top);
    let end = if bottom < last { rope.line_to_byte(bottom.saturating_add(1)) } else { rope.len_bytes() };
    let mut span = line_span_in(rope, Some(start..end))?;
    if bottom == last && rope.line_to_byte(bottom) == end {
        // A rectangle explicitly includes the empty last row.
        span.last_line = bottom;
        span.reaches_doc_end = true;
    }
    Some(span)
}

pub(crate) fn sort_plan(rope: &Rope, selection: Selection, block_selection: Option<BlockSelection>, command: SortCommand,
    format: SortFormat<'_>, locale_key: Option<LocaleKey>, cancel: &AtomicBool) -> Result<Option<SortPlan>, &'static str>
{
    let SortFormat { eol: document_eol, encoding } = format;
    check_cancel(cancel)?;
    if command.locale() && !matches!(document_eol, "\r" | "\n" | "\r\n") { return Err("Invalid document line ending; no lines sorted"); }
    let selection = selection.clamped(rope);
    let Some(span) = selected_span(rope, selection, block_selection) else { return Ok(None); };
    let range = span.block_start..span.block_end;
    if range.len() > MAX_SORT_BYTES { return Err("Select at most 32 MiB of text to sort lines"); }
    let mut block = String::new();
    block.try_reserve_exact(range.len()).map_err(|_| MEMORY_ERROR)?;
    let slice = rope.get_byte_slice(range.clone()).ok_or("Invalid line sorting range")?;
    for chunk in slice.chunks() { check_cancel(cancel)?; block.push_str(chunk); }
    let lines = lines_in(&block, span.reaches_doc_end && (!command.locale() || block_selection.is_some()), cancel)?;
    if lines.len() < 2 { return Ok(None); }
    let mut units = Vec::new();
    let mut unit_ranges = Vec::new();
    let mut keys = Vec::new();
    if !command.locale() {
        let (from, to) = basic_columns(rope, block_selection, encoding, cancel)?;
        units.try_reserve_exact(block.len()).map_err(|_| MEMORY_ERROR)?;
        unit_ranges.try_reserve_exact(lines.len()).map_err(|_| MEMORY_ERROR)?;
        for line in &lines {
            let start = units.len();
            for (index, unit) in line.body.encode_utf16().enumerate() {
                if index.is_multiple_of(8192) { check_cancel(cancel)?; }
                if to != 0 && from != to && index >= to { break; }
                if to == 0 || index >= from { units.push(unit); }
            }
            unit_ranges.push(start..units.len());
        }
    }
    if command.locale() {
        let key = locale_key.ok_or("Windows locale sorting needs the native NLS bridge; no lines sorted")?;
        keys.try_reserve_exact(lines.len()).map_err(|_| MEMORY_ERROR)?;
        let mut key_bytes = 0usize;
        for line in &lines {
            check_cancel(cancel)?;
            let text_key = sort_key(line.body, block_selection, cancel)?;
            let value = key(text_key)?;
            key_bytes = key_bytes.checked_add(value.len()).ok_or(MEMORY_ERROR)?;
            if key_bytes > 64 * 1024 * 1024 { return Err("Locale sort keys exceed 64 MiB; select fewer lines"); }
            keys.push(value);
        }
    }
    let mut order = Vec::new();
    order.try_reserve_exact(lines.len()).map_err(|_| MEMORY_ERROR)?;
    order.extend(0..lines.len());
    merge_order(&mut order, |a, b| {
        let result = match command {
            SortCommand::IgnoreCaseAsc | SortCommand::IgnoreCaseDesc => ignore_case_cmp(
                units.get(unit_ranges.get(a).ok_or("Invalid line sort order")?.clone()).ok_or("Invalid line sort key")?,
                units.get(unit_ranges.get(b).ok_or("Invalid line sort order")?.clone()).ok_or("Invalid line sort key")?, cancel)?,
            SortCommand::LengthAsc | SortCommand::LengthDesc => unit_ranges.get(a).ok_or("Invalid line sort order")?.len().cmp(&unit_ranges.get(b).ok_or("Invalid line sort order")?.len()),
            SortCommand::LocaleAsc | SortCommand::LocaleDesc => keys.get(a).ok_or("Invalid line sort order")?.cmp(keys.get(b).ok_or("Invalid line sort order")?),
        };
        Ok(if command.descending() { result.reverse() } else { result })
    }, cancel)?;
    let mut text = String::new();
    text.try_reserve_exact(block.len().saturating_add(2)).map_err(|_| MEMORY_ERROR)?;
    let mut previous_cr = span.after_cr;
    let missing_eol = command.locale() && lines.last().is_some_and(|line| line.eol.is_empty());
    let mut caret = selection.head;
    let mut body_end = range.start;
    for (position, index) in order.iter().copied().enumerate() {
        check_cancel(cancel)?;
        let line = lines.get(index).ok_or("Invalid line sort order")?;
        let output_start = text.len();
        let line_start = range.start.saturating_add(line.start);
        let line_end = line_start.saturating_add(line.body.len()).saturating_add(line.eol.len());
        if command.locale() && selection.is_empty()
            && selection.head >= line_start
            && (selection.head < line_end || line.eol.is_empty() && selection.head == line_end)
        { caret = range.start.saturating_add(output_start).saturating_add(selection.head.saturating_sub(line_start)); }
        let eol = if command.locale() {
            if line.eol.is_empty() { document_eol } else { line.eol }
        } else { lines.get(position).ok_or("Invalid line sort order")?.eol };
        let eol = if command.locale() && missing_eol && position.saturating_add(1) == lines.len() { "" } else { eol };
        if previous_cr && line.body.is_empty() && eol.starts_with('\n') {
            return Err("Line sorting skipped: mixed CR and LF endings would merge two lines");
        }
        text.push_str(line.body);
        body_end = range.start.saturating_add(text.len());
        text.push_str(eol);
        if !(line.body.is_empty() && eol.is_empty()) { previous_cr = eol == "\r"; }
    }
    let changed = text != block;
    let mut after = selection;
    if block_selection.is_some() {
        // Rectangular corners describe columns, including virtual space.
        // The same corners remain valid after whole-line permutation.
    } else if selection.is_empty() {
        after.collapse_to(if command.locale() { caret } else if changed { range.start } else { selection.head });
        if after.head >= range.start && after.head <= range.start.saturating_add(text.len()) {
            let mut local = after.head.saturating_sub(range.start);
            while !text.is_char_boundary(local) { local = local.saturating_sub(1); }
            after.collapse_to(range.start.saturating_add(local));
        }
    } else {
        let end = if command.locale() { range.start.saturating_add(text.len()) } else { body_end };
        if !command.locale() || selection.anchor <= selection.head { after.anchor = range.start; after.head = end; }
        else { after.anchor = end; after.head = range.start; }
        after.is_dragging = false;
    }
    Ok(Some(SortPlan { range, text, selection: after, changed }))
}

pub(crate) fn prepare_sort(rope: Rope, revision: ContentRevision, selection: Selection,
    block: Option<BlockSelection>, command: SortCommand, format: SortFormat<'_>, cancel: &AtomicBool) -> Result<PreparedGroup, &'static str>
{
    #[cfg(target_os = "windows")]
    let locale_key = Some(crate::locale_sort_windows::sort_key as LocaleKey);
    #[cfg(not(target_os = "windows"))]
    let locale_key = None;
    let plan = sort_plan(&rope, selection, block, command, format, locale_key, cancel)?;
    let mut edits = Vec::new();
    let after_selection = if let Some(plan) = plan {
        if plan.changed {
            edits.try_reserve_exact(1).map_err(|_| MEMORY_ERROR)?;
            edits.push(PrefixEdit { range: plan.range, text: plan.text });
        }
        plan.selection
    } else { selection };
    let mut group = PreparedGroup::build_line_edits(rope, revision, selection, edits, false, cancel)?;
    group.set_after_selection(after_selection);
    Ok(group)
}

impl Workspace {
    pub(crate) fn run_line_sort(&mut self, command: SortCommand, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false; };
        let refusal = if tab.view_mode != TabViewMode::Edit { Some("Switch to Source or Raw View to sort lines") }
            else if !window.is_window_active() || !self.focus_handle.is_focused(window) || self.input_focus(window) != InputFocus::Editor
            { Some("Focus the active Source or Raw View editor to sort lines") }
            else if tab.buffer.read_only { Some("Buffer is read-only") }
            else if tab.buffer.loading.is_some() { Some("Wait for the file to finish loading before sorting") }
            else if self.is_composing_text(window) { Some("Finish the active composition before sorting") }
            else { None };
        if let Some(message) = refusal { self.set_status(message); cx.notify(); return false; }
        self.start_prepared_sort(command, window, cx)
    }

    pub(crate) fn sort_lines_lex_ignore_case_asc(&mut self, _: &EditSortLinesLexIgnoreCaseAsc, window: &mut Window, cx: &mut Context<Self>) { self.run_line_sort(SortCommand::IgnoreCaseAsc, window, cx); }
    pub(crate) fn sort_lines_lex_ignore_case_desc(&mut self, _: &EditSortLinesLexIgnoreCaseDesc, window: &mut Window, cx: &mut Context<Self>) { self.run_line_sort(SortCommand::IgnoreCaseDesc, window, cx); }
    pub(crate) fn sort_lines_locale_asc(&mut self, _: &EditSortLinesLocaleAsc, window: &mut Window, cx: &mut Context<Self>) { self.run_line_sort(SortCommand::LocaleAsc, window, cx); }
    pub(crate) fn sort_lines_locale_desc(&mut self, _: &EditSortLinesLocaleDesc, window: &mut Window, cx: &mut Context<Self>) { self.run_line_sort(SortCommand::LocaleDesc, window, cx); }
    pub(crate) fn sort_lines_length_asc(&mut self, _: &EditSortLinesLengthAsc, window: &mut Window, cx: &mut Context<Self>) { self.run_line_sort(SortCommand::LengthAsc, window, cx); }
    pub(crate) fn sort_lines_length_desc(&mut self, _: &EditSortLinesLengthDesc, window: &mut Window, cx: &mut Context<Self>) { self.run_line_sort(SortCommand::LengthDesc, window, cx); }

}

#[cfg(test)]
#[path = "line_sort_tests.rs"]
mod tests;
