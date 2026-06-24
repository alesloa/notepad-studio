// SPDX-License-Identifier: Apache-2.0
//
// Parse a Notepad++-compatible theme XML into the in-memory model.
//
// Schema (subset we care about):
//
//   <NotepadPlus ...>
//       <LexerStyles>
//           <LexerType name="cpp" desc="C++" ext="" excluded="no" userExt="">
//               <WordsStyle name="DEFAULT" styleID="11"
//                           fgColor="E0E2E4" bgColor="293134"
//                           fontName="" fontStyle="0" fontSize=""
//                           keywordClass="instre1">keyword list...</WordsStyle>
//               ...
//           </LexerType>
//           ...
//       </LexerStyles>
//       <GlobalStyles>
//           <WidgetStyle name="Default Style" styleID="32"
//                        fgColor="E0E2E4" bgColor="293134"
//                        fontName="Courier New" fontStyle="0" fontSize="10" />
//           ...
//       </GlobalStyles>
//   </NotepadPlus>
//
// Sentinel handling — kept faithful so XML load → save → load round-trips:
//   * Missing colour attribute        => `Option::None`.
//   * `fgColor=""`                    => `Option::None`.
//   * `fgColor="FF000000"`            => `Option::None` (NPP "transparent / unset" sentinel).
//   * `fontName=""` / `fontSize=""`   => `None`.
//   * `fontStyle=""`                  => `None`.
//   * Missing `colorStyle`            => `ColourFlags::all()` (NPP default).

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::reader::Reader;

use super::keyword_class::KeywordClass;
use super::lexer::{LexerStyler, LexerStylerArray};
use super::style::{
    parse_hex_rgb, ColourFlags, FontFlags, Style, COLOUR_UNSET_HIGH_BYTE, STYLE_NOT_USED,
};

/// Parsed theme document: lexer-array + the flat list of `<WidgetStyle>` rows.
///
/// Global styles are stored as `Vec<Style>` (with `name` populated from the
/// `name="..."` attribute) so the same `Style` machinery works for both kinds
/// of element.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeDocument {
    pub lexers: LexerStylerArray,
    pub global_styles: Vec<Style>,
}

/// Errors produced by `parse`.
#[derive(Debug)]
pub enum ParseError {
    /// Underlying XML parser failure.
    Xml(quick_xml::Error),
    /// Encountered a UTF-8 issue extracting attribute text.
    Utf8(std::str::Utf8Error),
    /// The document is missing the `<NotepadPlus>` root.
    MissingRoot,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Xml(e) => write!(f, "XML parse error: {e}"),
            Self::Utf8(e) => write!(f, "UTF-8 error: {e}"),
            Self::MissingRoot => f.write_str("missing <NotepadPlus> root element"),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<quick_xml::Error> for ParseError {
    fn from(e: quick_xml::Error) -> Self {
        Self::Xml(e)
    }
}

impl From<std::str::Utf8Error> for ParseError {
    fn from(e: std::str::Utf8Error) -> Self {
        Self::Utf8(e)
    }
}

/// Parse a complete `<NotepadPlus>` theme document.
pub fn parse(xml: &str) -> Result<ThemeDocument, ParseError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut doc = ThemeDocument::default();
    let mut saw_root = false;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => match e.name().as_ref() {
                b"NotepadPlus" => {
                    saw_root = true;
                    parse_root_children(&mut reader, &mut doc)?;
                    break;
                }
                _ => {}
            },
            Event::Empty(_) => {}
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if !saw_root {
        return Err(ParseError::MissingRoot);
    }

    Ok(doc)
}

fn parse_root_children(
    reader: &mut Reader<&[u8]>,
    doc: &mut ThemeDocument,
) -> Result<(), ParseError> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => match e.name().as_ref() {
                b"LexerStyles" => parse_lexer_styles(reader, &mut doc.lexers)?,
                b"GlobalStyles" => parse_global_styles(reader, &mut doc.global_styles)?,
                _ => skip_to_end(reader, e.name())?,
            },
            Event::End(e) if e.name().as_ref() == b"NotepadPlus" => return Ok(()),
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

fn parse_lexer_styles(
    reader: &mut Reader<&[u8]>,
    out: &mut LexerStylerArray,
) -> Result<(), ParseError> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.name().as_ref() == b"LexerType" => {
                let mut lexer = LexerStyler::default();
                for attr in e.attributes().with_checks(false).flatten() {
                    let value = attr.unescape_value()?.to_string();
                    match attr.key.as_ref() {
                        b"name" => lexer.lexer_name = value,
                        b"desc" => lexer.lexer_desc = value,
                        b"ext" => lexer.lexer_user_ext = value,
                        b"userExt" => lexer.lexer_user_ext = value,
                        b"excluded" => lexer.excluded = value.eq_ignore_ascii_case("yes"),
                        _ => {}
                    }
                }
                parse_words_styles(reader, &mut lexer.styles)?;
                out.push(lexer);
            }
            Event::Empty(e) if e.name().as_ref() == b"LexerType" => {
                // Self-closing — odd but not impossible.
                let mut lexer = LexerStyler::default();
                for attr in e.attributes().with_checks(false).flatten() {
                    let value = attr.unescape_value()?.to_string();
                    match attr.key.as_ref() {
                        b"name" => lexer.lexer_name = value,
                        b"desc" => lexer.lexer_desc = value,
                        b"ext" => lexer.lexer_user_ext = value,
                        b"userExt" => lexer.lexer_user_ext = value,
                        b"excluded" => lexer.excluded = value.eq_ignore_ascii_case("yes"),
                        _ => {}
                    }
                }
                out.push(lexer);
            }
            Event::End(e) if e.name().as_ref() == b"LexerStyles" => return Ok(()),
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

