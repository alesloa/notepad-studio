// Excel-style fill-series pattern detection + projection.
//
// `detect` walks a slice of source values and picks the most specific
// pattern that matches the whole sequence. `project_with_source_len`
// extends that pattern by `count` cells either forward (drag down /
// right) or backward (drag up / left). The two functions stay pure — they
// operate on plain strings and never touch the CSV model — so the
// behaviour is exercised by simple unit tests.

use chrono::NaiveDate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextCase {
    Lower,
    Upper,
    Title,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DateFormat {
    Iso,  // YYYY-MM-DD
    Slashed, // MM/DD/YYYY
}

/// Numeric formatting captured from the source so projected cells
/// round-trip cleanly. Decimal width is the number of digits after
/// the dot (0 for integers). Thousands grouping is detected from the
/// presence of an embedded comma in any source cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NumberFormat {
    pub(crate) decimals: usize,
    pub(crate) thousands: bool,
    pub(crate) leading_zero_width: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pattern {
    Constant(String),
    NumericLinear {
        start: f64,
        step: f64,
        format: NumberFormat,
    },
    DateLinear {
        start: NaiveDate,
        step_days: i64,
        format: DateFormat,
    },
    /// Cycle through the seven weekday names starting at `start_idx`
    /// (Mon=0). Step is always +1 day; mixed-step weekday sequences fall
    /// back to CyclicCopy.
    WeekdayCycle {
        start_idx: usize,
        case: TextCase,
        abbreviated: bool,
    },
    MonthCycle {
        start_idx: usize,
        case: TextCase,
        abbreviated: bool,
    },
    /// Trailing-integer pattern like `Item 03` → step of 1, prefix
    /// `Item `, width 2.
    TextNumber {
        prefix: String,
        start: i64,
        step: i64,
        width: usize,
    },
    /// Fallback — repeat the source cells cyclically.
    CyclicCopy(Vec<String>),
}

/// Detect the pattern in `source`. Empty source yields `CyclicCopy` of
/// the empty vec (callers must guard).
pub(crate) fn detect(source: &[String]) -> Pattern {
    if source.is_empty() {
        return Pattern::CyclicCopy(Vec::new());
    }
    if source.len() == 1 {
        // Single-cell sources never increment — Excel treats a lone
        // numeric source as a constant fill (Ctrl-drag is the
        // increment override; we don't model that yet).
        return Pattern::Constant(source[0].clone());
    }

    if let Some(p) = detect_numeric_linear(source) {
        return p;
    }
    if let Some(p) = detect_date_linear(source) {
        return p;
    }
    if let Some(p) = detect_weekday_cycle(source) {
        return p;
    }
    if let Some(p) = detect_month_cycle(source) {
        return p;
    }
    if let Some(p) = detect_text_number(source) {
        return p;
    }
    Pattern::CyclicCopy(source.to_vec())
}

// Internal helpers --------------------------------------------------------

fn detect_numeric_linear(source: &[String]) -> Option<Pattern> {
    let parsed: Vec<f64> = source.iter().map(|s| parse_loose_number(s)).collect::<Option<_>>()?;
    if parsed.len() < 2 {
        return None;
    }
    let step = parsed[1] - parsed[0];
    // All consecutive diffs must equal step within float tolerance.
    for w in parsed.windows(2) {
        let d = w[1] - w[0];
        if (d - step).abs() > 1e-9 * (step.abs().max(1.0)) {
            return None;
        }
    }
    let format = detect_number_format(source);
    Some(Pattern::NumericLinear {
        start: parsed[parsed.len() - 1] - step * (parsed.len() as f64 - 1.0),
        step,
        format,
    })
}

fn detect_number_format(source: &[String]) -> NumberFormat {
    let mut decimals = 0usize;
    let mut thousands = false;
    let mut leading_zero_width = 0usize;
    for s in source {
        let t = s.trim();
        if t.contains(',') {
            thousands = true;
        }
        let stripped = t.trim_start_matches('-').trim_start_matches('+');
        let digits: String = stripped.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
        if let Some(dot) = digits.find('.') {
            let frac = &digits[dot + 1..];
            let frac = frac.trim_end_matches('0');
            decimals = decimals.max(frac.len());
        }
        // Detect leading-zero padding ("007" → width 3 on an integer).
        let no_sign = stripped.trim_start_matches('+').trim_start_matches('-');
        if !no_sign.contains('.') && !no_sign.contains(',') && no_sign.starts_with('0') && no_sign.len() > 1 {
            leading_zero_width = leading_zero_width.max(no_sign.len());
        }
    }
    NumberFormat { decimals, thousands, leading_zero_width }
}

fn parse_loose_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // Strip thousands separators when they look unambiguous (only
    // digits and commas surround the dot, if any).
    let normalized: String = t.chars().filter(|c| *c != ',').collect();
    normalized.parse::<f64>().ok().filter(|n| n.is_finite())
}

