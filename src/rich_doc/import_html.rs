//! Convert clipboard HTML into a `Vec<Block>` ready to splice into the rich-doc
//! AST during paste-with-formatting (Phase 2g.K).
//!
//! The HTML on macOS NSPasteboard typically comes from Safari / Chrome / Word.
//! Word wraps the relevant fragment in `<!--StartFragment-->…<!--EndFragment-->`
//! markers; we slice to those when present so we don't drag Office's chrome
//! (msword styles, conditional IE comments, etc.) into the document.
//!
//! Parser scope is deliberately narrow: we recognise the inline + block tags
//! that actually carry meaning for our AST. Anything else passes through as
//! text, dropping the markup. That keeps the converter compact and robust to
//! the wildly different HTML dialects clipboards produce.

use super::ast::{Alignment, Block, BlockProps, InlineRun, InlineStyle, ListMarker};

const MAX_HTML_BYTES: usize = 16 * 1024 * 1024;
const MAX_HTML_TAG_BYTES: usize = 16 * 1024;
const MAX_HTML_TOKENS: usize = 100_000;
const MAX_HTML_NESTING: usize = 256;

/// Public entry point — parse `html` and return the blocks to splice. Returns
/// an empty `Vec` when the input has no textual content or exceeds the parser's
/// size/complexity limits, so callers can fall back to plain-text paste.
pub fn html_to_blocks(html: &str) -> Vec<Block> {
    if html.len() > MAX_HTML_BYTES {
        return Vec::new();
    }
    let trimmed = slice_to_fragment(html);
    let Some(tokens) = tokenize(trimmed) else { return Vec::new() };
    let mut parser = Parser::new();
    for tok in tokens {
        parser.feed(tok);
        if parser.style_stack.len() > MAX_HTML_NESTING
            || parser.list_stack.len() > MAX_HTML_NESTING
            || parser.pre_depth > MAX_HTML_NESTING as u32 {
            return Vec::new();
        }
    }
    parser.finish()
}

/// Slice the input to Word's StartFragment / EndFragment markers when both are
/// present. Browsers don't emit these but Word + Outlook do, and the fragment
/// is the *only* part of the payload that represents what the user actually
/// selected — everything outside is generator boilerplate.
fn slice_to_fragment(html: &str) -> &str {
    if let (Some(s), Some(e)) = (html.find("<!--StartFragment-->"), html.find("<!--EndFragment-->")) {
        if s < e {
            let start = s + "<!--StartFragment-->".len();
            return html.get(start..e).unwrap_or(html);
        }
    }
    html
}

// ---- Tokens ---------------------------------------------------------------

#[derive(Debug, Clone)]
enum Token {
    /// `<tagname attr1="value1" attr2='value2'>` — `self_closing` true when the
    /// tag ends with `/>` (XHTML-style). HTML5 void elements (br, img, hr, …)
    /// are flagged separately by the parser.
    OpenTag { name: String, attrs: Vec<(String, String)>, self_closing: bool },
    /// `</tagname>`
    CloseTag { name: String },
    /// Plain text run, with HTML entities already decoded. Whitespace is
    /// preserved verbatim here — collapse happens in the parser, where
    /// `<pre>` context can disable it.
    Text(String),
}

// ---- Tokenizer ------------------------------------------------------------

fn tokenize(html: &str) -> Option<Vec<Token>> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if out.len() >= MAX_HTML_TOKENS {
            return None;
        }
        let b = *bytes.get(i)?;
        if b == b'<' {
            // Comment: <!-- ... -->
            if bytes.get(i..)?.starts_with(b"<!--") {
                if let Some(end) = find_subslice(bytes, i + 4, b"-->") {
                    i = end + 3;
                    continue;
                }
                // Malformed comment — drop the rest of the input.
                break;
            }
            // DOCTYPE or other <!...> declarations — skip to next '>'.
            if bytes.get(i + 1) == Some(&b'!') {
                if let Some(end) = find_byte(bytes, i + 2, b'>') {
                    i = end + 1;
                    continue;
                }
                break;
            }
            // Processing instruction <?xml ...?>
            if bytes.get(i + 1) == Some(&b'?') {
                if let Some(end) = find_byte(bytes, i + 2, b'>') {
                    i = end + 1;
                    continue;
                }
                break;
            }
            // Close tag </name>
            if bytes.get(i + 1) == Some(&b'/') {
                if let Some(end) = find_byte(bytes, i + 2, b'>') {
                    let raw = html.get(i + 2..end)?.trim();
                    out.push(Token::CloseTag { name: raw.to_ascii_lowercase() });
                    i = end + 1;
                    continue;
                }
                break;
            }
            // HTML treats '<' followed by a non-ASCII-letter as literal text.
            if !bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
                out.push(Token::Text("<".into()));
                i += 1;
                continue;
            }
            // Open tag <name ...>
            if let Some(end) = find_tag_end(bytes, i + 1) {
                if end.saturating_sub(i) > MAX_HTML_TAG_BYTES {
                    return None;
                }
                let raw = html.get(i + 1..end)?;
                let (name, attrs, self_closing) = parse_open_tag(raw);
                // <script> / <style> bodies: skip to the matching close tag —
                // their contents aren't user text.
                i = end + 1;
                if matches!(name.as_str(), "script" | "style") {
                    // HTML ignores '/>' on these elements. With no end tag,
                    // raw text continues to EOF; never rescan it as markup.
                    i = find_raw_text_end(bytes, i, &name).unwrap_or(bytes.len());
                } else {
                    out.push(Token::OpenTag { name, attrs, self_closing });
                }
                continue;
            }
            break;
        }
        // Text run — gather up to the next '<'.
        let next = find_byte(bytes, i, b'<').unwrap_or(bytes.len());
        let raw = html.get(i..next)?;
        if !raw.is_empty() {
            out.push(Token::Text(decode_entities(raw)));
        }
        i = next;
    }
    Some(out)
}

