// PowerEditor/src/NppIO.cpp:2213-2267; Buffer.cpp:365-400;
// NppNotification.cpp:52-61 — automatic names follow row-zero notifications.

use std::sync::atomic::{AtomicBool, Ordering};

use ropey::Rope;

pub(crate) mod history;

pub(crate) const FOREGROUND_BYTES: usize = 4096;
const NAME_UNITS: usize = 63;
const NAME_BYTES: usize = NAME_UNITS * 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    bytes: [u8; NAME_BYTES],
    len: usize,
}

impl Candidate {
    fn new() -> Self { Self { bytes: [0; NAME_BYTES], len: 0 } }

    fn push(&mut self, ch: char) -> Result<(), &'static str> {
        let mut encoded = [0; 4];
        let bytes = ch.encode_utf8(&mut encoded).as_bytes();
        let end = self.len.checked_add(bytes.len()).ok_or("Automatic name length overflow")?;
        self.bytes.get_mut(self.len..end).ok_or("Automatic name exceeds its length limit")?.copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }

    fn finish(mut self) -> Result<Option<Self>, &'static str> {
        let value = self.bytes.get(..self.len).ok_or("Invalid automatic name length")?;
        let start = value.iter().position(|&byte| byte != b' ').unwrap_or(self.len);
        let end = value.iter().rposition(|&byte| byte != b' ').map_or(start, |index| index.saturating_add(1));
        if start == end { return Ok(None); }
        self.bytes.copy_within(start..end, 0);
        self.len = end - start;
        Ok(Some(self))
    }

    pub(crate) fn as_str(&self) -> Result<&str, &'static str> {
        let bytes = self.bytes.get(..self.len).ok_or("Invalid automatic name length")?;
        std::str::from_utf8(bytes).map_err(|_| "Invalid automatic name characters")
    }

    pub(crate) fn to_string(&self) -> Result<String, &'static str> {
        let value = self.as_str()?;
        let mut output = String::new();
        output.try_reserve_exact(value.len()).map_err(|_| "Not enough memory for the automatic name")?;
        output.push_str(value);
        Ok(output)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Cursor {
    byte: usize,
    leading_spaces: bool,
    units: usize,
    name: Candidate,
}

#[derive(Debug)]
pub(crate) enum Scan {
    Complete(Option<Candidate>),
    Pending(Cursor),
}

impl Cursor {
    pub(crate) fn new() -> Self {
        Self { byte: 0, leading_spaces: true, units: 0, name: Candidate::new() }
    }

    pub(crate) fn scanned_bytes(&self) -> usize { self.byte }

    pub(crate) fn advance(self, rope: &Rope, budget: usize, cancel: &AtomicBool) -> Result<Scan, &'static str> {
        self.advance_counted(rope, budget, cancel, &mut 0)
    }

    pub(crate) fn advance_counted(mut self, rope: &Rope, budget: usize, cancel: &AtomicBool, visits: &mut usize) -> Result<Scan, &'static str> {
        let tail = rope.get_byte_slice(self.byte..).ok_or("Automatic name source no longer matches")?;
        let mut visited = 0usize;
        for ch in tail.chars() {
            if cancel.load(Ordering::Relaxed) { return Err("Automatic naming cancelled"); }
            let next = visited.checked_add(ch.len_utf8()).ok_or("Automatic name scan overflow")?;
            if next > budget { return Ok(Scan::Pending(self)); }
            visited = next;
            *visits = visits.checked_add(ch.len_utf8()).ok_or("Automatic name work count overflow")?;
            #[cfg(test)] crate::buffer::automatic_title_work::record_scan(ch.len_utf8());
            self.byte = self.byte.checked_add(ch.len_utf8()).ok_or("Automatic name offset overflow")?;
            if matches!(ch, '\0' | '\r' | '\n') { return Ok(Scan::Complete(self.name.finish()?)); }
            if self.leading_spaces && ch == ' ' { continue; }
            self.leading_spaces = false;
            if matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\t') { continue; }
            let units = self.units.checked_add(ch.len_utf16()).ok_or("Automatic name length overflow")?;
            if units > NAME_UNITS {
                // UTF8 cannot retain the oracle's truncated high surrogate.
                self.name.push('\u{fffd}')?;
                return Ok(Scan::Complete(self.name.finish()?));
            }
            self.name.push(ch)?;
            self.units = units;
            if units == NAME_UNITS { return Ok(Scan::Complete(self.name.finish()?)); }
        }
        if cancel.load(Ordering::Relaxed) { return Err("Automatic naming cancelled"); }
        Ok(Scan::Complete(self.name.finish()?))
    }
}

