// Convert Case to ▶ submenu — Proper / Sentence (force + blend), iNVERT,
// rANdOm. UPPERCASE and lowercase live in `workspace::edits`.
//
// Notepad++ source:
//   PowerEditor/src/ScintillaComponent/ScintillaEditView.cpp:3701
//     `ScintillaEditView::changeCase(strW, nbChars, TextCase)`
//
// Behaviour (per-codepoint scan over the selection text):
// - PROPER FORCE / BLEND
//     A letter is treated as a word-boundary letter when the previous char
//     is missing or non-alphanumeric. Boundary letters → upper. In FORCE,
//     non-boundary letters → lower. BLEND leaves them untouched. NPP's
//     single-quote exception keeps `it's` → `It's` (the `s` after `'` is
//     not promoted to a boundary because the char before `'` is alnum).
// - SENTENCE FORCE / BLEND
//     A sentence ends at `.`, `!`, `?` followed by non-alnum. The first
//     letter of the next sentence is promoted to upper. FORCE lowers the
//     rest, BLEND leaves them. NPP's standalone-`i` rule capitalises a
//     bare `i` between space/( /" and space/'.
// - INVERT — flip every alphabetic char's case.
// - RANDOM — coin-flip per alphabetic char. PRNG seeded fresh per call
//     from `SystemTime::now()` (xorshift64*; std-only — `rand` is not a
//     dep and we're not adding one for this).
//
// Selection rules match `case_convert` in `workspace::edits` — operate on
// the selection if any, else expand to the current word via prev/next
// word boundary helpers.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{Context, Window};

use crate::actions::*;
use crate::tab::InputFocus;
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn proper_case_force(
        &mut self,
        _: &EditProperCaseForce,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.case_transform(window, cx, proper_case_force);
    }

    pub(crate) fn proper_case_blend(
        &mut self,
        _: &EditProperCaseBlend,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.case_transform(window, cx, proper_case_blend);
    }

    pub(crate) fn sentence_case_force(
        &mut self,
        _: &EditSentenceCaseForce,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.case_transform(window, cx, sentence_case_force);
    }

    pub(crate) fn sentence_case_blend(
        &mut self,
        _: &EditSentenceCaseBlend,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.case_transform(window, cx, sentence_case_blend);
    }

    pub(crate) fn invert_case(
        &mut self,
        _: &EditInvertCase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.case_transform(window, cx, invert_case);
    }

    pub(crate) fn random_case(
        &mut self,
        _: &EditRandomCase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.case_transform(window, cx, random_case);
    }

    fn case_transform(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        transform: impl FnOnce(&str) -> String,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
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
        let new_text = transform(&text);
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
}

// --- Transform fns ---------------------------------------------------------

fn proper_case_force(s: &str) -> String {
    proper_case(s, true)
}

fn proper_case_blend(s: &str) -> String {
    proper_case(s, false)
}

fn proper_case(s: &str, force: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphabetic() {
            out.push(c);
            continue;
        }
        // NPP single-quote exception: when the previous char is a single
        // quote AND the char before that is alphanumeric, we are inside a
        // contraction (`don'T` etc.). Treat as non-boundary.
        let in_contraction = i >= 2
            && is_single_quote(chars[i - 1])
            && chars[i - 2].is_alphanumeric();

        let is_boundary = !in_contraction
            && (i == 0 || !chars[i - 1].is_alphanumeric());

        if is_boundary {
            for u in c.to_uppercase() {
                out.push(u);
            }
        } else if force {
            for l in c.to_lowercase() {
                out.push(l);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn sentence_case_force(s: &str) -> String {
    sentence_case(s, true)
}

fn sentence_case_blend(s: &str) -> String {
    sentence_case(s, false)
}

fn sentence_case(s: &str, force: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut is_new_sentence = true;
    let mut was_eol_r = false;
    let mut was_eol_n = false;
    for (i, &c) in chars.iter().enumerate() {
        if c.is_alphabetic() {
            // Standalone-`i` exception: `i` flanked by space/(/" before and
            // space/' after becomes `I`. NPP applies this regardless of
            // sentence position.
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let standalone_i = c == 'i'
                && matches!(prev, Some(p) if p.is_whitespace() || p == '(' || p == '"')
                && matches!(next, Some(n) if n.is_whitespace() || n == '\'');

            if is_new_sentence {
                for u in c.to_uppercase() {
                    out.push(u);
                }
                is_new_sentence = false;
            } else if standalone_i {
                out.push('I');
            } else if force {
                for l in c.to_lowercase() {
                    out.push(l);
                }
            } else {
                out.push(c);
            }
            was_eol_r = false;
            was_eol_n = false;
        } else {
            out.push(c);
            match c {
                '.' | '!' | '?' => {
                    let next_alnum = chars
                        .get(i + 1)
                        .map(|n| n.is_alphanumeric())
                        .unwrap_or(true);
                    is_new_sentence = !next_alnum;
                    was_eol_r = false;
                    was_eol_n = false;
                }
                '\r' => {
                    if was_eol_r {
                        is_new_sentence = true;
                    } else {
                        was_eol_r = true;
                    }
                }
                '\n' => {
                    if was_eol_n {
                        is_new_sentence = true;
                    } else {
                        was_eol_n = true;
                    }
                }
                _ => {
                    was_eol_r = false;
                    was_eol_n = false;
                }
            }
        }
    }
    out
}

fn invert_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_lowercase() {
            for u in c.to_uppercase() {
                out.push(u);
            }
        } else if c.is_uppercase() {
            for l in c.to_lowercase() {
                out.push(l);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn random_case(s: &str) -> String {
    let mut rng = XorShift64::seeded();
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if !c.is_alphabetic() {
            out.push(c);
            continue;
        }
        if rng.next() & 1 == 0 {
            for u in c.to_uppercase() {
                out.push(u);
            }
        } else {
            for l in c.to_lowercase() {
                out.push(l);
            }
        }
    }
    out
}

fn is_single_quote(c: char) -> bool {
    // Plain ASCII apostrophe + smart curly quotes (matches NPP's
    // `isCharSingleQuote`).
    matches!(c, '\'' | '\u{2018}' | '\u{2019}')
}

// xorshift64*. Std-only PRNG — fresh seed per call from SystemTime nanos
// xor'd with the address of a stack value to dodge the (extremely
// unlikely) case of two calls landing in the same nanosecond.
struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn seeded() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        let stack_marker = &nanos as *const u64 as u64;
        let seed = nanos ^ stack_marker;
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}
