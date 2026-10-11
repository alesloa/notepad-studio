// Behavior references: Notepad++ manual, Searching/Substitutions; Scintilla
// SCI_REPLACETARGETRE. Original bounded subset over Rust regex captures.
use std::ops::Range;

use regex::{Captures, Regex};

pub(crate) const MAX_TEMPLATE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_EXPANDED_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_EDITOR_REPLACEMENTS: usize = 4096;
const MAX_PARTS: usize = 65_536;
pub(crate) const MAX_CACHED_MATCHES: usize = 1_000_000;

#[derive(Debug)]
enum Part {
    Literal(String),
    Capture(usize),
}

#[derive(Debug)]
pub(crate) struct Template(Vec<Part>);

#[derive(Debug)]
pub(crate) struct Replacement {
    pub(crate) range: Range<usize>,
    pub(crate) text: String,
    pub(crate) changed: bool,
}

fn error(message: impl AsRef<str>) -> String {
    format!("Invalid regex replacement: {}", message.as_ref())
}

impl Template {
    pub(crate) fn parse(text: &str, regex: &Regex) -> Result<Self, String> {
        if text.len() > MAX_TEMPLATE_BYTES {
            return Err(error("template exceeds 1 MiB; shorten the replacement"));
        }
        let mut chars = text.chars().peekable();
        let mut parts = Vec::new();
        let mut literal = String::new();
        while let Some(ch) = chars.next() {
            let capture = match ch {
                '\\' => {
                    let Some(quoted) = chars.next() else {
                        return Err(error(
                            "trailing backslash; use \\\\ for a literal backslash",
                        ));
                    };
                    if quoted == '0' {
                        return Err(error(
                            "unsupported numeric/NUL escape \\0; use $0 or $& for the whole match",
                        ));
                    } else if quoted.is_ascii_digit() {
                        Some(quoted.to_digit(10).unwrap_or(0) as usize)
                    } else {
                        let value = match quoted {
                            'a' => '\u{7}',
                            'e' => '\u{1b}',
                            'f' => '\u{c}',
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            'v' => '\u{b}',
                            'l' | 'L' | 'u' | 'U' | 'E' | 'x' | 'o' | 'c' | 'g' | 'k' => {
                                return Err(error(format!("unsupported \\{quoted} operator; use captures or literal Unicode text")));
                            }
                            other => other,
                        };
                        literal.push(value);
                        None
                    }
                }
                '$' => match chars.peek().copied() {
                    Some('$') => {
                        chars.next();
                        literal.push('$');
                        None
                    }
                    Some('&') => {
                        chars.next();
                        Some(0)
                    }
                    Some('+') => {
                        chars.next();
                        if chars.next() != Some('{') {
                            return Err(error(
                                "unsupported $+ operator; use $+{name} for a named capture",
                            ));
                        }
                        let reference = read_braced(&mut chars)?;
                        Some(capture_index(&reference, regex)?)
                    }
                    Some('{') => {
                        chars.next();
                        let reference = read_braced(&mut chars)?;
                        Some(capture_index(&reference, regex)?)
                    }
                    Some('`' | '\'' | '^') => {
                        return Err(error("unsupported prefix/suffix/last-capture operator; use an explicit capture"));
                    }
                    Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
                        // Keep existing Find-in-Files Rust $name spellings readable.
                        // Braces disambiguate ${1}suffix from the legacy $1suffix.
                        let reference: String = std::iter::from_fn(|| {
                            chars.next_if(|c| c.is_ascii_alphanumeric() || *c == '_')
                        })
                        .collect();
                        Some(capture_index(&reference, regex)?)
                    }
                    _ => {
                        literal.push('$');
                        None
                    }
                },
                '(' | ')' | '?' => {
                    return Err(error(format!("unsupported grouping/conditional operator {ch}; escape it as \\{ch} for literal text")));
                }
                other => {
                    literal.push(other);
                    None
                }
            };
            if let Some(index) = capture {
                if index >= regex.captures_len() {
                    return Err(error(format!(
                        "capture {index} is not defined by the search pattern"
                    )));
                }
                if !literal.is_empty() {
                    parts.push(Part::Literal(std::mem::take(&mut literal)));
                }
                parts.push(Part::Capture(index));
                if parts.len() > MAX_PARTS {
                    return Err(error(
                        "too many capture references; simplify the replacement",
                    ));
                }
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Literal(literal));
        }
        Ok(Self(parts))
    }

    fn expand(&self, captures: &Captures<'_>, budget: &mut usize) -> Result<String, String> {
        let mut result = String::new();
        for part in &self.0 {
            let value = match part {
                Part::Literal(value) => value.as_str(),
                Part::Capture(index) => captures.get(*index).map_or("", |m| m.as_str()),
            };
            *budget = budget.checked_sub(value.len()).ok_or_else(|| {
                error("expanded text exceeds 16 MiB; use a smaller selection or replacement")
            })?;
            result.push_str(value);
        }
        Ok(result)
    }

    pub(crate) fn plan(
        &self,
        regex: &Regex,
        text: &str,
        scope: Option<&Range<usize>>,
        limit: usize,
    ) -> Result<Vec<Replacement>, String> {
        validate_source(text, scope)?;
        let mut edits = Vec::new();
        let mut budget = MAX_EXPANDED_BYTES;
        for captures in regex.captures_iter(text) {
            let Some(matched) = captures.get(0) else {
                continue;
            };
            let range = matched.start()..matched.end();
            if !within_scope(&range, scope) {
                continue;
            }
            if edits.len() >= limit {
                return Err(error(format!(
                    "more than {limit} replacements; use a smaller selection"
                )));
            }
            let replacement = self.expand(&captures, &mut budget)?;
            edits.push(Replacement {
                range,
                changed: replacement != matched.as_str(),
                text: replacement,
            });
        }
        Ok(edits)
    }

    pub(crate) fn plan_one(
        &self,
        regex: &Regex,
        text: &str,
        scope: Option<&Range<usize>>,
        preferred: Option<&Range<usize>>,
        caret: usize,
    ) -> Result<Vec<Replacement>, String> {
        validate_source(text, scope)?;
        let mut first = None;
        let mut following = None;
        let mut preferred_match = None;
        for captures in regex.captures_iter(text) {
            let Some(matched) = captures.get(0) else {
                continue;
            };
            let range = matched.start()..matched.end();
            if !within_scope(&range, scope) {
                continue;
            }
            if preferred == Some(&range) {
                preferred_match = Some(captures);
                break;
            }
            if following.is_none() && range.start >= caret {
                following = Some(captures);
                if preferred.is_none() {
                    break;
                }
            } else if first.is_none() {
                first = Some(captures);
            }
        }
        let Some(captures) = preferred_match.or(following).or(first) else {
            return Ok(Vec::new());
        };
        let Some(matched) = captures.get(0) else {
            return Ok(Vec::new());
        };
        let mut budget = MAX_EXPANDED_BYTES;
        let replacement = self.expand(&captures, &mut budget)?;
        Ok(vec![Replacement {
            range: matched.start()..matched.end(),
            changed: replacement != matched.as_str(),
            text: replacement,
        }])
    }
}

