// PowerEditor/src/ScintillaComponent/AutoCompletion.cpp:469 — buffer word candidates.
// Scan at most 128 KiB around the caret; skip identifiers over 256 bytes.
use std::collections::BTreeSet;
use super::{is_word_char, MAX_CANDIDATES, MIN_CANDIDATE_LEN};

pub(crate) const MAX_CANDIDATE_BYTES: usize = 256;
const MAX_SCAN_BYTES: usize = 128 * 1024;

pub(crate) fn collect_word_candidates(
    rope: &ropey::Rope,
    prefix: &str,
    ignore_numbers: bool,
    caret: usize,
) -> Vec<String> {
    collect_word_candidates_with_environment(rope, prefix, ignore_numbers, caret, None)
}

pub(super) fn collect_word_candidates_with_environment(
    rope: &ropey::Rope,
    prefix: &str,
    ignore_numbers: bool,
    caret: usize,
    environment: Option<&crate::api_registry::ApiEnvironment>,
) -> Vec<String> {
    if prefix.len() > MAX_CANDIDATE_BYTES {
        return Vec::new();
    }
    let end = caret.min(rope.len_bytes()).saturating_sub(MAX_SCAN_BYTES / 2)
        .saturating_add(MAX_SCAN_BYTES).min(rope.len_bytes());
    let start = end.saturating_sub(MAX_SCAN_BYTES);
    let mut start_char = rope.byte_to_char(start);
    if rope.char_to_byte(start_char) < start {
        start_char = start_char.saturating_add(1);
    }
    let end_char = rope.byte_to_char(end);
    let Some(slice) = rope.get_slice(start_char..end_char) else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut current = String::new();
    // A token cut by either scan boundary is not a complete identifier.
    let is_word = |ch| environment.map_or_else(|| is_word_char(ch), |env| env.is_word_char(ch));
    let mut skipping = start_char > 0 && rope.get_char(start_char - 1).is_some_and(is_word);
    let lower_prefix = prefix.to_ascii_lowercase();
    let push = |word: &str, seen: &mut BTreeSet<String>| {
        if word.len() >= MIN_CANDIDATE_LEN
            && word.len() > prefix.len()
            && environment.map_or_else(
                || word.to_ascii_lowercase().starts_with(&lower_prefix),
                |env| env.matches_prefix(word, prefix),
            )
            && !(ignore_numbers && word.chars().next().is_some_and(|c| c.is_ascii_digit()))
        {
            seen.insert(word.to_string());
        }
    };
    for ch in slice.chars() {
        if is_word(ch) {
            if !skipping {
                if current.len().saturating_add(ch.len_utf8()) > MAX_CANDIDATE_BYTES {
                    current.clear();
                    skipping = true;
                } else {
                    current.push(ch);
                }
            }
        } else {
            if !skipping {
                push(&current, &mut seen);
            }
            current.clear();
            skipping = false;
        }
        if seen.len() >= MAX_CANDIDATES * 4 {
            break;
        }
    }
    if !skipping && (end_char == rope.len_chars() || !rope.get_char(end_char).is_some_and(is_word)) {
        push(&current, &mut seen);
    }
    seen.into_iter().take(MAX_CANDIDATES).collect()
}
