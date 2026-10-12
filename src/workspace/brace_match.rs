// PowerEditor/src/Notepad_plus.cpp:3107-3169; Scintilla Document.cxx:3034-3070.
// Original worker matcher. HighlightKind is coarser than a Scintilla lexer style.
use crate::syntax::{HighlightKind, HighlightSpan};
use ropey::Rope;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) const MAX_STYLE_SPANS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pair {
    pub(super) origin: usize,
    pub(super) partner: usize,
}

impl Pair {
    pub(super) fn ordered(self) -> (usize, usize) {
        (self.origin.min(self.partner), self.origin.max(self.partner))
    }
}

fn bracket(byte: u8) -> Option<(u8, bool)> {
    match byte {
        b'(' => Some((b')', true)), b')' => Some((b'(', false)),
        b'[' => Some((b']', true)), b']' => Some((b'[', false)),
        b'{' => Some((b'}', true)), b'}' => Some((b'{', false)),
        _ => None,
    }
}

pub(super) fn origin(rope: &Rope, head: usize) -> Option<usize> {
    if head > rope.len_bytes() || crate::buffer::floor_char_boundary(rope, head) != head { return None; }
    if let Some(before) = head.checked_sub(1).filter(|&at| rope.get_byte(at).and_then(bracket).is_some()) {
        return Some(before);
    }
    rope.get_byte(head).and_then(bracket).map(|_| head)
}

fn style_at(spans: &[HighlightSpan], byte: usize) -> Option<HighlightKind> {
    let index = spans.partition_point(|span| span.range.end <= byte);
    spans.get(index).filter(|span| span.range.contains(&byte)).map(|span| span.kind)
}

pub(super) fn locate(rope: &Rope, head: usize, spans: &[HighlightSpan], cancel: &AtomicBool) -> Result<Option<Pair>, &'static str> {
    if cancel.load(Ordering::Relaxed) { return Err("Brace matching cancelled"); }
    let Some(origin) = origin(rope, head) else { return Ok(None); };
    if spans.len() > MAX_STYLE_SPANS { return Err("Brace matching supports at most 1,000,000 syntax spans"); }
    let mut end = 0;
    for (index, span) in spans.iter().enumerate() {
        if index.is_multiple_of(1024) && cancel.load(Ordering::Relaxed) { return Err("Brace matching cancelled"); }
        if span.range.start < end || span.range.start >= span.range.end || span.range.end > rope.len_bytes()
            || crate::buffer::floor_char_boundary(rope, span.range.start) != span.range.start
            || crate::buffer::floor_char_boundary(rope, span.range.end) != span.range.end {
            return Err("Brace matching requires current, ordered syntax spans");
        }
        end = span.range.end;
    }
    let Some(byte) = rope.get_byte(origin) else { return Ok(None); };
    let Some((target, forward)) = bracket(byte) else { return Ok(None); };
    let style = style_at(spans, origin);
    let start = if forward { origin.checked_add(1).ok_or("Brace offset is too large")? } else { origin };
    let mut iter = rope.bytes_at(start);
    let mut at = start;
    let mut depth = 1usize;
    let mut visited = 0usize;
    loop {
        if visited.is_multiple_of(65_536) && cancel.load(Ordering::Relaxed) { return Err("Brace matching cancelled"); }
        let next = if forward { iter.next() } else { iter.prev() };
        let Some(next) = next else { break; };
        if !forward { at = at.checked_sub(1).ok_or("Brace offset is invalid")?; }
        #[cfg(test)] tests::visited_byte();
        if (next == byte || next == target) && style_at(spans, at) == style {
            if next == byte { depth = depth.checked_add(1).ok_or("Brace nesting is too deep")?; }
            else {
                depth = depth.checked_sub(1).ok_or("Brace nesting is invalid")?;
                if depth == 0 { return Ok(Some(Pair { origin, partner: at })); }
            }
        }
        if forward { at = at.checked_add(1).ok_or("Brace offset is too large")?; }
        visited = visited.saturating_add(1);
    }
    if cancel.load(Ordering::Relaxed) { return Err("Brace matching cancelled"); }
    Ok(None)
}

#[cfg(test)]
#[path = "brace_match_tests.rs"]
pub(super) mod tests;
