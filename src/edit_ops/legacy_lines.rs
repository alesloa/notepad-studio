// PowerEditor/src/NppCommands.cpp:933–1063; ScintillaEditView.cpp:4474–4517.
// Original bounded implementation of the older line-operation contracts.
use std::{cmp::Ordering, ops::Range, sync::atomic::AtomicBool};
use ropey::Rope;
use gpui::{Context, Window};
use crate::{buffer::{BlockSelection, ContentRevision, Selection, prepared::{PrefixEdit, PreparedGroup}},
    macros::MacroEvent, workspace::Workspace};
use super::{line_sort::{MAX_SORT_BYTES, SortFormat, basic_columns, check_cancel, lines_in, merge_order, selected_span},
    legacy_numeric::{DecimalError, compare_units, decimal_key, natural_compare}};

const MEMORY: &str = "Not enough memory to prepare line operations";

#[derive(Clone, Copy, Debug)]
pub(crate) enum LegacyCommand { LexAsc, LexDesc, IntAsc, IntDesc, CommaAsc, CommaDesc, DotAsc, DotDesc, Reverse, Randomize, AnyDuplicate, ConsecutiveDuplicate }
impl LegacyCommand {
    pub(crate) fn event(self) -> MacroEvent {
        match self { Self::LexAsc => MacroEvent::SortLinesLexAsc, Self::LexDesc => MacroEvent::SortLinesLexDesc,
            Self::IntAsc => MacroEvent::SortLinesIntAsc, Self::IntDesc => MacroEvent::SortLinesIntDesc,
            Self::CommaAsc => MacroEvent::SortLinesDecCommaAsc, Self::CommaDesc => MacroEvent::SortLinesDecCommaDesc,
            Self::DotAsc => MacroEvent::SortLinesDecDotAsc, Self::DotDesc => MacroEvent::SortLinesDecDotDesc,
            Self::Reverse => MacroEvent::ReverseLineOrder, Self::Randomize => MacroEvent::RandomizeLineOrder,
            Self::AnyDuplicate => MacroEvent::RemoveDuplicateLines, Self::ConsecutiveDuplicate => MacroEvent::RemoveConsecutiveDuplicateLines }
    }
    fn descending(self) -> bool { matches!(self, Self::LexDesc | Self::IntDesc | Self::CommaDesc | Self::DotDesc) }
    fn separator(self) -> Option<u16> {
        match self { Self::CommaAsc | Self::CommaDesc => Some(44), Self::DotAsc | Self::DotDesc => Some(46), _ => None }
    }
    fn duplicates(self) -> bool { matches!(self,Self::AnyDuplicate | Self::ConsecutiveDuplicate) }
}

pub(crate) struct LegacyRequest { pub(crate) command: LegacyCommand, pub(crate) seed: u64 }

pub(crate) enum LegacyError { Refused(&'static str), Numeric { row: usize } }
impl From<&'static str> for LegacyError { fn from(message: &'static str) -> Self { Self::Refused(message) } }
impl LegacyError {
    pub(crate) fn message(self) -> String {
        match self { Self::Refused(message) => message.into(),
            Self::Numeric { row } => format!("Unable to convert line {row} into a number; no changes applied") }
    }
}

fn shuffle(order: &mut [usize], mut seed: u64, cancel: &AtomicBool) -> Result<(), &'static str> {
    if seed == 0 { seed = 0xa5a5_5a5a_a5a5_5a5a; }
    for at in (1..order.len()).rev() {
        if at.is_multiple_of(1024) { check_cancel(cancel)?; }
        seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
        let bound = at.checked_add(1).ok_or("Line shuffle position overflow")?;
        let other = (seed as usize) % bound;
        let a = *order.get(at).ok_or("Invalid line shuffle position")?;
        let b = *order.get(other).ok_or("Invalid line shuffle position")?;
        *order.get_mut(at).ok_or("Invalid line shuffle position")? = b;
        *order.get_mut(other).ok_or("Invalid line shuffle position")? = a;
    }
    Ok(())
}

