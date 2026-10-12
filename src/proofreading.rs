//! Offline English diagnostics. Call `check` on a background worker only.
//! Harper's scalar spans are translated against the exact admitted source.
use std::{cell::RefCell, ops::Range, sync::atomic::{AtomicBool, Ordering}};
use harper_core::{Dialect, Document, parsers::PlainEnglish, spell::FstDictionary};
use harper_core::linting::{LintGroup, LintKind, Linter, Suggestion};

pub(crate) const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_BATCH_BYTES: usize = 8 * 1024;
const MAX_BATCH_CHARS: usize = 2048;
const MAX_ISSUES: usize = 512;
const MAX_SUGGESTIONS: usize = 3;
const MAX_SUGGESTION_BYTES: usize = 1024;
const MAX_MESSAGE_BYTES: usize = 512;
const MAX_WORD_CHARS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IssueKind { Spelling, Grammar }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Issue {
    pub range: Range<usize>,
    pub kind: IssueKind,
    pub message: String,
    pub suggestions: Vec<String>,
    pub original: String,
}

thread_local! {
    // Retain one linter per executor thread; Harper's curated dictionary is
    // itself initialized once per process. No initialization belongs in paint.
    static ENGINE: RefCell<Option<LintGroup>> = const { RefCell::new(None) };
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) { Err("Proofreading cancelled".into()) } else { Ok(()) }
}

fn reserve<T>(values: &mut Vec<T>, additional: usize, limit: usize) -> Result<(), String> {
    let wanted = values.len().checked_add(additional).ok_or("Proofreading allocation is too large")?;
    if wanted > limit { return Err("Proofreading allocation is too large".into()); }
    values.try_reserve_exact(additional).map_err(|_| "Not enough memory for proofreading")?;
    if values.capacity() > limit { return Err("Proofreading allocation is too large".into()); }
    Ok(())
}

fn copy_text(text: &str, limit: usize) -> Result<String, String> {
    if text.len() > limit { return Err("Proofreading text exceeds its limit".into()); }
    let mut out = String::new();
    out.try_reserve_exact(text.len()).map_err(|_| "Not enough memory for proofreading")?;
    if out.capacity() > limit { return Err("Proofreading allocation is too large".into()); }
    out.push_str(text);
    Ok(out)
}

fn mark(protected: &mut [bool], range: Range<usize>) {
    if let Some(span) = protected.get_mut(range) { span.fill(true); }
}

fn opaque(token: &[char]) -> bool {
    if token.len() > MAX_WORD_CHARS { return true; }
    let mut lo = 0;
    let mut hi = token.len();
    while token.get(lo).is_some_and(|c| matches!(c, '(' | '[' | '{' | '<' | '"' | '\'')) { lo += 1; }
    while hi > lo && token.get(hi - 1).is_some_and(|c| matches!(c, ')' | ']' | '}' | '>' | '"' | '\'' | ',' | ';' | ':' | '!' | '?' | '.')) { hi -= 1; }
    let token = token.get(lo..hi).unwrap_or_default();
    if token.is_empty() { return false; }
    if token.len() > MAX_WORD_CHARS { return true; }
    if token.iter().any(|c| c.is_numeric() || matches!(c, '/' | '\\' | '_' | '@' | '=' | '`')) { return true; }
    // Hostnames, file names and dotted identifiers are indivisible non-prose.
    if token.windows(3).any(|w| matches!(w, [a, '.', b] if a.is_alphanumeric() && b.is_alphanumeric())) { return true; }
    if token.windows(2).any(|w| matches!(w, [a, b] if a.is_lowercase() && b.is_uppercase())) { return true; }
    false
}