fn detect_date_linear(source: &[String]) -> Option<Pattern> {
    let (dates, format) = parse_dates(source)?;
    if dates.len() < 2 {
        return None;
    }
    let step = (dates[1] - dates[0]).num_days();
    if step == 0 {
        return None;
    }
    for w in dates.windows(2) {
        if (w[1] - w[0]).num_days() != step {
            return None;
        }
    }
    Some(Pattern::DateLinear {
        start: dates[dates.len() - 1]
            .checked_sub_signed(chrono::Duration::days(step * (dates.len() as i64 - 1)))
            .unwrap_or(dates[0]),
        step_days: step,
        format,
    })
}

fn parse_dates(source: &[String]) -> Option<(Vec<NaiveDate>, DateFormat)> {
    // Try ISO first, then slashed. The format must be consistent across
    // every source cell.
    let try_format = |fmt: DateFormat| -> Option<Vec<NaiveDate>> {
        source
            .iter()
            .map(|s| parse_date_with_format(s, fmt))
            .collect::<Option<Vec<_>>>()
    };
    if let Some(dates) = try_format(DateFormat::Iso) {
        return Some((dates, DateFormat::Iso));
    }
    if let Some(dates) = try_format(DateFormat::Slashed) {
        return Some((dates, DateFormat::Slashed));
    }
    None
}

fn parse_date_with_format(s: &str, fmt: DateFormat) -> Option<NaiveDate> {
    let t = s.trim();
    let pattern = match fmt {
        DateFormat::Iso => "%Y-%m-%d",
        DateFormat::Slashed => "%m/%d/%Y",
    };
    NaiveDate::parse_from_str(t, pattern).ok()
}

fn format_date(d: NaiveDate, fmt: DateFormat) -> String {
    match fmt {
        DateFormat::Iso => d.format("%Y-%m-%d").to_string(),
        DateFormat::Slashed => d.format("%m/%d/%Y").to_string(),
    }
}

fn detect_weekday_cycle(source: &[String]) -> Option<Pattern> {
    let (indices, case, abbreviated) = parse_weekday_sequence(source)?;
    if indices.len() < 2 {
        return None;
    }
    // Require step of +1 day modulo 7.
    let start = indices[0];
    for (i, &v) in indices.iter().enumerate() {
        if v != (start + i) % 7 {
            return None;
        }
    }
    Some(Pattern::WeekdayCycle {
        start_idx: start,
        case,
        abbreviated,
    })
}

fn parse_weekday_sequence(source: &[String]) -> Option<(Vec<usize>, TextCase, bool)> {
    let mut indices = Vec::with_capacity(source.len());
    let mut abbreviated: Option<bool> = None;
    let mut case: Option<TextCase> = None;
    for s in source {
        let (idx, abbrev, c) = parse_weekday(s)?;
        if abbreviated.get_or_insert(abbrev) != &abbrev {
            return None;
        }
        if case.get_or_insert(c) != &c {
            return None;
        }
        indices.push(idx);
    }
    Some((indices, case?, abbreviated?))
}

fn parse_weekday(s: &str) -> Option<(usize, bool, TextCase)> {
    let t = s.trim();
    let case = text_case_of(t);
    let lower = t.to_lowercase();
    const FULL: [&str; 7] = [
        "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
    ];
    const ABBR: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    if let Some(i) = FULL.iter().position(|w| *w == lower) {
        return Some((i, false, case));
    }
    if let Some(i) = ABBR.iter().position(|w| *w == lower) {
        return Some((i, true, case));
    }
    None
}

