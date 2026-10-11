//! Fallible query/pattern buffers; regex compiler internals have separate limits.
use super::{find::{SearchFlags, search_pattern_chars}, find_current_budget::{Owned, Pool}, find_current_projection::ProjectionError};

pub(super) const QUERY_BYTES: usize = 64 * 1024;

pub(super) fn copy_text(text: &str, pool: &Pool, limit: usize) -> Result<Owned<String>, ProjectionError> {
    if text.len() > limit { return Err(ProjectionError::Limit); }
    let mut out = Owned::<String>::new(pool)?;
    out.reserve(text.len())?;
    out.data.push_str(text);
    Ok(out)
}

pub(super) fn pattern(query: &str, flags: &SearchFlags, pool: &Pool) -> Result<Owned<String>, ProjectionError> {
    if query.len() > QUERY_BYTES { return Err(ProjectionError::Limit); }
    let mut out = Owned::<String>::new(pool)?;
    search_pattern_chars(query, flags, |ch| {
        let needed = out.data.len().checked_add(ch.len_utf8()).ok_or(ProjectionError::Limit)?;
        if needed > QUERY_BYTES { return Err(ProjectionError::Limit); }
        if needed > out.data.capacity() {
            let next = out.data.capacity().max(256).saturating_mul(2).min(QUERY_BYTES).max(needed);
            out.reserve(next)?;
        }
        out.data.push(ch);
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::find_current_budget::Ledger;
    fn flags() -> SearchFlags {
        SearchFlags { regex: false, extended: false, whole_word: false, match_case: true, dot_matches_newline: false }
    }
    #[test]
    fn every_ascii_character_and_unicode_literal_matches_existing_escape_semantics() {
        let input: String = (0..=127).map(char::from).chain("中😀é".chars()).collect();
        let ledger = Ledger::default();
        let out = pattern(&input, &flags(), &ledger.pool(QUERY_BYTES)).unwrap();
        assert_eq!(out.data, regex::escape(&input));
        drop(out); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn extended_stream_is_byte_identical_for_valid_and_malformed_escapes() {
        let mut flags = flags(); flags.extended = true; flags.whole_word = true;
        let inputs = [r"\n\r\t\0\b\\", r"\x1B\xFF\d255\o377", r"\d999\o999\xZ", "a\\😀\\é\\xé\\dé\\oé", "tail\\"];
        let pool = Ledger::default().pool(QUERY_BYTES);
        for input in inputs {
            assert_eq!(pattern(input, &flags, &pool).unwrap().data,
                format!(r"\b{}\b", regex::escape(&crate::text::extended_escape::interpret_extended(input))));
        }
        // Every possible numeric byte, including values encoded as two-byte UTF-8.
        for byte in 0..=255 {
            let input = format!("\\x{byte:02x}\\d{byte:03}\\o{byte:03o}");
            assert_eq!(pattern(&input, &flags, &pool).unwrap().data,
                format!(r"\b{}\b", regex::escape(&crate::text::extended_escape::interpret_extended(&input))));
        }
    }
    #[test]
    fn oversized_escaped_pattern_refuses_whole_output_and_releases_growth_credits() {
        let ledger = Ledger::default(); let pool = ledger.pool(QUERY_BYTES);
        assert!(matches!(pattern(&".".repeat(QUERY_BYTES), &flags(), &pool), Err(ProjectionError::Limit)));
        assert_eq!(ledger.used(), 0);
        assert!(ledger.peak() <= QUERY_BYTES + QUERY_BYTES / 2);
        let mut flags = flags(); flags.regex = true; flags.extended = true;
        let exact = "a".repeat(QUERY_BYTES);
        assert_eq!(pattern(&exact, &flags, &pool).unwrap().data, exact);
    }
    #[test]
    fn malformed_unknown_unicode_escapes_never_split_a_codepoint() {
        let mut flags = flags(); flags.extended = true;
        let pool = Ledger::default().pool(QUERY_BYTES);
        let out = pattern("\\中\\😀", &flags, &pool).unwrap();
        assert_eq!(out.data, regex::escape("\\中\\😀"));
    }
}