fn seed() -> u64 {
    #[cfg(test)]
    if let Some(seed) = TEST_SEED.with(|value| value.get()) { return seed; }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_nanos() as u64 ^ time.as_secs().wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .unwrap_or(0xdead_beef_cafe_f00d)
}
#[cfg(test)]
thread_local! { static TEST_SEED: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(crate) struct SeedGuard { before: Option<u64>, thread: std::marker::PhantomData<std::rc::Rc<()>> }
#[cfg(test)]
impl Drop for SeedGuard { fn drop(&mut self) { TEST_SEED.with(|slot| slot.set(self.before)); } }
#[cfg(test)]
pub(crate) fn override_seed(value: u64) -> SeedGuard { SeedGuard { before: TEST_SEED.with(|slot| slot.replace(Some(value))), thread: std::marker::PhantomData } }

// Geometry always addresses the internal UTF8 Rope, including ANSI documents.
// Key column offsets separately follow the older sorter's encoding policy.
fn corner(rope: &Rope, line: usize, column: usize, cancel: &AtomicBool) -> Result<(usize, usize), &'static str> {
    let row = rope.get_line(line).ok_or("Rectangular line operation rows no longer match the document")?;
    let (mut bytes, mut chars) = (0usize, 0usize);
    for c in row.chars() {
        if chars == column || matches!(c, '\r' | '\n') { break; }
        if chars.is_multiple_of(8192) { check_cancel(cancel)?; }
        bytes = bytes.checked_add(c.len_utf8()).ok_or("Rectangular line operation position overflow")?;
        chars = chars.saturating_add(1);
    }
    let at = rope.line_to_byte(line).checked_add(bytes).ok_or("Rectangular line operation position overflow")?;
    let virtual_space = column.saturating_sub(chars);
    at.checked_add(virtual_space).ok_or("Rectangular line operation position overflow")?;
    Ok((at, virtual_space))
}

fn collapsed_rectangle(before: &Rope, block: BlockSelection, range: &Range<usize>, cancel: &AtomicBool) -> Result<BlockSelection, &'static str> {
    let (anchor, _) = corner(before, block.anchor_line, block.anchor_col, cancel)?;
    let (head, _) = corner(before, block.head_line, block.head_col, cancel)?;
    if anchor < range.start || head < range.start || anchor > range.end || head > range.end {
        return Err("Rectangular line operation endpoints no longer match the selected rows");
    }
    let line = before.byte_to_line(range.start);
    Ok(BlockSelection { anchor_line: line, head_line: line, anchor_col: 0, head_col: 0, is_dragging: false })
}

fn deleted_corner(mut position: (usize,usize), edits: &[PrefixEdit], cancel: &AtomicBool) -> Result<(usize,usize), &'static str> {
    for edit in edits.iter().rev() {
        check_cancel(cancel)?;
        if position.0 > edit.range.end {
            position.0 = position.0.checked_sub(edit.range.len()).ok_or("Rectangular deletion position overflow")?;
        } else if position.0 >= edit.range.start { position = (edit.range.start,0); }
    }
    Ok(position)
}

fn position_corner(rope: &Rope, position: (usize,usize), cancel: &AtomicBool) -> Result<(usize,usize), &'static str> {
    let (at,virtual_space) = position;
    if at > rope.len_bytes() || crate::buffer::floor_char_boundary(rope,at) != at {
        return Err("Rectangular deletion endpoint no longer matches the document");
    }
    let line = rope.byte_to_line(at);
    let prefix = rope.get_byte_slice(rope.line_to_byte(line)..at).ok_or("Invalid rectangular deletion endpoint")?;
    let mut column = 0usize;
    for c in prefix.chars() {
        if matches!(c,'\r'|'\n') { break; }
        if column.is_multiple_of(8192) { check_cancel(cancel)?; }
        column = column.checked_add(1).ok_or("Rectangular deletion column overflow")?;
    }
    Ok((line,column.checked_add(virtual_space).ok_or("Rectangular deletion column overflow")?))
}