fn weekday_name(idx: usize, abbreviated: bool) -> &'static str {
    const FULL: [&str; 7] = [
        "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
    ];
    const ABBR: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    if abbreviated { ABBR[idx] } else { FULL[idx] }
}

fn detect_month_cycle(source: &[String]) -> Option<Pattern> {
    let (indices, case, abbreviated) = parse_month_sequence(source)?;
    if indices.len() < 2 {
        return None;
    }
    let start = indices[0];
    for (i, &v) in indices.iter().enumerate() {
        if v != (start + i) % 12 {
            return None;
        }
    }
    Some(Pattern::MonthCycle {
        start_idx: start,
        case,
        abbreviated,
    })
}

fn parse_month_sequence(source: &[String]) -> Option<(Vec<usize>, TextCase, bool)> {
    let mut indices = Vec::with_capacity(source.len());
    let mut abbreviated: Option<bool> = None;
    let mut case: Option<TextCase> = None;
    for s in source {
        let (idx, abbrev, c) = parse_month(s)?;
        if abbreviated.get_or_insert(abbrev) != &abbrev {
            return None;
        }
        if case.get_or_insert(c) != &c {
            return None;
        }
        indices.push(idx);
    }
    Some((indices, case?, abbreviated?))
}

fn parse_month(s: &str) -> Option<(usize, bool, TextCase)> {
    let t = s.trim();
    let case = text_case_of(t);
    let lower = t.to_lowercase();
    const FULL: [&str; 12] = [
        "january", "february", "march", "april", "may", "june", "july", "august",
        "september", "october", "november", "december",
    ];
    const ABBR: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    if let Some(i) = FULL.iter().position(|w| *w == lower) {
        return Some((i, false, case));
    }
    if let Some(i) = ABBR.iter().position(|w| *w == lower) {
        return Some((i, true, case));
    }
    None
}

fn month_name(idx: usize, abbreviated: bool) -> &'static str {
    const FULL: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August",
        "September", "October", "November", "December",
    ];
    const ABBR: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if abbreviated { ABBR[idx] } else { FULL[idx] }
}

fn text_case_of(s: &str) -> TextCase {
    if s.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()) {
        TextCase::Lower
    } else if s.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) {
        TextCase::Upper
    } else {
        TextCase::Title
    }
}

fn format_text_case(s: &str, case: TextCase) -> String {
    match case {
        TextCase::Lower => s.to_lowercase(),
        TextCase::Upper => s.to_uppercase(),
        TextCase::Title => s.to_string(),
    }
}

fn detect_text_number(source: &[String]) -> Option<Pattern> {
    let parsed: Vec<(String, i64, usize)> =
        source.iter().map(|s| split_trailing_number(s)).collect::<Option<_>>()?;
    if parsed.len() < 2 {
        return None;
    }
    // All prefixes must match.
    let prefix = parsed[0].0.clone();
    if parsed.iter().any(|(p, _, _)| *p != prefix) {
        return None;
    }
    let width = parsed.iter().map(|(_, _, w)| *w).max().unwrap_or(0);
    let step = parsed[1].1 - parsed[0].1;
    for w in parsed.windows(2) {
        if w[1].1 - w[0].1 != step {
            return None;
        }
    }
    if step == 0 {
        return None;
    }
    Some(Pattern::TextNumber {
        prefix,
        start: parsed[parsed.len() - 1].1 - step * (parsed.len() as i64 - 1),
        step,
        width,
    })
}

/// Split a string into `(prefix, integer, width)`. Returns `None` when
/// the string has no trailing integer or the integer is non-positive
/// (negative trailing numbers are rare in user data and the prefix
/// rules conflict with "negative" signage).
fn split_trailing_number(s: &str) -> Option<(String, i64, usize)> {
    // ASCII digits are one byte each, so the length of the trimmed
    // string is a char boundary even when the prefix ends in a
    // multi-byte char ("é1", "中10").
    let prefix = s.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = s.get(prefix.len()..)?;
    if digits.is_empty() {
        return None;
    }
    let n: i64 = digits.parse().ok()?;
    Some((prefix.to_string(), n, digits.len()))
}