fn read_braced(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<String, String> {
    let mut reference = String::new();
    for ch in chars.by_ref() {
        if ch == '}' {
            return Ok(reference);
        }
        if reference.len() >= 1024 {
            return Err(error("capture reference exceeds 1024 bytes"));
        }
        reference.push(ch);
    }
    Err(error("unclosed capture reference; close it with }"))
}

fn capture_index(reference: &str, regex: &Regex) -> Result<usize, String> {
    if reference.len() > 1024 {
        return Err(error("capture reference exceeds 1024 bytes"));
    }
    if reference.is_empty() {
        return Err(error("empty capture reference"));
    }
    if reference.chars().all(|c| c.is_ascii_digit()) {
        reference
            .parse()
            .map_err(|_| error("capture number is too large"))
    } else if reference.starts_with('^') {
        Err(error(
            "unsupported special variable; use an explicit capture",
        ))
    } else {
        regex
            .capture_names()
            .position(|name| name == Some(reference))
            .ok_or_else(|| {
                error(format!(
                    "capture {reference} is not defined by the search pattern"
                ))
            })
    }
}

pub(crate) fn within_scope(range: &Range<usize>, scope: Option<&Range<usize>>) -> bool {
    scope.is_none_or(|scope| range.start >= scope.start && range.end <= scope.end)
}

pub(crate) fn validate_source(text: &str, scope: Option<&Range<usize>>) -> Result<(), String> {
    if text.len() > MAX_SOURCE_BYTES {
        return Err(error(
            "document exceeds 64 MiB; use Find in Files on a smaller file",
        ));
    }
    if scope.is_some_and(|range| range.start > range.end || text.get(range.clone()).is_none()) {
        return Err(error("selection range is stale; select the text again"));
    }
    Ok(())
}

pub(crate) fn shifted_scope(
    scope: Option<&Range<usize>>,
    edits: &[Replacement],
) -> Option<Range<usize>> {
    let scope = scope?;
    let mut end = scope.end;
    for edit in edits {
        end = end
            .checked_sub(edit.range.end.checked_sub(edit.range.start)?)?
            .checked_add(edit.text.len())?;
    }
    Some(scope.start..end.max(scope.start))
}

pub(crate) fn apply_to_string(text: &str, edits: &[Replacement]) -> Result<String, String> {
    validate_source(text, None)?;
    if edits.len() > MAX_CACHED_MATCHES {
        return Err(error("too many replacement ranges"));
    }
    let mut output = String::new();
    let mut cursor = 0;
    for edit in edits {
        let Some(prefix) = text.get(cursor..edit.range.start) else {
            return Err(error("stale or overlapping match ranges"));
        };
        if text.get(edit.range.clone()).is_none() {
            return Err(error("stale match range"));
        }
        if output
            .len()
            .saturating_add(prefix.len())
            .saturating_add(edit.text.len())
            > MAX_SOURCE_BYTES + MAX_EXPANDED_BYTES
        {
            return Err(error("result exceeds 80 MiB; use a smaller replacement"));
        }
        output.push_str(prefix);
        output.push_str(&edit.text);
        cursor = edit.range.end;
    }
    let Some(suffix) = text.get(cursor..) else {
        return Err(error("stale match range"));
    };
    if output.len().saturating_add(suffix.len()) > MAX_SOURCE_BYTES + MAX_EXPANDED_BYTES {
        return Err(error("result exceeds 80 MiB; use a smaller replacement"));
    }
    output.push_str(suffix);
    Ok(output)
}

#[cfg(test)]
#[path = "regex_replacement_tests.rs"]
mod tests;
