//! HTML/XML tag-pair finder for the `Highlighting ▸ Highlight Matching Tags`
//! Editing feature. When the caret sits anywhere inside `<tagname …>` or
//! `</tagname>`, [`find_tag_pair`] returns the byte ranges of BOTH halves'
//! tag names so the editor can paint a highlight on each.
//!
//! The scanner walks raw bytes (UTF-8 safe for ASCII tag syntax) and matches
//! the same permissive rules as [`super::close_tag::find_unclosed_tag`]:
//!
//! * Comments (`<!-- … -->`), CDATA (`<![CDATA[ … ]]>`), declarations
//!   (`<!DOCTYPE …>`), and processing instructions (`<?xml … ?>`) are
//!   skipped — clicking inside one returns `None` even when the inside text
//!   looks like a tag.
//! * Quoted attribute values (`href="<not-a-tag>"`) don't fool the matcher.
//! * Tag names are compared case-insensitively (HTML convention; XML is also
//!   tolerated since real-world XML attributes are mixed-case in practice).
//! * `<foo/>` self-closing tags and HTML void elements (`<br>`, `<img>` …)
//!   never have a matching close — the function returns `None` for them.
//!
//! Returns `None` for malformed input, cursors outside any tag, or any case
//! where the pair can't be resolved unambiguously. The caller paints nothing
//! when it gets back `None`, which matches NPP's "no highlight on broken
//! markup" behaviour.

use std::ops::Range;

/// Byte ranges of the open and close tag names for a matched HTML/XML pair.
/// Each range covers the *name* portion only — for `<div class="x">` the
/// range is the bytes covering `div`, not the full `<div class="x">`. The
/// editor highlights both ranges with the same accent so the user sees the
/// pair at a glance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TagPair {
    pub(crate) open: Range<usize>,
    pub(crate) close: Range<usize>,
}

/// Resolve the HTML/XML tag pair surrounding `cursor`. Returns `None` when:
/// * the cursor doesn't sit inside any `< … >` boundary,
/// * the surrounding boundary is a comment / CDATA / DOCTYPE / PI,
/// * the surrounding tag is self-closing / a void element (no pair to find),
/// * the matching half doesn't exist (open tag with no close, etc.).
pub(crate) fn find_tag_pair(
    source: &str,
    cursor: usize,
    html: bool,
) -> Option<TagPair> {
    let bytes = source.as_bytes();
    if cursor > bytes.len() {
        return None;
    }
    let (lt, gt, is_close) = surrounding_tag_bounds(source, cursor)?;

    let name_off = if is_close { lt + 2 } else { lt + 1 };
    let name_end = scan_tag_name_end(bytes, name_off);
    if name_end <= name_off {
        return None;
    }
    let this_name = source[name_off..name_end].to_ascii_lowercase();
    let this_range = name_off..name_end;

    if is_close {
        let open_range = find_matching_open(source, lt, &this_name, html)?;
        Some(TagPair {
            open: open_range,
            close: this_range,
        })
    } else {
        // Self-closing or void: no matching close exists.
        if gt > 0 && bytes[gt - 1] == b'/' {
            return None;
        }
        if html && is_html_void_element(&this_name) {
            return None;
        }
        let close_range = find_matching_close(source, gt + 1, &this_name, html)?;
        Some(TagPair {
            open: this_range,
            close: close_range,
        })
    }
}