fn find_byte(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    bytes.get(start..)?.iter().position(|b| *b == needle).map(|p| p + start)
}

// Quoted attribute values can contain '>'. Quotes in an unquoted value are
// ordinary characters, so title=don't must not hide the end of the tag.
fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    enum Value { Other, Start, Unquoted, Quoted(u8) }
    let mut value = Value::Other;
    for (offset, &byte) in bytes.get(start..)?.iter().enumerate() {
        match value {
            Value::Quoted(quote) if byte == quote => value = Value::Other,
            Value::Quoted(_) => {},
            _ if byte == b'>' => return Some(start + offset),
            Value::Start if byte.is_ascii_whitespace() => {},
            Value::Start if matches!(byte, b'\'' | b'"') => value = Value::Quoted(byte),
            Value::Start => value = Value::Unquoted,
            Value::Unquoted if byte.is_ascii_whitespace() => value = Value::Other,
            Value::Unquoted => {},
            Value::Other if byte == b'=' => value = Value::Start,
            Value::Other => {},
        }
    }
    None
}

fn find_subslice(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || start >= bytes.len() {
        return None;
    }
    let slice = bytes.get(start..)?;
    slice
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + start)
}

// WHATWG HTML 13.2.5.14/17: the matching end-tag name must be followed by
// whitespace, '/' or '>'; a prefix such as </stylesheet> is still raw text.
// Script comment escape states (13.2.5.18–31) can hide an inner </script>.
// https://html.spec.whatwg.org/multipage/parsing.html#rawtext-end-tag-name-state
fn find_raw_text_end(bytes: &[u8], start: usize, name: &str) -> Option<usize> {
    let marker = format!("</{name}");
    let mut escaped = false;
    let mut double_escaped = false;
    for (offset, &byte) in bytes.get(start..)?.iter().enumerate() {
        let tail = bytes.get(start + offset..)?;
        if escaped && byte == b'-' && tail.starts_with(b"-->") {
            escaped = false;
            double_escaped = false;
        }
        if byte != b'<' {
            continue;
        }
        if name == "script" {
            if !escaped && tail.starts_with(b"<!--") {
                escaped = true;
                continue;
            }
            if escaped && !double_escaped && matches_raw_tag(tail, b"<script") {
                double_escaped = true;
                continue;
            }
        }
        if matches_raw_tag(tail, marker.as_bytes()) {
            if double_escaped {
                double_escaped = false;
                continue;
            }
            return find_tag_end(bytes, start + offset + marker.len()).map(|end| end + 1);
        }
    }
    None
}

fn matches_raw_tag(bytes: &[u8], marker: &[u8]) -> bool {
    bytes.get(marker.len()).is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
        && bytes.get(..marker.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(marker))
}

/// Split a `<...>` body into (lowercase_name, attrs, self_closing). Quoted
/// attribute values may contain spaces and `=`; unquoted values stop at the
/// next whitespace or `>`. Matches HTML5 quirks loosely enough to handle the
/// real-world variations clipboards produce.
fn parse_open_tag(raw: &str) -> (String, Vec<(String, String)>, bool) {
    let mut chars = raw.char_indices().peekable();
    // Tag name up to whitespace.
    let mut name = String::new();
    while let Some(&(_, c)) = chars.peek() {
        if c.is_whitespace() || c == '/' || c == '>' {
            break;
        }
        name.push(c);
        chars.next();
    }
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;
    loop {
        // Skip whitespace.
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }
        match chars.peek() {
            None => break,
            Some(&(_, '/')) => {
                self_closing = true;
                chars.next();
                continue;
            }
            Some(&(_, '>')) => break,
            _ => {}
        }
        // Attribute name.
        let mut attr_name = String::new();
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace() || c == '=' || c == '/' || c == '>' {
                break;
            }
            attr_name.push(c);
            chars.next();
        }
        // Skip whitespace before '='.
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }
        let mut attr_value = String::new();
        if let Some(&(_, '=')) = chars.peek() {
            chars.next();
            // Skip whitespace after '='.
            while let Some(&(_, c)) = chars.peek() {
                if c.is_whitespace() {
                    chars.next();
                } else {
                    break;
                }
            }
            match chars.peek().copied() {
                Some((_, '"')) => {
                    chars.next();
                    while let Some((_, c)) = chars.next() {
                        if c == '"' {
                            break;
                        }
                        attr_value.push(c);
                    }
                }
                Some((_, '\'')) => {
                    chars.next();
                    while let Some((_, c)) = chars.next() {
                        if c == '\'' {
                            break;
                        }
                        attr_value.push(c);
                    }
                }
                _ => {
                    while let Some(&(_, c)) = chars.peek() {
                        if c.is_whitespace() || c == '>' {
                            break;
                        }
                        attr_value.push(c);
                        chars.next();
                    }
                }
            }
        }
        if !attr_name.is_empty() {
            attrs.push((attr_name.to_ascii_lowercase(), decode_entities(&attr_value)));
        }
    }
    (name.to_ascii_lowercase(), attrs, self_closing)
}