pub(crate) fn prepare_legacy(rope: Rope, revision: ContentRevision, selection: Selection,
    block: Option<BlockSelection>, request: LegacyRequest, format: SortFormat<'_>,
    cancel: &AtomicBool) -> Result<PreparedGroup, LegacyError>
{
    let LegacyRequest { command, seed } = request;
    check_cancel(cancel)?;
    let selection = selection.clamped(&rope);
    if let Some(block) = block {
        corner(&rope, block.anchor_line, block.anchor_col, cancel)?;
        corner(&rope, block.head_line, block.head_col, cancel)?;
    }
    let mut edits = Vec::new(); let mut after = None; let mut after_block = block; let mut deleted_corners = None;
    let span = if command.duplicates() {
        let selected = if let Some(block) = block {
            let a = corner(&rope,block.anchor_line,block.anchor_col,cancel)?.0;
            let b = corner(&rope,block.head_line,block.head_col,cancel)?.0;
            Some(a.min(b)..a.max(b))
        } else if selection.is_empty() { None } else { Some(selection.ordered()) };
        super::line_ops::line_span_in(&rope,selected).filter(|span| span.first_line != span.last_line)
    } else { selected_span(&rope,selection,block) };
    if let Some(span) = span {
        let range = span.block_start..span.block_end;
        if range.len() > MAX_SORT_BYTES { return Err("Select at most 32 MiB of text for line operations".into()); }
        let mut captured = String::new(); captured.try_reserve_exact(range.len()).map_err(|_| MEMORY)?;
        let slice = rope.get_byte_slice(range.clone()).ok_or("Invalid line operation range")?;
        for chunk in slice.chunks() { check_cancel(cancel)?; captured.push_str(chunk); }
        let lines = lines_in(&captured, span.reaches_doc_end, cancel)?;
        if lines.len() >= 2 {
            if command.duplicates() {
                edits = if matches!(command,LegacyCommand::AnyDuplicate) {
                    super::legacy_dedupe::any_duplicate(&captured,&lines,range.start,span.after_cr,cancel)?
                } else { super::legacy_dedupe::consecutive(&captured,&lines,range.start,span.after_cr,cancel)? };
                if let Some(block) = block.filter(|_| !edits.is_empty()) {
                    if matches!(command,LegacyCommand::AnyDuplicate) {
                        after_block = Some(collapsed_rectangle(&rope,block,&range,cancel)?);
                    } else {
                        deleted_corners = Some((deleted_corner(corner(&rope,block.anchor_line,block.anchor_col,cancel)?,&edits,cancel)?,
                            deleted_corner(corner(&rope,block.head_line,block.head_col,cancel)?,&edits,cancel)?));
                    }
                }
            } else {
                let mut order = Vec::new(); order.try_reserve_exact(lines.len()).map_err(|_| MEMORY)?;
                order.extend(0..lines.len());
                match command {
                    LegacyCommand::LexAsc | LegacyCommand::LexDesc | LegacyCommand::IntAsc | LegacyCommand::IntDesc
                        | LegacyCommand::CommaAsc | LegacyCommand::CommaDesc | LegacyCommand::DotAsc | LegacyCommand::DotDesc => {
                        let (from, to) = basic_columns(&rope, block, format.encoding, cancel)?;
                        let mut units = Vec::new(); units.try_reserve_exact(captured.len()).map_err(|_| MEMORY)?;
                        let mut keys = Vec::new(); keys.try_reserve_exact(lines.len()).map_err(|_| MEMORY)?;
                        for line in &lines {
                            let start = units.len();
                            for (index, unit) in line.body.encode_utf16().enumerate() {
                                if index.is_multiple_of(8192) { check_cancel(cancel)?; }
                                if to != 0 && from != to && index >= to { break; }
                                if to == 0 || index >= from { units.push(unit); }
                            }
                            keys.push(start..units.len());
                        }
                        if let Some(separator) = command.separator() {
                            let mut numbers = Vec::new(); numbers.try_reserve_exact(lines.len()).map_err(|_| MEMORY)?;
                            for (index,key) in keys.iter().enumerate() {
                                check_cancel(cancel)?;
                                let key = units.get(key.clone()).ok_or("Invalid decimal key")?;
                                let number = match decimal_key(key,separator,cancel) {
                                    Ok(number) => number,
                                    Err(DecimalError::Refused(message)) => return Err(message.into()),
                                    Err(DecimalError::Invalid) => return Err(LegacyError::Numeric {
                                        row: span.first_line.checked_add(index).and_then(|n| n.checked_add(1)).ok_or("Decimal error row overflow")? }),
                                };
                                numbers.push(number);
                            }
                            merge_order(&mut order, |a,b| {
                                let a = numbers.get(a).ok_or("Invalid decimal key")?; let b = numbers.get(b).ok_or("Invalid decimal key")?;
                                let order = match (a,b) { (None,None) => Ordering::Equal, (None,Some(_)) => Ordering::Less,
                                    (Some(_),None) => Ordering::Greater,
                                    (Some(a),Some(b)) => a.partial_cmp(b).ok_or("Invalid prepared decimal key")? };
                                Ok(if command.descending() { order.reverse() } else { order })
                            },cancel)?;
                        } else {
                            merge_order(&mut order, |a, b| {
                                let a = units.get(keys.get(a).ok_or("Invalid line key")?.clone()).ok_or("Invalid line key")?;
                                let b = units.get(keys.get(b).ok_or("Invalid line key")?.clone()).ok_or("Invalid line key")?;
                                let order = if matches!(command, LegacyCommand::IntAsc | LegacyCommand::IntDesc) { natural_compare(a,b,cancel)? }
                                    else { compare_units(a,b,cancel)? };
                                Ok(if command.descending() { order.reverse() } else { order })
                            }, cancel)?;
                        }
                    }
                    LegacyCommand::Reverse => order.reverse(),
                    LegacyCommand::Randomize => shuffle(&mut order, seed, cancel)?,
                    LegacyCommand::AnyDuplicate | LegacyCommand::ConsecutiveDuplicate => return Err("Duplicate removal cannot permute lines".into()),
                }
                let mut text = String::new(); text.try_reserve_exact(captured.len()).map_err(|_| MEMORY)?;
                let mut previous_cr = span.after_cr; let mut body_end = range.start;
                for (position, index) in order.into_iter().enumerate() {
                    check_cancel(cancel)?;
                    let line = lines.get(index).ok_or("Invalid line permutation")?;
                    let eol = lines.get(position).ok_or("Invalid line terminator")?.eol;
                    if previous_cr && line.body.is_empty() && eol.starts_with('\n') {
                        return Err("Line operation skipped: mixed CR and LF endings would merge two lines".into());
                    }
                    text.push_str(line.body); body_end = range.start.saturating_add(text.len()); text.push_str(eol);
                    if !(line.body.is_empty() && eol.is_empty()) { previous_cr = eol == "\r"; }
                }
                let changed = text != captured;
                if let Some(block) = block {
                    if changed {
                        let mapped = collapsed_rectangle(&rope, block, &range, cancel)?;
                        after = Some(Selection { anchor:range.start,head:range.start,is_dragging:false }); after_block = Some(mapped);
                    }
                } else if !selection.is_empty() {
                    after = Some(Selection { anchor: range.start, head: body_end, is_dragging: false });
                } else if changed { after = Some(Selection { anchor:range.start,head:range.start,is_dragging:false }); }
                if changed { edits.try_reserve_exact(1).map_err(|_| MEMORY)?; edits.push(PrefixEdit { range, text }); }
            }
        }
    }
    let mut group = PreparedGroup::build_line_edits(rope, revision, selection, edits, false, cancel)?;
    if let Some(after) = after { group.set_after_selection(after); }
    if let Some((anchor,head)) = deleted_corners {
        let rope = group.target(false).0;
        let (anchor_line,anchor_col) = position_corner(&rope,anchor,cancel)?;
        let (head_line,head_col) = position_corner(&rope,head,cancel)?;
        after_block = Some(BlockSelection { anchor_line,head_line,anchor_col,head_col,is_dragging:false });
    }
    group.set_block_history(block, after_block)?;
    Ok(group)
}

impl Workspace {
    pub(crate) fn run_legacy_line(&mut self, command: LegacyCommand, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.legacy_line_available(window, cx) { return false; }
        self.start_prepared_legacy(command, seed(), window, cx)
    }
}
