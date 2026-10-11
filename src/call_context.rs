// PowerEditor/src/ScintillaComponent/FunctionCallTip.cpp:306-379 —
// function names, overload parameters and the active argument.
// Original lexical scan; runs on an exact document prefix, never stale styles.

use std::ops::Range;
use crate::api_registry::ApiEnvironment;

pub(crate) const MAX_CALL_SCAN_BYTES: usize = 8 * 1024 * 1024;
const MAX_DEPTH: usize = 128;
const MAX_NAME_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallLanguage { Plain, General, JavaScript, Rust, Python, Shell, PowerShell, Ruby, Lua, Sql }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CallContext {
    pub(crate) name: String,
    pub(crate) name_range: Range<usize>,
    pub(crate) open_byte: usize,
    pub(crate) argument: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanLimit { PrefixTooLong, NestingTooDeep, LiteralDelimiterTooLong }

struct Frame { close: char, call: Option<CallContext> }

/// `source` must begin at document byte zero. A local slice may start in a
/// multiline literal, so callers must not pretend its initial state is code.
/// Flatten at most this bounded prefix on a background executor.
pub(crate) fn find_call(
    source: &str, caret: usize, environment: &ApiEnvironment, language: CallLanguage,
) -> Result<Option<CallContext>, ScanLimit> {
    let caret = source.floor_char_boundary(caret.min(source.len()));
    if caret > MAX_CALL_SCAN_BYTES { return Err(ScanLimit::PrefixTooLong); }
    let text = source.get(..caret).unwrap_or_default();
    let mut frames: Vec<Frame> = Vec::new();
    let mut at = 0;
    let mut operand = true;
    let mut word: Option<Range<usize>> = None;
    let mut continuing_word = false;
    while let Some(rest) = text.get(at..) {
        let Some(ch) = rest.chars().next() else { break; };
        if oversized_literal_delimiter(rest, language) { return Err(ScanLimit::LiteralDelimiterTooLong); }
        let regex_operand = operand || (language == CallLanguage::JavaScript && word.as_ref()
            .and_then(|range| text.get(range.clone())).is_some_and(|name| matches!(name,
                "return" | "throw" | "yield" | "case" | "delete" | "typeof" | "void" | "new" | "in" | "of" | "instanceof")));
        if let Some(end) = literal_end(text, at, language, regex_operand) {
            at = end;
            if !comment_start(rest, language) { operand = false; word = None; }
            continuing_word = false;
            continue;
        }
        if ch == environment.stop {
            operand = close_frame(&mut frames, ch) && language == CallLanguage::JavaScript;
        } else if ch == environment.start {
            if frames.len() >= MAX_DEPTH { return Err(ScanLimit::NestingTooDeep); }
            frames.push(Frame { close: environment.stop, call: word.as_ref().and_then(|range| {
                let name = text.get(range.clone())?;
                if name.len() > MAX_NAME_BYTES { return None; }
                Some(CallContext { name: name.to_owned(), name_range: range.clone(), open_byte: at, argument: 0 })
            }) });
            operand = true;
        } else if matches!(ch, '(' | '[' | '{') {
            if frames.len() >= MAX_DEPTH { return Err(ScanLimit::NestingTooDeep); }
            frames.push(Frame { close: match ch { '(' => ')', '[' => ']', _ => '}' }, call: None });
            operand = true;
        } else if matches!(ch, ')' | ']' | '}') {
            operand = close_frame(&mut frames, ch) && language == CallLanguage::JavaScript;
        } else if ch == environment.separator {
            if let Some(call) = frames.last_mut().and_then(|frame| frame.call.as_mut()) {
                call.argument = call.argument.saturating_add(1);
            }
            operand = true;
        } else if ch == environment.terminal {
            // A statement inside a nested lambda/body is not the end of its
            // enclosing call. A terminator at the call's own depth is.
            if frames.last().is_none_or(|frame| frame.call.is_some()) { frames.clear(); }
            operand = true;
        } else if !ch.is_whitespace() {
            operand = matches!(ch, '=' | ':' | '!' | '?' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '~' | '<' | '>');
        }
        let next = at.saturating_add(ch.len_utf8());
        if environment.is_word_char(ch) && ch != environment.start && ch != environment.stop && ch != environment.separator && ch != environment.terminal {
            if continuing_word { if let Some(range) = word.as_mut() { range.end = next; } }
            else { word = Some(at..next); }
            continuing_word = true;
        } else {
            continuing_word = false;
            if !ch.is_whitespace() { word = None; }
        }
        at = next;
    }
    Ok(frames.into_iter().rev().find_map(|frame| frame.call))
}

fn close_frame(frames: &mut Vec<Frame>, close: char) -> bool {
    if frames.last().is_some_and(|frame| frame.close == close) {
        return frames.pop().and_then(|frame| frame.call).is_some_and(|call|
            matches!(call.name.as_str(), "if" | "for" | "while" | "switch" | "with" | "catch"));
    }
    frames.clear();
    false
}

fn comment_start(rest: &str, language: CallLanguage) -> bool {
    match language {
        CallLanguage::General | CallLanguage::JavaScript | CallLanguage::Rust => rest.starts_with("//") || rest.starts_with("/*"),
        CallLanguage::Sql => rest.starts_with("--") || rest.starts_with("/*"),
        CallLanguage::Lua => rest.starts_with("--"),
        CallLanguage::PowerShell => rest.starts_with('#') || rest.starts_with("<#"),
        CallLanguage::Python | CallLanguage::Shell | CallLanguage::Ruby => rest.starts_with('#'),
        CallLanguage::Plain => false,
    }
}

fn oversized_literal_delimiter(rest: &str, language: CallLanguage) -> bool {
    if language == CallLanguage::Rust {
        let prefix = if rest.starts_with("br") { 2 } else if rest.starts_with('r') { 1 } else { return false; };
        let hashes = rest.get(prefix..).unwrap_or_default().bytes().take_while(|&byte| byte == b'#').count();
        return hashes > 32 && rest.get(prefix.saturating_add(hashes)..).is_some_and(|tail| tail.starts_with('"'));
    }
    if language == CallLanguage::Lua {
        let candidate = rest.strip_prefix("--").unwrap_or(rest);
        if let Some(tail) = candidate.strip_prefix('[') {
            let equals = tail.bytes().take_while(|&byte| byte == b'=').count();
            return equals > 32 && tail.get(equals..).is_some_and(|tail| tail.starts_with('['));
        }
    }
    false
}

fn literal_end(text: &str, at: usize, language: CallLanguage, operand: bool) -> Option<usize> {
    let rest = text.get(at..)?;
    let ch = rest.chars().next()?;
    let c_comments = matches!(language, CallLanguage::General | CallLanguage::JavaScript | CallLanguage::Rust | CallLanguage::Sql);
    if c_comments && rest.starts_with("/*") {
        return Some(block_comment(text, at, "/*", "*/", matches!(language, CallLanguage::Rust | CallLanguage::Sql)));
    }
    if c_comments && language != CallLanguage::Sql && rest.starts_with("//") { return Some(line_end(text, at)); }
    if language == CallLanguage::PowerShell && rest.starts_with("<#") { return Some(block_comment(text, at, "<#", "#>", false)); }
    if language == CallLanguage::Lua && rest.starts_with("--") {
        if let Some((length, stop)) = lua_delimiter(rest.get(2..)?) {
            return Some(delimited_end(text, at.saturating_add(2 + length), &stop));
        }
        return Some(line_end(text, at));
    }
    if language == CallLanguage::Sql && rest.starts_with("--") { return Some(line_end(text, at)); }
    if ch == '#' && matches!(language, CallLanguage::Python | CallLanguage::Shell | CallLanguage::PowerShell | CallLanguage::Ruby) {
        return Some(line_end(text, at));
    }
    if language == CallLanguage::Rust {
        if let Some((length, stop)) = rust_delimiter(rest) { return Some(delimited_end(text, at.saturating_add(length), &stop)); }
        if ch == '\'' && !rust_character(rest) { return None; }
    }
    if language == CallLanguage::General && rest.starts_with("R\"") {
        let delimiter = rest.get(2..)?.split_once('(')?.0;
        if delimiter.len() <= 16 && !delimiter.chars().any(|c| c.is_whitespace() || matches!(c, ')' | '\\')) {
            let stop = format!("){delimiter}\"");
            return Some(delimited_end(text, at.saturating_add(3 + delimiter.len()), &stop));
        }
    }
    if language == CallLanguage::Lua {
        if let Some((length, stop)) = lua_delimiter(rest) { return Some(delimited_end(text, at.saturating_add(length), &stop)); }
    }
    if language == CallLanguage::PowerShell && (rest.starts_with("@\"\n") || rest.starts_with("@'\n") || rest.starts_with("@\"\r\n") || rest.starts_with("@'\r\n")) {
        let quote = rest.chars().nth(1)?;
        let stop = format!("\n{quote}@");
        return Some(delimited_end(text, at.saturating_add(2), &stop));
    }
    if language == CallLanguage::Python && (rest.starts_with("\"\"\"") || rest.starts_with("'''")) {
        return Some(quoted_end(text, at, ch, 3, true, false, '\\'));
    }
    if language == CallLanguage::JavaScript && ch == '/' && operand { return Some(regex_end(text, at)); }
    if ch == '[' && language == CallLanguage::Sql {
        return Some(quoted_end(text, at, ']', 1, false, true, '\\'));
    }
    if matches!(ch, '\'' | '"' | '`') {
        let doubled = language == CallLanguage::Sql || (language == CallLanguage::PowerShell && ch == '\'');
        let escape = if language == CallLanguage::PowerShell { '`' } else { '\\' };
        let escaping = !((language == CallLanguage::PowerShell && ch == '\'') || (language == CallLanguage::General && ch == '`'));
        return Some(quoted_end(text, at, ch, 1, escaping, doubled, escape));
    }
    None
}

fn line_end(text: &str, at: usize) -> usize {
    text.get(at..).and_then(|rest| rest.find(['\r', '\n'])).map(|length| at.saturating_add(length)).unwrap_or(text.len())
}

fn delimited_end(text: &str, at: usize, delimiter: &str) -> usize {
    text.get(at..).and_then(|rest| rest.find(delimiter)).map(|length| at.saturating_add(length).saturating_add(delimiter.len())).unwrap_or(text.len())
}

fn block_comment(text: &str, at: usize, open: &str, close: &str, nested: bool) -> usize {
    let mut depth = 1usize;
    let mut at = at.saturating_add(open.len());
    while let Some(rest) = text.get(at..) {
        let Some(ch) = rest.chars().next() else { break; };
        if rest.starts_with(close) {
            depth = depth.saturating_sub(1);
            at = at.saturating_add(close.len());
            if depth == 0 { return at; }
        } else if nested && rest.starts_with(open) {
            depth = depth.saturating_add(1);
            at = at.saturating_add(open.len());
        } else { at = at.saturating_add(ch.len_utf8()); }
    }
    text.len()
}

fn quoted_end(text: &str, at: usize, quote: char, length: usize, escaping: bool, doubled: bool, escape: char) -> usize {
    let delimiter = quote.to_string().repeat(length);
    let mut at = at.saturating_add(length);
    while let Some(rest) = text.get(at..) {
        let Some(ch) = rest.chars().next() else { break; };
        if escaping && ch == escape {
            at = at.saturating_add(ch.len_utf8());
            if let Some(next) = text.get(at..).and_then(|rest| rest.chars().next()) { at = at.saturating_add(next.len_utf8()); }
        } else if rest.starts_with(&delimiter) {
            at = at.saturating_add(delimiter.len());
            if doubled && text.get(at..).is_some_and(|rest| rest.starts_with(&delimiter)) { at = at.saturating_add(delimiter.len()); }
            else { return at; }
        } else { at = at.saturating_add(ch.len_utf8()); }
    }
    text.len()
}

fn rust_character(rest: &str) -> bool {
    let Some(after) = rest.get(1..) else { return false; };
    if after.starts_with('\\') { return true; }
    let Some(ch) = after.chars().next() else { return false; };
    after.get(ch.len_utf8()..).is_some_and(|tail| tail.starts_with('\''))
}

fn rust_delimiter(rest: &str) -> Option<(usize, String)> {
    let prefix = if rest.starts_with("br") { 2 } else if rest.starts_with('r') { 1 } else { return None; };
    let hashes = rest.get(prefix..)?.bytes().take_while(|&byte| byte == b'#').count();
    if hashes > 32 || !rest.get(prefix + hashes..)?.starts_with('"') { return None; }
    Some((prefix + hashes + 1, format!("\"{}", "#".repeat(hashes))))
}

fn lua_delimiter(rest: &str) -> Option<(usize, String)> {
    let tail = rest.strip_prefix('[')?;
    let equals = tail.bytes().take_while(|&byte| byte == b'=').count();
    if equals > 32 || !tail.get(equals..)?.starts_with('[') { return None; }
    Some((equals + 2, format!("]{}]", "=".repeat(equals))))
}

fn regex_end(text: &str, at: usize) -> usize {
    let mut at = at.saturating_add(1);
    let mut class = false;
    while let Some(rest) = text.get(at..) {
        let Some(ch) = rest.chars().next() else { break; };
        at = at.saturating_add(ch.len_utf8());
        match ch {
            '\\' => { if let Some(next) = text.get(at..).and_then(|rest| rest.chars().next()) { at = at.saturating_add(next.len_utf8()); } }
            '[' => class = true,
            ']' => class = false,
            '/' if !class => return at,
            '\r' | '\n' => return at,
            _ => {}
        }
    }
    text.len()
}

#[cfg(test)]
#[path = "call_context_tests.rs"]
mod tests;