/// HTML entity decoder — handles named entities, decimal `&#N;`, hex `&#xHH;`.
fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            // Look for the terminating `;` within 12 bytes (covers every
            // entity we care about — longest named one is `&thinsp;` etc.).
            let end = bytes[i + 1..]
                .iter()
                .take(12)
                .position(|b| *b == b';')
                .map(|p| p + i + 1);
            if let Some(semi) = end {
                let body = std::str::from_utf8(&bytes[i + 1..semi]).unwrap_or("");
                if let Some(decoded) = decode_one_entity(body) {
                    out.push_str(&decoded);
                    i = semi + 1;
                    continue;
                }
            }
        }
        // Pass byte through. We're walking a UTF-8 string so multi-byte chars
        // are safe to copy byte-for-byte.
        out.push(bytes[i] as char);
        i += 1;
    }
    // Whitespace-only fixups for non-ASCII chars (chars that ended up as the
    // raw bytes above need to be re-parsed). Simpler: rebuild from the original
    // when there were no entities at all.
    if !input.contains('&') {
        return input.to_string();
    }
    // Re-decode from the original bytes properly so non-ASCII text isn't broken
    // by the byte-as-char path above.
    let mut fixed = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            let mut entity = String::new();
            let mut found = false;
            let mut consumed = 0usize;
            for nc in chars.clone().take(12) {
                consumed += 1;
                if nc == ';' {
                    found = true;
                    break;
                }
                entity.push(nc);
            }
            if found {
                if let Some(decoded) = decode_one_entity(&entity) {
                    for _ in 0..consumed {
                        chars.next();
                    }
                    fixed.push_str(&decoded);
                    continue;
                }
            }
            fixed.push('&');
        } else {
            fixed.push(c);
        }
    }
    fixed
}

fn decode_one_entity(body: &str) -> Option<String> {
    if let Some(num) = body.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse::<u32>().ok()?
        };
        return char::from_u32(code).map(|c| c.to_string());
    }
    let s = match body {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{00A0}",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "bull" => "•",
        "middot" => "·",
        _ => return None,
    };
    Some(s.to_string())
}

// ---- Parser ---------------------------------------------------------------

/// Active list context — tracks ordered/unordered nesting so `<li>` knows
/// which marker to attach to the paragraph it produces.
#[derive(Debug, Clone, Copy)]
struct ListContext {
    ordered: bool,
    level: u8,
}

#[derive(Debug, Clone)]
struct StyleFrame {
    style: InlineStyle,
}

struct Parser {
    blocks: Vec<Block>,
    /// Inline style stack — `style_stack.last()` is the active style applied
    /// to text we collect. Empty stack = `InlineStyle::default()`.
    style_stack: Vec<StyleFrame>,
    /// Open lists from outermost (index 0) to innermost (index n-1). `<li>`
    /// reads the innermost.
    list_stack: Vec<ListContext>,
    /// Current paragraph's runs being built up. Flushed by block-end events
    /// and the initial open of a new block.
    current_runs: Vec<InlineRun>,
    /// Heading level for the in-progress block (1..=6). `None` = paragraph.
    current_heading_level: Option<u8>,
    /// Block-level props for the in-progress block (alignment, indent, list).
    current_props: BlockProps,
    /// Inside `<pre>` we preserve whitespace verbatim instead of collapsing.
    pre_depth: u32,
    /// Tracks whether the last appended character was whitespace, so back-to-
    /// back whitespace tokens collapse to a single space across run boundaries.
    last_was_whitespace: bool,
    /// `true` between block-start and the first non-whitespace text — used to
    /// drop the leading whitespace inside `<p>foo</p>` style payloads (HTML
    /// strips leading whitespace at the start of a block).
    at_block_start: bool,
}