/// Locate the `<` … `>` bracket pair surrounding `cursor`. Returns the byte
/// position of `<`, the byte position of `>`, and whether the tag is a close
/// tag (`</…>`). Returns `None` if cursor is outside any tag or sits inside
/// a comment / CDATA / declaration / PI.
fn surrounding_tag_bounds(source: &str, cursor: usize) -> Option<(usize, usize, bool)> {
    let bytes = source.as_bytes();
    // Walk backward from cursor to find the nearest `<` that isn't inside a
    // skip-zone (comment/CDATA/etc.). A small forward sweep from earlier in
    // the document classifies every `<` we'd cross; whichever zone covers
    // the cursor (open or skip) tells us if there's a tag to match.
    //
    // Cheap pre-filter: if the bytes before the cursor hold no `<`, there's
    // no tag at all. Byte-level so a cursor inside a multi-byte char can't
    // slice the str off a char boundary.
    let before = bytes.get(..cursor)?;
    if !before.contains(&b'<') {
        return None;
    }
    // Walk forward from byte 0 building a list of (lt, gt, is_close, is_skip).
    // Then pick the entry whose [lt..gt] range contains `cursor`. O(N) per
    // refresh, capped by SMART_HILITE_BUFFER_LIMIT_BYTES at the caller.
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &source[i..];
        // Skip zones — comments / CDATA / declarations / PIs.
        if rest.starts_with("<!--") {
            match source[i + 4..].find("-->") {
                Some(off) => i = i + 4 + off + 3,
                None => return None,
            }
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            match source[i + 9..].find("]]>") {
                Some(off) => i = i + 9 + off + 3,
                None => return None,
            }
            continue;
        }
        if i + 1 >= bytes.len() {
            break;
        }
        let next = bytes[i + 1];
        if next == b'!' || next == b'?' {
            match rest.find('>') {
                Some(off) => i += off + 1,
                None => return None,
            }
            continue;
        }
        let is_close = next == b'/';
        if !is_close && !next.is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        // Walk to the `>` that ends this tag, honouring quoted attribute
        // values so `class="<foo>"` doesn't fool the search.
        let Some(close_at) = find_tag_close(bytes, i + 1) else {
            return None;
        };
        if cursor >= i && cursor <= close_at {
            return Some((i, close_at, is_close));
        }
        i = close_at + 1;
    }
    None
}

/// Walk `bytes` from `from` looking for the `>` that ends the current tag,
/// skipping over quoted attribute values. Returns the byte offset of `>` or
/// `None` if the tag is unterminated.
fn find_tag_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    let mut quote: Option<u8> = None;
    while j < bytes.len() {
        let b = bytes[j];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// Walk forward from `from` to find the close tag matching `name`, honouring
/// nested same-named tags via depth counting. Returns the byte range of the
/// matching close tag's name (inside `</…>`).
fn find_matching_close(
    source: &str,
    from: usize,
    name: &str,
    html: bool,
) -> Option<Range<usize>> {
    let bytes = source.as_bytes();
    let mut depth: usize = 1;
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &source[i..];
        if rest.starts_with("<!--") {
            i = source[i + 4..]
                .find("-->")
                .map(|off| i + 4 + off + 3)
                .unwrap_or(bytes.len());
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = source[i + 9..]
                .find("]]>")
                .map(|off| i + 9 + off + 3)
                .unwrap_or(bytes.len());
            continue;
        }
        if i + 1 >= bytes.len() {
            break;
        }
        let next = bytes[i + 1];
        if next == b'!' || next == b'?' {
            i = rest.find('>').map(|off| i + off + 1).unwrap_or(bytes.len());
            continue;
        }
        if next == b'/' {
            let name_start = i + 2;
            let name_end = scan_tag_name_end(bytes, name_start);
            let tag = source[name_start..name_end].to_ascii_lowercase();
            let close_at = find_tag_close(bytes, name_end)?;
            if tag == name {
                depth -= 1;
                if depth == 0 {
                    return Some(name_start..name_end);
                }
            }
            i = close_at + 1;
            continue;
        }
        if next.is_ascii_alphabetic() {
            let name_start = i + 1;
            let name_end = scan_tag_name_end(bytes, name_start);
            let tag = source[name_start..name_end].to_ascii_lowercase();
            let close_at = find_tag_close(bytes, name_end)?;
            let self_closing = close_at > name_end && bytes[close_at - 1] == b'/';
            let is_void = html && is_html_void_element(&tag);
            if tag == name && !self_closing && !is_void {
                depth += 1;
            }
            i = close_at + 1;
            continue;
        }
        i += 1;
    }
    None
}

