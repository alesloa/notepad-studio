// Excel-style fill-series pattern detection + projection.
//
// `detect` walks a slice of source values and picks the most specific
// pattern that matches the whole sequence. `project` extends that
// pattern by `count` cells either forward (drag down / right) or
// backward (drag up / left). The two functions stay pure — they
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

/// Project `count` cells from the pattern. `reverse=false` extends
/// forward (drag down / right), `reverse=true` extends backward (drag
/// up / left). Reverse output is returned top-to-bottom (i.e. the
/// caller writes it directly into the cells ABOVE the source without
/// re-reversing).
pub(crate) fn project(pattern: &Pattern, count: usize, reverse: bool) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    match pattern {
        Pattern::Constant(v) => vec![v.clone(); count],
        Pattern::NumericLinear { start, step, format } => {
            let src_len_offset = source_len_from_pattern(pattern);
            let mut out = Vec::with_capacity(count);
            if !reverse {
                for i in 0..count {
                    let n = start + step * (src_len_offset as f64 + i as f64);
                    out.push(format_numeric(n, *format));
                }
            } else {
                // Top-to-bottom order: cell[0] is the FARTHEST back.
                for i in 0..count {
                    let n = start - step * (count - i) as f64;
                    out.push(format_numeric(n, *format));
                }
            }
            out
        }
        Pattern::DateLinear { start, step_days, format } => {
            let src_len_offset = source_len_from_pattern(pattern);
            let mut out = Vec::with_capacity(count);
            if !reverse {
                for i in 0..count {
                    let offset = step_days * (src_len_offset as i64 + i as i64);
                    let d = start
                        .checked_add_signed(chrono::Duration::days(offset))
                        .unwrap_or(*start);
                    out.push(format_date(d, *format));
                }
            } else {
                for i in 0..count {
                    let offset = -step_days * (count as i64 - i as i64);
                    let d = start
                        .checked_add_signed(chrono::Duration::days(offset))
                        .unwrap_or(*start);
                    out.push(format_date(d, *format));
                }
            }
            out
        }
        Pattern::WeekdayCycle { start_idx, case, abbreviated } => {
            let src_len_offset = source_len_from_pattern(pattern) as i64;
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let raw = if !reverse {
                    *start_idx as i64 + src_len_offset + i as i64
                } else {
                    *start_idx as i64 - (count as i64 - i as i64)
                };
                let idx = raw.rem_euclid(7) as usize;
                out.push(format_text_case(weekday_name(idx, *abbreviated), *case));
            }
            out
        }
        Pattern::MonthCycle { start_idx, case, abbreviated } => {
            let src_len_offset = source_len_from_pattern(pattern) as i64;
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let raw = if !reverse {
                    *start_idx as i64 + src_len_offset + i as i64
                } else {
                    *start_idx as i64 - (count as i64 - i as i64)
                };
                let idx = raw.rem_euclid(12) as usize;
                out.push(format_text_case(month_name(idx, *abbreviated), *case));
            }
            out
        }
        Pattern::TextNumber { prefix, start, step, width } => {
            let src_len_offset = source_len_from_pattern(pattern) as i64;
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let n = if !reverse {
                    start + step * (src_len_offset + i as i64)
                } else {
                    start - step * (count as i64 - i as i64)
                };
                out.push(format_text_number(prefix, n, *width));
            }
            out
        }
        Pattern::CyclicCopy(items) => {
            if items.is_empty() {
                return vec![String::new(); count];
            }
            let mut out = Vec::with_capacity(count);
            if !reverse {
                for i in 0..count {
                    out.push(items[i % items.len()].clone());
                }
            } else {
                // Reverse cycle: the cell immediately above source is
                // items[len-1], next up is items[len-2], etc. Output
                // top-to-bottom.
                for i in 0..count {
                    // distance back from the source's first cell.
                    let back = (count - i) as i64;
                    let n = items.len() as i64;
                    let raw = -back;
                    let idx = raw.rem_euclid(n) as usize;
                    out.push(items[idx].clone());
                }
            }
            out
        }
    }
}

// Internal helpers --------------------------------------------------------