fn protected_chars(chars: &[char], markdown: bool, cancel: &AtomicBool) -> Result<Vec<bool>, String> {
    let mut protected = Vec::new(); reserve(&mut protected, chars.len(), MAX_SOURCE_BYTES)?;
    protected.resize(chars.len(), false);
    if markdown {
        let mut line_start = 0;
        let mut fence: Option<(char, usize)> = None;
        while line_start < chars.len() {
            cancelled(cancel)?;
            let line_end = chars.get(line_start..).and_then(|tail| tail.iter().position(|c| *c == '\n'))
                .map_or(chars.len(), |n| line_start.saturating_add(n).saturating_add(1));
            let mut first = line_start;
            while chars.get(first).is_some_and(|c| matches!(c, ' ' | '\t')) { first += 1; }
            let marker = chars.get(first).copied();
            let count = chars.get(first..line_end).map_or(0, |line| line.iter().take_while(|c| Some(**c) == marker).count());
            if let Some((kind, minimum)) = fence {
                mark(&mut protected, line_start..line_end);
                if marker == Some(kind) && count >= minimum
                    && chars.get(first.saturating_add(count)..line_end)
                        .is_some_and(|tail| tail.iter().all(|c| c.is_whitespace())) { fence = None; }
            } else if matches!(marker, Some('`' | '~')) && count >= 3 {
                mark(&mut protected, line_start..line_end);
                if let Some(kind) = marker { fence = Some((kind, count)); }
            }
            line_start = line_end;
        }
        let mut i = 0;
        while i < chars.len() {
            if i % 4096 == 0 { cancelled(cancel)?; }
            if protected.get(i).copied().unwrap_or(true) { i += 1; continue; }
            if chars.get(i) == Some(&'`') {
                let run = chars.get(i..).map_or(0, |tail| tail.iter().take_while(|c| **c == '`').count());
                let mut end = i.saturating_add(run);
                while end < chars.len() && chars.get(end) != Some(&'\n') {
                    let closes = chars.get(end..).map_or(0, |tail| tail.iter().take_while(|c| **c == '`').count());
                    if closes == run { end = end.saturating_add(run); break; }
                    end = end.saturating_add(closes.max(1));
                }
                mark(&mut protected, i..end); i = end; continue;
            }
            // Skip the complete Markdown link, including its destination.
            if chars.get(i) == Some(&'[') {
                let mut end = i + 1;
                while end < chars.len() && !matches!(chars.get(end), Some(']' | '\n')) {
                    if end % 4096 == 0 { cancelled(cancel)?; }
                    end += 1;
                }
                if chars.get(end) == Some(&']') && matches!(chars.get(end + 1), Some('(' | '[')) {
                    let opener = chars.get(end + 1).copied();
                    let closer = if opener == Some('(') { ')' } else { ']' };
                    let mut depth = 1usize; end += 2;
                    while end < chars.len() && chars.get(end) != Some(&'\n') {
                        if chars.get(end).copied() == opener { depth = depth.saturating_add(1); }
                        if chars.get(end) == Some(&closer) { depth = depth.saturating_sub(1); }
                        end += 1;
                        if depth == 0 || depth > 32 { break; }
                    }
                    mark(&mut protected, i..end); i = end; continue;
                }
                // This label scan already inspected every intervening opener.
                // Advance past it so malformed bracket runs stay linear.
                i = end.saturating_add(1); continue;
            }
            i += 1;
        }
    }
    // Inspect whole lexical tokens before splitting into words: a URL/path or
    // identifier must never leak an English-looking fragment into diagnostics.
    let mut start = 0;
    while start < chars.len() {
        if start % 4096 == 0 { cancelled(cancel)?; }
        if chars.get(start).is_some_and(|c| c.is_whitespace()) { start += 1; continue; }
        let end = chars.get(start..).and_then(|tail| tail.iter().position(|c| c.is_whitespace()))
            .map_or(chars.len(), |n| start.saturating_add(n));
        if chars.get(start..end).is_some_and(opaque) { mark(&mut protected, start..end); }
        start = end;
    }
    Ok(protected)
}