/// Walk backward from `lt` (position of the `<` of the close tag) to find
/// the matching open tag. Uses the same forward-scan logic but in reverse
/// order via collecting tag positions then matching from the close backward.
fn find_matching_open(
    source: &str,
    lt: usize,
    name: &str,
    html: bool,
) -> Option<Range<usize>> {
    // Collect every (kind, name_range, is_self_closing) before `lt` by
    // walking forward from 0. Skip-zones are handled inline. Then walk the
    // collected list backward with depth counting.
    let bytes = source.as_bytes();
    let mut tags: Vec<(bool, Range<usize>, bool)> = Vec::new(); // (is_close, name_range, self_closing_or_void)
    let mut i = 0usize;
    while i < lt {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &source[i..];
        if rest.starts_with("<!--") {
            i = source[i + 4..]
                .find("-->")
                .map(|off| i + 4 + off + 3)
                .unwrap_or(bytes.len());
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = source[i + 9..]
                .find("]]>")
                .map(|off| i + 9 + off + 3)
                .unwrap_or(bytes.len());
            continue;
        }
        if i + 1 >= bytes.len() {
            break;
        }
        let next = bytes[i + 1];
        if next == b'!' || next == b'?' {
            i = rest.find('>').map(|off| i + off + 1).unwrap_or(bytes.len());
            continue;
        }
        if next == b'/' {
            let name_start = i + 2;
            let name_end = scan_tag_name_end(bytes, name_start);
            let close_at = find_tag_close(bytes, name_end)?;
            tags.push((true, name_start..name_end, false));
            i = close_at + 1;
            continue;
        }
        if next.is_ascii_alphabetic() {
            let name_start = i + 1;
            let name_end = scan_tag_name_end(bytes, name_start);
            let close_at = find_tag_close(bytes, name_end)?;
            let self_closing = close_at > name_end && bytes[close_at - 1] == b'/';
            let tag = source[name_start..name_end].to_ascii_lowercase();
            let is_void = html && is_html_void_element(&tag);
            tags.push((false, name_start..name_end, self_closing || is_void));
            i = close_at + 1;
            continue;
        }
        i += 1;
    }
    // Walk backward: every open we find with a matching name decrements
    // depth; same-named closes increment depth.
    let mut depth: usize = 1;
    for (is_close, range, self_or_void) in tags.into_iter().rev() {
        let tag_name = source[range.clone()].to_ascii_lowercase();
        if tag_name != name {
            continue;
        }
        if is_close {
            depth += 1;
        } else if !self_or_void {
            depth -= 1;
            if depth == 0 {
                return Some(range);
            }
        }
    }
    None
}

/// Tag-name byte scan: stops at the first non-name byte. Matches
/// `super::close_tag::scan_tag_name_end` exactly — duplicated here to keep
/// this module standalone (close_tag.rs is private to its file).
fn scan_tag_name_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':' {
            i += 1;
        } else {
            break;
        }
    }
    i
}

/// Collect every attribute name+value range inside the open tag containing
/// `cursor`. Each returned range covers `name[=value]` — for `<div class="x">`
/// the range is the bytes covering `class="x"` (including the `=` and quotes).
/// Returns an empty vec when:
/// * the cursor isn't inside any tag boundary,
/// * the surrounding tag is a close tag (`</div>` has no attributes),
/// * the surrounding zone is a comment / CDATA / declaration / PI,
/// * the tag has no attributes.
pub(crate) fn find_attribute_ranges(source: &str, cursor: usize) -> Vec<Range<usize>> {
    let bytes = source.as_bytes();
    if cursor > bytes.len() {
        return Vec::new();
    }
    let Some((lt, gt, is_close)) = surrounding_tag_bounds(source, cursor) else {
        return Vec::new();
    };
    if is_close {
        return Vec::new();
    }
    // Walk from past the tag name to gt-1, collecting attribute ranges.
    let name_off = lt + 1;
    let mut i = scan_tag_name_end(bytes, name_off);
    let end = if gt > 0 && bytes[gt - 1] == b'/' { gt - 1 } else { gt };
    let mut out: Vec<Range<usize>> = Vec::new();
    while i < end {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Attribute name byte set matches XML's NameChar liberally — letters,
        // digits, `-`, `_`, `:`, `.`. We also accept `/` as a sentinel so a
        // stray `/` (e.g. inside `<input disabled />`) doesn't get swallowed.
        let name_start = i;
        while i < end {
            let c = bytes[i];
            if c.is_ascii_alphanumeric()
                || c == b'-'
                || c == b'_'
                || c == b':'
                || c == b'.'
            {
                i += 1;
            } else {
                break;
            }
        }
        if i == name_start {
            i += 1;
            continue;
        }
        let mut attr_end = i;
        // Optional `=` then value.
        let mut j = i;
        while j < end && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < end && bytes[j] == b'=' {
            j += 1;
            while j < end && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < end {
                let v = bytes[j];
                if v == b'"' || v == b'\'' {
                    let quote = v;
                    j += 1;
                    while j < end && bytes[j] != quote {
                        j += 1;
                    }
                    if j < end {
                        j += 1;
                    }
                    attr_end = j;
                } else {
                    // Unquoted value — runs until whitespace or end.
                    while j < end && !bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    attr_end = j;
                }
            }
            i = attr_end;
        }
        out.push(name_start..attr_end);
    }
    out
}

