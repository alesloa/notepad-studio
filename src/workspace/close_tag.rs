//! HTML/XML close-tag autocompletion helper.
//!
//! When the user types `/` immediately after a `<` in an HTML or XML buffer,
//! `find_unclosed_tag` scans every byte before the `<` and returns the name
//! of the innermost still-open element. The caller splices `tagname>` after
//! the `/` so the buffer reaches `</tagname>` in one keystroke.
//!
//! The scanner is deliberately permissive — it tolerates stray `<`, missing
//! `>`, unmatched closers — so a half-typed document never blocks completion.
//! NPP behavior parity: case-insensitive name match, HTML void elements
//! (`br`, `img`, …) never push onto the open stack, self-closing `<foo/>`
//! never pushes either, and `<!-- … -->` / `<![CDATA[ … ]]>` / `<!DOCTYPE …>`
//! / `<? … ?>` segments are skipped entirely.

/// Walks `source` and returns the innermost unclosed open-tag name, or
/// `None` if every tag is balanced (or none exist). When `html` is true, the
/// HTML void-element list is honoured so `<br>` doesn't pollute the stack;
/// XML mode requires explicit `</>` or `/>` to close every tag.
pub(crate) fn find_unclosed_tag(source: &str, html: bool) -> Option<String> {
    let bytes = source.as_bytes();
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &source[i..];
        // <!-- ... -->
        if rest.starts_with("<!--") {
            match source[i + 4..].find("-->") {
                Some(off) => i = i + 4 + off + 3,
                None => return stack.pop(),
            }
            continue;
        }
        // <![CDATA[ ... ]]>
        if rest.starts_with("<![CDATA[") {
            match source[i + 9..].find("]]>") {
                Some(off) => i = i + 9 + off + 3,
                None => return stack.pop(),
            }
            continue;
        }
        if i + 1 >= bytes.len() {
            break;
        }
        let next = bytes[i + 1];
        // <!DOCTYPE ...> or other declaration / <?xml ...?>
        if next == b'!' || next == b'?' {
            match rest.find('>') {
                Some(off) => i += off + 1,
                None => return stack.pop(),
            }
            continue;
        }
        // </name>
        if next == b'/' {
            let name_start = i + 2;
            let name_end = scan_tag_name_end(bytes, name_start);
            if name_end > name_start {
                let name = source[name_start..name_end].to_ascii_lowercase();
                if let Some(pos) = stack.iter().rposition(|t| *t == name) {
                    stack.truncate(pos);
                }
            }
            match source[name_end..].find('>') {
                Some(off) => i = name_end + off + 1,
                None => return stack.pop(),
            }
            continue;
        }
        // <name attrs...>
        if next.is_ascii_alphabetic() {
            let name_start = i + 1;
            let name_end = scan_tag_name_end(bytes, name_start);
            let name = source[name_start..name_end].to_ascii_lowercase();
            // Walk attrs to find `>`, skipping over double- and single-quoted
            // attribute values that may contain `<` or `>` literals.
            let mut j = name_end;
            let mut quote: Option<u8> = None;
            let mut closing: Option<usize> = None;
            while j < bytes.len() {
                let b = bytes[j];
                if let Some(q) = quote {
                    if b == q {
                        quote = None;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = Some(b);
                } else if b == b'>' {
                    closing = Some(j);
                    break;
                }
                j += 1;
            }
            let Some(close_at) = closing else {
                return stack.pop();
            };
            let self_closing = close_at > name_end && bytes[close_at - 1] == b'/';
            i = close_at + 1;
            if !name.is_empty()
                && !self_closing
                && !(html && is_html_void_element(&name))
            {
                stack.push(name);
            }
            continue;
        }
        // Stray `<` (e.g. `a < b` in JS embedded in HTML). Skip it.
        i += 1;
    }
    stack.pop()
}