fn format_text_number(prefix: &str, n: i128, width: usize) -> String {
    if n < 0 {
        return format!("{}{}", prefix, n);
    }
    if width == 0 {
        format!("{}{}", prefix, n)
    } else {
        format!("{}{:0>width$}", prefix, n, width = width)
    }
}

fn format_numeric(n: f64, format: NumberFormat) -> String {
    let raw = if format.decimals == 0 {
        // Round to integer when source had no decimal places. `{:.0}`
        // prints every digit of the rounded value, where a cast to i64
        // would clamp anything past 9.2e18. Adding 0.0 turns -0.0 into
        // 0.0 so a value that rounds to zero doesn't print as "-0".
        let rounded = n.round() + 0.0;
        if format.leading_zero_width > 0 {
            // Integer padding.
            let sign_prefix = if rounded < 0.0 { "-" } else { "" };
            return format!(
                "{}{:0>width$.0}",
                sign_prefix,
                rounded.abs(),
                width = format.leading_zero_width.saturating_sub(sign_prefix.len())
            );
        }
        format!("{rounded:.0}")
    } else {
        format!("{:.*}", format.decimals, n)
    };
    if format.thousands {
        insert_thousands(&raw)
    } else {
        raw
    }
}

fn insert_thousands(s: &str) -> String {
    let (sign, rest) = if let Some(stripped) = s.strip_prefix('-') {
        ("-", stripped)
    } else {
        ("", s)
    };
    let (int_part, frac_part) = match rest.find('.') {
        Some(i) => (&rest[..i], Some(&rest[i..])),
        None => (rest, None),
    };
    let mut out = String::new();
    let chars: Vec<char> = int_part.chars().rev().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(*c);
    }
    let int_with_commas: String = out.chars().rev().collect();
    let mut result = String::from(sign);
    result.push_str(&int_with_commas);
    if let Some(f) = frac_part {
        result.push_str(f);
    }
    result
}

/// Project `count` cells from `pattern`, which was detected from
/// `source_len` cells. `reverse = false` extends forward (drag down or
/// right); `reverse = true` extends backward (drag up or left) and
/// returns the cells top-to-bottom, so the caller writes them straight
/// into the cells above or left of the source.
///
/// A `None` entry is a value the pattern can't represent: a date outside
/// chrono's calendar, or a number that isn't finite. The caller leaves
/// that cell as it is rather than writing a wrong value into it.
pub(crate) fn project_with_source_len(
    pattern: &Pattern,
    source_len: usize,
    count: usize,
    reverse: bool,
) -> Vec<Option<String>> {
    (0..count)
        .map(|i| {
            // Position of the projected cell counted from the first
            // source cell: source_len, source_len + 1, .. forward and
            // -count, .., -1 backward. i128 holds either for any usize,
            // and a step times it for any i64 step.
            let k = if reverse {
                -(i128::try_from(count - i).ok()?)
            } else {
                i128::try_from(source_len).ok()?.checked_add(i128::try_from(i).ok()?)?
            };
            project_cell(pattern, k)
        })
        .collect()
}

/// The value `k` cells after the first source cell (before it when `k`
/// is negative).
fn project_cell(pattern: &Pattern, k: i128) -> Option<String> {
    match pattern {
        Pattern::Constant(v) => Some(v.clone()),
        Pattern::NumericLinear { start, step, format } => {
            let n = start + step * k as f64;
            n.is_finite().then(|| format_numeric(n, *format))
        }
        Pattern::DateLinear { start, step_days, format } => {
            let days = i64::try_from(i128::from(*step_days).checked_mul(k)?).ok()?;
            let d = start.checked_add_signed(chrono::TimeDelta::try_days(days)?)?;
            Some(format_date(d, *format))
        }
        Pattern::WeekdayCycle { start_idx, case, abbreviated } => {
            let idx = cycle_index(*start_idx, k, 7)?;
            Some(format_text_case(weekday_name(idx, *abbreviated), *case))
        }
        Pattern::MonthCycle { start_idx, case, abbreviated } => {
            let idx = cycle_index(*start_idx, k, 12)?;
            Some(format_text_case(month_name(idx, *abbreviated), *case))
        }
        Pattern::TextNumber { prefix, start, step, width } => {
            let n = i128::from(*start).checked_add(i128::from(*step).checked_mul(k)?)?;
            Some(format_text_number(prefix, n, *width))
        }
        Pattern::CyclicCopy(items) => {
            if items.is_empty() {
                return Some(String::new());
            }
            items.get(cycle_index(0, k, items.len())?).cloned()
        }
    }
}