/// Resolve the comment / CDATA / PI / ASP zone surrounding `cursor` and return
/// the byte ranges of its OPEN and CLOSE delimiters as a [`TagPair`]:
///
/// | Zone shape           | open range          | close range |
/// |----------------------|---------------------|-------------|
/// | `<!-- … -->`         | `<!--`              | `-->`       |
/// | `<![CDATA[ … ]]>`    | `<![CDATA[`         | `]]>`       |
/// | `<? … ?>` / `<?php`  | `<?` (full prefix)  | `?>`        |
/// | `<% … %>`            | `<%`                | `%>`        |
///
/// Returns `None` when the cursor isn't inside any of these zones. The caller
/// paints both delimiter ranges in the smart-highlight colour — the user sees
/// the comment / PI / ASP opener and its matching terminator at a glance.
pub(crate) fn find_zone_pair(source: &str, cursor: usize) -> Option<TagPair> {
    let bytes = source.as_bytes();
    if cursor > bytes.len() {
        return None;
    }
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &source[i..];
        // HTML comment.
        if rest.starts_with("<!--") {
            let from = i + 4;
            let end_off = source[from..].find("-->");
            let close_start = match end_off {
                Some(off) => from + off,
                None => return None,
            };
            let close_end = close_start + 3;
            if cursor >= i && cursor <= close_end {
                return Some(TagPair {
                    open: i..i + 4,
                    close: close_start..close_end,
                });
            }
            i = close_end;
            continue;
        }
        // CDATA section.
        if rest.starts_with("<![CDATA[") {
            let from = i + 9;
            let end_off = source[from..].find("]]>");
            let close_start = match end_off {
                Some(off) => from + off,
                None => return None,
            };
            let close_end = close_start + 3;
            if cursor >= i && cursor <= close_end {
                return Some(TagPair {
                    open: i..i + 9,
                    close: close_start..close_end,
                });
            }
            i = close_end;
            continue;
        }
        // Processing instruction (covers `<?xml`, `<?php`, `<? … ?>`).
        if i + 1 < bytes.len() && bytes[i + 1] == b'?' {
            // Open delim runs from `<?` through the optional language tag
            // (e.g. `php`, `xml`). Walk while alphanumeric to consume it.
            let mut open_end = i + 2;
            while open_end < bytes.len() && bytes[open_end].is_ascii_alphabetic() {
                open_end += 1;
            }
            let end_off = source[open_end..].find("?>");
            let close_start = match end_off {
                Some(off) => open_end + off,
                None => return None,
            };
            let close_end = close_start + 2;
            if cursor >= i && cursor <= close_end {
                return Some(TagPair {
                    open: i..open_end,
                    close: close_start..close_end,
                });
            }
            i = close_end;
            continue;
        }
        // ASP / JSP / EJS / Twig: `<%` … `%>`.
        if i + 1 < bytes.len() && bytes[i + 1] == b'%' {
            let from = i + 2;
            let end_off = source[from..].find("%>");
            let close_start = match end_off {
                Some(off) => from + off,
                None => return None,
            };
            let close_end = close_start + 2;
            if cursor >= i && cursor <= close_end {
                return Some(TagPair {
                    open: i..i + 2,
                    close: close_start..close_end,
                });
            }
            i = close_end;
            continue;
        }
        // SGML declaration (`<!DOCTYPE …>`) — treated as a one-shot zone with
        // no useful pair (the `>` is its own terminator), so skip past it.
        if i + 1 < bytes.len() && bytes[i + 1] == b'!' {
            i = rest.find('>').map(|off| i + off + 1).unwrap_or(bytes.len());
            continue;
        }
        i += 1;
    }
    None
}

/// HTML void-element list. Duplicated from `super::close_tag` for the
/// reasons noted on `scan_tag_name_end`.
fn is_html_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

#[cfg(test)]
mod tests {
    use super::{find_tag_pair, TagPair};

    fn pair(open: std::ops::Range<usize>, close: std::ops::Range<usize>) -> TagPair {
        TagPair { open, close }
    }

    #[test]
    fn cursor_inside_open_tag_finds_close() {
        let src = "<div>hello</div>";
        // cursor anywhere on "div" inside <div> should resolve to <div>...</div>
        let got = find_tag_pair(src, 2, true).expect("expected pair");
        assert_eq!(got, pair(1..4, 12..15));
    }

