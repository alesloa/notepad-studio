// SPDX-License-Identifier: Apache-2.0
//
// Serialise the in-memory theme model back to a Notepad++-compatible XML
// document. Mirrors `Parameters.cpp::writeStyle2Element` rules:
//
//   * `fgColor` / `bgColor` — write `%06X` when `Some`. Omit otherwise.
//   * `colorStyle`          — write only when not `ColourFlags::ALL` (the default).
//   * `fontName`            — always emit; empty string when `None`. (Matches NPP
//                              output. Useful so reload preserves attribute presence.)
//   * `fontStyle`           — always emit; empty string when `None`.
//   * `fontSize`            — always emit; empty string when `None`.
//   * `keywordClass`        — write the textual token when `Some`; omit otherwise.
//   * `keywords`            — text content child when present.
//
// The output is UTF-8, 4-space indented, LF-terminated. Tests round-trip
// `parse → write → parse` for semantic equality (not byte-for-byte).

use std::fmt::Write as _;

use super::keyword_class::KeywordClass;
use super::lexer::{LexerStyler, LexerStylerArray};
use super::style::{format_hex_rgb, ColourFlags, Style, STYLE_NOT_USED};
use super::xml_load::ThemeDocument;

/// Serialise a complete theme document to XML.
pub fn write(doc: &ThemeDocument) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n");
    out.push_str("<NotepadPlus>\n");

    out.push_str("    <LexerStyles>\n");
    for lex in doc.lexers.lexers.iter() {
        write_lexer(&mut out, lex);
    }
    out.push_str("    </LexerStyles>\n");

    out.push_str("    <GlobalStyles>\n");
    for s in &doc.global_styles {
        write_widget_style(&mut out, s);
    }
    out.push_str("    </GlobalStyles>\n");

    out.push_str("</NotepadPlus>\n");
    out
}

fn write_lexer(out: &mut String, lex: &LexerStyler) {
    out.push_str("        <LexerType");
    write_attr(out, "name", &lex.lexer_name);
    write_attr(out, "desc", &lex.lexer_desc);
    if !lex.lexer_user_ext.is_empty() {
        write_attr(out, "ext", &lex.lexer_user_ext);
    } else {
        // Match NPP: emit empty ext attribute even when blank.
        write_attr(out, "ext", "");
    }
    if lex.excluded {
        write_attr(out, "excluded", "yes");
    }
    if lex.styles.is_empty() {
        out.push_str(" />\n");
        return;
    }
    out.push_str(">\n");
    for s in &lex.styles {
        write_words_style(out, s);
    }
    out.push_str("        </LexerType>\n");
}

fn write_words_style(out: &mut String, s: &Style) {
    out.push_str("            <WordsStyle");
    write_style_core_attrs(out, s);
    write_keyword_class(out, s);
    write_with_text_or_empty(out, s);
}

fn write_widget_style(out: &mut String, s: &Style) {
    out.push_str("        <WidgetStyle");
    write_style_core_attrs(out, s);
    // WidgetStyle never has keywordClass, but we emit it if present for
    // forward compatibility.
    write_keyword_class(out, s);
    write_with_text_or_empty(out, s);
}

fn write_style_core_attrs(out: &mut String, s: &Style) {
    write_attr(out, "name", &s.name);

    // styleID: emit even when 0 (some widget styles are identified purely by
    // name and use styleID=0). Skip only the STYLE_NOT_USED sentinel.
    if s.style_id != STYLE_NOT_USED {
        let _ = write!(out, " styleID=\"{}\"", s.style_id);
    }

    if let Some(fg) = s.fg_color {
        write_attr(out, "fgColor", &format_hex_rgb(fg));
    }
    if let Some(bg) = s.bg_color {
        write_attr(out, "bgColor", &format_hex_rgb(bg));
    }
    if s.colour_style != ColourFlags::all() {
        let _ = write!(out, " colorStyle=\"{}\"", s.colour_style.bits());
    }

    // Emit fontName / fontStyle / fontSize as empty strings when None — matches
    // NPP's verbose output (e.g. `fontName="" fontStyle="0" fontSize=""`).
    let font_name = s.font_name.as_deref().unwrap_or("");
    write_attr(out, "fontName", font_name);

    let font_style_text = match s.font_style {
        Some(flags) => flags.bits().to_string(),
        None => String::new(),
    };
    write_attr(out, "fontStyle", &font_style_text);

    let font_size_text = match s.font_size {
        Some(n) => n.to_string(),
        None => String::new(),
    };
    write_attr(out, "fontSize", &font_size_text);

    if s.nesting != 0 {
        let _ = write!(out, " nesting=\"{}\"", s.nesting);
    }
}

fn write_keyword_class(out: &mut String, s: &Style) {
    if let Some(n) = s.keyword_class {
        if let Some(kc) = KeywordClass::from_int(n) {
            write_attr(out, "keywordClass", &kc.to_string());
        }
    }
}

fn write_with_text_or_empty(out: &mut String, s: &Style) {
    match &s.keywords {
        Some(text) if !text.is_empty() => {
            out.push('>');
            push_xml_escaped(out, text);
            // Closing tag matches the opener: WordsStyle vs WidgetStyle.
            // We've already written `<WordsStyle...` or `<WidgetStyle...` —
            // detect by walking back to the most recent `<` start tag.
            let close_tag = if recent_open_is(out, "WordsStyle") {
                "</WordsStyle>"
            } else {
                "</WidgetStyle>"
            };
            out.push_str(close_tag);
            out.push('\n');
        }
        _ => out.push_str(" />\n"),
    }
}