impl Parser {
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            style_stack: Vec::new(),
            list_stack: Vec::new(),
            current_runs: Vec::new(),
            current_heading_level: None,
            current_props: BlockProps::default(),
            pre_depth: 0,
            last_was_whitespace: false,
            at_block_start: true,
        }
    }

    fn feed(&mut self, tok: Token) {
        match tok {
            Token::OpenTag { name, attrs, self_closing } => self.handle_open(&name, &attrs, self_closing),
            Token::CloseTag { name } => self.handle_close(&name),
            Token::Text(text) => self.handle_text(&text),
        }
    }

    fn handle_open(&mut self, name: &str, attrs: &[(String, String)], self_closing: bool) {
        // Skip void / structural tags that don't carry style or block change.
        match name {
            "html" | "head" | "body" | "meta" | "link" | "title" | "base" | "wbr" | "col"
            | "colgroup" | "tbody" | "thead" | "tfoot" => return,
            // Tables flatten to plain paragraphs in Phase 2g.K — the AST has
            // Block::Table but rendering tables from clipboard HTML is a follow-
            // up.
            "table" | "tr" | "td" | "th" => {
                self.flush_block_if_dirty();
                return;
            }
            "img" => {
                // Render alt text inline (best we can do without fetching).
                let alt = attrs.iter().find(|(k, _)| k == "alt").map(|(_, v)| v.clone()).unwrap_or_default();
                if !alt.is_empty() {
                    self.append_text(&alt);
                }
                return;
            }
            "br" => {
                // Inside a block, <br> ends the current paragraph and starts a
                // new one — we don't have a soft-break primitive in the AST so
                // hard-paragraph is the closest fit.
                self.flush_block();
                return;
            }
            "hr" => {
                self.flush_block();
                // A horizontal rule renders as an empty paragraph (no
                // dedicated AST node yet).
                self.blocks.push(Block::Paragraph {
                    runs: vec![InlineRun::plain("")],
                    props: BlockProps::default(),
                });
                return;
            }
            _ => {}
        }

        match name {
            "p" | "div" | "blockquote" | "section" | "article" | "header" | "footer" | "nav"
            | "main" | "aside" | "figure" | "figcaption" | "dl" | "dt" | "dd" | "address"
            | "form" => {
                self.flush_block_if_dirty();
                self.current_heading_level = None;
                self.current_props = BlockProps::default();
                self.apply_block_style_attr(attrs);
                if let Some(ctx) = self.list_stack.last() {
                    // Nested paragraph inside <li> — inherit list membership
                    // so a multi-paragraph list item stays in the list.
                    self.current_props.list = Some(ListMarker { ordered: ctx.ordered, level: ctx.level });
                }
                self.at_block_start = true;
                self.last_was_whitespace = false;
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush_block_if_dirty();
                let level = name.as_bytes()[1] - b'0';
                self.current_heading_level = Some(level);
                self.current_props = BlockProps::default();
                self.apply_block_style_attr(attrs);
                self.at_block_start = true;
                self.last_was_whitespace = false;
            }
            "pre" => {
                self.flush_block_if_dirty();
                self.current_heading_level = None;
                self.current_props = BlockProps::default();
                self.pre_depth = self.pre_depth.saturating_add(1);
                self.at_block_start = true;
                self.last_was_whitespace = false;
            }
            "ul" => {
                self.flush_block_if_dirty();
                let level = self.list_stack.len().min(u8::MAX as usize) as u8;
                self.list_stack.push(ListContext { ordered: false, level });
            }
            "ol" => {
                self.flush_block_if_dirty();
                let level = self.list_stack.len().min(u8::MAX as usize) as u8;
                self.list_stack.push(ListContext { ordered: true, level });
            }
            "li" => {
                self.flush_block_if_dirty();
                self.current_heading_level = None;
                self.current_props = BlockProps::default();
                if let Some(ctx) = self.list_stack.last() {
                    self.current_props.list = Some(ListMarker { ordered: ctx.ordered, level: ctx.level });
                }
                self.apply_block_style_attr(attrs);
                self.at_block_start = true;
                self.last_was_whitespace = false;
            }
            "strong" | "b" => self.push_style(|s| s.bold = true, attrs),
            "em" | "i" | "cite" | "var" | "dfn" => self.push_style(|s| s.italic = true, attrs),
            "u" | "ins" => self.push_style(|s| s.underline = true, attrs),
            "s" | "strike" | "del" => self.push_style(|s| s.strike = true, attrs),
            "a" => {
                let href = attrs.iter().find(|(k, _)| k == "href").map(|(_, v)| v.clone());
                // Identity-style frame — link decoration is added by render
                // (underline + theme link color) once `style.link` is set.
                self.push_style(|_s| {}, attrs);
                if let Some(last) = self.style_stack.last_mut() {
                    last.style.link = href;
                }
            }
            "span" | "font" | "mark" | "small" | "sub" | "sup" | "code" | "kbd" | "samp"
            | "abbr" | "q" => {
                // Inline pass-through — capture style attr but no implicit
                // weight / italic / underline.
                self.push_style(|_s| {}, attrs);
            }
            _ => {
                // Unknown / unsupported tag — push an identity style frame so
                // the close tag can pop without unbalancing the stack.
                self.push_style(|_s| {}, attrs);
            }
        }
        if self_closing {
            self.handle_close(name);
        }
    }

    fn handle_close(&mut self, name: &str) {
        match name {
            "html" | "head" | "body" | "meta" | "link" | "title" | "base" | "br" | "hr"
            | "img" | "wbr" | "col" | "colgroup" | "tbody" | "thead" | "tfoot" => return,
            "table" | "tr" | "td" | "th" => return,
            _ => {}
        }
        match name {
            "p" | "div" | "blockquote" | "section" | "article" | "header" | "footer" | "nav"
            | "main" | "aside" | "figure" | "figcaption" | "dl" | "dt" | "dd" | "address"
            | "form" | "li" => self.flush_block_if_dirty(),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.flush_block_if_dirty(),
            "pre" => {
                self.flush_block_if_dirty();
                self.pre_depth = self.pre_depth.saturating_sub(1);
            }
            "ul" | "ol" => {
                self.flush_block_if_dirty();
                self.list_stack.pop();
            }
            "strong" | "b" | "em" | "i" | "cite" | "var" | "dfn" | "u" | "ins" | "a" | "span"
            | "font" | "mark" | "small" | "sub" | "sup" | "code" | "kbd" | "samp" | "abbr"
            | "q" => {
                self.pop_style();
            }
            _ => {
                self.pop_style();
            }
        }
    }

    fn handle_text(&mut self, text: &str) {
        if self.pre_depth > 0 {
            // Preserve whitespace verbatim inside <pre>.
            self.append_text(text);
            return;
        }
        if text.is_empty() {
            return;
        }
        // HTML whitespace collapse — runs of whitespace become a single ' '.
        // Drop leading whitespace when we're at the start of a block.
        let mut collapsed = String::with_capacity(text.len());
        let mut prev_ws = self.last_was_whitespace || self.at_block_start;
        for c in text.chars() {
            if is_html_whitespace(c) {
                if !prev_ws {
                    collapsed.push(' ');
                }
                prev_ws = true;
            } else {
                collapsed.push(c);
                prev_ws = false;
            }
        }
        if collapsed.is_empty() {
            return;
        }
        self.append_text(&collapsed);
        self.last_was_whitespace = collapsed.ends_with(' ');
        self.at_block_start = false;
    }

    fn append_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let style = self.current_style();
        // Merge into the tail run when the style matches, so consecutive
        // text tokens under the same style stay as one run (better undo /
        // smaller AST).
        if let Some(last) = self.current_runs.last_mut() {
            if last.style == style {
                last.text.push_str(text);
                return;
            }
        }
        self.current_runs.push(InlineRun { text: text.to_string(), style });
    }

    fn current_style(&self) -> InlineStyle {
        self.style_stack.last().map(|f| f.style.clone()).unwrap_or_default()
    }

    fn push_style<F: FnOnce(&mut InlineStyle)>(&mut self, mutator: F, attrs: &[(String, String)]) {
        let mut next = self.current_style();
        mutator(&mut next);
        apply_inline_style_attr(&mut next, attrs);
        self.style_stack.push(StyleFrame { style: next });
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }

    fn apply_block_style_attr(&mut self, attrs: &[(String, String)]) {
        if let Some((_, raw)) = attrs.iter().find(|(k, _)| k == "style") {
            for decl in raw.split(';') {
                let mut parts = decl.splitn(2, ':');
                let key = parts.next().map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
                let val = parts.next().map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
                match key.as_str() {
                    "text-align" => match val.as_str() {
                        "center" => self.current_props.alignment = Alignment::Center,
                        "right" => self.current_props.alignment = Alignment::Right,
                        "justify" => self.current_props.alignment = Alignment::Justify,
                        "left" => self.current_props.alignment = Alignment::Left,
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        // Word emits align="left|right|center|justify" on <p>.
        if let Some((_, align)) = attrs.iter().find(|(k, _)| k == "align") {
            match align.to_ascii_lowercase().as_str() {
                "center" => self.current_props.alignment = Alignment::Center,
                "right" => self.current_props.alignment = Alignment::Right,
                "justify" => self.current_props.alignment = Alignment::Justify,
                "left" => self.current_props.alignment = Alignment::Left,
                _ => {}
            }
        }
    }

    fn flush_block(&mut self) {
        // An empty block (no runs at all) is meaningful — that's a blank line
        // — but a block whose only content is a trailing space gets the space
        // trimmed so we don't carry a phantom whitespace into the document.
        let mut runs = std::mem::take(&mut self.current_runs);
        if let Some(last) = runs.last_mut() {
            if last.text.ends_with(' ') && !self.at_block_start {
                last.text.pop();
                if last.text.is_empty() && runs.len() > 1 {
                    runs.pop();
                }
            }
        }
        if runs.is_empty() {
            runs.push(InlineRun::plain(""));
        }
        let props = std::mem::take(&mut self.current_props);
        let block = match self.current_heading_level.take() {
            Some(level) => Block::Heading { level: level.clamp(1, 6), runs, props },
            None => Block::Paragraph { runs, props },
        };
        self.blocks.push(block);
        self.at_block_start = true;
        self.last_was_whitespace = false;
    }

    fn flush_block_if_dirty(&mut self) {
        if !self.current_runs.is_empty() {
            self.flush_block();
        }
    }

    fn finish(mut self) -> Vec<Block> {
        // Drain whatever's left in the in-progress block — payloads sometimes
        // end mid-content (e.g. a clipboard fragment without an explicit </p>).
        if !self.current_runs.is_empty() {
            self.flush_block();
        }
        self.blocks
    }
}

fn is_html_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000C}')
}

/// Apply CSS-style declarations from a `style="..."` attribute onto an inline
/// style. We only honor the properties that map onto our `InlineStyle` —
/// `font-weight`, `font-style`, `text-decoration`, `color`. Anything else is
/// ignored (no font-family / size / background — those don't exist in our
/// AST yet).
fn apply_inline_style_attr(style: &mut InlineStyle, attrs: &[(String, String)]) {
    let Some((_, raw)) = attrs.iter().find(|(k, _)| k == "style") else { return };
    for decl in raw.split(';') {
        let mut parts = decl.splitn(2, ':');
        let key = parts.next().map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
        let val = parts.next().map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
        match key.as_str() {
            "font-weight" => {
                style.bold = matches!(
                    val.as_str(),
                    "bold" | "bolder" | "600" | "700" | "800" | "900"
                );
            }
            "font-style" => {
                style.italic = matches!(val.as_str(), "italic" | "oblique");
            }
            "text-decoration" | "text-decoration-line" => {
                style.underline = val.contains("underline");
                style.strike = val.contains("line-through");
            }
            "color" => {
                if let Some(rgb) = parse_css_color(&val) {
                    style.color = Some(rgb);
                }
            }
            _ => {}
        }
    }
}

/// Parse a subset of CSS color values into a 0xRRGGBB integer.
/// Recognises: `#RGB`, `#RRGGBB`, `rgb(r, g, b)`, `rgba(r, g, b, a)`,
/// and a few common named colors. Returns `None` for anything else so the
/// caller can leave color = None (inherit theme).
fn parse_css_color(input: &str) -> Option<u32> {
    let s = input.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        return match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                Some(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
            }
            6 => u32::from_str_radix(hex, 16).ok(),
            _ => None,
        };
    }
    if let Some(rest) = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")) {
        let rest = rest.trim_end_matches(')');
        let parts: Vec<&str> = rest.split(',').map(|p| p.trim()).collect();
        if let [r, g, b, ..] = parts.as_slice() {
            let r = r.parse::<u32>().ok()?.min(255);
            let g = g.parse::<u32>().ok()?.min(255);
            let b = b.parse::<u32>().ok()?.min(255);
            return Some((r << 16) | (g << 8) | b);
        }
    }
    match s {
        "black" => Some(0x000000),
        "white" => Some(0xFFFFFF),
        "red" => Some(0xFF0000),
        "green" => Some(0x008000),
        "blue" => Some(0x0000FF),
        "yellow" => Some(0xFFFF00),
        "orange" => Some(0xFFA500),
        "purple" => Some(0x800080),
        "gray" | "grey" => Some(0x808080),
        "silver" => Some(0xC0C0C0),
        "navy" => Some(0x000080),
        "teal" => Some(0x008080),
        "olive" => Some(0x808000),
        "maroon" => Some(0x800000),
        "lime" => Some(0x00FF00),
        "aqua" | "cyan" => Some(0x00FFFF),
        "fuchsia" | "magenta" => Some(0xFF00FF),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich_doc::ast::Block;

    fn paragraph_text(b: &Block) -> String {
        match b {
            Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => {
                runs.iter().map(|r| r.text.as_str()).collect()
            }
            _ => String::new(),
        }
    }

    #[test]
    fn plain_paragraph() {
        let out = html_to_blocks("<p>hello world</p>");
        assert_eq!(out.len(), 1);
        assert_eq!(paragraph_text(&out[0]), "hello world");
    }

    #[test]
    fn bold_italic_inline() {
        let out = html_to_blocks("<p>a <b>bold</b> and <i>italic</i> word</p>");
        assert_eq!(out.len(), 1);
        if let Block::Paragraph { runs, .. } = &out[0] {
            let bold_run = runs.iter().find(|r| r.text == "bold").expect("bold run");
            assert!(bold_run.style.bold);
            let italic_run = runs.iter().find(|r| r.text == "italic").expect("italic run");
            assert!(italic_run.style.italic);
        } else {
            panic!("expected paragraph");
        }
    }

    #[test]
    fn heading_levels() {
        let out = html_to_blocks("<h1>Title</h1><h3>Sub</h3>");
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], Block::Heading { level: 1, .. }));
        assert!(matches!(out[1], Block::Heading { level: 3, .. }));
    }

    #[test]
    fn entity_decoding() {
        let out = html_to_blocks("<p>5 &lt; 10 &amp; 3 &gt; 2</p>");
        assert_eq!(paragraph_text(&out[0]), "5 < 10 & 3 > 2");
    }

    #[test]
    fn ordered_list_marks_paragraphs() {
        let out = html_to_blocks("<ol><li>one</li><li>two</li></ol>");
        assert_eq!(out.len(), 2);
        for b in &out {
            if let Block::Paragraph { props, .. } = b {
                let m = props.list.expect("list marker");
                assert!(m.ordered);
                assert_eq!(m.level, 0);
            } else {
                panic!("expected paragraph");
            }
        }
    }

    #[test]
    fn nested_unordered_list_tracks_level() {
        let out = html_to_blocks("<ul><li>outer<ul><li>inner</li></ul></li></ul>");
        assert_eq!(out.len(), 2);
        if let Block::Paragraph { props, .. } = &out[0] {
            assert_eq!(props.list.unwrap().level, 0);
        } else {
            panic!();
        }
        if let Block::Paragraph { props, .. } = &out[1] {
            assert_eq!(props.list.unwrap().level, 1);
        } else {
            panic!();
        }
    }

    #[test]
    fn anchor_carries_link() {
        let out = html_to_blocks("<p>see <a href=\"https://example.com\">site</a> now</p>");
        if let Block::Paragraph { runs, .. } = &out[0] {
            let link_run = runs.iter().find(|r| r.text == "site").expect("link run");
            assert_eq!(link_run.style.link.as_deref(), Some("https://example.com"));
        } else {
            panic!();
        }
    }

    #[test]
    fn css_color_parses() {
        assert_eq!(parse_css_color("#ff0000"), Some(0xFF0000));
        assert_eq!(parse_css_color("#abc"), Some(0xAABBCC));
        assert_eq!(parse_css_color("rgb(10, 20, 30)"), Some(0x000A141E & 0xFFFFFF));
        assert_eq!(parse_css_color("red"), Some(0xFF0000));
        assert_eq!(parse_css_color("nonsense"), None);
    }

    #[test]
    fn word_style_attribute_bold_italic_underline() {
        let html = "<p><span style=\"font-weight:bold;font-style:italic;text-decoration:underline\">x</span></p>";
        let out = html_to_blocks(html);
        if let Block::Paragraph { runs, .. } = &out[0] {
            let r = runs.iter().find(|r| r.text == "x").expect("x run");
            assert!(r.style.bold);
            assert!(r.style.italic);
            assert!(r.style.underline);
        } else {
            panic!();
        }
    }

    #[test]
    fn slice_to_fragment_uses_word_markers() {
        let html = "noise<!--StartFragment--><p>real</p><!--EndFragment-->trailing";
        let out = html_to_blocks(html);
        assert_eq!(out.len(), 1);
        assert_eq!(paragraph_text(&out[0]), "real");
    }

    #[test]
    fn whitespace_collapses() {
        let out = html_to_blocks("<p>  a    b\n\nc   </p>");
        assert_eq!(paragraph_text(&out[0]), "a b c");
    }

    #[test]
    fn pre_preserves_whitespace() {
        // HTML <pre> preserves every space verbatim, including trailing ones —
        // that's the whole point of the tag, so we don't trim here.
        let out = html_to_blocks("<pre>  a    b  </pre>");
        assert_eq!(paragraph_text(&out[0]), "  a    b  ");
    }

    #[test]
    fn script_and_style_are_dropped() {
        let out = html_to_blocks("<p>before</p><script>alert(1)</script><style>p{color:red}</style><p>after</p>");
        assert_eq!(out.len(), 2);
        assert_eq!(paragraph_text(&out[0]), "before");
        assert_eq!(paragraph_text(&out[1]), "after");
    }

    #[test]
    fn unclosed_raw_text_does_not_become_document_content() {
        for tag in ["script", "STYLE"] {
            let html = format!("<p>\u{FEFF}\u{00E9}\u{1F600}</p><{tag}>hidden<b>also hidden");
            let out = html_to_blocks(&html);
            assert_eq!(out.len(), 1, "{tag}");
            assert_eq!(paragraph_text(&out[0]), "\u{FEFF}\u{00E9}\u{1F600}");
        }
    }

    #[test]
    fn repeated_unclosed_scripts_are_one_hidden_body() {
        let html = format!("<p>kept</p>{}<p>hidden</p>", "<script>".repeat(2048));
        assert_eq!(html_to_blocks(&html).iter().map(paragraph_text).collect::<Vec<_>>(), vec!["kept"]);
    }

    #[test]
    fn script_comment_escape_states_keep_the_body_hidden() {
        for body in [
            "<!--<ScRiPt></script><p>hidden</p>",
            "<!--<script/>hidden</script >",
            "<!--<script>-->hidden",
            "<!--<scriptx>hidden",
        ] {
            let html = format!("<p>before</p><script>{body}</script><p>after</p>");
            assert_eq!(html_to_blocks(&html).iter().map(paragraph_text).collect::<Vec<_>>(),
                vec!["before", "after"], "{body}");
        }
        let out = html_to_blocks("<p>kept</p><script><!--<script></script><p>hidden at EOF</p>");
        assert_eq!(out.iter().map(paragraph_text).collect::<Vec<_>>(), vec!["kept"]);
        // CSS has no script escape states; its first matching end tag closes it.
        let out = html_to_blocks("<style><!--<script></style><p>shown</p>");
        assert_eq!(out.iter().map(paragraph_text).collect::<Vec<_>>(), vec!["shown"]);
    }

    #[test]
    fn raw_text_close_tags_accept_html_delimiters() {
        for close in ["</ScRiPt \r\n>", "</script/>", "</script data-x='>'>"] {
            let html = format!("<p>before</p><script>hidden{close}<p>\u{00E9}after</p>");
            let out = html_to_blocks(&html);
            assert_eq!(out.iter().map(paragraph_text).collect::<Vec<_>>(),
                vec!["before", "\u{00E9}after"], "{close}");
        }
        let out = html_to_blocks("<style>hidden</stylesheet><p>hidden</p></STYLE \t><p>shown</p>");
        assert_eq!(out.iter().map(paragraph_text).collect::<Vec<_>>(), vec!["shown"]);
    }

    #[test]
    fn raw_text_html_elements_ignore_the_self_closing_flag() {
        for tag in ["script", "style"] {
            let html = format!("<p>before</p><{tag}/>hidden<b>hidden</b></{tag}><p>after</p>");
            assert_eq!(html_to_blocks(&html).iter().map(paragraph_text).collect::<Vec<_>>(),
                vec!["before", "after"]);
        }
    }

    #[test]
    fn quoted_attribute_delimiters_preserve_text_and_links() {
        let out = html_to_blocks("<p><a href='https://example.test/?q=\u{00E9}>x&amp;y=1'>\u{1F600} &amp; \u{4E2D}</a></p>");
        assert_eq!(out.len(), 1);
        assert_eq!(paragraph_text(&out[0]), "\u{1F600} & \u{4E2D}");
        let Block::Paragraph { runs, .. } = &out[0] else { panic!("paragraph") };
        assert_eq!(runs[0].style.link.as_deref(), Some("https://example.test/?q=\u{00E9}>x&y=1"));
        let out = html_to_blocks("<p><span title=don't>x</span></p>");
        assert_eq!(paragraph_text(&out[0]), "x");
    }

    #[test]
    fn unquoted_link_slashes_are_attribute_content() {
        let out = html_to_blocks("<a href=https://example.test/path?q=1&amp;b=2>link</a>");
        let Block::Paragraph { runs, .. } = &out[0] else { panic!("paragraph") };
        assert_eq!(runs[0].style.link.as_deref(), Some("https://example.test/path?q=1&b=2"));
    }

    #[test]
    fn invalid_tag_starts_remain_literal_utf8_text() {
        let out = html_to_blocks("<pre>\u{FEFF}5 < 7 \u{00E9}\u{1F600}\r\n<\u{4E2D}> <</pre>");
        assert_eq!(out.len(), 1);
        assert_eq!(paragraph_text(&out[0]), "\u{FEFF}5 < 7 \u{00E9}\u{1F600}\r\n<\u{4E2D}> <");
        let out = html_to_blocks("tail <");
        assert_eq!(paragraph_text(&out[0]), "tail <");
    }

    #[test]
    fn nested_formatting_preserves_outer_styles_around_raw_text() {
        let html = format!("<p><b>{}before<style>hidden</style>after{}</b> plain</p>",
            "<i>".repeat(64), "</i>".repeat(64));
        let out = html_to_blocks(&html);
        assert_eq!(paragraph_text(&out[0]), "beforeafter plain");
        let Block::Paragraph { runs, .. } = &out[0] else { panic!("paragraph") };
        assert!(runs[0].style.bold && runs[0].style.italic);
        assert_eq!(runs.last().expect("plain run").style, InlineStyle::default());
    }

    #[test]
    fn excessive_html_nesting_uses_the_plain_paste_fallback() {
        for (open, close) in [("<b>", "</b>"), ("<ul>", "</ul>"), ("<pre>", "</pre>")] {
            let html = format!("{}x{}", open.repeat(300), close.repeat(300));
            assert!(html_to_blocks(&html).is_empty(), "{open}");
        }
    }

    #[test]
    fn excessive_html_attributes_use_the_plain_paste_fallback() {
        let html = format!("<a href='{}'>text</a>", "x".repeat(64 * 1024));
        assert!(html_to_blocks(&html).is_empty());
    }

    #[test]
    fn excessive_html_token_counts_use_the_plain_paste_fallback() {
        assert!(html_to_blocks(&"<p>x</p>".repeat(50_001)).is_empty());
    }

    #[test]
    fn oversized_html_uses_the_plain_paste_fallback() {
        assert!(html_to_blocks(&"x".repeat(17 * 1024 * 1024)).is_empty());
    }

    #[test]
    fn css_color_hex_with_multibyte_chars_is_rejected() {
        assert_eq!(parse_css_color("#é1"), None);
        assert_eq!(parse_css_color("#1é"), None);
        assert_eq!(parse_css_color("#中"), None);
        assert_eq!(parse_css_color("#éé12"), None);
        let out = html_to_blocks("<p><span style=\"color:#é1\">x</span></p>");
        assert_eq!(paragraph_text(&out[0]), "x");
    }

    #[test]
    fn css_rgb_components_clamp_to_255() {
        assert_eq!(parse_css_color("rgb(300, 0, 4294967295)"), Some(0xFF00FF));
        assert_eq!(parse_css_color("rgb(255, 256, 0)"), Some(0xFFFF00));
    }

    #[test]
    fn br_splits_paragraphs() {
        let out = html_to_blocks("<p>one<br>two</p>");
        assert_eq!(out.len(), 2);
        assert_eq!(paragraph_text(&out[0]), "one");
        assert_eq!(paragraph_text(&out[1]), "two");
    }

    #[test]
    fn empty_html_returns_empty_vec() {
        assert!(html_to_blocks("").is_empty());
    }
}