    #[test]
    fn cursor_inside_close_tag_finds_open() {
        let src = "<div>hello</div>";
        // cursor inside </div>'s name (offset 13 is 'i' of 'div')
        let got = find_tag_pair(src, 13, true).expect("expected pair");
        assert_eq!(got, pair(1..4, 12..15));
    }

    #[test]
    fn nested_same_name() {
        let src = "<a><a>inner</a></a>";
        // cursor on outer <a> -> matches the outermost </a> at the end
        let outer = find_tag_pair(src, 1, true).expect("outer pair");
        assert_eq!(outer, pair(1..2, 17..18));
        // cursor on inner <a> -> matches the inner </a> at index 11..14
        let inner = find_tag_pair(src, 4, true).expect("inner pair");
        assert_eq!(inner, pair(4..5, 13..14));
    }

    #[test]
    fn self_closing_returns_none() {
        let src = "<br/>";
        assert!(find_tag_pair(src, 1, true).is_none());
    }

    #[test]
    fn html_void_returns_none() {
        let src = "<br>after";
        assert!(find_tag_pair(src, 1, true).is_none());
    }

    #[test]
    fn xml_void_is_NOT_void_so_unmatched_returns_none() {
        let src = "<br>after";
        // No matching </br> exists -> None
        assert!(find_tag_pair(src, 1, false).is_none());
    }

    #[test]
    fn cursor_in_attribute_still_matches() {
        let src = r#"<div class="x">hi</div>"#;
        // cursor inside `class="x"` should still find the pair
        let got = find_tag_pair(src, 7, true).expect("pair");
        assert_eq!(got.open, 1..4);
        assert_eq!(&src[got.close.clone()], "div");
    }

    #[test]
    fn cursor_inside_comment_returns_none() {
        let src = "<!-- <div> -->";
        // cursor inside the comment text where it LOOKS like a tag
        assert!(find_tag_pair(src, 6, true).is_none());
    }

    #[test]
    fn cursor_inside_cdata_returns_none() {
        let src = "<![CDATA[<div>foo</div>]]>";
        assert!(find_tag_pair(src, 10, false).is_none());
    }

    #[test]
    fn case_insensitive_match() {
        let src = "<DIV>x</div>";
        let got = find_tag_pair(src, 2, true).expect("pair");
        assert_eq!(&src[got.open.clone()], "DIV");
        assert_eq!(&src[got.close.clone()], "div");
    }

    #[test]
    fn quoted_attr_with_angle_brackets() {
        let src = r#"<a href="<x>"><b>i</b></a>"#;
        // cursor on outer <a> -> the `<x>` inside the attr should be ignored
        let got = find_tag_pair(src, 1, true).expect("outer pair");
        assert_eq!(&src[got.open.clone()], "a");
        assert_eq!(&src[got.close.clone()], "a");
    }

    #[test]
    fn unmatched_open_returns_none() {
        let src = "<a><b>";
        // <a> has no matching close -> None
        assert!(find_tag_pair(src, 1, true).is_none());
    }

    #[test]
    fn cursor_outside_any_tag_returns_none() {
        let src = "<a>hello</a>";
        // cursor inside the text content
        assert!(find_tag_pair(src, 5, true).is_none());
    }

    // ---------- Attribute ranges -----------------------------------------