fn parse_words_styles(
    reader: &mut Reader<&[u8]>,
    out: &mut Vec<Style>,
) -> Result<(), ParseError> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.name().as_ref() == b"WordsStyle" => {
                let mut style = parse_style_attrs(&e)?;
                // Read text content + closing tag.
                let mut text = String::new();
                loop {
                    let mut inner = Vec::new();
                    match reader.read_event_into(&mut inner)? {
                        Event::Text(t) => text.push_str(&t.unescape()?),
                        Event::CData(t) => text.push_str(std::str::from_utf8(&t)?),
                        Event::End(end) if end.name().as_ref() == b"WordsStyle" => break,
                        Event::Eof => break,
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    style.keywords = Some(text);
                }
                out.push(style);
            }
            Event::Empty(e) if e.name().as_ref() == b"WordsStyle" => {
                out.push(parse_style_attrs(&e)?);
            }
            Event::End(e) if e.name().as_ref() == b"LexerType" => return Ok(()),
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

fn parse_global_styles(
    reader: &mut Reader<&[u8]>,
    out: &mut Vec<Style>,
) -> Result<(), ParseError> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Empty(e) if e.name().as_ref() == b"WidgetStyle" => {
                out.push(parse_style_attrs(&e)?);
            }
            Event::Start(e) if e.name().as_ref() == b"WidgetStyle" => {
                let mut style = parse_style_attrs(&e)?;
                let mut text = String::new();
                loop {
                    let mut inner = Vec::new();
                    match reader.read_event_into(&mut inner)? {
                        Event::Text(t) => text.push_str(&t.unescape()?),
                        Event::CData(t) => text.push_str(std::str::from_utf8(&t)?),
                        Event::End(end) if end.name().as_ref() == b"WidgetStyle" => break,
                        Event::Eof => break,
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    style.keywords = Some(text);
                }
                out.push(style);
            }
            Event::End(e) if e.name().as_ref() == b"GlobalStyles" => return Ok(()),
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

fn parse_style_attrs(e: &quick_xml::events::BytesStart) -> Result<Style, ParseError> {
    let mut style = Style::empty();
    let mut colour_style_explicit: Option<u8> = None;

    for attr in e.attributes().with_checks(false).flatten() {
        let value = attr.unescape_value()?.to_string();
        match attr.key.as_ref() {
            b"name" => style.name = value,
            b"styleID" => {
                if let Ok(n) = value.parse::<i32>() {
                    style.style_id = n;
                }
            }
            b"fgColor" => style.fg_color = parse_colour_attr(&value),
            b"bgColor" => style.bg_color = parse_colour_attr(&value),
            b"colorStyle" => {
                if let Ok(n) = value.parse::<u8>() {
                    colour_style_explicit = Some(n);
                }
            }
            b"fontName" => {
                style.font_name = if value.is_empty() { None } else { Some(value) };
            }
            b"fontStyle" => {
                style.font_style = if value.is_empty() {
                    None
                } else {
                    value
                        .parse::<u8>()
                        .ok()
                        .map(|bits| FontFlags::new(bits & FontFlags::ALL))
                };
            }
            b"fontSize" => {
                style.font_size = if value.is_empty() {
                    None
                } else {
                    value.parse::<u32>().ok()
                };
            }
            b"nesting" => {
                if let Ok(n) = value.parse::<u32>() {
                    style.nesting = n;
                }
            }
            b"keywordClass" => {
                style.keyword_class = KeywordClass::from_token(&value).map(|k| k.to_int());
            }
            _ => {}
        }
    }

    style.colour_style = match colour_style_explicit {
        Some(bits) => ColourFlags::new(bits),
        None => ColourFlags::all(),
    };

    // styleID may not have appeared (very rare for malformed XML); leave as
    // STYLE_NOT_USED. This matches NPP's default-construction behaviour.
    if style.style_id == 0 && style.name.is_empty() {
        // Genuinely empty element — keep as-is, leave style_id=0.
    } else if style.style_id == STYLE_NOT_USED {
        // Was never set — leave the sentinel.
    }

    Ok(style)
}

fn parse_colour_attr(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    // The "FF000000" sentinel: alpha byte 0xFF flags transparent / not set.
    // NPP stores RGB as (raw & 0x00FF_FFFF) but inspects (raw & 0xFF00_0000).
    let raw = u32::from_str_radix(s.trim(), 16).ok()?;
    if (raw & 0xFF00_0000) == COLOUR_UNSET_HIGH_BYTE {
        return None;
    }
    parse_hex_rgb(s)
}

fn skip_to_end(reader: &mut Reader<&[u8]>, name: QName<'_>) -> Result<(), ParseError> {
    // Skip until matching End event for `name`. Used for unknown root children.
    let target = name.as_ref().to_vec();
    let mut depth: i32 = 1;
    let mut buf = Vec::new();
    while depth > 0 {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.name().as_ref() == target => depth += 1,
            Event::End(e) if e.name().as_ref() == target => depth -= 1,
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <LexerStyles>
        <LexerType name="cpp" desc="C++" ext="">
            <WordsStyle name="DEFAULT" styleID="11" fgColor="E0E2E4" bgColor="293134" fontName="" fontStyle="0" fontSize="" />
            <WordsStyle name="COMMENT" styleID="1" fgColor="66747B" bgColor="" fontName="" fontStyle="2" fontSize="" keywordClass="instre1">if else for while</WordsStyle>
            <WordsStyle name="UNSET" styleID="2" fgColor="FF000000" bgColor="FF000000" fontName="" fontStyle="" fontSize="" />
        </LexerType>
    </LexerStyles>
    <GlobalStyles>
        <WidgetStyle name="Default Style" styleID="32" fgColor="E0E2E4" bgColor="293134" fontName="Courier New" fontStyle="0" fontSize="10" />
        <WidgetStyle name="Caret colour" styleID="0" fgColor="A7A7A7" />
        <WidgetStyle name="Edge colour" styleID="0" />
    </GlobalStyles>
</NotepadPlus>
"#;

    #[test]
    fn parse_minimal() {
        let doc = parse(MINIMAL).expect("parse");
        assert_eq!(doc.lexers.len(), 1);
        let cpp = doc.lexers.get(0).unwrap();
        assert_eq!(cpp.lexer_name, "cpp");
        assert_eq!(cpp.lexer_desc, "C++");
        assert_eq!(cpp.styles.len(), 3);

        let default = &cpp.styles[0];
        assert_eq!(default.name, "DEFAULT");
        assert_eq!(default.style_id, 11);
        assert_eq!(default.fg_color, Some(0x00E0_E2E4));
        assert_eq!(default.bg_color, Some(0x0029_3134));
        assert_eq!(default.font_name, None);
        assert_eq!(default.font_style.map(|f| f.bits()), Some(0));
        assert_eq!(default.font_size, None);

        let comment = &cpp.styles[1];
        assert!(comment.font_style.unwrap().italic());
        assert_eq!(comment.keyword_class, Some(0)); // instre1 -> 0
        assert_eq!(
            comment.keywords.as_deref(),
            Some("if else for while")
        );
        assert_eq!(comment.bg_color, None); // empty attr → None

        let unset = &cpp.styles[2];
        assert_eq!(unset.fg_color, None); // FF000000 sentinel
        assert_eq!(unset.bg_color, None);
        assert_eq!(unset.font_style, None);

        assert_eq!(doc.global_styles.len(), 3);
        let caret = doc
            .global_styles
            .iter()
            .find(|s| s.name == "Caret colour")
            .unwrap();
        assert_eq!(caret.fg_color, Some(0x00A7_A7A7));
        assert!(caret.bg_color.is_none()); // attribute omitted entirely
    }

    #[test]
    fn missing_root_errors() {
        let err = parse("<unknown />").unwrap_err();
        assert!(matches!(err, ParseError::MissingRoot));
    }

    #[test]
    fn empty_doc_missing_root() {
        let err = parse("").unwrap_err();
        assert!(matches!(err, ParseError::MissingRoot));
    }

    #[test]
    fn handles_self_closing_lexer_type() {
        let xml = r#"<NotepadPlus><LexerStyles><LexerType name="empty" desc="Empty" /></LexerStyles></NotepadPlus>"#;
        let doc = parse(xml).expect("parse");
        assert_eq!(doc.lexers.len(), 1);
        assert!(doc.lexers.get(0).unwrap().styles.is_empty());
    }

    #[test]
    fn font_style_high_bits_clamped() {
        // We mask to FontFlags::ALL (0x07) — bits beyond underline are dropped.
        let xml = r#"<NotepadPlus><LexerStyles><LexerType name="t" desc="T">
            <WordsStyle name="X" styleID="1" fontStyle="255" />
        </LexerType></LexerStyles></NotepadPlus>"#;
        let doc = parse(xml).expect("parse");
        let s = &doc.lexers.get(0).unwrap().styles[0];
        // 255 & 0x07 = 7
        assert_eq!(s.font_style.unwrap().bits(), 0x07);
    }
}