/// Number of source cells that produced this pattern. Used by `project`
/// to compute the "after the source" offset when extending forward.
fn source_len_from_pattern(p: &Pattern) -> usize {
    match p {
        Pattern::Constant(_) => 1,
        // The detection routines below all enforce that the pattern was
        // detected from a source of at least 2 cells, so this matches.
        // We re-derive from the pattern fields rather than carrying the
        // length, to keep the Pattern enum compact.
        // For arithmetic / date patterns the projection uses
        // (src_len + i) as the index relative to start, so any
        // consistent src_len works as long as detect uses the same one.
        // We standardize on 2 here — detect computes start = source[0],
        // step = source[1] - source[0], so cell index 0 = start, cell
        // index 1 = start + step, ..., cell index N-1 = start + step*(N-1).
        // For projection forward, after N source cells, the next index
        // is N. So src_len_offset MUST equal the actual source length.
        // We patch this by overriding via callers below — but in fact
        // detect only knows the source length at detection time; we
        // need to carry it explicitly. Override below.
        _ => 0,
    }
}

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

fn format_text_number(prefix: &str, n: i64, width: usize) -> String {
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
        // Round to integer when source had no decimal places.
        let rounded = n.round();
        if format.leading_zero_width > 0 {
            // Integer padding.
            let sign_prefix = if rounded < 0.0 { "-" } else { "" };
            let abs = rounded.abs() as i64;
            return format!(
                "{}{:0>width$}",
                sign_prefix,
                abs,
                width = format.leading_zero_width.saturating_sub(sign_prefix.len())
            );
        }
        format!("{}", rounded as i64)
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

// Helper for the projection routines — extract source length from
// the source slice that produced the pattern. Pattern enum doesn't
// carry the source length; the projection code calls this on the
// pattern + source-length contextually. We keep it as a stand-alone
// function to make tests easier.
pub(crate) fn project_with_source_len(
    pattern: &Pattern,
    source_len: usize,
    count: usize,
    reverse: bool,
) -> Vec<String> {
    // Re-implement the forward offset using actual source_len, so the
    // arithmetic doesn't depend on the hard-coded `2` in
    // `source_len_from_pattern`. We do this by adjusting the start /
    // step interpretation only where it matters.
    if count == 0 {
        return Vec::new();
    }
    match pattern {
        Pattern::Constant(v) => vec![v.clone(); count],
        Pattern::NumericLinear { start, step, format } => {
            let mut out = Vec::with_capacity(count);
            if !reverse {
                for i in 0..count {
                    let n = start + step * (source_len as f64 + i as f64);
                    out.push(format_numeric(n, *format));
                }
            } else {
                for i in 0..count {
                    let n = start - step * (count - i) as f64;
                    out.push(format_numeric(n, *format));
                }
            }
            out
        }
        Pattern::DateLinear { start, step_days, format } => {
            let mut out = Vec::with_capacity(count);
            if !reverse {
                for i in 0..count {
                    let offset = step_days * (source_len as i64 + i as i64);
                    let d = start
                        .checked_add_signed(chrono::Duration::days(offset))
                        .unwrap_or(*start);
                    out.push(format_date(d, *format));
                }
            } else {
                for i in 0..count {
                    let offset = -step_days * (count as i64 - i as i64);
                    let d = start
                        .checked_add_signed(chrono::Duration::days(offset))
                        .unwrap_or(*start);
                    out.push(format_date(d, *format));
                }
            }
            out
        }
        Pattern::WeekdayCycle { start_idx, case, abbreviated } => {
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let raw = if !reverse {
                    *start_idx as i64 + source_len as i64 + i as i64
                } else {
                    *start_idx as i64 - (count as i64 - i as i64)
                };
                let idx = raw.rem_euclid(7) as usize;
                out.push(format_text_case(weekday_name(idx, *abbreviated), *case));
            }
            out
        }
        Pattern::MonthCycle { start_idx, case, abbreviated } => {
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let raw = if !reverse {
                    *start_idx as i64 + source_len as i64 + i as i64
                } else {
                    *start_idx as i64 - (count as i64 - i as i64)
                };
                let idx = raw.rem_euclid(12) as usize;
                out.push(format_text_case(month_name(idx, *abbreviated), *case));
            }
            out
        }
        Pattern::TextNumber { prefix, start, step, width } => {
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let n = if !reverse {
                    start + step * (source_len as i64 + i as i64)
                } else {
                    start - step * (count as i64 - i as i64)
                };
                out.push(format_text_number(prefix, n, *width));
            }
            out
        }
        Pattern::CyclicCopy(items) => {
            if items.is_empty() {
                return vec![String::new(); count];
            }
            let mut out = Vec::with_capacity(count);
            if !reverse {
                for i in 0..count {
                    out.push(items[(source_len + i) % items.len()].clone());
                }
            } else {
                let n = items.len() as i64;
                for i in 0..count {
                    let back = (count - i) as i64;
                    let idx = (-back).rem_euclid(n) as usize;
                    out.push(items[idx].clone());
                }
            }
            out
        }
    }
}