/// Look at the last `<...` start tag in `out` and check whether it begins with
/// `<{tag}`. Used to choose the correct closing-tag literal without threading
/// the tag name through the helpers.
fn recent_open_is(out: &str, tag: &str) -> bool {
    let bytes = out.as_bytes();
    // Walk back to the last unescaped `<`.
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'<' {
            // Compare following bytes to `tag`.
            let start = i + 1;
            let end = (start + tag.len()).min(bytes.len());
            return &bytes[start..end] == tag.as_bytes();
        }
    }
    false
}

fn write_attr(out: &mut String, key: &str, value: &str) {
    out.push(' ');
    out.push_str(key);
    out.push_str("=\"");
    push_xml_escaped(out, value);
    out.push('"');
}

fn push_xml_escaped(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style_model::xml_load::parse;
    use crate::style_model::Style;

    const MINIMAL: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <LexerStyles>
        <LexerType name="cpp" desc="C++" ext="">
            <WordsStyle name="DEFAULT" styleID="11" fgColor="E0E2E4" bgColor="293134" fontName="" fontStyle="0" fontSize="" />
            <WordsStyle name="COMMENT" styleID="1" fgColor="66747B" fontName="" fontStyle="2" fontSize="" keywordClass="instre1">if else for while</WordsStyle>
            <WordsStyle name="UNSET" styleID="2" fontName="" fontStyle="" fontSize="" />
        </LexerType>
    </LexerStyles>
    <GlobalStyles>
        <WidgetStyle name="Default Style" styleID="32" fgColor="E0E2E4" bgColor="293134" fontName="Courier New" fontStyle="0" fontSize="10" />
        <WidgetStyle name="Caret colour" styleID="0" fgColor="A7A7A7" fontName="" fontStyle="" fontSize="" />
        <WidgetStyle name="Edge colour" styleID="0" fontName="" fontStyle="" fontSize="" />
    </GlobalStyles>
</NotepadPlus>
"#;

    #[test]
    fn round_trip_semantic_equality() {
        let parsed = parse(MINIMAL).expect("parse first");
        let written = write(&parsed);
        let reparsed = parse(&written).expect("parse second");
        assert_eq!(parsed, reparsed, "round-trip should preserve all fields");
    }

    #[test]
    fn unset_colour_round_trips_as_none() {
        // FF000000 sentinel comes back as None on parse. On write we omit
        // the attr. After re-parse, the field stays None.
        let xml = r#"<NotepadPlus><LexerStyles><LexerType name="t" desc="T">
            <WordsStyle name="X" styleID="1" fgColor="FF000000" bgColor="FF000000" />
        </LexerType></LexerStyles><GlobalStyles></GlobalStyles></NotepadPlus>"#;
        let parsed = parse(xml).expect("parse");
        assert!(parsed.lexers.get(0).unwrap().styles[0].fg_color.is_none());
        let written = write(&parsed);
        assert!(
            !written.contains("fgColor=\"FF000000\""),
            "writer must not re-emit the sentinel literal"
        );
        let reparsed = parse(&written).expect("re-parse");
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn keyword_class_round_trip() {
        let xml = r#"<NotepadPlus><LexerStyles><LexerType name="t" desc="T">
            <WordsStyle name="X" styleID="1" keywordClass="instre1">a b c</WordsStyle>
            <WordsStyle name="Y" styleID="2" keywordClass="type3">x y</WordsStyle>
        </LexerType></LexerStyles><GlobalStyles></GlobalStyles></NotepadPlus>"#;
        let parsed = parse(xml).expect("parse");
        let written = write(&parsed);
        let reparsed = parse(&written).expect("parse");
        assert_eq!(parsed, reparsed);
        assert!(written.contains("keywordClass=\"instre1\""));
        assert!(written.contains("keywordClass=\"type3\""));
    }

    #[test]
    fn keywords_xml_escaped() {
        let mut style = Style::named(1, "X");
        style.keywords = Some("a < b & c > d".to_string());
        let mut lex = LexerStyler::default();
        lex.lexer_name = "t".into();
        lex.lexer_desc = "T".into();
        lex.styles.push(style);
        let mut doc = ThemeDocument::default();
        doc.lexers.push(lex);
        let xml = write(&doc);
        assert!(xml.contains("a &lt; b &amp; c &gt; d"));
        let reparsed = parse(&xml).expect("re-parse");
        assert_eq!(
            reparsed.lexers.get(0).unwrap().styles[0]
                .keywords
                .as_deref(),
            Some("a < b & c > d")
        );
    }

    #[test]
    fn empty_styles_self_close() {
        let mut lex = LexerStyler::default();
        lex.lexer_name = "empty".into();
        lex.lexer_desc = "Empty".into();
        let mut doc = ThemeDocument::default();
        doc.lexers.push(lex);
        let xml = write(&doc);
        assert!(xml.contains("<LexerType name=\"empty\""));
        assert!(xml.contains(" />"));
    }
}