/// `/` is about to be typed at byte `cursor`. When the char right before the
/// caret is `<`, return the innermost tag still open before that `<`, so the
/// caller can complete `</name>`. Scans the whole prefix, as Notepad++ does.
pub(crate) fn close_tag_for_slash(rope: &ropey::Rope, cursor: usize, html: bool) -> Option<String> {
    let cursor = super::edits::floor_rope_char_boundary(rope, cursor);
    let char_idx = rope.byte_to_char(cursor);
    let prev_idx = char_idx.checked_sub(1)?;
    if rope.get_char(prev_idx)? != '<' {
        return None;
    }
    let prev_char_start = rope.char_to_byte(prev_idx);
    let prefix: String = rope.byte_slice(..prev_char_start).to_string();
    find_unclosed_tag(&prefix, html)
}

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
    use super::find_unclosed_tag;

    #[test]
    fn single_open_tag_returns_name() {
        assert_eq!(
            find_unclosed_tag("<div>hello", true),
            Some("div".into())
        );
    }

    #[test]
    fn balanced_tags_return_none() {
        assert_eq!(
            find_unclosed_tag("<div>hi</div>", true),
            None
        );
    }

    #[test]
    fn nested_returns_innermost() {
        assert_eq!(
            find_unclosed_tag("<div><span>text", true),
            Some("span".into())
        );
    }

    #[test]
    fn nested_innermost_closed_returns_outer() {
        assert_eq!(
            find_unclosed_tag("<section><p>hi</p>", true),
            Some("section".into())
        );
    }

    #[test]
    fn case_insensitive_match() {
        assert_eq!(
            find_unclosed_tag("<DIV>hello", true),
            Some("div".into())
        );
        assert_eq!(
            find_unclosed_tag("<Section>hi</section>", true),
            None
        );
    }

    #[test]
    fn html_void_does_not_push() {
        assert_eq!(
            find_unclosed_tag("<p>line<br>more", true),
            Some("p".into())
        );
    }

    #[test]
    fn xml_void_does_push() {
        // In XML, `br` is just a normal element.
        assert_eq!(
            find_unclosed_tag("<doc><br>", false),
            Some("br".into())
        );
    }

    #[test]
    fn self_closing_does_not_push() {
        assert_eq!(
            find_unclosed_tag("<root><leaf/>", false),
            Some("root".into())
        );
        assert_eq!(
            find_unclosed_tag("<root><leaf />", false),
            Some("root".into())
        );
    }

    #[test]
    fn quoted_attrs_with_angle_brackets_are_ignored() {
        assert_eq!(
            find_unclosed_tag(r#"<a href="https://example.com/?x=<y>">"#, true),
            Some("a".into())
        );
        assert_eq!(
            find_unclosed_tag("<a title='not<a closer>'>", true),
            Some("a".into())
        );
    }

    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            find_unclosed_tag("<div><!-- <span> -->", true),
            Some("div".into())
        );
    }

    #[test]
    fn cdata_is_skipped() {
        assert_eq!(
            find_unclosed_tag("<root><![CDATA[ <child> ]]>", false),
            Some("root".into())
        );
    }

    #[test]
    fn doctype_is_skipped() {
        assert_eq!(
            find_unclosed_tag("<!DOCTYPE html><html>", true),
            Some("html".into())
        );
    }

    #[test]
    fn processing_instruction_is_skipped() {
        assert_eq!(
            find_unclosed_tag("<?xml version='1.0'?><root>", false),
            Some("root".into())
        );
    }

    #[test]
    fn stray_open_angle_ignored() {
        assert_eq!(
            find_unclosed_tag("if (a < b) {} <p>", true),
            Some("p".into())
        );
    }

    #[test]
    fn extra_close_tag_ignored() {
        // </nope> with no matching open should just be a no-op.
        assert_eq!(
            find_unclosed_tag("<div></nope>", true),
            Some("div".into())
        );
    }

    #[test]
    fn unterminated_open_returns_stack_so_far() {
        // `<bad attr=` never finds `>` — we surrender and return what we have.
        assert_eq!(
            find_unclosed_tag("<root><bad attr=", false),
            Some("root".into())
        );
    }

    #[test]
    fn deeply_nested() {
        let src = "<html><body><div><section><p>text";
        assert_eq!(find_unclosed_tag(src, true), Some("p".into()));
    }

    #[test]
    fn empty_input_returns_none() {
        assert_eq!(find_unclosed_tag("", true), None);
    }

    #[test]
    fn no_tags_returns_none() {
        assert_eq!(find_unclosed_tag("just some prose with < and > in it", true), None);
    }

    #[test]
    fn edge_inputs_never_panic() {
        let sources = [
            "", "<", "</", "<!", "<?", "<!--", "<![CDATA[", "<a", "<a b='", "<a\r\n<b>\r",
            "<é>中</é><p>", "<p>👨‍👩‍👧<br>🇫🇷", "<a/><b></b", "<<<>>>", "</a></b></c>",
        ];
        for src in sources {
            for html in [true, false] {
                let _ = find_unclosed_tag(src, html);
            }
        }
    }

    #[test]
    fn deep_nesting_is_linear_not_recursive() {
        let src = "<d>".repeat(100_000);
        assert_eq!(find_unclosed_tag(&src, true), Some("d".into()));
    }

    use super::close_tag_for_slash;
    use ropey::Rope;

    #[test]
    fn slash_after_angle_completes_innermost_tag() {
        let rope = Rope::from_str("<div><span>x<");
        assert_eq!(close_tag_for_slash(&rope, rope.len_bytes(), true), Some("span".into()));
        let rope = Rope::from_str("<root>\r\n  <");
        assert_eq!(close_tag_for_slash(&rope, rope.len_bytes(), false), Some("root".into()));
    }

    #[test]
    fn slash_without_angle_before_caret_is_none() {
        let rope = Rope::from_str("<div>x");
        assert_eq!(close_tag_for_slash(&rope, rope.len_bytes(), true), None);
        assert_eq!(close_tag_for_slash(&Rope::new(), 0, true), None);
        assert_eq!(close_tag_for_slash(&rope, 0, true), None);
    }

    #[test]
    fn slash_with_caret_past_end_or_mid_char_does_not_panic() {
        // A caret past EOF used to hit `byte_to_char` out of bounds; a caret
        // inside `é` used to end `byte_slice` off a char boundary.
        let rope = Rope::from_str("<p>é<");
        assert_eq!(close_tag_for_slash(&rope, 99, true), Some("p".into()));
        assert_eq!(close_tag_for_slash(&rope, 4, true), None);
        let rope = Rope::from_str("<p><👍");
        for cursor in 0..=rope.len_bytes() + 2 {
            let _ = close_tag_for_slash(&rope, cursor, true);
        }
    }
}