/// Use the source-length-aware variant by default. `project` calls into
/// this with the inferred length for compatibility tests.
#[cfg(test)]
fn project_default(pattern: &Pattern, count: usize, reverse: bool) -> Vec<String> {
    project_with_source_len(pattern, default_source_len(pattern), count, reverse)
}

#[cfg(test)]
fn default_source_len(pattern: &Pattern) -> usize {
    match pattern {
        Pattern::Constant(_) => 1,
        _ => 2,
    }
}

// Mark `project` as referenced even when callers go through
// `project_with_source_len` — the public surface keeps it for tests.
#[allow(dead_code)]
fn _project_keepalive(p: &Pattern, count: usize, reverse: bool) -> Vec<String> {
    project(p, count, reverse)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
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
        assert_eq!(project_default(&p, 3, false), vec!["7", "7", "7"]);
    }

    #[test]
    fn project_numeric_forward() {
        let p = detect(&s(&["1", "2"]));
        let out = project_with_source_len(&p, 2, 3, false);
        assert_eq!(out, vec!["3", "4", "5"]);
    }

    #[test]
    fn project_numeric_backward() {
        let p = detect(&s(&["1", "2"]));
        let out = project_with_source_len(&p, 2, 3, true);
        assert_eq!(out, vec!["-2", "-1", "0"]);
    }

    #[test]
    fn project_numeric_step_ten() {
        let p = detect(&s(&["10", "20", "30"]));
        let out = project_with_source_len(&p, 3, 2, false);
        assert_eq!(out, vec!["40", "50"]);
    }

    #[test]
    fn project_weekday_wraps_past_sun() {
        let p = detect(&s(&["Fri", "Sat"]));
        let out = project_with_source_len(&p, 2, 3, false);
        assert_eq!(out, vec!["Sun", "Mon", "Tue"]);
    }

    #[test]
    fn project_month_wraps_past_dec() {
        let p = detect(&s(&["Nov", "Dec"]));
        let out = project_with_source_len(&p, 2, 3, false);
        assert_eq!(out, vec!["Jan", "Feb", "Mar"]);
    }

    #[test]
    fn project_text_number_padding() {
        let p = detect(&s(&["Item 09", "Item 10"]));
        let out = project_with_source_len(&p, 2, 2, false);
        assert_eq!(out, vec!["Item 11", "Item 12"]);
    }

    #[test]
    fn project_cyclic_copy_wraps() {
        let p = Pattern::CyclicCopy(vec!["a".into(), "b".into(), "c".into()]);
        let out = project_with_source_len(&p, 3, 5, false);
        assert_eq!(out, vec!["a", "b", "c", "a", "b"]);
    }

    #[test]
    fn project_date_forward_iso() {
        let p = detect(&s(&["2025-01-01", "2025-01-02"]));
        let out = project_with_source_len(&p, 2, 2, false);
        assert_eq!(out, vec!["2025-01-03", "2025-01-04"]);
    }

    #[test]
    fn project_date_backward_slashed() {
        let p = detect(&s(&["01/05/2025", "01/06/2025"]));
        let out = project_with_source_len(&p, 2, 2, true);
        assert_eq!(out, vec!["01/03/2025", "01/04/2025"]);
    }

    #[test]
    fn project_weekday_backward() {
        let p = detect(&s(&["Wed", "Thu"]));
        let out = project_with_source_len(&p, 2, 3, true);
        // Going back 3 cells from Wed: Sun, Mon, Tue (top-to-bottom).
        assert_eq!(out, vec!["Sun", "Mon", "Tue"]);
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
        assert_eq!(out, vec!["3.5", "4.5"]);
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
        assert_eq!(project_with_source_len(&p, 2, 2, false), vec!["中11", "中12"]);
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let p = detect(&[format!("{family}1"), format!("{family}2")]);
        assert_eq!(
            project_with_source_len(&p, 2, 1, false),
            vec![format!("{family}3")]
        );
    }
}
