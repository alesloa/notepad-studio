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
// word boundary helpers. A column (block) selection converts each row on
// its own, as NPP `convertSelectedTextTo` does for multi-selections
// (ScintillaEditView.cpp:3835).

use std::ops::Range;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{Context, Window};
use ropey::Rope;

use crate::actions::*;
use crate::buffer::BlockSelection;
use crate::edit_ops::util::{clamp_byte_range, floor_char_boundary};
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
        transform: impl Fn(&str) -> String,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        if self.has_block_selection() {
            if let Some(block) = self.block_selection() {
                self.case_transform_block(block, &transform, cx);
            }
            return;
        }
        let sel = self.selection();
        let range = if !sel.is_empty() {
            sel.ordered()
        } else {
            // Clamp first: the word helpers index the rope by char.
            let head = floor_char_boundary(&self.buffer().rope, sel.head);
            let start = self.prev_word_boundary(head);
            let end = self.next_word_boundary(head);
            if start == end {
                return;
            }
            start..end
        };
        let Some((range, new_text)) = case_edit(&self.buffer().rope, range, &transform) else {
            return;
        };
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

    /// Convert every row of a column selection as one undo step. The block
    /// itself stays; the caret goes to its head corner, as `block_replace`
    /// leaves it.
    fn case_transform_block(
        &mut self,
        block: BlockSelection,
        transform: &impl Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) {
        self.block_case(block, transform, cx);
    }
}

/// The replacement for `range` of `rope` under `transform`, or `None` when
/// the range is empty or the text comes back unchanged. The range is clamped
/// to the rope and floored to char boundaries first, so a stale selection
/// can't panic the slice.
fn case_edit(
    rope: &Rope,
    range: Range<usize>,
    transform: &impl Fn(&str) -> String,
) -> Option<(Range<usize>, String)> {
    let range = clamp_byte_range(rope, range);
    let text = rope.get_byte_slice(range.clone())?.to_string();
    if text.is_empty() {
        return None;
    }
    let new_text = transform(&text);
    (new_text != text).then_some((range, new_text))
}

/// One edit per row of a column selection, bottom row first, so applying
/// them in order leaves the offsets of the rows above valid. Rows shorter
/// than the block's left column have nothing selected and are skipped.
#[cfg(test)]
fn block_case_edits(rope: &Rope, block: &BlockSelection, transform: &impl Fn(&str) -> String) -> Vec<(Range<usize>, String)> {
    crate::workspace::column_plan::case(rope, *block, transform, 0)
        .map(|plan| plan.edits.into_iter().map(|edit| (edit.range, edit.text)).collect()).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit_ops::util::split_lines_eol;

    const INPUTS: &[&str] = &[
        "",
        "abc",
        "hello wORLD it's",
        "a\r\nb\rc\nd\n",
        "straße",
        "İstanbul",
        "ǅemal",
        "é中👨‍👩‍👧🇯🇵 x",
        "it's a test. ok! fine? i think so\n\nnew para",
        "   \t ",
        "ΣΑΣ σας",
        "a\u{2028}b. c\u{85}d\u{b}e\u{c}f\u{2029}g",
    ];

    type Transform = fn(&str) -> String;

    fn transforms() -> [(&'static str, Transform); 6] {
        [
            ("proper_force", proper_case_force),
            ("proper_blend", proper_case_blend),
            ("sentence_force", sentence_case_force),
            ("sentence_blend", sentence_case_blend),
            ("invert", invert_case),
            ("random", random_case),
        ]
    }

    fn apply(text: &str, edits: Vec<(Range<usize>, String)>) -> String {
        let mut s = text.to_string();
        for (r, t) in edits {
            s.replace_range(r, &t);
        }
        s
    }

    #[test]
    fn transforms_change_only_letters() {
        let eols = |s: &str| -> Vec<String> {
            split_lines_eol(s).into_iter().map(|p| p.1.to_string()).collect()
        };
        let non_letters = |s: &str| -> String {
            s.chars().filter(|c| c.is_ascii() && !c.is_ascii_alphabetic()).collect()
        };
        for input in INPUTS {
            for (name, f) in transforms() {
                let out = f(input);
                assert_eq!(eols(&out), eols(input), "{name} on {input:?}");
                assert_eq!(non_letters(&out), non_letters(input), "{name} on {input:?}");
            }
        }
    }

    #[test]
    fn transforms_handle_length_changing_case_maps() {
        // ß -> SS and İ -> i̇ change byte length; nothing may assume it holds.
        assert_eq!(invert_case("straße"), "STRASSE");
        assert_eq!(invert_case("İ"), "i\u{307}");
        assert_eq!(proper_case_force("hello wORLD it's"), "Hello World It's");
        assert_eq!(proper_case_blend("hello wORLD"), "Hello WORLD");
        assert_eq!(sentence_case_force("hello. WORLD! i think"), "Hello. World! I think");
        assert_eq!(random_case("abc def").to_lowercase(), "abc def");
    }

    #[test]
    fn case_edit_clamps_stale_and_mid_char_ranges() {
        // A range ending inside `中` used to panic `byte_slice`.
        let rope = Rope::from_str("aé中");
        assert_eq!(case_edit(&rope, 2..5, &invert_case), Some((1..3, "É".to_string())));
        let rope = Rope::from_str("abc");
        assert_eq!(case_edit(&rope, 1..99, &invert_case), Some((1..3, "BC".to_string())));
        assert_eq!(case_edit(&rope, 50..99, &invert_case), None);
        assert_eq!(case_edit(&Rope::new(), 0..4, &invert_case), None);
        assert_eq!(case_edit(&Rope::from_str("1 2"), 0..3, &invert_case), None);
    }

    #[test]
    fn block_selection_converts_each_row() {
        let text = "abcdef\nab\n\nabcdefgh\r\nx";
        let rope = Rope::from_str(text);
        let block = BlockSelection {
            anchor_line: 0,
            anchor_col: 1,
            head_line: 4,
            head_col: 4,
            is_dragging: false,
        };
        let edits = block_case_edits(&rope, &block, &invert_case);
        assert_eq!(apply(text, edits), "aBCDef\naB\n\naBCDefgh\r\nx");

        // Byte length changes per row; bottom-first order keeps rows above valid.
        let text = "aßc\nxßy\nz";
        let block = BlockSelection {
            anchor_line: 1,
            anchor_col: 2,
            head_line: 0,
            head_col: 1,
            is_dragging: false,
        };
        let edits = block_case_edits(&Rope::from_str(text), &block, &invert_case);
        assert_eq!(apply(text, edits), "aSSc\nxSSy\nz");

        // A block left over from a longer buffer selects nothing.
        let block = BlockSelection {
            anchor_line: 10,
            anchor_col: 0,
            head_line: 12,
            head_col: 3,
            is_dragging: false,
        };
        assert!(block_case_edits(&Rope::from_str(text), &block, &invert_case).is_empty());

        // LS and NEL stay in the selected physical row's body.
        let text = "ab\u{2028}cd\u{85}ef\ngh";
        let block = BlockSelection {
            anchor_line: 0,
            anchor_col: 1,
            head_line: 1,
            head_col: 9,
            is_dragging: false,
        };
        let edits = block_case_edits(&Rope::from_str(text), &block, &invert_case);
        assert_eq!(apply(text, edits), "aB\u{2028}CD\u{85}EF\ngH");
    }
}