fn suggestion_text(suggestion: &Suggestion, original: &str) -> Result<Option<String>, String> {
    let (prefix, chars): (&str, &[char]) = match suggestion {
        Suggestion::ReplaceWith(chars) => ("", chars),
        Suggestion::InsertAfter(chars) => (original, chars),
        Suggestion::Remove => return Ok(Some(String::new())),
    };
    let bytes = chars.iter().try_fold(prefix.len(), |n, c| n.checked_add(c.len_utf8()));
    let Some(bytes) = bytes.filter(|n| *n <= MAX_SUGGESTION_BYTES) else { return Ok(None); };
    let mut text = copy_text(prefix, MAX_SUGGESTION_BYTES)?;
    text.try_reserve_exact(bytes.saturating_sub(text.len())).map_err(|_| "Not enough memory for proofreading")?;
    if text.capacity() > MAX_SUGGESTION_BYTES { return Err("Proofreading allocation is too large".into()); }
    text.extend(chars); Ok(Some(text))
}

/// Bounded, synchronous worker operation; caller owns debounce and publication
/// revision checks. The limits cover adapter buffers, not Harper's opaque
/// dictionary/cache allocations. Cancellation cannot interrupt a lint call.
pub(crate) fn check(source: &str, markdown: bool, spelling: bool, grammar: bool, cancel: &AtomicBool) -> Result<Vec<Issue>, String> {
    cancelled(cancel)?;
    if source.len() > MAX_SOURCE_BYTES { return Err("Proofreading is limited to 1 MiB per snapshot".into()); }
    if source.is_empty() || (!spelling && !grammar) { return Ok(Vec::new()); }
    let mut chars = Vec::new(); reserve(&mut chars, source.len(), MAX_SOURCE_BYTES)?;
    let mut offsets = Vec::new(); reserve(&mut offsets, source.len().saturating_add(1), MAX_SOURCE_BYTES + 1)?;
    for (byte, c) in source.char_indices() { offsets.push(byte); chars.push(c); }
    offsets.push(source.len());
    let protected = protected_chars(&chars, markdown, cancel)?;
    // Character-for-character whitespace masking preserves original newlines
    // and the independent original scalar-to-byte map.
    for (c, protected) in chars.iter_mut().zip(&protected) {
        if *protected && !matches!(c, '\r' | '\n') { *c = ' '; }
    }
    let mut issues = Vec::new(); reserve(&mut issues, MAX_ISSUES, MAX_ISSUES)?;
    ENGINE.with(|engine| {
        let mut engine = engine.try_borrow_mut().map_err(|_| "Proofreading engine is already in use")?;
        let dictionary = FstDictionary::curated();
        let linter = engine.get_or_insert_with(|| {
            let mut linter = LintGroup::new_curated(dictionary.clone(), Dialect::American);
            // Space runs are layout (table padding, column alignment), not grammar.
            linter.config.set_rule_enabled("Spaces", false);
            linter
        });
        let mut start = 0;
        let mut run_end = 0;
        while start < chars.len() && issues.len() < MAX_ISSUES {
            cancelled(cancel)?;
            if protected.get(start).copied().unwrap_or(true) { start += 1; continue; }
            if start >= run_end {
                run_end = protected.get(start..).and_then(|tail| tail.iter().position(|p| *p))
                    .map_or(chars.len(), |n| start.saturating_add(n));
            }
            let maximum = start.saturating_add(MAX_BATCH_CHARS).min(run_end);
            let byte_start = offsets.get(start).copied().ok_or("Invalid proofreading offset")?;
            let mut end = maximum;
            while end > start && offsets.get(end).copied().unwrap_or(source.len()).saturating_sub(byte_start) > MAX_BATCH_BYTES { end -= 1; }
            // Complete paragraphs retain grammar context. Artificial fragments
            // receive spelling only, avoiding false quote/capitalization warnings.
            if let Some(split) = chars.get(start..end).and_then(|slice| slice.iter().position(|c| *c == '\n')) {
                end = start.saturating_add(split).saturating_add(1);
            } else if end < run_end {
                if let Some(split) = chars.get(start..end).and_then(|slice| slice.iter().rposition(|c| c.is_whitespace())) {
                    end = start.saturating_add(split).saturating_add(1);
                } else { return Err("Proofreading paragraph has no bounded word boundary".into()); }
            }
            let whole_paragraph = (start == 0 || chars.get(start.saturating_sub(1)) == Some(&'\n') || chars.get(start) == Some(&'\n'))
                && (end == chars.len() || chars.get(end.saturating_sub(1)) == Some(&'\n') || chars.get(end) == Some(&'\n'));
            let byte_end = offsets.get(end).copied().ok_or("Invalid proofreading offset")?;
            let text = source.get(byte_start..byte_end).ok_or("Invalid proofreading text boundary")?;
            if text.trim().is_empty() { start = end; continue; }
            let document = Document::new(text, &PlainEnglish, &dictionary);
            let lints = linter.lint(&document);
            cancelled(cancel)?;
            for lint in lints {
                let kind = if lint.lint_kind == LintKind::Spelling {
                    if !spelling { continue; } IssueKind::Spelling
                } else {
                    if !grammar || !whole_paragraph { continue; } IssueKind::Grammar
                };
                let lo = start.checked_add(lint.span.start).ok_or("Invalid proofreading offset")?;
                let hi = start.checked_add(lint.span.end).ok_or("Invalid proofreading offset")?;
                if lo >= hi || hi > end { continue; }
                let Some((&lo, &hi)) = offsets.get(lo).zip(offsets.get(hi)) else { continue; };
                let Some(original) = source.get(lo..hi) else { continue; };
                let mut suggestions = Vec::new(); reserve(&mut suggestions, MAX_SUGGESTIONS, MAX_SUGGESTIONS)?;
                for suggestion in lint.suggestions.iter().take(MAX_SUGGESTIONS) {
                    if let Some(text) = suggestion_text(suggestion, original)? { suggestions.push(text); }
                }
                let mut message_end = lint.message.len().min(MAX_MESSAGE_BYTES);
                while !lint.message.is_char_boundary(message_end) { message_end = message_end.saturating_sub(1); }
                let message = copy_text(lint.message.get(..message_end).unwrap_or_default(), MAX_MESSAGE_BYTES)?;
                issues.push(Issue { range: lo..hi, kind, message, suggestions, original: copy_text(original, MAX_BATCH_BYTES)? });
                if issues.len() == MAX_ISSUES { break; }
            }
            start = end;
        }
        cancelled(cancel)?;
        issues.sort_by_key(|issue| (issue.range.start, issue.range.end));
        issues.dedup_by(|a, b| a.range == b.range && a.kind == b.kind && a.suggestions == b.suggestions);
        Ok(issues)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn checked(text: &str, markdown: bool) -> Vec<Issue> {
        check(text, markdown, true, true, &AtomicBool::new(false)).unwrap()
    }
    #[test]
    fn english_typo_has_correction() {
        let issues = checked("This is a sentnce.", false);
        assert!(issues.iter().any(|i| i.kind == IssueKind::Spelling && i.original == "sentnce" && i.suggestions.iter().any(|s| s == "sentence")), "{issues:?}");
    }
    #[test]
    fn article_and_repeated_word_use_real_grammar() {
        for source in ["This is an test.", "This is the the test."] {
            let issues = checked(source, false);
            assert!(issues.iter().any(|i| i.kind == IssueKind::Grammar && !i.suggestions.is_empty()), "{source}: {issues:?}");
        }
    }
    #[test]
    fn unicode_and_crlf_ranges_are_exact_bytes() {
        let source = "\u{1f600} caf\u{e9}\r\nThis is a sentnce. e\u{301}";
        let issues = checked(source, false);
        let issue = issues.iter().find(|i| i.original == "sentnce").unwrap();
        assert_eq!(source.get(issue.range.clone()), Some("sentnce"));
        assert_eq!(issue.range.start, source.find("sentnce").unwrap());
    }
    #[test]
    fn non_prose_is_not_split_into_misspelled_fragments() {
        let source = "https://sentnce.example/mispell user@sentnce.example sentnce.example C:\\sentnce\\mispell.txt /sentnce/mispell sentnce_value sentnceValue 123abc";
        assert!(checked(source, false).is_empty());
    }
    #[test]
    fn markdown_code_links_and_fences_are_protected() {
        let source = "`sentnce` [sentnce](https://example.com/mispell)\n```rust\nsentnce mispell\n```\nThis is a sentnce.";
        let issues = checked(source, true);
        assert!(issues.iter().any(|i| i.original == "sentnce"));
        let last = source.rfind("sentnce").unwrap();
        assert!(issues.iter().all(|i| i.range.start >= last.saturating_sub(10)), "{issues:?}");
    }
    #[test]
    fn masks_break_grammar_context() {
        let issues = checked("the https://example.com the", false);
        assert!(!issues.iter().any(|i| i.original.contains("https") || i.original == "the https://example.com the"));
    }
    #[test]
    fn limits_and_cancel_are_explicit() {
        assert!(check(&"a".repeat(MAX_SOURCE_BYTES + 1), false, true, true, &AtomicBool::new(false)).is_err());
        assert!(check("sentnce", false, true, true, &AtomicBool::new(true)).is_err());
        assert!(check("sentnce", false, false, false, &AtomicBool::new(false)).unwrap().is_empty());
    }
    #[test]
    fn protected_long_token_never_becomes_short_word_fragments() {
        assert!(checked(&"x".repeat(MAX_BATCH_BYTES * 2), false).is_empty());
    }

    #[test]
    fn malformed_markdown_brackets_do_not_repeat_suffix_scans() {
        let source = format!("{}\nThis is a sentnce.", "[".repeat(256 * 1024));
        let issues = checked(&source, true);
        assert!(issues.iter().any(|issue| issue.original == "sentnce"));
        assert!(issues.iter().all(|issue| issue.range.start > 256 * 1024));
    }

    #[test]
    fn fence_marker_with_trailing_code_does_not_end_code_protection() {
        let source = "```\nsentnce\n``` still_code\nsentnce\n```\nThis is a sentnce.";
        let issues = checked(source, true);
        assert_eq!(issues.iter().filter(|issue| issue.original == "sentnce").count(), 1);
        assert!(issues.iter().any(|issue| issue.range.start == source.rfind("sentnce").unwrap()));
    }

    #[test]
    fn space_runs_in_tables_and_prose_are_not_marked() {
        let source = "┌────┬──────────┬──────────┐\n│ #  │   File   │   URL    │\n├────┼──────────┼──────────┤\n│ 1  │ upSellPage.vue │ http://localhost:8416/glp1 │\n└────┴──────────┴──────────┘\nTwo  spaces here.    Then four.\nThis is an test.";
        let issues = checked(source, false);
        assert!(issues.iter().all(|issue| !issue.original.trim().is_empty()), "{issues:?}");
        assert!(issues.iter().any(|issue| issue.kind == IssueKind::Grammar && issue.original.contains("an")), "{issues:?}");
    }

    #[test]
    fn split_paragraphs_and_protected_links_do_not_create_grammar_context() {
        for source in [format!("\"{} end.\"", "This sentence contains several ordinary words. ".repeat(80)),
            "\"This sentence contains https://example.com several ordinary words.\"".into()] {
            let issues = checked(&source, false);
            assert!(issues.iter().all(|issue| issue.kind == IssueKind::Spelling), "{issues:?}");
        }
        let issues = checked("This https://example.com sentnce.\nThis is an test.", false);
        assert!(issues.iter().any(|issue| issue.original == "sentnce"));
        assert!(issues.iter().any(|issue| issue.kind == IssueKind::Grammar));
    }
}