/// `(start + k) mod len`, never negative.
fn cycle_index(start: usize, k: i128, len: usize) -> Option<usize> {
    let len = i128::try_from(len).ok().filter(|&l| l > 0)?;
    let raw = i128::try_from(start).ok()?.checked_add(k)?;
    usize::try_from(raw.rem_euclid(len)).ok()
}

#[cfg(test)]
fn project_default(pattern: &Pattern, count: usize, reverse: bool) -> Vec<Option<String>> {
    project_with_source_len(pattern, default_source_len(pattern), count, reverse)
}

#[cfg(test)]
fn default_source_len(pattern: &Pattern) -> usize {
    match pattern {
        Pattern::Constant(_) => 1,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn some(items: &[&str]) -> Vec<Option<String>> {
        items.iter().map(|s| Some(s.to_string())).collect()
    }

    #[test]
    fn detect_constant_for_single_cell() {
        assert_eq!(detect(&s(&["5"])), Pattern::Constant("5".into()));
        assert_eq!(detect(&s(&["hello"])), Pattern::Constant("hello".into()));
    }

    #[test]
    fn detect_numeric_linear_simple() {
        match detect(&s(&["1", "2", "3"])) {
            Pattern::NumericLinear { start, step, .. } => {
                assert!((start - 1.0).abs() < 1e-9);
                assert!((step - 1.0).abs() < 1e-9);
            }
            p => panic!("expected NumericLinear, got {p:?}"),
        }
    }

    #[test]
    fn detect_numeric_linear_step_ten() {
        match detect(&s(&["10", "20", "30"])) {
            Pattern::NumericLinear { start, step, .. } => {
                assert!((start - 10.0).abs() < 1e-9);
                assert!((step - 10.0).abs() < 1e-9);
            }
            p => panic!("expected NumericLinear, got {p:?}"),
        }
    }

    #[test]
    fn detect_numeric_linear_negative_step() {
        match detect(&s(&["100", "90", "80"])) {
            Pattern::NumericLinear { start, step, .. } => {
                assert!((start - 100.0).abs() < 1e-9);
                assert!((step - -10.0).abs() < 1e-9);
            }
            p => panic!("expected NumericLinear, got {p:?}"),
        }
    }

    #[test]
    fn detect_date_linear_iso() {
        match detect(&s(&["2025-01-01", "2025-01-02"])) {
            Pattern::DateLinear { step_days, format, .. } => {
                assert_eq!(step_days, 1);
                assert_eq!(format, DateFormat::Iso);
            }
            p => panic!("expected DateLinear, got {p:?}"),
        }
    }

    #[test]
    fn detect_date_linear_slashed() {
        match detect(&s(&["01/01/2025", "01/02/2025"])) {
            Pattern::DateLinear { step_days, format, .. } => {
                assert_eq!(step_days, 1);
                assert_eq!(format, DateFormat::Slashed);
            }
            p => panic!("expected DateLinear, got {p:?}"),
        }
    }

    #[test]
    fn detect_weekday_cycle_abbr() {
        match detect(&s(&["Mon", "Tue", "Wed"])) {
            Pattern::WeekdayCycle { start_idx, abbreviated, case } => {
                assert_eq!(start_idx, 0);
                assert!(abbreviated);
                assert_eq!(case, TextCase::Title);
            }
            p => panic!("expected WeekdayCycle, got {p:?}"),
        }
    }

    #[test]
    fn detect_weekday_cycle_full() {
        match detect(&s(&["Monday", "Tuesday"])) {
            Pattern::WeekdayCycle { abbreviated, .. } => assert!(!abbreviated),
            p => panic!("expected WeekdayCycle, got {p:?}"),
        }
    }

    #[test]
    fn detect_month_cycle_abbr() {
        match detect(&s(&["Jan", "Feb"])) {
            Pattern::MonthCycle { start_idx, abbreviated, .. } => {
                assert_eq!(start_idx, 0);
                assert!(abbreviated);
            }
            p => panic!("expected MonthCycle, got {p:?}"),
        }
    }

    #[test]
    fn detect_text_number_basic() {
        match detect(&s(&["Item 1", "Item 2"])) {
            Pattern::TextNumber { prefix, start, step, width } => {
                assert_eq!(prefix, "Item ");
                assert_eq!(start, 1);
                assert_eq!(step, 1);
                assert_eq!(width, 1);
            }
            p => panic!("expected TextNumber, got {p:?}"),
        }
    }

    #[test]
    fn detect_text_number_padded() {
        match detect(&s(&["Item 01", "Item 02"])) {
            Pattern::TextNumber { width, .. } => assert_eq!(width, 2),
            p => panic!("expected TextNumber, got {p:?}"),
        }
    }

    #[test]
    fn detect_falls_back_cyclic_for_mixed() {
        match detect(&s(&["alpha", "beta", "gamma"])) {
            Pattern::CyclicCopy(items) => {
                assert_eq!(items, vec!["alpha", "beta", "gamma"]);
            }
            p => panic!("expected CyclicCopy, got {p:?}"),
        }
    }

    #[test]
    fn detect_mixed_number_and_text_falls_back() {
        match detect(&s(&["1", "alpha"])) {
            Pattern::CyclicCopy(items) => {
                assert_eq!(items, vec!["1", "alpha"]);
            }
            p => panic!("expected CyclicCopy, got {p:?}"),
        }
    }

    #[test]
    fn project_constant_extends() {
        let p = Pattern::Constant("7".into());
        assert_eq!(project_default(&p, 3, false), some(&["7", "7", "7"]));
    }

    #[test]
    fn project_numeric_forward() {
        let p = detect(&s(&["1", "2"]));
        let out = project_with_source_len(&p, 2, 3, false);
        assert_eq!(out, some(&["3", "4", "5"]));
    }

    #[test]
    fn project_numeric_backward() {
        let p = detect(&s(&["1", "2"]));
        let out = project_with_source_len(&p, 2, 3, true);
        assert_eq!(out, some(&["-2", "-1", "0"]));
    }

    #[test]
    fn project_numeric_step_ten() {
        let p = detect(&s(&["10", "20", "30"]));
        let out = project_with_source_len(&p, 3, 2, false);
        assert_eq!(out, some(&["40", "50"]));
    }

    #[test]
    fn project_weekday_wraps_past_sun() {
        let p = detect(&s(&["Fri", "Sat"]));
        let out = project_with_source_len(&p, 2, 3, false);
        assert_eq!(out, some(&["Sun", "Mon", "Tue"]));
    }

    #[test]
    fn project_month_wraps_past_dec() {
        let p = detect(&s(&["Nov", "Dec"]));
        let out = project_with_source_len(&p, 2, 3, false);
        assert_eq!(out, some(&["Jan", "Feb", "Mar"]));
    }

    #[test]
    fn project_text_number_padding() {
        let p = detect(&s(&["Item 09", "Item 10"]));
        let out = project_with_source_len(&p, 2, 2, false);
        assert_eq!(out, some(&["Item 11", "Item 12"]));
    }

    #[test]
    fn project_cyclic_copy_wraps() {
        let p = Pattern::CyclicCopy(vec!["a".into(), "b".into(), "c".into()]);
        let out = project_with_source_len(&p, 3, 5, false);
        assert_eq!(out, some(&["a", "b", "c", "a", "b"]));
    }

    #[test]
    fn project_date_forward_iso() {
        let p = detect(&s(&["2025-01-01", "2025-01-02"]));
        let out = project_with_source_len(&p, 2, 2, false);
        assert_eq!(out, some(&["2025-01-03", "2025-01-04"]));
    }

    #[test]
    fn project_date_backward_slashed() {
        let p = detect(&s(&["01/05/2025", "01/06/2025"]));
        let out = project_with_source_len(&p, 2, 2, true);
        assert_eq!(out, some(&["01/03/2025", "01/04/2025"]));
    }

    #[test]
    fn project_weekday_backward() {
        let p = detect(&s(&["Wed", "Thu"]));
        let out = project_with_source_len(&p, 2, 3, true);
        // Going back 3 cells from Wed: Sun, Mon, Tue (top-to-bottom).
        assert_eq!(out, some(&["Sun", "Mon", "Tue"]));
    }

    #[test]
    fn detect_numeric_preserves_decimals() {
        match detect(&s(&["1.5", "2.5", "3.5"])) {
            Pattern::NumericLinear { format, .. } => assert_eq!(format.decimals, 1),
            p => panic!("expected NumericLinear, got {p:?}"),
        }
    }

    #[test]
    fn project_numeric_with_decimals() {
        let p = detect(&s(&["1.5", "2.5"]));
        let out = project_with_source_len(&p, 2, 2, false);
        assert_eq!(out, some(&["3.5", "4.5"]));
    }

    #[test]
    fn detect_text_number_after_multibyte_prefix() {
        // The last non-digit char is 2, 3 or 4 bytes wide, so the digit
        // run starts several bytes after it, not one.
        match detect(&s(&["é1", "é2"])) {
            Pattern::TextNumber { prefix, start, step, width } => {
                assert_eq!(prefix, "é");
                assert_eq!((start, step, width), (1, 1, 1));
            }
            p => panic!("expected TextNumber, got {p:?}"),
        }
        let p = detect(&s(&["中9", "中10"]));
        assert_eq!(project_with_source_len(&p, 2, 2, false), some(&["中11", "中12"]));
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let p = detect(&[format!("{family}1"), format!("{family}2")]);
        assert_eq!(
            project_with_source_len(&p, 2, 1, false),
            vec![Some(format!("{family}3"))]
        );
    }

    #[test]
    fn project_text_number_past_i64_max() {
        let p = detect(&s(&["x0", "x9223372036854775807"]));
        assert_eq!(
            project_with_source_len(&p, 2, 2, false),
            some(&["x18446744073709551614", "x27670116110564327421"])
        );
        assert_eq!(
            project_with_source_len(&p, 2, 2, true),
            some(&["x-18446744073709551614", "x-9223372036854775807"])
        );
    }

    #[test]
    fn project_date_past_chrono_range() {
        // Step is 3,652,058 days. Chrono's calendar ends near year
        // 262,000, about 26 steps out, and the 29,220th cell's offset is
        // past what a `TimeDelta` can hold at all.
        let p = detect(&s(&["0001-01-01", "9999-12-31"]));
        let fwd = project_with_source_len(&p, 2, 30_000, false);
        assert_eq!(fwd.len(), 30_000);
        assert!(fwd.iter().take(20).all(Option::is_some));
        assert!(fwd.iter().skip(100).all(Option::is_none));
        let back = project_with_source_len(&p, 2, 30_000, true);
        assert_eq!(back.len(), 30_000);
        assert!(back.iter().rev().take(20).all(Option::is_some));
        assert!(back.iter().take(29_900).all(Option::is_none));
        // The cells past the range are left alone; none is refilled
        // with the start date.
        assert!(!fwd.contains(&Some("0001-01-01".to_string())));
        assert!(!back.contains(&Some("0001-01-01".to_string())));
    }

    #[test]
    fn project_numeric_step_past_f64_leaves_cells() {
        // 1.7e308 - (-1e308) is infinite, so every projected value is
        // NaN or infinite and has no text to write.
        let p = detect(&s(&["-1e308", "1.7e308"]));
        assert_eq!(project_with_source_len(&p, 2, 2, false), vec![None, None]);
        assert_eq!(project_with_source_len(&p, 2, 2, true), vec![None, None]);
    }

    #[test]
    fn project_numeric_past_i64_keeps_digits() {
        let p = detect(&s(&["100000000000000000000", "200000000000000000000"]));
        assert_eq!(
            project_with_source_len(&p, 2, 1, false),
            some(&["300000000000000000000"])
        );
        let p = detect(&s(&["000000000000000000001", "100000000000000000000"]));
        assert_eq!(
            project_with_source_len(&p, 2, 1, false),
            some(&["200000000000000000000"])
        );
        let p = detect(&s(&["-100000000000000000000", "-200000000000000000000"]));
        assert_eq!(
            project_with_source_len(&p, 2, 1, false),
            some(&["-300000000000000000000"])
        );
    }
}