    #[test]
    fn attrs_quoted_value() {
        let src = r#"<div class="foo">x</div>"#;
        let got = super::find_attribute_ranges(src, 7);
        assert_eq!(got.len(), 1);
        assert_eq!(&src[got[0].clone()], r#"class="foo""#);
    }

    #[test]
    fn attrs_multiple() {
        let src = r#"<a href="x" id='y' disabled>z</a>"#;
        let got = super::find_attribute_ranges(src, 3);
        assert_eq!(got.len(), 3);
        assert_eq!(&src[got[0].clone()], r#"href="x""#);
        assert_eq!(&src[got[1].clone()], "id='y'");
        assert_eq!(&src[got[2].clone()], "disabled");
    }

    #[test]
    fn attrs_unquoted_value() {
        let src = "<input type=text>";
        let got = super::find_attribute_ranges(src, 2);
        assert_eq!(got.len(), 1);
        assert_eq!(&src[got[0].clone()], "type=text");
    }

    #[test]
    fn attrs_self_closing_strips_trailing_slash() {
        let src = "<br class='x' />";
        let got = super::find_attribute_ranges(src, 2);
        assert_eq!(got.len(), 1);
        assert_eq!(&src[got[0].clone()], "class='x'");
    }

    #[test]
    fn attrs_close_tag_returns_empty() {
        let src = "<div>x</div>";
        let got = super::find_attribute_ranges(src, 8);
        assert!(got.is_empty());
    }

    #[test]
    fn attrs_no_attrs_returns_empty() {
        let src = "<div>x</div>";
        let got = super::find_attribute_ranges(src, 2);
        assert!(got.is_empty());
    }

    // ---------- Zone pairs -----------------------------------------------

    #[test]
    fn zone_html_comment() {
        let src = "abc <!-- hello world --> def";
        let got = super::find_zone_pair(src, 10).expect("inside comment");
        assert_eq!(&src[got.open.clone()], "<!--");
        assert_eq!(&src[got.close.clone()], "-->");
    }

    #[test]
    fn zone_cdata() {
        let src = "<![CDATA[ payload ]]> rest";
        let got = super::find_zone_pair(src, 10).expect("inside cdata");
        assert_eq!(&src[got.open.clone()], "<![CDATA[");
        assert_eq!(&src[got.close.clone()], "]]>");
    }

    #[test]
    fn zone_pi_php() {
        let src = "<?php echo 'hi'; ?>";
        let got = super::find_zone_pair(src, 8).expect("inside php");
        assert_eq!(&src[got.open.clone()], "<?php");
        assert_eq!(&src[got.close.clone()], "?>");
    }

    #[test]
    fn zone_pi_xml() {
        let src = r#"<?xml version="1.0"?>"#;
        let got = super::find_zone_pair(src, 5).expect("inside xml decl");
        assert_eq!(&src[got.open.clone()], "<?xml");
        assert_eq!(&src[got.close.clone()], "?>");
    }

    #[test]
    fn zone_asp() {
        let src = "<% Response.Write(\"hi\") %>";
        let got = super::find_zone_pair(src, 5).expect("inside asp");
        assert_eq!(&src[got.open.clone()], "<%");
        assert_eq!(&src[got.close.clone()], "%>");
    }

    #[test]
    fn zone_cursor_outside_returns_none() {
        let src = "<!-- one --> outside <!-- two -->";
        // cursor between the two comments
        let got = super::find_zone_pair(src, 16);
        assert!(got.is_none());
    }

    #[test]
    fn cursor_mid_char_does_not_panic() {
        // `é` spans bytes 3..5; a caret byte of 4 sits inside it.
        let src = "<a>é</a>";
        assert!(find_tag_pair(src, 4, true).is_none());
        assert!(super::find_attribute_ranges(src, 4).is_empty());
        assert!(super::find_zone_pair(src, 4).is_none());
    }

    #[test]
    fn every_cursor_on_edge_inputs_is_safe() {
        let sources = [
            "",
            "<",
            "<a",
            "</",
            "<!--",
            "<![CDATA[",
            "<?",
            "<%",
            "<a href=\"é\">中</a>",
            "<p>👨‍👩‍👧<br/>🇫🇷</p>",
            "<a>\r\n<b>\r</b>\n</a>",
            "<a b='<'>x</A>",
            "<x:y-z_1>é</x:y-z_1>",
        ];
        for src in sources {
            for cursor in 0..=src.len() + 2 {
                for html in [true, false] {
                    if let Some(p) = find_tag_pair(src, cursor, html) {
                        assert!(src.get(p.open.clone()).is_some(), "{src:?} @ {cursor}");
                        assert!(src.get(p.close.clone()).is_some(), "{src:?} @ {cursor}");
                    }
                }
                for r in super::find_attribute_ranges(src, cursor) {
                    assert!(src.get(r).is_some(), "{src:?} @ {cursor}");
                }
                if let Some(p) = super::find_zone_pair(src, cursor) {
                    assert!(src.get(p.open.clone()).is_some(), "{src:?} @ {cursor}");
                    assert!(src.get(p.close.clone()).is_some(), "{src:?} @ {cursor}");
                }
            }
        }
    }

    #[test]
    fn zone_cursor_at_opener_boundary() {
        let src = "before <!-- x --> after";
        // cursor sitting at the `<` of `<!--`
        let got = super::find_zone_pair(src, 7).expect("on opener");
        assert_eq!(&src[got.open.clone()], "<!--");
    }
}
