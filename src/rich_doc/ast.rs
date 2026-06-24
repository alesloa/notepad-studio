//! Internal rich-document AST. Format-agnostic; populated by per-format parsers.

use std::sync::Arc;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph { runs: Vec<InlineRun>, props: BlockProps },
    Heading { level: u8, runs: Vec<InlineRun>, props: BlockProps },
    Table { rows: Vec<TableRow> },
    Image {
        source: ImageSource,
        width_px: Option<u32>,
        height_px: Option<u32>,
        alt: Option<String>,
    },
}

/// Block-level paragraph properties (alignment, left indent, list membership,
/// …). Lives on `Block::Paragraph` and `Block::Heading` since the formats we
/// round-trip (DOCX `<w:pPr>`, ODT `<style:paragraph-properties>`, RTF
/// `\ql…\liN…\pn`) all model these per paragraph. Defaults to `Alignment::Left`
/// + zero indent + no list so existing constructions stay behavior-equivalent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockProps {
    pub alignment: Alignment,
    /// Left indent in "levels" where each level = 720 twips (= 0.5 inch =
    /// 1.27 cm), matching Word's default Increase Indent step. Capped at
    /// 16 levels (8 inches) by the toolbar; parsers may yield higher
    /// values, which are honored on render but clamped on commit.
    pub indent_level: u8,
    /// Paragraph list membership. `None` = plain paragraph, `Some` = the
    /// paragraph renders inside a bulleted or numbered list at the given
    /// level. Phase 5d.2 only exposes level 0 via the toolbar; parsers
    /// preserve higher levels for round-trip but the renderer flattens
    /// them visually to a single bullet/number prefix per level.
    pub list: Option<ListMarker>,
}

/// Bulleted vs. numbered + nesting level. Level is 0-based: level 0 is the
/// outermost list. Bulleted lists render as `•` for every level (matching
/// Word's default Bullets style which uses the same glyph at every nesting);
/// numbered lists render as `1.`, `2.`, `3.` per consecutive same-level run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListMarker {
    pub ordered: bool,
    pub level: u8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCell {
    pub blocks: Vec<Block>,
}

/// Source of bytes for a `Block::Image`. `Arc` so the AST stays cheap to clone
/// even when image payloads are large (real docs often embed multi-MB PNGs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    /// In-memory bytes + IANA mime type (e.g. `image/png`, `image/jpeg`).
    Inline { bytes: Arc<Vec<u8>>, mime: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineRun {
    pub text: String,
    pub style: InlineStyle,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Strikethrough (horizontal line through the run). Mirrors `underline`'s
    /// model — a per-run boolean toggle. Round-trips through DOCX `<w:strike/>`,
    /// RTF `\strike`, ODT `text:line-through-style`, and HTML import
    /// (`<s>`, `<strike>`, `<del>`, plus CSS `text-decoration: line-through`).
    pub strike: bool,
    /// RGB color packed as 0xRRGGBB. `None` = inherit theme text color.
    pub color: Option<u32>,
    /// Hyperlink target. `None` = not a link.
    pub link: Option<String>,
}

impl InlineRun {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), style: InlineStyle::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_default_is_empty() {
        let d = Document::default();
        assert!(d.blocks.is_empty());
    }

    #[test]
    fn inline_run_plain_has_default_style() {
        let r = InlineRun::plain("hi");
        assert_eq!(r.text, "hi");
        assert!(!r.style.bold);
        assert!(!r.style.italic);
        assert!(!r.style.underline);
    }

    #[test]
    fn table_cell_holds_blocks() {
        let cell = TableCell {
            blocks: vec![Block::Paragraph { runs: vec![InlineRun::plain("hi")], props: BlockProps::default() }],
        };
        assert_eq!(cell.blocks.len(), 1);
    }

    #[test]
    fn image_inline_source_round_trip() {
        let src = ImageSource::Inline {
            bytes: Arc::new(vec![1, 2, 3]),
            mime: "image/png".into(),
        };
        match src {
            ImageSource::Inline { bytes, mime } => {
                assert_eq!(*bytes, vec![1, 2, 3]);
                assert_eq!(mime, "image/png");
            }
        }
    }
}