pub(crate) fn row_zero_notification(rope: &Rope, position: usize) -> bool {
    let position = crate::buffer::floor_char_boundary(rope, position.min(rope.len_bytes()));
    rope.byte_to_line(position) == 0
}

pub(crate) fn same_full_name(left: &str, right: &[u8]) -> bool {
    let prefix = right.get(..right.len().min(NAME_BYTES + 1)).unwrap_or_default();
    let end = prefix.iter().position(|&byte| byte == 0).unwrap_or(prefix.len());
    left.as_bytes().eq_ignore_ascii_case(prefix.get(..end).unwrap_or_default())
}

pub(crate) fn same_name(left: &str, right: &str) -> bool { same_full_name(left, right.as_bytes()) }

#[derive(Clone, Debug, Default)]
pub(crate) enum Summary {
    #[default]
    Identity,
    Conditional(Candidate),
    Constant(Candidate),
}

impl Summary {
    pub(crate) fn append(&mut self, candidate: Candidate) -> Result<(), &'static str> {
        let name = candidate.as_str()?;
        match self {
            Self::Identity => *self = Self::Conditional(candidate),
            Self::Conditional(previous) | Self::Constant(previous) => {
                if !same_name(previous.as_str()?, name) { *self = Self::Constant(candidate); }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn compose(&mut self, next: Self) -> Result<(), &'static str> {
        match next {
            Self::Identity => {},
            Self::Conditional(candidate) => self.append(candidate)?,
            Self::Constant(candidate) => *self = Self::Constant(candidate),
        }
        Ok(())
    }

    pub(crate) fn resolve(&self, current: &str) -> Result<Option<&Candidate>, &'static str> {
        match self {
            Self::Identity => Ok(None),
            Self::Conditional(candidate) if same_name(candidate.as_str()?, current) => Ok(None),
            Self::Conditional(candidate) | Self::Constant(candidate) => Ok(Some(candidate)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str, budget: usize) -> Option<String> {
        let rope = Rope::from_str(text);
        let cancel = AtomicBool::new(false);
        let mut cursor = Cursor::new();
        loop {
            match cursor.advance(&rope, budget, &cancel).expect("valid title scan") {
                Scan::Complete(value) => return value.map(|value| value.to_string().expect("bounded UTF8 name")),
                Scan::Pending(next) => cursor = next,
            }
        }
    }

    #[test]
    fn physical_first_line_and_nul_stop_normalization() {
        for ending in ["\r\n", "\n", "\r"] {
            assert_eq!(name(&format!("{ending}later"), 7), None);
            assert_eq!(name(&format!("first{ending}later"), 7).as_deref(), Some("first"));
        }
        assert_eq!(name("first\0hidden", 7).as_deref(), Some("first"));
        assert_eq!(name("\0hidden", 7), None);
    }

    #[test]
    fn ascii_filter_and_two_space_trims_keep_unicode() {
        assert_eq!(name("  a\\/:*?\"<>|\tb  \r\nlater", 7).as_deref(), Some("ab"));
        assert_eq!(name("\u{a0}café\u{2028}中\u{85}😀\u{2029}tail\u{a0}", 7).as_deref(), Some("\u{a0}café\u{2028}中\u{85}😀\u{2029}tail\u{a0}"));
        assert_eq!(name("\t  name  ", 7).as_deref(), Some("name"));
        assert_eq!(name(&format!("\t{}name", " ".repeat(63)), 7), None);
        assert_eq!(name(&format!("{}name", " ".repeat(9000)), 7).as_deref(), Some("name"));
    }

    #[test]
    fn unit_limit_and_lone_surrogate_presentation_leave_source_unchanged() {
        assert_eq!(name(&"a".repeat(80), 7), Some("a".repeat(63)));
        let text = format!("{}😀tail", "a".repeat(62));
        let rope = Rope::from_str(&text);
        assert_eq!(name(&text, 7), Some(format!("{}\u{fffd}", "a".repeat(62))));
        assert_eq!(rope.to_string(), text);
        assert_eq!(name(&format!("{}😀tail", "a".repeat(61)), 7), Some(format!("{}😀", "a".repeat(61))));
    }

    #[test]
    fn long_filtered_prefix_yields_and_continues_without_copying_it() {
        let rope = Rope::from_str(&format!("{}café中😀\nsecond", "/".repeat(12582912)));
        let cancel = AtomicBool::new(false);
        let Scan::Pending(cursor) = Cursor::new().advance(&rope, FOREGROUND_BYTES, &cancel).expect("bounded scan") else { panic!("long prefix must yield") };
        assert_eq!(cursor.byte, FOREGROUND_BYTES);
        assert_eq!(cursor.name.len, 0);
        let mut cursor = cursor;
        loop {
            match cursor.advance(&rope, 65536, &cancel).expect("continuation") {
                Scan::Pending(next) => cursor = next,
                Scan::Complete(candidate) => { assert_eq!(candidate.expect("nonempty").as_str().expect("UTF8"), "café中😀"); break; }
            }
        }
    }

    #[test]
    fn cancellation_and_zero_budget_do_not_scan() {
        let rope = Rope::from_str("name");
        let cancel = AtomicBool::new(true);
        assert!(Cursor::new().advance(&rope, FOREGROUND_BYTES, &cancel).is_err());
        let cancel = AtomicBool::new(false);
        let Scan::Pending(cursor) = Cursor::new().advance(&rope, 0, &cancel).expect("zero budget") else { panic!("zero budget must yield") };
        assert_eq!(cursor.byte, 0);
    }

    #[test]
    fn row_membership_is_post_modification_and_full_names_fold_ascii_only() {
        let before = Rope::from_str("first\r\nsecond");
        assert!(!row_zero_notification(&before, 7));
        let after = Rope::from_str("firstsecond");
        assert!(row_zero_notification(&after, 5));
        assert!(!row_zero_notification(&before, usize::MAX));
        assert!(same_name("Shared", "SHARED"));
        assert!(!same_name("É", "é"));
        assert!(!same_name("shared", "C:\\samples\\shared"));
    }

    #[test]
    fn fixed_collision_summary_matches_direct_acceptance_and_every_split() {
        fn fold(value: &str) -> String { value.chars().map(|ch| if ch.is_ascii_uppercase() { ch.to_ascii_lowercase() } else { ch }).collect() }
        fn candidate(text: &str) -> Candidate {
            let rope = Rope::from_str(text);
            let Scan::Complete(Some(candidate)) = Cursor::new().advance(&rope, FOREGROUND_BYTES, &AtomicBool::new(false)).expect("candidate") else { panic!("nonempty short name") };
            candidate
        }
        fn direct(current: &str, names: &[&str], conflicts: &[&str]) -> String {
            let mut current = current.to_owned();
            for name in names {
                if !name.is_empty() && fold(name) != fold(&current) && !conflicts.iter().any(|other| fold(name) == fold(other)) { current = (*name).to_owned(); }
            }
            current
        }
        fn summary(names: &[&str], conflicts: &[&str]) -> Summary {
            let mut summary = Summary::Identity;
            for name in names {
                if !name.is_empty() && !conflicts.iter().any(|other| fold(name) == fold(other)) {
                    summary.append(candidate(name)).expect("bounded append");
                }
            }
            summary
        }
        let names = ["alpha", "ALPHA", "beta", "blocked", "É", "é", ""];
        let conflicts: [&[&str]; 4] = [&[], &["blocked"], &["ALPHA"], &["É", "blocked"]];
        let mut checked = 0usize;
        for length in 0..=4u32 {
            for number in 0..names.len().pow(length) {
                let mut value = number;
                let mut sequence = Vec::new();
                for _ in 0..length {
                    sequence.push(*names.get(value % names.len()).expect("finite test index"));
                    value /= names.len();
                }
                for conflicts in conflicts {
                    for initial in ["new 1", "alpha", "ALPHA", "beta", "É", "é"] {
                        let expected = direct(initial, &sequence, conflicts);
                        let reduced = summary(&sequence, conflicts);
                        assert_eq!(reduced.resolve(initial).expect("summary").map(|name| name.as_str().expect("UTF8")).unwrap_or(initial), expected);
                        for split in 0..=sequence.len() {
                            let mut composed = summary(sequence.get(..split).expect("prefix"), conflicts);
                            composed.compose(summary(sequence.get(split..).expect("suffix"), conflicts)).expect("compose");
                            assert_eq!(composed.resolve(initial).expect("summary").map(|name| name.as_str().expect("UTF8")).unwrap_or(initial), expected);
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(checked, 324936);
    }
}
