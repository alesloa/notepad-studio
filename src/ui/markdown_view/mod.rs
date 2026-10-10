// Markdown preview pane.
//
// Renders the active tab's buffer as styled markdown using `pulldown-cmark`
// for parsing and a thin walk over the offset-bearing event stream for
// layout. v1 covers: headings, paragraphs, bold/italic/strikethrough,
// inline code, links, fenced code blocks (plain monospace, no syntax
// highlighting yet), block quotes, ordered + unordered lists, task list
// checkboxes (read-only), and horizontal rules. Tables, embedded HTML, and
// inline images are deferred — see UNTESTED.md.
//
// Toggled by the toolbar's `code.svg` / `book_open.svg` pair, gated on the
// active tab having a `.md`/`.markdown` extension. State lives on `Tab`
// as `view_mode: TabViewMode`.
//
// Selection: mouse click + drag in headings / paragraphs picks a source
// byte range and stores it on `tab.selection`. The formatting toolbar
// reads that selection through the same helpers as source-mode editing,
// so clicking Bold / Italic / etc. while looking at the preview wraps
// the matching source text and the next render re-paints the highlighted
// rectangle around the new styling. Code blocks, list items, and block
// quotes are not selectable in v1.
//
// Editing (Phase 1+2, WYSIWYG-lite): a click into a heading / paragraph
// also focuses the workspace's main focus handle, and a
// `MarkdownInputBridge` overlay (see `input` submodule) calls
// `window.handle_input(...)` against the workspace so the OS routes
// typed characters / IME / paste through
// `Workspace::EntityInputHandler::replace_text_in_range`. Because that
// handler operates on `Tab.selection` (which is in source bytes), edits
// land at exactly the source byte the click placed the caret on. Arrow
// keys, Backspace, Delete, Cmd+A/C/X are already wired as Workspace
// actions at the root and dispatch as long as the workspace handle is
// focused — they need no preview-specific code. Inline markers
// (**bold**, _italic_, `code`) appear as literal source in the rendered
// text edit path, since Phase 2 doesn't hide them. Phase 2 adds: caret
// painting (via `position_for_index`), visible focus tint on the pane,
// `\n\n` paragraph split on Enter inside preview mode, and a focused
// outline on the active block. See UNTESTED.md for remaining caveats
// (no cross-block backspace merge, non-1:1 source/render runs snap to
// run start). Lists, code blocks, block quotes, alignment containers
// stay render-only.

mod input;
mod scrollbar;

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AnyElement, Bounds, Entity, ExternalPaths, FontWeight, Hsla, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Rgba, ScrollHandle, SharedString,
    StrikethroughStyle, Styled, StyledText, TextRun, UnderlineStyle, div, fill, prelude::*, px,
    relative, rems, rgb, rgba, size,
};
use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

use crate::theme::Theme;
use crate::workspace::Workspace;
use input::MarkdownInputBridge;
use scrollbar::{MD_PREVIEW_SCROLLBAR_THICKNESS, MarkdownPreviewScrollbar};

/// Returns true when the buffer at `path` should offer the markdown preview
/// toggle. Looks at the file extension only — content-sniffing isn't worth
/// the complexity for a binary that already uses the path elsewhere for
/// syntax detection.
pub(crate) fn is_markdown_path(path: Option<&std::path::Path>) -> bool {
    path.and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            e == "md" || e == "markdown" || e == "mkd" || e == "mdown"
        })
        .unwrap_or(false)
}

// --- Block tree ---------------------------------------------------------

/// One contiguous styled run of inline text within a block. `source_range`
/// is the byte range in the original markdown source that produced
/// `text`; for ordinary Text events the two have the same byte length so
/// rendered-offset → source-offset is a 1:1 add. Smart-punctuation events
/// (where `"` becomes a 3-byte `"`) are uncommon enough that the mapping
/// degrades gracefully — clicks anywhere in the synthetic glyph snap to
/// `source_range.start`. SoftBreak / HardBreak runs render a single space
/// or newline; their source range covers the line break in the source.
#[derive(Clone)]
struct InlineRun {
    text: String,
    source_range: Range<usize>,
    bold: bool,
    italic: bool,
    strike: bool,
    /// `<u>` / `<ins>` HTML tags. Painted as a 1px underline in the
    /// body-text colour. Links also paint an underline (in the link
    /// colour) and that takes precedence — see `make_run`.
    underline: bool,
    /// `<mark>` HTML tag. Highlight rectangle behind the text using
    /// `theme.mark_highlight`. Wins over inline `code` chrome.
    mark: bool,
    /// Inline `code` background + monospace. Set both by markdown
    /// `` `inline code` `` and the HTML tags `<code>` / `<kbd>` /
    /// `<samp>` / `<var>`.
    code: bool,
    /// Link destination if this run is part of a clickable link.
    link: Option<String>,
    /// Per-run color override used by fenced-code syntax highlighting.
    /// Set by `split_runs_with_highlights`; `None` for every other code
    /// path so default `make_run` color selection (link/code/text) wins.
    color_override: Option<Hsla>,
}

/// A single inline emphasis marker tracked at parse time so the renderer
/// can selectively reveal it when the caret enters the styled region.
/// Markdown markers (`**`, `*` / `_`, `~~`, `` ` ``) carry visual weight
/// but the WYSIWYG view hides them by default — when the user clicks
/// into the styled text or selects over it, we paint the literal marker
/// characters in `theme.disabled_text` so they can be edited like any
/// other source byte.
#[derive(Clone)]
struct EmphasisTag {
    /// Source byte range of the open marker (e.g. `[10..12]` for `**`).
    open: Range<usize>,
    /// Source byte range of the close marker (e.g. `[17..19]` for `**`).
    close: Range<usize>,
    /// Literal marker text sliced from the source — usually `**`, `*`,
    /// `_`, `~~`, or one or more backticks. Stored so the renderer
    /// doesn't need a second source-slice pass.
    open_text: SharedString,
    close_text: SharedString,
}

impl EmphasisTag {
    /// Inclusive byte range covering both markers and the inner content
    /// — i.e. the entire `**Hello**` region. Used to test whether the
    /// caret / selection is "in" the styled region.
    fn full_range(&self) -> Range<usize> {
        self.open.start..self.close.end
    }
}

#[derive(Clone)]
enum Block {
    Heading {
        level: u8,
        runs: Vec<InlineRun>,
        tags: Vec<EmphasisTag>,
    },
    Paragraph {
        runs: Vec<InlineRun>,
        tags: Vec<EmphasisTag>,
    },
    /// Fenced or indented code block. `lang` is the info-string after the
    /// opening fence (`rust`, `python`, …) — `language_for_info_string`
    /// maps it to a tree-sitter grammar so `render_code_block` can paint
    /// per-kind colors; unknown / missing info-string falls back to flat
    /// `chrome_text`. `runs` carries one `InlineRun` per `Event::Text`
    /// chunk inside the fence so the renderer can route through
    /// `build_selectable_line` like a paragraph — gives
    /// click-to-place-caret + typing edits the source bytes inside the
    /// fence.
    CodeBlock {
        lang: Option<String>,
        runs: Vec<InlineRun>,
    },
    BlockQuote(Vec<Block>),
    List {
        ordered: bool,
        start: u64,
        items: Vec<ListItem>,
    },
    Rule,
    /// GitHub-flavoured table. `alignments` carries one entry per column
    /// (None = unaligned, Left, Center, Right). `header` is the single
    /// header row; `rows` is the body. Each cell holds inline runs +
    /// emphasis tags so the standard selectable-line renderer paints
    /// them with the same selection / marker behaviour as paragraphs.
    Table {
        alignments: Vec<Alignment>,
        header: Vec<TableCell>,
        rows: Vec<Vec<TableCell>>,
    },
    /// Wrapper around a sequence of blocks with a horizontal text
    /// alignment. Produced by recognising `<center>`, `<div align="…">`,
    /// or `<p align="…">` HTML containers. Inner content is parsed as
    /// regular markdown (CommonMark switches back to markdown mode after
    /// a blank line inside a block-HTML region), so headings, paragraphs,
    /// images, etc. all render normally — just centred / right-aligned.
    Aligned {
        align: gpui::TextAlign,
        blocks: Vec<Block>,
    },
    /// Footnote definition `[^label]: …`. Rendered as a small block with a
    /// `[^label]:` prefix and the inner content indented. Pulldown-cmark
    /// emits these in source-order (not gathered at end-of-doc); v1
    /// renders them where they appear rather than reflowing.
    FootnoteDefinition {
        label: SharedString,
        blocks: Vec<Block>,
    },
    /// Inline image rendered as a block-level element. Only paragraphs
    /// whose ONLY content is an image (`![alt](url)` on its own line)
    /// turn into this — mid-paragraph images go to `ParagraphWithImages`.
    /// `url` carries the literal `dest_url` from pulldown-cmark; the
    /// renderer is responsible for path resolution and bytes loading.
    /// `alt` is the user-visible text inside `![…]`. `source_range`
    /// covers the entire `![alt](url)` span in the source buffer so
    /// hit-testing routes clicks back to the markdown source (same as
    /// other runs).
    Image {
        url: String,
        alt: String,
        #[allow(dead_code)]
        source_range: Range<usize>,
    },
    /// Paragraph that mixes text and one-or-more inline images.
    /// Pulldown-cmark emits images as `Start(Tag::Image) … End(TagEnd::Image)`
    /// inside the paragraph's event stream; we split the paragraph
    /// around each image so the text segments render as regular
    /// selectable lines and each image renders via `render_md_image`.
    /// True inline flow (text flowing around the image on the same
    /// baseline) is not attempted — GPUI's `StyledText` is one
    /// monolithic block, so the segments stack vertically inside a
    /// single `flex_col`. That's still a strict win over the previous
    /// behaviour, where the alt text leaked into the paragraph body
    /// and the image dropped silently.
    ParagraphWithImages {
        segments: Vec<ParagraphSegment>,
    },
}

#[derive(Clone)]
enum ParagraphSegment {
    Text {
        runs: Vec<InlineRun>,
        tags: Vec<EmphasisTag>,
    },
    Image {
        url: String,
        alt: String,
        #[allow(dead_code)]
        source_range: Range<usize>,
    },
}

#[derive(Clone)]
struct ListItem {
    /// `Some(true)` = `[x]`, `Some(false)` = `[ ]`, `None` = plain item.
    checkbox: Option<bool>,
    blocks: Vec<Block>,
}

#[derive(Clone)]
struct TableCell {
    runs: Vec<InlineRun>,
    tags: Vec<EmphasisTag>,
}

// --- HTML allowlist -----------------------------------------------------

/// Tiny HTML tag descriptor, populated by `parse_html_tag`. Holds just
/// what we need from the GitHub-flavoured allowlist: name, whether it's
/// a closing tag, and any attributes (only `align` and `href` are read,
/// but the rest are kept for future use).
struct ParsedTag {
    name: String,
    closing: bool,
    attrs: Vec<(String, String)>,
}

/// Look up an attribute case-insensitively. Returns the first match
/// (HTML's "duplicate attribute" rule says the first one wins).
fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

/// Parse a single HTML tag. Accepts `<name>`, `</name>`, `<name/>`,
/// `<name attr=val attr2="val2">`, `<name attr='val'/>`. Returns `None`
/// for input that isn't a complete `<…>` token. Tag and attribute names
/// are lower-cased; attribute values keep their original casing because
/// `href` URLs are case-sensitive.
fn parse_html_tag(raw: &str) -> Option<ParsedTag> {
    let s = raw.trim();
    if !s.starts_with('<') || !s.ends_with('>') {
        return None;
    }
    let inner = s[1..s.len() - 1].trim();
    let (closing, body) = if let Some(rest) = inner.strip_prefix('/') {
        (true, rest.trim_start())
    } else {
        (false, inner)
    };
    let body = body.trim_end();
    let body = body.strip_suffix('/').unwrap_or(body).trim_end();
    let mut split = body.splitn(2, char::is_whitespace);
    let name = split.next()?.to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    let attrs = split.next().map(parse_html_attrs).unwrap_or_default();
    Some(ParsedTag { name, closing, attrs })
}

/// Parse only the first tag in `s` (which may be a multi-line block-HTML
/// chunk like `"<div align=\"center\">\n\n"`). Used at block level where
/// pulldown-cmark hands us `Event::Html` chunks that include the opening
/// tag plus trailing newlines / whitespace.
fn parse_html_tag_prefix(s: &str) -> Option<ParsedTag> {
    let s = s.trim_start();
    let end = s.find('>')?;
    parse_html_tag(&s[..=end])
}

fn parse_html_attrs(s: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            chars.next();
        }
        let Some(&c) = chars.peek() else { break };
        if c == '/' || c == '>' {
            break;
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c.is_ascii_whitespace() || c == '/' || c == '>' {
                break;
            }
            name.push(c.to_ascii_lowercase());
            chars.next();
        }
        if name.is_empty() {
            chars.next();
            continue;
        }
        while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            chars.next();
        }
        let value = if chars.peek() == Some(&'=') {
            chars.next();
            while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                chars.next();
            }
            let quote = match chars.peek() {
                Some(&'"') | Some(&'\'') => chars.next(),
                _ => None,
            };
            let mut val = String::new();
            if let Some(q) = quote {
                while let Some(c) = chars.next() {
                    if c == q {
                        break;
                    }
                    val.push(c);
                }
            } else {
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_whitespace() || c == '>' {
                        break;
                    }
                    val.push(c);
                    chars.next();
                }
            }
            val
        } else {
            String::new()
        };
        out.push((name, value));
    }
    out
}

/// Map an opening HTML tag to a text alignment if it's one of the
/// containers we render: `<center>`, `<div align="…">`, `<p align="…">`.
/// Closing tags and tags without a recognised alignment return `None`.
fn container_alignment(tag: &ParsedTag) -> Option<gpui::TextAlign> {
    if tag.closing {
        return None;
    }
    match tag.name.as_str() {
        "center" => Some(gpui::TextAlign::Center),
        "div" | "p" => attr(&tag.attrs, "align").and_then(|v| {
            match v.to_ascii_lowercase().as_str() {
                "center" => Some(gpui::TextAlign::Center),
                "right" => Some(gpui::TextAlign::Right),
                "left" => Some(gpui::TextAlign::Left),
                _ => None,
            }
        }),
        _ => None,
    }
}

// --- Parser -------------------------------------------------------------

/// `(Event, source_range)` pair as produced by `Parser::into_offset_iter`.
type EventWithRange<'a> = (Event<'a>, Range<usize>);

fn parser_options() -> Options {
    let mut o = Options::empty();
    o.insert(Options::ENABLE_TABLES);
    o.insert(Options::ENABLE_FOOTNOTES);
    o.insert(Options::ENABLE_STRIKETHROUGH);
    o.insert(Options::ENABLE_TASKLISTS);
    o.insert(Options::ENABLE_SMART_PUNCTUATION);
    o.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    o.insert(Options::ENABLE_GFM);
    o
}

/// Deepest container nesting (block quotes, lists, footnote definitions,
/// alignment wrappers) the preview builds as nested blocks. The parser,
/// `render_block`, and gpui's layout / prepaint / paint passes all recurse
/// once per level, every level costs up to three divs, and gpui runs those
/// passes on the main thread, whose stack is 1 MB on Windows. 10,000
/// nested `> ` markers overflowed a 2 MB test thread in the parser alone.
/// Content nested deeper than this
/// still renders, flattened into the innermost container that fits.
/// Hand-written markdown nests a handful of levels, so 16 leaves room.
const MAX_BLOCK_NESTING: usize = 16;

fn parse(source: &str) -> Vec<Block> {
    let parser = Parser::new_ext(source, parser_options());
    let events: Vec<EventWithRange> = parser.into_offset_iter().collect();
    let mut idx = 0;
    let mut out = Vec::new();
    while idx < events.len() {
        if let Some(block) = read_block(source, &events, &mut idx, 0) {
            out.push(block);
        } else {
            idx += 1;
        }
    }
    out
}

/// Pulls one block starting at `*idx`. `depth` counts the containers
/// already open around it (0 at the top level). Returns `None` on events
/// that don't start a block (stray ends, table content we don't render
/// yet). Advances `*idx` past whatever it consumed.
fn read_block(
    source: &str,
    events: &[EventWithRange],
    idx: &mut usize,
    depth: usize,
) -> Option<Block> {
    while *idx < events.len() {
        match &events[*idx].0 {
            Event::Start(tag) => {
                let tag = tag.clone();
                *idx += 1;
                return Some(match tag {
                    Tag::Heading { level, .. } => {
                        let (runs, tags) = read_inline_runs_until(
                            source,
                            events,
                            idx,
                            |t| matches!(t, TagEnd::Heading(_)),
                            InlineStyle::default(),
                        );
                        Block::Heading {
                            level: heading_level(level),
                            runs,
                            tags,
                        }
                    }
                    Tag::Paragraph => {
                        // Standalone-image paragraph: pulldown wraps a
                        // line that contains ONLY `![alt](url)` in a
                        // Tag::Paragraph whose inner events are exactly
                        // Start(Image) … End(Image). Promote those to a
                        // top-level Block::Image so the renderer can
                        // load + display the bytes.
                        if let Some(img) = peek_standalone_image(events, *idx) {
                            // Advance past the image events and the
                            // closing TagEnd::Paragraph.
                            while *idx < events.len() {
                                let consumed = matches!(
                                    &events[*idx].0,
                                    Event::End(TagEnd::Paragraph)
                                );
                                *idx += 1;
                                if consumed {
                                    break;
                                }
                            }
                            return Some(img);
                        }
                        // Mixed text + image paragraph: split around
                        // each image into ParagraphSegments so both
                        // surrounding text and the image render.
                        if paragraph_contains_inline_image(events, *idx) {
                            let segments = read_paragraph_segments(source, events, idx);
                            return Some(Block::ParagraphWithImages { segments });
                        }
                        let (runs, tags) = read_inline_runs_until(
                            source,
                            events,
                            idx,
                            |t| matches!(t, TagEnd::Paragraph),
                            InlineStyle::default(),
                        );
                        Block::Paragraph { runs, tags }
                    }
                    Tag::CodeBlock(kind) => {
                        let lang = match kind {
                            CodeBlockKind::Fenced(s) if !s.is_empty() => Some(s.to_string()),
                            _ => None,
                        };
                        let runs = read_code_block_runs(events, idx);
                        Block::CodeBlock { lang, runs }
                    }
                    Tag::BlockQuote(_) => {
                        let inner = read_blocks_until(
                            source,
                            events,
                            idx,
                            |t| matches!(t, TagEnd::BlockQuote(_)),
                            depth + 1,
                        );
                        Block::BlockQuote(inner)
                    }
                    Tag::List(start) => {
                        let ordered = start.is_some();
                        let start = start.unwrap_or(1);
                        let items = read_list_items(source, events, idx, depth + 1);
                        Block::List {
                            ordered,
                            start,
                            items,
                        }
                    }
                    Tag::Table(alignments) => read_table(source, events, idx, alignments),
                    Tag::FootnoteDefinition(label) => {
                        let blocks = read_blocks_until(
                            source,
                            events,
                            idx,
                            |t| matches!(t, TagEnd::FootnoteDefinition),
                            depth + 1,
                        );
                        Block::FootnoteDefinition {
                            label: label.to_string().into(),
                            blocks,
                        }
                    }
                    _ => {
                        // Unknown / unsupported block start — skip until its
                        // matching end so we don't leak structure.
                        skip_until_end(events, idx);
                        continue;
                    }
                });
            }
            Event::Rule => {
                *idx += 1;
                return Some(Block::Rule);
            }
            Event::Html(s) => {
                // Block-level HTML. Recognise the alignment containers
                // (`<center>`, `<div align>`, `<p align>`) — pulldown-cmark
                // emits the opening tag as one Event::Html, the inner
                // markdown blocks normally, then the closing tag as
                // another Event::Html. Walk subsequent events until the
                // matching close, wrap the captured blocks in
                // `Block::Aligned`.
                //
                // Two safety guards: (a) require the close tag to appear
                // in a LATER event, not the current chunk — that filters
                // out single-line `<center>foo</center>` (which would
                // otherwise consume the rest of the document looking for
                // a close that's actually in the same event); and (b)
                // require the close tag to exist somewhere ahead at all
                // — without this, an unclosed `<center>` would centre
                // every block that follows.
                // Past `MAX_BLOCK_NESTING` the wrapper is dropped and its
                // blocks are read as siblings, which keeps the recursion
                // bounded.
                if let Some(tag) = parse_html_tag_prefix(s).filter(|_| depth + 1 < MAX_BLOCK_NESTING) {
                    if let Some(align) = container_alignment(&tag) {
                        let close_marker = format!("</{}", tag.name).to_ascii_lowercase();
                        let close_found = events.iter().skip(*idx + 1).any(|(e, _)| {
                            if let Event::Html(s2) = e {
                                s2.to_ascii_lowercase().contains(&close_marker)
                            } else {
                                false
                            }
                        });
                        if close_found {
                            let name = tag.name.clone();
                            *idx += 1;
                            let mut inner = Vec::new();
                            // No depth tracking: nested alignment
                            // containers are recursively consumed by
                            // `read_block` (which handles its own close
                            // tag inside the recursion), so they never
                            // surface here. Edge case not handled:
                            // nested same-named NON-alignment containers
                            // (e.g. `<div align>` outer, `<div class>`
                            // inner) — the inner close would terminate
                            // us early. Documented in UNTESTED.md.
                            while *idx < events.len() {
                                if let Event::Html(s2) = &events[*idx].0 {
                                    if let Some(t2) = parse_html_tag_prefix(s2) {
                                        if t2.name == name && t2.closing {
                                            *idx += 1;
                                            break;
                                        }
                                    }
                                }
                                if let Some(b) = read_block(source, events, idx, depth + 1) {
                                    inner.push(b);
                                } else {
                                    *idx += 1;
                                }
                            }
                            return Some(Block::Aligned { align, blocks: inner });
                        }
                    }
                }
                // Other block-level HTML (or single-line / unclosed
                // alignment containers) — drop silently.
                *idx += 1;
            }
            Event::InlineHtml(_) => {
                // Stray inline HTML between blocks — drop.
                *idx += 1;
            }
            Event::End(_) => {
                // Stray end (shouldn't happen at the top level, but be
                // defensive). Skip it.
                *idx += 1;
            }
            _ => {
                *idx += 1;
            }
        }
    }
    None
}

/// Peek inside an open Tag::Paragraph (positioned at `start_idx`,
/// pointing at the FIRST event *after* the Tag::Paragraph open). If the
/// paragraph's content is exactly one image and nothing else (allowing
/// whitespace-only text noise around it), return the corresponding
/// `Block::Image`. Otherwise return `None` so the caller falls back to
/// regular paragraph parsing.
///
/// "Nothing else" means: only Event::Start(Tag::Image), Event::Text
/// chunks that are entirely whitespace (markdown sometimes emits a stray
/// "" or "\n" inside the wrapping paragraph), and Event::End(TagEnd::Image)
/// before the matching TagEnd::Paragraph. Alt text accumulates from the
/// Text events that appear BETWEEN Start(Image) and End(Image).
fn peek_standalone_image(events: &[EventWithRange], start_idx: usize) -> Option<Block> {
    let mut i = start_idx;
    let mut image: Option<(String, Range<usize>)> = None;
    let mut alt = String::new();
    let mut inside_image = false;
    while i < events.len() {
        match &events[i].0 {
            Event::End(TagEnd::Paragraph) => break,
            Event::Start(Tag::Image { dest_url, .. }) if !inside_image && image.is_none() => {
                image = Some((dest_url.to_string(), events[i].1.clone()));
                inside_image = true;
            }
            Event::End(TagEnd::Image) if inside_image => {
                inside_image = false;
            }
            Event::Text(s) if inside_image => {
                alt.push_str(s);
            }
            Event::Text(s) if !inside_image => {
                // Whitespace between paragraph open and image is fine —
                // it's how pulldown emits trailing newlines. Anything
                // non-whitespace means the paragraph mixes text and
                // image; bail.
                if !s.chars().all(char::is_whitespace) {
                    return None;
                }
            }
            Event::SoftBreak | Event::HardBreak => {}
            _ => return None,
        }
        i += 1;
    }
    let (url, source_range) = image?;
    Some(Block::Image { url, alt, source_range })
}

/// True when the paragraph starting at `start_idx` (positioned right
/// after `Tag::Paragraph`) contains at least one `Start(Tag::Image)`
/// event before the matching `TagEnd::Paragraph`. Used by `read_block`
/// to decide whether the paragraph needs to be split into
/// `ParagraphSegment`s. Standalone-image paragraphs (the only-an-image
/// case) are already handled upstream by `peek_standalone_image`, so
/// reaching this fn implies the paragraph has SOMETHING other than
/// pure image content.
fn paragraph_contains_inline_image(
    events: &[EventWithRange],
    start_idx: usize,
) -> bool {
    let mut depth: usize = 0;
    for i in start_idx..events.len() {
        match &events[i].0 {
            Event::Start(Tag::Image { .. }) if depth == 0 => return true,
            Event::Start(_) => depth += 1,
            Event::End(TagEnd::Paragraph) if depth == 0 => return false,
            Event::End(_) => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    false
}

/// Walk a paragraph that mixes text and inline images, emitting a
/// sequence of `ParagraphSegment`s — alternating `Text` and `Image`.
/// Positioned just after `Tag::Paragraph` is consumed (matching the
/// other readers' contract); advances `*idx` past the matching
/// `TagEnd::Paragraph`.
///
/// Strategy: scan forward for the next inline-image boundary or the
/// paragraph close, slice the event window up to that boundary, run
/// the existing `read_inline_runs_until` against the slice to produce
/// a `Text` segment, then (if the boundary was an image) consume the
/// image events to produce an `Image` segment, and repeat. Empty
/// text segments (consecutive images with nothing between, or a
/// paragraph starting with an image) are dropped so the renderer
/// doesn't paint zero-height divs.
///
/// Depth tracking matches `paragraph_contains_inline_image`: only
/// images at the paragraph's top inline depth (depth == 0 of inline
/// Start/End events) split the paragraph. An image inside, say, a
/// link wrapper would still split, but link-wrapped images are
/// uncommon and the segment renderer doesn't carry the link click
/// behaviour for images regardless.
fn read_paragraph_segments(
    source: &str,
    events: &[EventWithRange],
    idx: &mut usize,
) -> Vec<ParagraphSegment> {
    let mut segments: Vec<ParagraphSegment> = Vec::new();

    loop {
        // Locate the next image start or the paragraph close, at
        // inline depth 0 relative to the current `*idx`. The text
        // segment is `[*idx, boundary)`.
        let mut depth: usize = 0;
        let mut boundary = events.len();
        let mut boundary_is_image = false;
        for i in *idx..events.len() {
            match &events[i].0 {
                Event::Start(Tag::Image { .. }) if depth == 0 => {
                    boundary = i;
                    boundary_is_image = true;
                    break;
                }
                Event::Start(_) => depth += 1,
                Event::End(TagEnd::Paragraph) if depth == 0 => {
                    boundary = i;
                    break;
                }
                Event::End(_) => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }

        // Read the text segment by giving `read_inline_runs_until`
        // a slice that ends at the boundary. Its loop terminates by
        // running off the end of the slice (no End(Paragraph) to
        // match against in the slice when the boundary is an image);
        // when the boundary IS the paragraph end, the End event sits
        // at `boundary` so we let the matching stop predicate fire.
        let sliced_end = if boundary_is_image {
            boundary
        } else {
            // Include the End(Paragraph) so the stop predicate fires.
            boundary + 1
        };
        let sliced = &events[..sliced_end.min(events.len())];
        let mut local_idx = *idx;
        let (runs, tags) = read_inline_runs_until(
            source,
            sliced,
            &mut local_idx,
            |t| matches!(t, TagEnd::Paragraph),
            InlineStyle::default(),
        );
        *idx = local_idx;
        if !runs.is_empty() {
            segments.push(ParagraphSegment::Text { runs, tags });
        }

        if !boundary_is_image {
            // We just consumed up through (and past) End(Paragraph)
            // when the boundary was the paragraph close. Done.
            return segments;
        }

        // Consume the image: Start(Image), interleaved Text events
        // (alt text), End(Image). `*idx` is positioned at the Image
        // Start because read_inline_runs_until didn't advance past it
        // (no End to match against in the slice that ended there).
        let mut url = String::new();
        let mut alt = String::new();
        let Some((image_event, image_range)) = events.get(*idx) else {
            return segments;
        };
        let image_range = image_range.clone();
        if let Event::Start(Tag::Image { dest_url, .. }) = image_event {
            url = dest_url.to_string();
        }
        *idx += 1;
        while *idx < events.len() {
            match &events[*idx].0 {
                Event::End(TagEnd::Image) => {
                    *idx += 1;
                    break;
                }
                Event::Text(s) => {
                    alt.push_str(s);
                    *idx += 1;
                }
                // Pulldown can emit nested inline tags inside the
                // alt-text region (e.g. `![**bold** alt](url)`).
                // Skip the structure but accumulate any inner text
                // into `alt` so it remains a single string.
                _ => {
                    *idx += 1;
                }
            }
        }
        segments.push(ParagraphSegment::Image {
            url,
            alt,
            source_range: image_range,
        });
    }
}

/// Reads the blocks of an open block quote or footnote definition.
/// `depth` is the nesting level of those blocks; at `MAX_BLOCK_NESTING`
/// they are read flat.
fn read_blocks_until<F: Fn(&TagEnd) -> bool>(
    source: &str,
    events: &[EventWithRange],
    idx: &mut usize,
    stop: F,
    depth: usize,
) -> Vec<Block> {
    if depth >= MAX_BLOCK_NESTING {
        return read_flattened(source, events, idx);
    }
    let mut out = Vec::new();
    while *idx < events.len() {
        if let Event::End(end) = &events[*idx].0 {
            if stop(end) {
                *idx += 1;
                return out;
            }
        }
        if let Some(block) = read_block(source, events, idx, depth) {
            out.push(block);
        }
    }
    out
}

/// True for the end of a container `read_flattened` walks through.
fn is_container_end(end: &TagEnd) -> bool {
    matches!(
        end,
        TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item | TagEnd::FootnoteDefinition
    )
}

/// Reads the rest of an open container (block quote, list item, footnote
/// definition) whose contents sit at `MAX_BLOCK_NESTING`. Containers
/// inside it are not built; their leaf blocks are appended in source
/// order, so the text still shows and nothing recurses. Leaves `*idx`
/// past the End event that closes the open container.
fn read_flattened(source: &str, events: &[EventWithRange], idx: &mut usize) -> Vec<Block> {
    let mut out = Vec::new();
    // Containers opened inside the flattened region and not yet closed.
    let mut open = 0usize;
    while let Some((event, _)) = events.get(*idx) {
        match event {
            Event::Start(
                Tag::BlockQuote(_) | Tag::List(_) | Tag::Item | Tag::FootnoteDefinition(_),
            ) => {
                open += 1;
                *idx += 1;
            }
            Event::End(end) if is_container_end(end) => {
                *idx += 1;
                if open == 0 {
                    return out;
                }
                open -= 1;
            }
            Event::Start(
                Tag::Paragraph | Tag::Heading { .. } | Tag::CodeBlock(_) | Tag::Table(_),
            ) => {
                // Leaf blocks: `read_block` reads these without recursing.
                if let Some(block) = read_block(source, events, idx, MAX_BLOCK_NESTING) {
                    out.push(block);
                }
            }
            Event::Start(tag) if is_block_start_tag(tag) => {
                // HTML blocks, metadata, stray table parts: skipped, as
                // `read_block` does.
                *idx += 1;
                skip_until_end(events, idx);
            }
            Event::Rule => {
                *idx += 1;
                out.push(Block::Rule);
            }
            Event::TaskListMarker(_) | Event::End(_) => {
                *idx += 1;
            }
            _ => {
                // Inline content of a tight list item.
                let before = *idx;
                let (runs, tags) =
                    read_implicit_paragraph_until(source, events, idx, is_container_end);
                if !runs.is_empty() {
                    out.push(Block::Paragraph { runs, tags });
                }
                if *idx == before {
                    *idx += 1;
                }
            }
        }
    }
    out
}

fn read_list_items(
    source: &str,
    events: &[EventWithRange],
    idx: &mut usize,
    depth: usize,
) -> Vec<ListItem> {
    let mut items = Vec::new();
    while *idx < events.len() {
        match &events[*idx].0 {
            Event::End(TagEnd::List(_)) => {
                *idx += 1;
                return items;
            }
            Event::Start(Tag::Item) => {
                *idx += 1;
                let mut checkbox = None;
                if let Some((Event::TaskListMarker(checked), _)) = events.get(*idx) {
                    checkbox = Some(*checked);
                    *idx += 1;
                }
                let blocks = read_item_blocks(source, events, idx, depth);
                items.push(ListItem { checkbox, blocks });
            }
            _ => {
                *idx += 1;
            }
        }
    }
    items
}

/// Walk events between `Start(Item)` and the matching `End(Item)`,
/// handling both "loose" items (where pulldown-cmark wraps inline
/// content in a `Start(Paragraph)` / `End(Paragraph)` pair) and "tight"
/// items (where inline events fire directly inside the item). For tight
/// items we collect contiguous inline events into an implicit Paragraph
/// block; block-level events (nested lists, code blocks) still go
/// through `read_block`. `depth` is the nesting level of the item's
/// blocks; at `MAX_BLOCK_NESTING` they are read flat.
fn read_item_blocks(
    source: &str,
    events: &[EventWithRange],
    idx: &mut usize,
    depth: usize,
) -> Vec<Block> {
    if depth >= MAX_BLOCK_NESTING {
        return read_flattened(source, events, idx);
    }
    let mut out = Vec::new();
    while *idx < events.len() {
        let (event, _) = &events[*idx];
        match event {
            Event::End(TagEnd::Item) => {
                *idx += 1;
                return out;
            }
            Event::Rule => {
                *idx += 1;
                out.push(Block::Rule);
            }
            Event::Start(tag) if is_block_start_tag(tag) => {
                if let Some(block) = read_block(source, events, idx, depth) {
                    out.push(block);
                }
            }
            Event::End(_) => {
                // Stray end at top level of an item — defensive skip.
                *idx += 1;
            }
            _ => {
                // Inline event (Text / Code / SoftBreak / HardBreak /
                // Html / InlineHtml / Start(Link/Emphasis/...)) — read
                // a paragraph's worth of inline content, stopping
                // either at End(Item) or when the next block-level
                // Start appears.
                let (runs, tags) = read_implicit_paragraph_until(
                    source,
                    events,
                    idx,
                    |t| matches!(t, TagEnd::Item),
                );
                if !runs.is_empty() {
                    out.push(Block::Paragraph { runs, tags });
                }
            }
        }
    }
    out
}

/// True for any `Tag` that pulldown-cmark emits at the block level —
/// these break out of an implicit paragraph collection so they can be
/// rendered as their own `Block`.
fn is_block_start_tag(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::Item
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::MetadataBlock(_)
    )
}

/// Read inline events into a paragraph's worth of runs + emphasis tags,
/// stopping non-destructively at either a top-level (depth-0) block
/// Start tag or a top-level End matching `stop`. The boundary event is
/// LEFT IN PLACE so the caller (typically `read_item_blocks`) can
/// dispatch it. Used to materialise implicit paragraphs inside tight
/// list items where pulldown-cmark omits the wrapping `Tag::Paragraph`.
fn read_implicit_paragraph_until<F: Fn(&TagEnd) -> bool>(
    source: &str,
    events: &[EventWithRange],
    idx: &mut usize,
    stop: F,
) -> (Vec<InlineRun>, Vec<EmphasisTag>) {
    // Walk forward tracking inline-tag depth; stop at the first event
    // where depth==0 and either a block Start or a matching End fires.
    let start_idx = *idx;
    let mut depth = 0usize;
    let mut boundary = events.len();
    for i in start_idx..events.len() {
        match &events[i].0 {
            Event::Start(tag) => {
                if depth == 0 && is_block_start_tag(tag) {
                    boundary = i;
                    break;
                }
                depth += 1;
            }
            Event::End(end) => {
                if depth == 0 && stop(end) {
                    boundary = i;
                    break;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    // Run the inline reader against a slice that ENDS at `boundary` so
    // it naturally exhausts without consuming the boundary event. The
    // caller's outer loop then picks up the boundary event on its next
    // iteration.
    let sliced = &events[..boundary.min(events.len())];
    let mut local_idx = *idx;
    let (runs, tags) = read_inline_runs_until(
        source,
        sliced,
        &mut local_idx,
        |_| false,
        InlineStyle::default(),
    );
    *idx = local_idx;
    (runs, tags)
}

/// Parse a GFM table starting just after the consumed `Tag::Table`.
/// Pulldown-cmark emits the table as: Start(TableHead), TableCell+,
/// End(TableHead), then Start(TableRow), TableCell+, End(TableRow) for
/// each body row, then End(Table).
fn read_table(
    source: &str,
    events: &[EventWithRange],
    idx: &mut usize,
    alignments: Vec<Alignment>,
) -> Block {
    let mut header: Vec<TableCell> = Vec::new();
    let mut rows: Vec<Vec<TableCell>> = Vec::new();
    while *idx < events.len() {
        match &events[*idx].0 {
            Event::End(TagEnd::Table) => {
                *idx += 1;
                return Block::Table {
                    alignments,
                    header,
                    rows,
                };
            }
            Event::Start(Tag::TableHead) => {
                *idx += 1;
                header = read_table_row(source, events, idx, |t| {
                    matches!(t, TagEnd::TableHead)
                });
            }
            Event::Start(Tag::TableRow) => {
                *idx += 1;
                let row = read_table_row(source, events, idx, |t| {
                    matches!(t, TagEnd::TableRow)
                });
                rows.push(row);
            }
            _ => {
                *idx += 1;
            }
        }
    }
    Block::Table {
        alignments,
        header,
        rows,
    }
}

fn read_table_row<F: Fn(&TagEnd) -> bool>(
    source: &str,
    events: &[EventWithRange],
    idx: &mut usize,
    stop: F,
) -> Vec<TableCell> {
    let mut cells = Vec::new();
    while *idx < events.len() {
        match &events[*idx].0 {
            Event::End(end) if stop(end) => {
                *idx += 1;
                return cells;
            }
            Event::Start(Tag::TableCell) => {
                *idx += 1;
                let (runs, tags) = read_inline_runs_until(
                    source,
                    events,
                    idx,
                    |t| matches!(t, TagEnd::TableCell),
                    InlineStyle::default(),
                );
                cells.push(TableCell { runs, tags });
            }
            _ => {
                *idx += 1;
            }
        }
    }
    cells
}

/// Collect one `InlineRun` per `Event::Text` chunk between the consumed
/// `Tag::CodeBlock` and its matching `End(CodeBlock)`. Each run carries
/// the event's source byte range straight from `into_offset_iter` so
/// click hit-testing maps rendered bytes back to source bytes inside
/// the fence — exactly the same path paragraphs use. `code: false`
/// because the outer chrome wrapper (see `render_code_block`) already
/// paints the per-block background; per-run code chrome would stack on
/// top of it.
fn read_code_block_runs(
    events: &[EventWithRange],
    idx: &mut usize,
) -> Vec<InlineRun> {
    let mut runs = Vec::new();
    while *idx < events.len() {
        let (event, range) = &events[*idx];
        match event {
            Event::End(TagEnd::CodeBlock) => {
                *idx += 1;
                return runs;
            }
            Event::Text(s) => {
                runs.push(InlineRun {
                    text: s.to_string(),
                    source_range: range.clone(),
                    bold: false,
                    italic: false,
                    strike: false,
                    underline: false,
                    mark: false,
                    code: false,
                    link: None,
                    color_override: None,
                });
                *idx += 1;
            }
            _ => {
                *idx += 1;
            }
        }
    }
    runs
}

#[derive(Clone, Copy, Default)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    mark: bool,
    /// Set inside `<code>` / `<kbd>` / `<samp>` / `<var>` HTML tag scopes
    /// so any text emitted there inherits monospace + chrome bg without
    /// the per-event `Event::Code` path. Markdown's own `Event::Code`
    /// still sets `InlineRun.code = true` directly, so both paths
    /// converge on the same rendered styling.
    code: bool,
}

/// Tracks one in-flight emphasis tag while walking the inline event
/// stream. Pulldown-cmark hands us the tag's full source range on the
/// `Start` event but doesn't tell us where the marker characters end —
/// we infer that from the first / last inline event nested inside the
/// tag (whose source ranges cover only the inner content).
#[derive(Clone)]
struct PendingTag {
    /// Full source range of the emphasis region (e.g. `[10..19]` for
    /// `**Hello**` — covers both markers and the inner text).
    full_range: Range<usize>,
    /// `range.start` of the first inline event we saw inside this tag,
    /// once one fires. Used to compute the open marker as
    /// `full_range.start..first_inner_start`.
    first_inner_start: Option<usize>,
    /// `range.end` of the most recent inline event we saw inside this
    /// tag. Updated as events come in; the final value gives us the
    /// close marker as `last_inner_end..full_range.end`.
    last_inner_end: Option<usize>,
}

/// Walk inline events into a flat list of styled runs and a list of
/// emphasis-marker source ranges, respecting nested emphasis/strong/
/// strike + links. `stop` decides which end-tag terminates the read.
/// Each emitted `InlineRun` carries the source byte range from
/// `into_offset_iter` so mouse hit-testing can resolve back to the
/// source buffer; each `EmphasisTag` records the marker chars so the
/// renderer can reveal them when the caret enters the styled region.
fn read_inline_runs_until<F: Fn(&TagEnd) -> bool>(
    source: &str,
    events: &[EventWithRange],
    idx: &mut usize,
    stop: F,
    base: InlineStyle,
) -> (Vec<InlineRun>, Vec<EmphasisTag>) {
    let mut out = Vec::new();
    let mut tags: Vec<EmphasisTag> = Vec::new();
    let mut style_stack: Vec<InlineStyle> = vec![base];
    let mut link_stack: Vec<String> = Vec::new();
    let mut tag_stack: Vec<PendingTag> = Vec::new();

    let emit_text = |text: String,
                     range: Range<usize>,
                     style_stack: &Vec<InlineStyle>,
                     link_stack: &Vec<String>,
                     tag_stack: &mut Vec<PendingTag>,
                     out: &mut Vec<InlineRun>,
                     force_code: bool| {
        let cur = style_stack.last().copied().unwrap_or(base);
        // Update inner-extent tracking for the open emphasis tags so the
        // post-End marker computation knows where the inner content
        // ended. Cover the full original range, regardless of whether
        // autolinking splits the text into multiple runs. Tags still
        // waiting for their first content sit at the top of the stack,
        // and only the innermost tag records the end: the End handler
        // passes it to the parent. Touching every open tag here made
        // deep emphasis over many lines quadratic.
        for pending in tag_stack.iter_mut().rev() {
            if pending.first_inner_start.is_some() {
                break;
            }
            pending.first_inner_start = Some(range.start);
        }
        if let Some(innermost) = tag_stack.last_mut() {
            innermost.last_inner_end = Some(range.end);
        }
        let current_link = link_stack.last().cloned();
        let in_code = cur.code || force_code;
        // Bare-URL autolinking (GFM): only scan when no explicit link is
        // active and this isn't inline code — pulldown-cmark 0.13 doesn't
        // emit `Tag::Link` for bare URLs, so we synthesise the link here.
        // Splitting the source range assumes pulldown-cmark gave us text
        // bytes at consecutive source offsets (true for non-entity text).
        let autolinks = if current_link.is_none() && !in_code {
            scan_autolinks(&text)
        } else {
            Vec::new()
        };
        if autolinks.is_empty() {
            out.push(InlineRun {
                text,
                source_range: range,
                bold: cur.bold,
                italic: cur.italic,
                strike: cur.strike,
                underline: cur.underline,
                mark: cur.mark,
                code: in_code,
                link: current_link,
                color_override: None,
            });
            return;
        }
        let push_run = |out: &mut Vec<InlineRun>,
                        text_slice: &str,
                        sub_range: Range<usize>,
                        link: Option<String>| {
            if text_slice.is_empty() {
                return;
            }
            out.push(InlineRun {
                text: text_slice.to_string(),
                source_range: sub_range,
                bold: cur.bold,
                italic: cur.italic,
                strike: cur.strike,
                underline: cur.underline,
                mark: cur.mark,
                code: in_code,
                link,
                color_override: None,
            });
        };
        let base = range.start;
        let mut prev = 0usize;
        for (start, end, url) in autolinks {
            if start > prev {
                push_run(
                    out,
                    &text[prev..start],
                    (base + prev)..(base + start),
                    None,
                );
            }
            push_run(
                out,
                &text[start..end],
                (base + start)..(base + end),
                Some(url),
            );
            prev = end;
        }
        if prev < text.len() {
            push_run(out, &text[prev..], (base + prev)..range.end, None);
        }
    };

    let finalize_tag = |pending: PendingTag,
                        source: &str,
                        tags: &mut Vec<EmphasisTag>| {
        let (Some(fi), Some(li)) = (pending.first_inner_start, pending.last_inner_end)
        else {
            return;
        };
        let open = pending.full_range.start..fi;
        let close = li..pending.full_range.end;
        if open.end <= open.start || close.end <= close.start {
            return;
        }
        if close.end > source.len() {
            return;
        }
        if !source.is_char_boundary(open.start)
            || !source.is_char_boundary(open.end)
            || !source.is_char_boundary(close.start)
            || !source.is_char_boundary(close.end)
        {
            return;
        }
        let open_text = SharedString::from(source[open.clone()].to_string());
        let close_text = SharedString::from(source[close.clone()].to_string());
        tags.push(EmphasisTag {
            open,
            close,
            open_text,
            close_text,
        });
    };

    while *idx < events.len() {
        let (event, range) = &events[*idx];
        match event {
            Event::End(end) if stop(end) => {
                // Terminate regardless of whether style / link stacks
                // are perfectly balanced. Unbalanced HTML (e.g. `<b>`
                // without a matching `</b>`) would otherwise loop past
                // the block end and consume the rest of the document.
                // Each `read_inline_runs_until` call starts with a
                // fresh stack, so the imbalance dies with this block.
                *idx += 1;
                return (out, tags);
            }
            Event::Start(tag) => {
                let cur = style_stack.last().copied().unwrap_or(base);
                // Track tag_stack alongside style_stack so the matching
                // End event can pop the right pending entry and emit
                // its marker ranges.
                let is_emphasis = matches!(
                    tag,
                    Tag::Emphasis | Tag::Strong | Tag::Strikethrough
                );
                if is_emphasis {
                    // A nested tag ends its parent's open marker, so each
                    // marker covers only its own delimiter run.
                    if let Some(parent) = tag_stack.last_mut() {
                        if parent.first_inner_start.is_none() {
                            parent.first_inner_start = Some(range.start);
                        }
                    }
                    tag_stack.push(PendingTag {
                        full_range: range.clone(),
                        first_inner_start: None,
                        last_inner_end: None,
                    });
                }
                match tag {
                    Tag::Emphasis => style_stack.push(InlineStyle { italic: true, ..cur }),
                    Tag::Strong => style_stack.push(InlineStyle { bold: true, ..cur }),
                    Tag::Strikethrough => {
                        style_stack.push(InlineStyle { strike: true, ..cur })
                    }
                    Tag::Link { dest_url, .. } => {
                        link_stack.push(dest_url.to_string());
                        style_stack.push(cur);
                    }
                    // Image / FootnoteDefinition / etc. — push a copy so the
                    // matching End pops cleanly, but don't consume content
                    // as renderable text. Image alt text gets dropped here;
                    // v1 doesn't render images.
                    _ => style_stack.push(cur),
                }
                *idx += 1;
            }
            Event::End(end) => {
                if matches!(end, TagEnd::Link) {
                    link_stack.pop();
                }
                if matches!(
                    end,
                    TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough
                ) {
                    if let Some(pending) = tag_stack.pop() {
                        // The parent's content runs at least to the end of
                        // this tag, close marker included.
                        if let Some(parent) = tag_stack.last_mut() {
                            parent.last_inner_end = Some(pending.full_range.end);
                        }
                        finalize_tag(pending, source, &mut tags);
                    }
                }
                if style_stack.len() > 1 {
                    style_stack.pop();
                }
                *idx += 1;
            }
            Event::Text(s) => {
                emit_text(
                    s.to_string(),
                    range.clone(),
                    &style_stack,
                    &link_stack,
                    &mut tag_stack,
                    &mut out,
                    false,
                );
                *idx += 1;
            }
            Event::Code(s) => {
                // Inline code is emitted as a single event with the
                // full source range (markers + inner). Synthesize an
                // EmphasisTag inline so the renderer can reveal the
                // backticks the same way it reveals `**` / `*` / `~~`,
                // and emit the inner text with a source range covering
                // only the inner bytes — otherwise the run's display
                // width and source-byte width mismatch and clicks land
                // at the wrong caret position, breaking editing.
                let inner_len = s.len();
                let total_len = range.end.saturating_sub(range.start);
                let mut inner_range = range.clone();
                if total_len > inner_len && total_len - inner_len >= 2 {
                    let marker_each = (total_len - inner_len) / 2;
                    let open_range = range.start..(range.start + marker_each);
                    let close_range = (range.end - marker_each)..range.end;
                    if open_range.end <= source.len()
                        && close_range.end <= source.len()
                        && source.is_char_boundary(open_range.start)
                        && source.is_char_boundary(open_range.end)
                        && source.is_char_boundary(close_range.start)
                        && source.is_char_boundary(close_range.end)
                    {
                        let open_text =
                            SharedString::from(source[open_range.clone()].to_string());
                        let close_text =
                            SharedString::from(source[close_range.clone()].to_string());
                        inner_range = open_range.end..close_range.start;
                        tags.push(EmphasisTag {
                            open: open_range,
                            close: close_range,
                            open_text,
                            close_text,
                        });
                    }
                }
                emit_text(
                    s.to_string(),
                    inner_range,
                    &style_stack,
                    &link_stack,
                    &mut tag_stack,
                    &mut out,
                    true,
                );
                *idx += 1;
            }
            Event::SoftBreak => {
                emit_text(
                    " ".to_string(),
                    range.clone(),
                    &style_stack,
                    &link_stack,
                    &mut tag_stack,
                    &mut out,
                    false,
                );
                *idx += 1;
            }
            Event::HardBreak => {
                emit_text(
                    "\n".to_string(),
                    range.clone(),
                    &style_stack,
                    &link_stack,
                    &mut tag_stack,
                    &mut out,
                    false,
                );
                *idx += 1;
            }
            Event::FootnoteReference(label) => {
                // Inline `[^label]` reference. GPUI text runs have no
                // baseline-shift / superscript hook (see `sub`/`sup` HTML
                // tag note in `apply_inline_html_style`), so emit the
                // literal source slice with inline-code chrome to make
                // the reference visually distinctive. Range covers the
                // full `[^label]` source, so caret hit-testing lines up
                // with the source bytes for editing.
                //
                // Tag the run with the synthetic `notepad-fn:def:<label>`
                // scheme so the cmd/ctrl-click handler in
                // `markdown_preview_pane` routes through
                // `Workspace::preview_jump_to_footnote_def` instead of
                // `cx.open_url`. `emit_text` reads only the top of the
                // link stack, so a one-entry stack carries the link
                // without copying every open `<a>` for each reference.
                let rendered = format!("[^{}]", label);
                let mut style = style_stack.last().copied().unwrap_or(base);
                style.code = true;
                let synthetic_stack = vec![style];
                let synthetic_links = vec![format!("notepad-fn:def:{}", label)];
                emit_text(
                    rendered,
                    range.clone(),
                    &synthetic_stack,
                    &synthetic_links,
                    &mut tag_stack,
                    &mut out,
                    true,
                );
                *idx += 1;
            }
            Event::InlineHtml(s) | Event::Html(s) => {
                // Inline HTML — recognise a small allowlist matching what
                // GitHub-flavoured markdown renders (b/strong, i/em, u/ins,
                // s/strike/del, mark, code/kbd/samp/var, sub/sup, br, a).
                // Recognised tags push/pop on the style stack so nested
                // content inherits styling; unrecognised tags are dropped
                // silently (instead of leaking literal "<tag>" into the
                // rendered text the way the previous v1 behaviour did).
                if let Some(tag) = parse_html_tag(s) {
                    if tag.name == "br" && !tag.closing {
                        emit_text(
                            "\n".to_string(),
                            range.clone(),
                            &style_stack,
                            &link_stack,
                            &mut tag_stack,
                            &mut out,
                            false,
                        );
                    } else if tag.name == "a" {
                        if tag.closing {
                            link_stack.pop();
                            if style_stack.len() > 1 {
                                style_stack.pop();
                            }
                        } else if let Some(href) = attr(&tag.attrs, "href") {
                            link_stack.push(href.to_string());
                            let cur = style_stack.last().copied().unwrap_or(base);
                            style_stack.push(cur);
                        }
                    } else {
                        apply_inline_html_style(&mut style_stack, &tag);
                    }
                }
                *idx += 1;
            }
            _ => {
                *idx += 1;
            }
        }
    }
    (out, tags)
}

/// Push or pop on `style_stack` for an inline HTML tag matching the
/// GitHub-flavoured markdown allowlist. Unrecognised tags get a no-op
/// push (open) / pop (close) so the stack stays balanced — that way an
/// `<unknown>` open followed by an `</unknown>` close doesn't unbalance
/// the surrounding emphasis state.
fn apply_inline_html_style(stack: &mut Vec<InlineStyle>, tag: &ParsedTag) {
    if tag.closing {
        if stack.len() > 1 {
            stack.pop();
        }
        return;
    }
    let cur = stack.last().copied().unwrap_or_default();
    let next = match tag.name.as_str() {
        "b" | "strong" => InlineStyle { bold: true, ..cur },
        "i" | "em" => InlineStyle { italic: true, ..cur },
        "u" | "ins" => InlineStyle { underline: true, ..cur },
        "s" | "strike" | "del" => InlineStyle { strike: true, ..cur },
        "mark" => InlineStyle { mark: true, ..cur },
        "code" | "kbd" | "samp" | "var" => InlineStyle { code: true, ..cur },
        // sub / sup: recognised so the close tag pops cleanly, but GPUI's
        // text-run model has no per-run font-size or baseline-shift hook
        // — there's no way to render true subscript / superscript
        // without splitting the line into separate elements. Tracked in
        // UNTESTED.md.
        "sub" | "sup" => cur,
        _ => cur,
    };
    stack.push(next);
}

/// Bare-URL autolink scanner. Mirrors GFM extension behaviour:
/// recognises `http://`, `https://`, `ftp://`, and scheme-less `www.`
/// prefixes; consumes URL-safe bytes; strips trailing sentence
/// punctuation; and avoids matching URLs preceded by alphanumeric /
/// dot-style characters (so `foo.www.example.com` doesn't autolink the
/// `www.` part).
///
/// Returns `(start_byte, end_byte, href)` triples in source-byte order.
/// For `www.` matches the href is prefixed with `http://`.
fn scan_autolinks(text: &str) -> Vec<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let prev = if i == 0 { None } else { Some(bytes[i - 1]) };
        let bare_www_allowed = match prev {
            None => true,
            Some(b) => !matches!(
                b,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'@'
            ),
        };
        let scheme_len: Option<usize> = if bytes[i..].starts_with(b"https://") {
            Some(8)
        } else if bytes[i..].starts_with(b"http://") {
            Some(7)
        } else if bytes[i..].starts_with(b"ftp://") {
            Some(6)
        } else if bytes[i..].starts_with(b"www.") && bare_www_allowed {
            Some(0)
        } else {
            None
        };
        let Some(scheme_len) = scheme_len else {
            i += 1;
            continue;
        };
        let body_start = i + scheme_len;
        let mut end = body_start;
        while end < bytes.len() && is_url_byte(bytes[end]) {
            end += 1;
        }
        // Require at least one host byte after the scheme / `www.` prefix.
        let scheme_body_min = if scheme_len == 0 { 4 } else { scheme_len };
        if end <= i + scheme_body_min {
            i += 1;
            continue;
        }
        // Strip trailing sentence punctuation. Parens are counted once and
        // the close count kept in step as `)` bytes are stripped.
        let opens = bytes[i..end].iter().filter(|b| **b == b'(').count();
        let mut closes = bytes[i..end].iter().filter(|b| **b == b')').count();
        while end > body_start {
            let c = bytes[end - 1];
            if matches!(c, b'.' | b',' | b';' | b'!' | b'?' | b':') {
                end -= 1;
                continue;
            }
            if c == b')' && closes > opens {
                end -= 1;
                closes -= 1;
                continue;
            }
            break;
        }
        if end <= body_start {
            i += 1;
            continue;
        }
        let href = if scheme_len == 0 {
            format!("http://{}", &text[i..end])
        } else {
            text[i..end].to_string()
        };
        out.push((i, end, href));
        i = end;
    }
    out
}

fn is_url_byte(b: u8) -> bool {
    matches!(
        b,
        b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'.'
            | b'-'
            | b'_'
            | b'~'
            | b':'
            | b'/'
            | b'?'
            | b'#'
            | b'['
            | b']'
            | b'@'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b'%'
            | b'('
            | b')'
    )
}

#[cfg(test)]
mod autolink_tests {
    use super::scan_autolinks;

    #[test]
    fn http_basic() {
        let r = scan_autolinks("see http://example.com here");
        assert_eq!(r, vec![(4, 22, "http://example.com".to_string())]);
    }

    #[test]
    fn https_with_path() {
        let r = scan_autolinks("https://github.com/foo/bar");
        assert_eq!(r, vec![(0, 26, "https://github.com/foo/bar".to_string())]);
    }

    #[test]
    fn www_prefix_gets_http() {
        let r = scan_autolinks("visit www.example.com today");
        assert_eq!(r, vec![(6, 21, "http://www.example.com".to_string())]);
    }

    #[test]
    fn strips_trailing_period() {
        let r = scan_autolinks("Go to https://example.com.");
        assert_eq!(r, vec![(6, 25, "https://example.com".to_string())]);
    }

    #[test]
    fn strips_unbalanced_close_paren() {
        let r = scan_autolinks("see (https://example.com)");
        assert_eq!(r, vec![(5, 24, "https://example.com".to_string())]);
    }

    #[test]
    fn keeps_balanced_parens_in_url() {
        // Wikipedia-style URL with parens inside should keep them.
        let r = scan_autolinks("https://en.wikipedia.org/wiki/Foo_(bar)");
        assert_eq!(
            r,
            vec![(0, 39, "https://en.wikipedia.org/wiki/Foo_(bar)".to_string())]
        );
    }

    #[test]
    fn skips_email_like_www() {
        // `foo.www.example.com` — the `www.` is preceded by `.`, so don't autolink.
        let r = scan_autolinks("foo.www.example.com");
        assert_eq!(r, Vec::new());
    }

    #[test]
    fn multiple_urls() {
        let r = scan_autolinks("http://a.com and http://b.com");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].2, "http://a.com");
        assert_eq!(r[1].2, "http://b.com");
    }

    #[test]
    fn long_run_of_trailing_close_parens() {
        // Stripping each `)` used to recount every paren in the URL, which
        // hung the preview on a long line of them.
        let text = format!("http://a{}", ")".repeat(200_000));
        assert_eq!(scan_autolinks(&text), vec![(0, 8, "http://a".to_string())]);
        let text = format!("http://a({}", ")".repeat(200_000));
        assert_eq!(scan_autolinks(&text), vec![(0, 10, "http://a()".to_string())]);
    }
}

fn skip_until_end(events: &[EventWithRange], idx: &mut usize) {
    let mut depth = 1;
    while *idx < events.len() && depth > 0 {
        match &events[*idx].0 {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            _ => {}
        }
        *idx += 1;
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

// --- Renderer -----------------------------------------------------------

/// Public entry — replaces `editor_pane` when the active tab is in
/// `MarkdownPreview` mode. Wraps the rendered tree in a vertical scroll
/// container so long documents stay scrollable.
pub(crate) fn markdown_preview_pane(
    theme: &Theme,
    source: &str,
    workspace: Entity<Workspace>,
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    scroll_handle: ScrollHandle,
    doc_dir: Option<std::path::PathBuf>,
) -> impl IntoElement {
    let blocks = parse(source);
    let doc_dir = doc_dir.map(std::sync::Arc::new);
    // Window-level mouse-up handler. `on_mouse_event` lives on the window
    // (registered every paint, dropped before next paint), so each render
    // re-installs a fresh listener that clears the active drag flag once
    // the user releases the button anywhere inside the window. Without
    // this the per-paragraph `on_mouse_move` would keep extending the
    // selection forever after a release outside any block.
    let release_workspace = workspace.clone();
    // Outer-pane mouse-down clears the selection. Per-line handlers run
    // first (bubble phase: child → parent) and call `cx.stop_propagation`
    // when they handled the click, so this fires only when the click
    // missed every paragraph / heading — clicking blank space, a code
    // block, a list bullet, etc. — matching how a normal text field
    // drops its highlight when you click outside the text.
    let clear_workspace = workspace.clone();
    // Fallback drop target: drops that miss every per-line hitbox land
    // here (drops on padding, between blocks, on a code block / list /
    // blockquote). Image files get appended at the end of the buffer.
    // Per-line `on_drop` listeners run first (bubble phase) and stop
    // propagation when they handle the drop, so this only fires when
    // none of them claimed the position.
    let pane_drop_workspace = workspace.clone();
    // Right-click anywhere in the pane (over text or blank space) opens
    // the standard Cut / Copy / Paste / Undo / Redo / Select All context
    // menu. Focus the workspace first so the menu's actions land against
    // the source buffer (the same paths source-mode uses).
    let menu_workspace = workspace.clone();

    // The persistent ScrollHandle is owned by the workspace and passed in
    // here — taking a `cx` and reading the entity from inside the render
    // pass would double-lease panic, since render itself already holds
    // the mutable lease.

    let mut scroll = div()
        .id("markdown-preview-pane")
        .flex_1()
        // Bound the scroll container to the viewport on BOTH axes —
        // without `min-w/min-h: 0`, flex's default `min-size: auto` lets
        // content's intrinsic size push the pane past the parent's
        // bounds. On the Y axis that breaks scrolling; on the X axis it
        // breaks word wrap, because StyledText only wraps when its
        // available width is definite, and an auto-min-width flex item
        // grows to its longest unbreakable line instead of shrinking to
        // the parent's width.
        //
        // No `h_full` here — the outer wrapper is `flex_col`, so
        // `flex_1` on this child already sizes us to the parent's full
        // main-axis (vertical) extent. Adding `h_full` on top of that
        // confuses Taffy into measuring `bounds.size.height` as the
        // intrinsic content height, which makes `max_offset` (computed
        // as `padded_content - bounds.size`) come out short — the last
        // few hundred pixels of content stayed unreachable by the
        // scrollbar.
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .track_scroll(&scroll_handle)
        .pl(rems(20. / 16.))
        // Right padding reserves space for the absolute-overlay
        // scrollbar so the last word of long lines doesn't slide under
        // the thumb when the user scrolls.
        .pr(px(MD_PREVIEW_SCROLLBAR_THICKNESS + 8.))
        .py(rems(1.))
        .bg(rgb(theme.bg))
        .text_color(rgb(theme.text))
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
            // Focus the workspace before mutating selection so any
            // subsequent typed character / IME composition flows through
            // `Workspace::EntityInputHandler::replace_text_in_range` and
            // edits the source buffer at the (now-collapsed) selection.
            // Without this focus call, `input_focus(window)` returns
            // whatever was focused before, so character input has nowhere
            // to land and the preview behaves as read-only.
            let handle = clear_workspace.read(cx).focus_handle.clone();
            window.focus(&handle, cx);
            // Click in dead space (right margin, between blocks, the
            // trailing scroll-past-end spacer) — per-block handlers all
            // skipped this one (they each `cx.stop_propagation()` when
            // hit). Instead of just clearing the selection, hit-test
            // against the registered block list and start a drag at the
            // nearest block's source byte. Falls back to clearing only
            // when the preview is empty (no targets) — usually means an
            // empty file or the very first paint before any block has
            // rendered.
            let extend = event.modifiers.shift;
            let position = event.position;
            clear_workspace.update(cx, |w, cx| {
                if !w.preview_drag_to_pixel(position, extend, !extend, cx) {
                    w.preview_clear_selection(cx);
                }
            });
        })
        .on_mouse_up(MouseButton::Left, move |_event, _window, cx| {
            release_workspace.update(cx, |w, cx| {
                w.preview_end_drag(cx);
            });
        })
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            let handle = menu_workspace.read(cx).focus_handle.clone();
            window.focus(&handle, cx);
            menu_workspace.update(cx, |w, cx| {
                w.show_context_menu(event.position, cx);
            });
            cx.stop_propagation();
        })
        .on_drop(move |paths: &ExternalPaths, _window, cx| {
            let images: Vec<&PathBuf> = paths
                .paths()
                .iter()
                .filter(|p| is_image_path(p))
                .collect();
            if images.is_empty() {
                // Non-image drop: leave it for the workspace-root
                // handler that opens dropped files as tabs.
                return;
            }
            let snippet = build_image_snippet(&images);
            pane_drop_workspace.update(cx, |w, cx| {
                w.preview_append_image_markup(snippet, cx);
            });
            cx.stop_propagation();
        });
    let mut next_block_id: u32 = 0;
    let doc_dir_ref = doc_dir.as_deref().map(|p| p.as_path());
    for block in &blocks {
        scroll = scroll.child(render_block(
            theme,
            block,
            0,
            selection.clone(),
            caret,
            &workspace,
            &mut next_block_id,
            doc_dir_ref,
        ));
    }
    // Trailing spacer below the last block so the user can scroll past
    // the end of content and the last lines aren't clipped against the
    // pane bottom. Sized generously (~half a viewport) so the cursor
    // never sits flush with the bottom edge when editing the final
    // paragraph — matches the "scroll-past-end" behaviour every modern
    // text editor offers (VS Code's `editor.scrollBeyondLastLine`).
    scroll = scroll.child(div().h(rems(192. / 16.)));

    // Outer wrapper: `relative` positioning context for the absolute
    // overlay siblings (scrollbar / input bridge / focus ring). The
    // scroll div is the sole flow child; the overlays are removed from
    // flow by `Position::Absolute` so they don't compete for layout
    // space. We use `flex_col` (not `flex_row`) so `flex_1` on the
    // scroll child grows along the vertical main axis. With `flex_row`
    // the child would grow horizontally and need `h_full` for height,
    // which mismeasured `bounds.size.height` against intrinsic content
    // and truncated `max_offset` — see the comment on the scroll
    // child's `min_h` for details.
    div()
        .flex_1()
        // min_w(0) so this flex_col child can shrink to its parent's
        // width instead of inflating to its content's intrinsic width
        // (longest unbreakable line). Without this the whole preview
        // pane grows past the workspace bounds and word wrap never
        // kicks in.
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .relative()
        .overflow_hidden()
        .child(scroll)
        .child(MarkdownPreviewScrollbar {
            workspace: workspace.clone(),
            handle: scroll_handle,
            color: rgba(theme.scrollbar).into(),
            color_hover: rgba(theme.scrollbar_hover).into(),
        })
        // IME / typed-character bridge. Sits as a transparent absolute
        // overlay covering the whole preview pane; its `paint` calls
        // `window.handle_input(workspace_focus, ...)` so the OS text
        // input subsystem routes typed chars / paste / IME composition
        // through `Workspace::EntityInputHandler::replace_text_in_range`.
        // No mouse handlers — does not intercept clicks / drags / scrolls.
        // Bounds are used by the OS to position the IME composition rect;
        // we pass the full pane so dead-key composition lands near the
        // top-left of the preview rather than at (0,0). Without this
        // element, the preview is read-only for character input even
        // when the workspace handle is focused — `handle_input` is only
        // active for the paint cycle it's called in, and the source
        // editor's call (in `editor_element.rs`) doesn't run while the
        // preview pane is mounted.
        .child(MarkdownInputBridge {
            workspace: workspace.clone(),
        })
}

fn render_block(
    theme: &Theme,
    block: &Block,
    indent: u32,
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: &Entity<Workspace>,
    next_id: &mut u32,
    doc_dir: Option<&std::path::Path>,
) -> AnyElement {
    match block {
        Block::Heading { level, runs, tags } => {
            let id = *next_id;
            *next_id += 1;
            render_heading(theme, *level, runs, tags, selection, caret, workspace.clone(), id)
                .into_any_element()
        }
        Block::Paragraph { runs, tags } => {
            let id = *next_id;
            *next_id += 1;
            render_paragraph(theme, runs, tags, selection, caret, workspace.clone(), id)
                .into_any_element()
        }
        Block::CodeBlock { lang, runs } => {
            let id = *next_id;
            *next_id += 1;
            render_code_block(
                theme,
                lang.as_deref(),
                runs,
                selection.clone(),
                caret,
                workspace.clone(),
                id,
            )
            .into_any_element()
        }
        Block::BlockQuote(inner) => {
            render_block_quote(theme, inner, indent, selection, caret, workspace, next_id, doc_dir)
                .into_any_element()
        }
        Block::List {
            ordered,
            start,
            items,
        } => render_list(
            theme, *ordered, *start, items, indent, selection, caret, workspace, next_id, doc_dir,
        )
        .into_any_element(),
        Block::Rule => render_rule(theme).into_any_element(),
        Block::Table {
            alignments,
            header,
            rows,
        } => render_table(
            theme, alignments, header, rows, selection, caret, workspace, next_id,
        )
        .into_any_element(),
        Block::Aligned { align, blocks } => render_aligned(
            theme, *align, blocks, indent, selection, caret, workspace, next_id, doc_dir,
        )
        .into_any_element(),
        Block::FootnoteDefinition { label, blocks } => render_footnote_definition(
            theme, label, blocks, indent, selection, caret, workspace, next_id, doc_dir,
        )
        .into_any_element(),
        Block::Image { url, alt, .. } => render_md_image(theme, url, alt, doc_dir),
        Block::ParagraphWithImages { segments } => render_paragraph_with_images(
            theme, segments, selection, caret, workspace, next_id, doc_dir,
        ),
    }
}

/// Render `Block::Image` as a block-level element. URL resolution:
///   - `file:///...`  → absolute filesystem path
///   - `http://...`, `https://...`, `data:...` → not loaded; falls back
///      to a placeholder chip (network/data-URI fetch is v2 scope)
///   - any other string → treated as a filesystem path; absolute paths
///     are read as-is, relative paths resolve against `doc_dir` (the
///     parent directory of the currently-open `.md` file). If the doc
///     hasn't been saved yet (`doc_dir` is None), relative paths can't
///     resolve and we render the placeholder.
///
/// Failure modes (file missing, unreadable, unknown extension, decode
/// error) all collapse to the same placeholder chip showing the URL and
/// alt text so the user sees what they typed.
fn render_md_image(
    theme: &Theme,
    url: &str,
    alt: &str,
    doc_dir: Option<&std::path::Path>,
) -> AnyElement {
    let trimmed = url.trim();
    let is_remote = trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("data:");
    if is_remote {
        return render_md_image_placeholder(theme, url, alt, "remote URL");
    }
    let path: std::path::PathBuf = if let Some(rest) = trimmed.strip_prefix("file://") {
        // `file:///` form has a leading slash that becomes the path root;
        // `file://host/...` is not supported here (no SMB fetch). Trim a
        // leading host segment if present.
        let after_host = rest.find('/').map(|i| &rest[i..]).unwrap_or("/");
        std::path::PathBuf::from(after_host)
    } else {
        let p = std::path::PathBuf::from(trimmed);
        if p.is_absolute() {
            p
        } else if let Some(base) = doc_dir {
            base.join(&p)
        } else {
            return render_md_image_placeholder(theme, url, alt, "unsaved document");
        }
    };
    // Format first, so a path without an image extension is never read.
    let mime = mime_for_extension(&path);
    let Some(format) = mime.and_then(gpui::ImageFormat::from_mime_type) else {
        return render_md_image_placeholder(theme, url, alt, "unsupported format");
    };
    let bytes = match read_image_file(&path) {
        Ok(b) => b,
        Err(reason) => return render_md_image_placeholder(theme, url, alt, reason),
    };
    let image = std::sync::Arc::new(gpui::Image::from_bytes(format, bytes));
    div()
        .py(px(4.))
        .child(gpui::img(image))
        .into_any_element()
}

/// Reads a local image for the preview. Only regular files are read, and
/// no more than the length the open file reports, so a path that resolves
/// to `/dev/zero`, a FIFO or another device shows the placeholder instead
/// of hanging the preview or filling memory. The error is the placeholder
/// reason.
fn read_image_file(path: &std::path::Path) -> Result<Vec<u8>, &'static str> {
    use std::io::Read;
    // Checked before opening: opening a FIFO for reading blocks until a
    // writer appears.
    let meta = std::fs::metadata(path).map_err(|_| "file not found")?;
    if !meta.is_file() {
        return Err("not a file");
    }
    let file = std::fs::File::open(path).map_err(|_| "file not found")?;
    let meta = file.metadata().map_err(|_| "file not found")?;
    if !meta.is_file() {
        return Err("not a file");
    }
    let mut bytes = Vec::new();
    file.take(meta.len())
        .read_to_end(&mut bytes)
        .map_err(|_| "file not found")?;
    Ok(bytes)
}

/// Render a paragraph that mixes text segments and inline images.
/// Each text segment goes through `build_selectable_line` so click /
/// drag / typing behave exactly like a regular paragraph; each image
/// goes through `render_md_image` so file-vs-remote handling, mime
/// detection, and placeholder fallback all match the standalone case.
/// Segments stack vertically inside a `flex_col` (GPUI's StyledText
/// is monolithic and won't text-flow around an inline element). One
/// `block_id` is consumed per segment so caret-overlay registration,
/// drag hit-testing, and selection rectangles stay correct across the
/// composite paragraph.
fn render_paragraph_with_images(
    theme: &Theme,
    segments: &[ParagraphSegment],
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: &Entity<Workspace>,
    next_id: &mut u32,
    doc_dir: Option<&std::path::Path>,
) -> AnyElement {
    let mut container = div().my(px(6.)).flex().flex_col();
    for segment in segments {
        match segment {
            ParagraphSegment::Text { runs, tags } => {
                let id = *next_id;
                *next_id += 1;
                let line = build_selectable_line(
                    theme,
                    runs,
                    tags,
                    13.5,
                    FontWeight::NORMAL,
                    selection.clone(),
                    caret,
                    workspace.clone(),
                    id,
                );
                container = container.child(line);
            }
            ParagraphSegment::Image { url, alt, .. } => {
                container = container.child(render_md_image(theme, url, alt, doc_dir));
            }
        }
    }
    container.into_any_element()
}

fn render_md_image_placeholder(
    theme: &Theme,
    url: &str,
    alt: &str,
    reason: &str,
) -> AnyElement {
    let label: SharedString = if alt.is_empty() {
        format!("[image: {url} — {reason}]").into()
    } else {
        format!("[image: {alt} — {url} — {reason}]").into()
    };
    div()
        .my(px(4.))
        .px(px(8.))
        .py(px(4.))
        .border_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.disabled_text))
        .text_size(rems(12. / 16.))
        .child(label)
        .into_any_element()
}

/// Map a file extension to an IANA mime type. Only the formats GPUI's
/// `ImageFormat::from_mime_type` accepts are listed — anything else
/// returns None so the caller routes to the placeholder.
fn mime_for_extension(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "tif" | "tiff" => Some("image/tiff"),
        "ico" => Some("image/x-icon"),
        _ => None,
    }
}

fn render_footnote_definition(
    theme: &Theme,
    label: &SharedString,
    blocks: &[Block],
    indent: u32,
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: &Entity<Workspace>,
    next_id: &mut u32,
    doc_dir: Option<&std::path::Path>,
) -> impl IntoElement {
    let prefix: SharedString = format!("[^{}]:", label).into();
    let mut body = div().flex_1().min_w(px(0.));
    for b in blocks {
        body = body.child(render_block(
            theme,
            b,
            indent,
            selection.clone(),
            caret,
            workspace,
            next_id,
            doc_dir,
        ));
    }
    // Burn one id for the back-link element so the dispatch-tree key
    // stays unique across all interactive elements in this render pass.
    let id = *next_id;
    *next_id += 1;
    let back_label = label.to_string();
    let back_workspace = workspace.clone();
    // Back-arrow trailing the prefix — plain click jumps to the first
    // `[^label]` reference in the source. Distinct from the cmd-click
    // affordance forward links use; the arrow is unambiguously a UI
    // control, so requiring a modifier would be surprising. Glyph sits
    // outside the prefix div so the prefix text remains a plain caret
    // target if the user just wants to edit `[^label]:` itself.
    let back_link = div()
        .id(("md-fn-back", id))
        .flex_none()
        .text_color(rgb(theme.link))
        .text_size(rems(0.85))
        .cursor(gpui::CursorStyle::PointingHand)
        .child(SharedString::from("↩"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let label = back_label.clone();
            back_workspace.update(cx, |w, cx| {
                w.preview_jump_to_footnote_ref(&label, cx);
            });
            cx.stop_propagation();
        });
    div()
        .my(px(4.))
        .flex()
        .flex_row()
        .items_start()
        .gap(px(6.))
        .child(
            div()
                .flex_none()
                .text_color(rgb(theme.disabled_text))
                .text_size(rems(0.85))
                .child(prefix),
        )
        .child(body)
        .child(back_link)
}

/// Wrap a sequence of blocks in a div with `text_align` set. Alignment
/// cascades through GPUI's text-style inheritance, so headings,
/// paragraphs, and inline runs inside all pick it up without needing to
/// be re-styled individually.
fn render_aligned(
    theme: &Theme,
    align: gpui::TextAlign,
    blocks: &[Block],
    indent: u32,
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: &Entity<Workspace>,
    next_id: &mut u32,
    doc_dir: Option<&std::path::Path>,
) -> impl IntoElement {
    let mut wrap = div().text_align(align);
    for b in blocks {
        wrap = wrap.child(render_block(
            theme,
            b,
            indent,
            selection.clone(),
            caret,
            workspace,
            next_id,
            doc_dir,
        ));
    }
    wrap
}

fn render_heading(
    theme: &Theme,
    level: u8,
    runs: &[InlineRun],
    tags: &[EmphasisTag],
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: Entity<Workspace>,
    block_id: u32,
) -> impl IntoElement {
    // Sizes in `rems` (relative to body 16px base) so headings scale with
    // window zoom alongside paragraph text. Line-height uses `relative(N)`
    // (N × font_size) so vertical spacing always tracks the heading's own
    // size — not the workspace's inherited 20px line-height, which caused
    // letter overlap on H1 at default zoom and got worse on zoom-out.
    let (size, weight, mt, mb, border, lh) = match level {
        1 => (28., FontWeight::BOLD, 24., 8., true, 1.35),
        2 => (22., FontWeight::BOLD, 20., 8., true, 1.35),
        3 => (18., FontWeight::BOLD, 16., 6., false, 1.35),
        4 => (16., FontWeight::SEMIBOLD, 14., 6., false, 1.4),
        5 => (14., FontWeight::SEMIBOLD, 12., 4., false, 1.45),
        _ => (13.5, FontWeight::SEMIBOLD, 12., 4., false, 1.5),
    };
    let line = build_selectable_line(
        theme, runs, tags, size, weight, selection, caret, workspace, block_id,
    );
    let mut wrap = div()
        .mt(px(mt))
        .mb(px(mb))
        .child(
            div()
                .child(line)
                .text_size(rems(size / 16.))
                .line_height(relative(lh)),
        );
    if border {
        wrap = wrap
            .pb(px(4.))
            .border_b_1()
            .border_color(rgb(theme.border));
    }
    wrap
}

fn render_paragraph(
    theme: &Theme,
    runs: &[InlineRun],
    tags: &[EmphasisTag],
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: Entity<Workspace>,
    block_id: u32,
) -> impl IntoElement {
    let line = build_selectable_line(
        theme,
        runs,
        tags,
        13.5,
        FontWeight::NORMAL,
        selection,
        caret,
        workspace,
        block_id,
    );
    div().my(px(6.)).child(line)
}

fn render_code_block(
    theme: &Theme,
    lang: Option<&str>,
    runs: &[InlineRun],
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: Entity<Workspace>,
    block_id: u32,
) -> impl IntoElement {
    // When the fence has a known language, tree-sitter highlights the
    // concatenated run text and we split each `InlineRun` at span
    // boundaries, attaching the kind-color as `color_override`. Unknown
    // / missing info-string → original runs untouched → falls back to
    // the flat `chrome_text` color via `make_run`. The same syntax
    // palette the editor uses (`theme.syntax`) feeds both paths so
    // colors agree between source view and preview.
    let highlighted: Vec<InlineRun>;
    let runs_for_line: &[InlineRun] = match lang.and_then(crate::syntax::language_for_info_string) {
        Some(lang_id) => {
            let combined: String = runs.iter().map(|r| r.text.as_str()).collect();
            let spans = crate::syntax::highlight(lang_id, &combined);
            if spans.is_empty() {
                runs
            } else {
                highlighted = split_runs_with_highlights(runs, &spans, &theme.syntax);
                &highlighted
            }
        }
        None => runs,
    };
    // Route content through `build_selectable_line` so clicks place a
    // caret and typed characters edit the source bytes inside the fence
    // — same pipeline paragraphs use. No emphasis tags (code blocks
    // don't parse inline markdown), `code: false` on each run so the
    // outer chrome wrapper supplies the background instead of stacking
    // per-run chrome.
    let line = build_selectable_line(
        theme,
        runs_for_line,
        &[],
        12.5,
        FontWeight::NORMAL,
        selection,
        caret,
        workspace,
        block_id,
    );
    // Code blocks preserve source formatting — no soft-wrapping. Long
    // lines scroll horizontally inside the block instead, matching how
    // every standard markdown renderer (GitHub, VS Code, etc.) treats
    // fenced and indented code. `overflow_x_scroll` needs a stateful div
    // (an `.id(...)`), hence the block id.
    let id_str: SharedString = SharedString::from(format!("md-code-block-{block_id}"));
    div()
        .id(id_str)
        .my(px(8.))
        .px(px(12.))
        .py(px(10.))
        .bg(rgb(theme.chrome))
        .border_1()
        .border_color(rgb(theme.border))
        .rounded(px(4.))
        .text_size(rems(12.5 / 16.))
        .line_height(relative(1.45))
        .whitespace_nowrap()
        .overflow_x_scroll()
        .child(line)
}

/// Split fenced-code `InlineRun`s at highlight-span boundaries, attaching
/// `color_override` from the editor's `SyntaxTheme` to the slices that
/// fall inside a span. Spans are byte ranges in the concatenation of all
/// runs' text — same coordinate system that `syntax::highlight` produces.
/// Runs are walked once with a moving cursor (`code_pos`) tracking the
/// byte offset into the concatenated code source; spans are walked once
/// with `span_idx`, advancing past spans that end before the current
/// cursor. UTF-8 boundary safety: if a span edge lands inside a
/// codepoint inside a run, that run emits whole (uncolored) — better
/// than panicking on `&str[..]` with a non-boundary index.
fn split_runs_with_highlights(
    runs: &[InlineRun],
    spans: &[crate::syntax::HighlightSpan],
    syntax_theme: &crate::theme::SyntaxTheme,
) -> Vec<InlineRun> {
    let mut out: Vec<InlineRun> = Vec::with_capacity(runs.len());
    let mut code_pos: usize = 0;
    let mut span_idx: usize = 0;

    for r in runs {
        let run_start = code_pos;
        let run_end = run_start + r.text.len();
        code_pos = run_end;

        if r.text.is_empty() {
            out.push(r.clone());
            continue;
        }

        // Advance past any spans that ended before this run.
        while span_idx < spans.len() && spans[span_idx].range.end <= run_start {
            span_idx += 1;
        }

        // No overlapping spans — emit untouched.
        if span_idx >= spans.len() || spans[span_idx].range.start >= run_end {
            out.push(r.clone());
            continue;
        }

        // Walk overlapping spans, emitting (unstyled) gap + (styled) slice
        // pairs. `local_cursor` tracks position inside `r.text`.
        let mut local_cursor: usize = 0;
        let mut probe = span_idx;
        while probe < spans.len() && spans[probe].range.start < run_end {
            let span = &spans[probe];
            // Clamp to the uncoloured remainder so an overlapping or
            // nested span can neither underflow nor repeat text.
            let span_start_local = span.range.start.saturating_sub(run_start).max(local_cursor);
            let span_end_local = span.range.end.min(run_end).saturating_sub(run_start);
            if span_end_local < span_start_local {
                probe += 1;
                continue;
            }

            // UTF-8 safety — if either edge lands mid-codepoint we can't
            // safely slice. Emit the rest of the run uncolored and bail
            // out of this run's span loop.
            if !r.text.is_char_boundary(span_start_local)
                || !r.text.is_char_boundary(span_end_local)
            {
                if local_cursor < r.text.len() {
                    push_slice(&mut out, r, local_cursor, r.text.len(), None);
                }
                local_cursor = r.text.len();
                break;
            }

            if span_start_local > local_cursor {
                push_slice(&mut out, r, local_cursor, span_start_local, None);
            }
            let color = hsla(crate::syntax::kind_color(span.kind, syntax_theme));
            push_slice(&mut out, r, span_start_local, span_end_local, Some(color));
            local_cursor = span_end_local;
            probe += 1;
        }

        if local_cursor < r.text.len() {
            push_slice(&mut out, r, local_cursor, r.text.len(), None);
        }
    }

    out
}

fn push_slice(
    out: &mut Vec<InlineRun>,
    src: &InlineRun,
    local_start: usize,
    local_end: usize,
    color: Option<Hsla>,
) {
    if local_start >= local_end {
        return;
    }
    let source_start = src.source_range.start + local_start;
    let source_end = src.source_range.start + local_end;
    out.push(InlineRun {
        text: src.text[local_start..local_end].to_string(),
        source_range: source_start..source_end,
        bold: src.bold,
        italic: src.italic,
        strike: src.strike,
        underline: src.underline,
        mark: src.mark,
        code: src.code,
        link: src.link.clone(),
        color_override: color,
    });
}

fn render_block_quote(
    theme: &Theme,
    inner: &[Block],
    indent: u32,
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: &Entity<Workspace>,
    next_id: &mut u32,
    doc_dir: Option<&std::path::Path>,
) -> impl IntoElement {
    let mut quote = div()
        .my(px(6.))
        .pl(px(12.))
        .border_l_2()
        .border_color(rgb(theme.disabled_text))
        .text_color(rgb(theme.disabled_text));
    for b in inner {
        quote = quote.child(render_block(
            theme,
            b,
            indent + 1,
            selection.clone(),
            caret,
            workspace,
            next_id,
            doc_dir,
        ));
    }
    quote
}

fn render_rule(theme: &Theme) -> impl IntoElement {
    div().my(px(12.)).h(px(1.)).bg(rgb(theme.border))
}

/// Render a GFM table as a header row plus body rows, each row using
/// `flex_1` cells so columns share width evenly. Header row gets a
/// chrome background + bold weight. Per-column alignment from the
/// pulldown `Alignment` enum applies to every cell in that column.
#[allow(clippy::too_many_arguments)]
fn render_table(
    theme: &Theme,
    alignments: &[Alignment],
    header: &[TableCell],
    rows: &[Vec<TableCell>],
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: &Entity<Workspace>,
    next_id: &mut u32,
) -> impl IntoElement {
    let column_count = header
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));

    let align_for = |col: usize| -> gpui::TextAlign {
        match alignments.get(col).copied().unwrap_or(Alignment::None) {
            Alignment::Left | Alignment::None => gpui::TextAlign::Left,
            Alignment::Center => gpui::TextAlign::Center,
            Alignment::Right => gpui::TextAlign::Right,
        }
    };

    let make_cell = |cell: &TableCell,
                     col: usize,
                     weight: FontWeight,
                     is_last_col: bool,
                     next_id: &mut u32,
                     workspace: &Entity<Workspace>,
                     selection: Option<Range<usize>>,
                     caret: Option<usize>|
     -> gpui::Div {
        let id = *next_id;
        *next_id += 1;
        let line = build_selectable_line(
            theme,
            &cell.runs,
            &cell.tags,
            13.5,
            weight,
            selection,
            caret,
            workspace.clone(),
            id,
        );
        let mut div_cell = div()
            .flex_1()
            .min_w(px(0.))
            .px(px(10.))
            .py(px(6.))
            .text_align(align_for(col))
            .child(line);
        if !is_last_col {
            div_cell = div_cell.border_r_1().border_color(rgb(theme.border));
        }
        div_cell
    };

    let mut table = div()
        .my(px(8.))
        .border_1()
        .border_color(rgb(theme.border))
        .rounded(px(4.));

    // Header row.
    if !header.is_empty() {
        let mut header_row = div().flex().flex_row().bg(rgb(theme.chrome));
        for (col, cell) in header.iter().enumerate() {
            let is_last = col + 1 == column_count;
            header_row = header_row.child(make_cell(
                cell,
                col,
                FontWeight::BOLD,
                is_last,
                next_id,
                workspace,
                selection.clone(),
                caret,
            ));
        }
        table = table.child(
            div()
                .border_b_1()
                .border_color(rgb(theme.border))
                .child(header_row),
        );
    }

    // Body rows.
    for (row_idx, row) in rows.iter().enumerate() {
        let mut row_div = div().flex().flex_row();
        for (col, cell) in row.iter().enumerate() {
            let is_last = col + 1 == column_count;
            row_div = row_div.child(make_cell(
                cell,
                col,
                FontWeight::NORMAL,
                is_last,
                next_id,
                workspace,
                selection.clone(),
                caret,
            ));
        }
        let is_last_row = row_idx + 1 == rows.len();
        let mut wrap = div().child(row_div);
        if !is_last_row {
            wrap = wrap.border_b_1().border_color(rgb(theme.border));
        }
        table = table.child(wrap);
    }

    table
}

fn render_list(
    theme: &Theme,
    ordered: bool,
    start: u64,
    items: &[ListItem],
    indent: u32,
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: &Entity<Workspace>,
    next_id: &mut u32,
    doc_dir: Option<&std::path::Path>,
) -> impl IntoElement {
    let mut list = div().my(px(4.)).pl(px(if indent == 0 { 0. } else { 16. }));
    for (i, item) in items.iter().enumerate() {
        let prefix: SharedString = if let Some(checked) = item.checkbox {
            if checked { "[x]".into() } else { "[ ]".into() }
        } else if ordered {
            format!("{}.", start + i as u64).into()
        } else {
            "•".into()
        };
        let mut row = div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.))
            .my(px(2.));
        row = row.child(
            div()
                .w(px(24.))
                .flex_none()
                .text_align(gpui::TextAlign::Right)
                .text_color(rgb(theme.disabled_text))
                .child(prefix),
        );
        let mut body = div().flex_1().min_w(px(0.));
        for b in &item.blocks {
            body = body.child(render_block(
                theme,
                b,
                indent + 1,
                selection.clone(),
                caret,
                workspace,
                next_id,
                doc_dir,
            ));
        }
        row = row.child(body);
        list = list.child(row);
    }
    list
}

// --- Selection-aware inline rendering -----------------------------------

/// Build a wrapped, click-and-drag selectable line. Each paragraph or
/// heading goes through here so a single selection range can extend
/// across blocks (mouse-down in block A, drag into block B continues
/// updating the head as the cursor enters B's bounds and B's own
/// `on_mouse_move` fires).
///
/// Also paints a caret if the workspace focus handle is focused and
/// `Tab.selection.head` resolves to a source byte that lies inside one
/// of this line's `HitRun`s. The caret is painted as a thin vertical
/// quad inside a custom overlay Element (see `CaretOverlay`) so it
/// follows the StyledText's wrapped layout pixel-perfectly.
fn build_selectable_line(
    theme: &Theme,
    runs: &[InlineRun],
    tags: &[EmphasisTag],
    size_px: f32,
    base_weight: FontWeight,
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    workspace: Entity<Workspace>,
    block_id: u32,
) -> impl IntoElement {
    let sel_color: Hsla = rgba(theme.selection).into();
    let marker_color: Hsla = hsla(theme.disabled_text);
    let (combined, text_runs, hit_runs) = build_runs_with_selection(
        theme,
        runs,
        tags,
        size_px,
        base_weight,
        selection.as_ref(),
        caret,
        sel_color,
        marker_color,
    );

    if combined.is_empty() {
        return div().into_any_element();
    }

    let combined_shared = SharedString::from(combined);
    let styled = StyledText::new(combined_shared).with_runs(text_runs);
    let layout = styled.layout().clone();
    let hit_runs = Arc::new(hit_runs);

    let id_str: SharedString = SharedString::from(format!("md-line-{block_id}"));
    let down_workspace = workspace.clone();
    let down_layout = layout.clone();
    let down_hits = hit_runs.clone();

    let move_workspace = workspace.clone();
    let move_layout = layout.clone();
    let move_hits = hit_runs.clone();

    let drop_workspace = workspace.clone();
    let drop_layout = layout.clone();
    let drop_hits = hit_runs.clone();

    let caret_workspace = workspace.clone();
    let caret_layout = layout.clone();
    let caret_hits = hit_runs.clone();
    let caret_color: Hsla = hsla(theme.text);

    div()
        .id(id_str)
        .cursor(gpui::CursorStyle::IBeam)
        .relative()
        .child(styled)
        .child(CaretOverlay {
            workspace: caret_workspace,
            layout: caret_layout,
            hits: caret_hits,
            color: caret_color,
        })
        .on_drop(move |paths: &ExternalPaths, window, cx| {
            let images: Vec<&PathBuf> = paths
                .paths()
                .iter()
                .filter(|p| is_image_path(p))
                .collect();
            if images.is_empty() {
                return;
            }
            // gpui's drop listener doesn't carry the drop position —
            // pull it from the window. `index_for_position` reads
            // window-relative coords (same path the mouse-down handler
            // uses), so we can reuse `layout_index_at` directly.
            let pos = window.mouse_position();
            let Some(idx) = layout_index_at(&drop_layout, pos) else {
                return;
            };
            let source_byte = render_byte_to_source(&drop_hits, idx);
            let snippet = build_image_snippet(&images);
            drop_workspace.update(cx, |w, cx| {
                w.preview_insert_image_markup(source_byte, snippet, cx);
            });
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, w, cx| {
            let Some(idx) = layout_index_at(&down_layout, e.position) else {
                return;
            };
            // Cmd/Ctrl-click on a link opens it in the system browser.
            // Plain click keeps placing the caret so users can still
            // select text inside link runs.
            if (e.modifiers.platform || e.modifiers.control) && e.click_count == 1 {
                if let Some(href) = link_at(&down_hits, idx) {
                    // Synthetic `notepad-fn:def:<label>` links — routed
                    // to the in-document footnote-jump path instead of
                    // the system browser. Emitted by the
                    // `Event::FootnoteReference` handler so each `[^x]`
                    // ref becomes a cmd-click anchor that scrolls to
                    // `[^x]:` and drops the caret there.
                    if let Some(label) = href.strip_prefix("notepad-fn:def:") {
                        let label = label.to_string();
                        down_workspace.update(cx, |w, cx| {
                            w.preview_jump_to_footnote_def(&label, cx);
                        });
                        cx.stop_propagation();
                        return;
                    }
                    cx.open_url(&href);
                    cx.stop_propagation();
                    return;
                }
            }
            // Focus the workspace so typed characters / IME route through
            // `Workspace::EntityInputHandler::replace_text_in_range`
            // (InputFocus::Editor branch) and edit the source buffer at
            // the selection we're about to set. Without this, character
            // input has nowhere to go and the preview stays read-only
            // even though clicks place a caret.
            let handle = down_workspace.read(cx).focus_handle.clone();
            w.focus(&handle, cx);
            let source_byte = render_byte_to_source(&down_hits, idx);
            let extend = e.modifiers.shift;
            down_workspace.update(cx, |w, cx| match e.click_count {
                2 => w.preview_select_word(source_byte, cx),
                3 => w.preview_select_line(source_byte, cx),
                c if c >= 4 => w.preview_select_all(cx),
                _ => w.preview_set_caret(source_byte, extend, !extend, cx),
            });
            // Eat the event so the outer pane's `on_mouse_down` (which
            // clears the selection) doesn't fire and immediately
            // overwrite the caret / word / line we just set.
            cx.stop_propagation();
        })
        .on_mouse_move(move |e: &MouseMoveEvent, _w, cx| {
            move_workspace.update(cx, |w, cx| {
                if !w.preview_is_dragging() {
                    return;
                }
                let Some(idx) = layout_index_at(&move_layout, e.position) else {
                    return;
                };
                let source_byte = render_byte_to_source(&move_hits, idx);
                w.preview_set_caret(source_byte, true, false, cx);
            });
        })
        .into_any_element()
}

/// Compact mirror of an `InlineRun` used by the click handler to snap
/// rendered byte offsets back to source byte offsets. We only keep the
/// rendered length (so we know how much room each run occupies in the
/// combined string) and the source-range start it maps to. Selection
/// painting reuses the same vector. `link` is `Some` when this run is
/// inside a `[text](href)` so cmd/ctrl-click can open the URL.
#[derive(Clone)]
pub(crate) struct HitRun {
    rendered_len: usize,
    source_start: usize,
    source_end: usize,
    link: Option<SharedString>,
}

/// Per-block hit-test entry registered every paint by `CaretOverlay`.
/// `Workspace.preview_block_targets` accumulates one of these per
/// rendered paragraph / heading line per frame. Used by the
/// window-level mouse-move listener (drag across block gaps) and the
/// outer-pane mouse-down handler (start drag from dead space) — both
/// need pixel → source-byte mapping for blocks the per-block listener
/// doesn't cover.
#[derive(Clone)]
pub(crate) struct PreviewBlockTarget {
    pub(crate) layout: gpui::TextLayout,
    pub(crate) hits: std::sync::Arc<Vec<HitRun>>,
}

impl PreviewBlockTarget {
    /// Map a window-space pixel position to the nearest source byte
    /// inside this block. `layout.index_for_position` clamps to the
    /// closest line + column when the position is outside the layout
    /// bounds, and `render_byte_to_source` then maps the rendered
    /// byte back to the original markdown source.
    pub(crate) fn source_byte_at(
        &self,
        position: gpui::Point<gpui::Pixels>,
    ) -> Option<usize> {
        let idx = layout_index_at(&self.layout, position)?;
        Some(render_byte_to_source(&self.hits, idx))
    }
}

/// One step in the linear render stream. Inline runs and emphasis
/// markers are sorted into source-byte order before emission so the
/// combined text matches the original markdown's layout exactly.
enum RenderStep<'a> {
    /// A parsed inline run (paragraph / heading body content).
    Run(&'a InlineRun),
    /// An emphasis open marker (e.g. the leading `**` of `**bold**`)
    /// that should be painted because the caret / selection enters the
    /// styled region. Carries the literal marker text and its source
    /// byte range so click hit-testing maps the marker back to source.
    MarkerOpen { text: SharedString, range: Range<usize> },
    /// Emphasis close marker (e.g. the trailing `**`).
    MarkerClose { text: SharedString, range: Range<usize> },
}

impl<'a> RenderStep<'a> {
    /// Source byte position used as the primary sort key. Open markers
    /// sit at the open marker's start (== full_range.start); close
    /// markers sit at the close marker's start (== inner content's
    /// end); runs sit at the run's source start.
    fn sort_key(&self) -> (usize, u8) {
        // Tie-breaker `priority` orders events that share a position:
        //   0 = MarkerOpen (must precede the inner run starting here)
        //   1 = Run (inner content)
        //   2 = MarkerClose (must follow the run ending here)
        match self {
            RenderStep::MarkerOpen { range, .. } => (range.start, 0),
            RenderStep::Run(r) => (r.source_range.start, 1),
            RenderStep::MarkerClose { range, .. } => (range.start, 2),
        }
    }
}

/// True when a tag's full source region overlaps the caret position or
/// the (possibly-empty) selection — i.e. the user is editing inside the
/// styled region and should see the literal marker characters.
fn tag_is_visible(
    tag: &EmphasisTag,
    selection: Option<&Range<usize>>,
    caret: Option<usize>,
) -> bool {
    let full = tag.full_range();
    if let Some(c) = caret {
        if c >= full.start && c <= full.end {
            return true;
        }
    }
    if let Some(sel) = selection {
        // Overlap (strict for non-empty selection; inclusive endpoint
        // is fine — markers reveal even when selection just touches the
        // tag boundary).
        if sel.start <= full.end && sel.end >= full.start {
            return true;
        }
    }
    false
}

/// Build the combined text + per-run TextRuns with (a) selection
/// background painted into any run that overlaps the source-byte
/// selection range and (b) emphasis markers (`**`, `*`, `~~`,
/// `` ` ``) revealed in disabled-text colour when the caret / selection
/// touches the marker's tag region. Also returns the parallel `HitRun`
/// table the click handler uses for reverse-mapping rendered byte →
/// source byte.
#[allow(clippy::too_many_arguments)]
fn build_runs_with_selection(
    theme: &Theme,
    runs: &[InlineRun],
    tags: &[EmphasisTag],
    size_px: f32,
    base_weight: FontWeight,
    selection: Option<&Range<usize>>,
    caret: Option<usize>,
    sel_bg: Hsla,
    marker_color: Hsla,
) -> (String, Vec<TextRun>, Vec<HitRun>) {
    let mut combined = String::new();
    let mut text_runs: Vec<TextRun> = Vec::new();
    let mut hit_runs: Vec<HitRun> = Vec::new();

    // Build the linear emission order: inline runs interleaved with
    // visible emphasis markers, sorted by source byte position.
    let mut steps: Vec<RenderStep> = Vec::with_capacity(runs.len() + tags.len() * 2);
    for r in runs {
        if r.text.is_empty() {
            continue;
        }
        steps.push(RenderStep::Run(r));
    }
    for tag in tags {
        if !tag_is_visible(tag, selection, caret) {
            continue;
        }
        steps.push(RenderStep::MarkerOpen {
            text: tag.open_text.clone(),
            range: tag.open.clone(),
        });
        steps.push(RenderStep::MarkerClose {
            text: tag.close_text.clone(),
            range: tag.close.clone(),
        });
    }
    steps.sort_by_key(|s| s.sort_key());

    for step in steps {
        match step {
            RenderStep::Run(r) => {
                let rendered_len = r.text.len();
                let source_len = r.source_range.end.saturating_sub(r.source_range.start);
                let hit_selection = selection
                    .filter(|s| s.start < r.source_range.end && s.end > r.source_range.start);
                let in_selection = hit_selection.is_some();

                // Source-aligned 1:1 case: split the run into pre / sel / post on
                // matching text-byte boundaries so selection paints exactly under
                // the user-visible characters that map to the selected source
                // bytes. The is_char_boundary check guards against splitting
                // multi-byte UTF-8 codepoints — if either edge lands inside a
                // codepoint we fall through to the all-or-nothing branch.
                if let Some(sel) = hit_selection.filter(|_| rendered_len == source_len) {
                    let local_start = sel
                        .start
                        .saturating_sub(r.source_range.start)
                        .min(rendered_len);
                    let local_end = sel
                        .end
                        .saturating_sub(r.source_range.start)
                        .min(rendered_len);
                    if local_start <= local_end
                        && r.text.is_char_boundary(local_start)
                        && r.text.is_char_boundary(local_end)
                    {
                        let pre = &r.text[..local_start];
                        let mid = &r.text[local_start..local_end];
                        let post = &r.text[local_end..];
                        emit_run_slice(
                            theme,
                            r,
                            size_px,
                            base_weight,
                            pre,
                            None,
                            &mut combined,
                            &mut text_runs,
                        );
                        emit_run_slice(
                            theme,
                            r,
                            size_px,
                            base_weight,
                            mid,
                            Some(sel_bg),
                            &mut combined,
                            &mut text_runs,
                        );
                        emit_run_slice(
                            theme,
                            r,
                            size_px,
                            base_weight,
                            post,
                            None,
                            &mut combined,
                            &mut text_runs,
                        );
                        hit_runs.push(HitRun {
                            rendered_len,
                            source_start: r.source_range.start,
                            source_end: r.source_range.end,
                            link: r.link.clone().map(SharedString::from),
                        });
                        continue;
                    }
                }

                // Either no overlap or a non-1:1 source-to-render mapping — render
                // the whole run with selection bg if it overlaps at all, or no bg
                // otherwise. Coarse but visually sensible.
                let bg = if in_selection { Some(sel_bg) } else { None };
                combined.push_str(&r.text);
                text_runs.push(make_run(theme, r, size_px, base_weight, rendered_len, bg));
                hit_runs.push(HitRun {
                    rendered_len,
                    source_start: r.source_range.start,
                    source_end: r.source_range.end,
                    link: r.link.clone().map(SharedString::from),
                });
            }
            RenderStep::MarkerOpen { text, range }
            | RenderStep::MarkerClose { text, range } => {
                emit_marker_run(
                    text.as_ref(),
                    range,
                    marker_color,
                    selection,
                    sel_bg,
                    &mut combined,
                    &mut text_runs,
                    &mut hit_runs,
                );
            }
        }
    }
    (combined, text_runs, hit_runs)
}

/// Emit a marker run (the literal `**` / `*` / `~~` / `` ` `` chars).
/// Painted in `marker_color` (disabled text) so it reads as chrome
/// rather than content. If the selection overlaps the marker bytes the
/// selection background still paints — markers are part of the source
/// and selecting over them is the user's intent.
fn emit_marker_run(
    text: &str,
    source_range: Range<usize>,
    marker_color: Hsla,
    selection: Option<&Range<usize>>,
    sel_bg: Hsla,
    combined: &mut String,
    text_runs: &mut Vec<TextRun>,
    hit_runs: &mut Vec<HitRun>,
) {
    if text.is_empty() {
        return;
    }
    let in_selection = selection
        .map(|s| s.start < source_range.end && s.end > source_range.start)
        .unwrap_or(false);
    let bg = if in_selection { Some(sel_bg) } else { None };
    let rendered_len = text.len();
    combined.push_str(text);
    text_runs.push(TextRun {
        len: rendered_len,
        font: gpui::Font {
            family: crate::fonts::mono(),
            features: Default::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: gpui::FontStyle::Normal,
        },
        color: marker_color,
        background_color: bg,
        underline: None,
        strikethrough: None,
    });
    hit_runs.push(HitRun {
        rendered_len,
        source_start: source_range.start,
        source_end: source_range.end,
        link: None,
    });
}

fn emit_run_slice(
    theme: &Theme,
    r: &InlineRun,
    size_px: f32,
    base_weight: FontWeight,
    text: &str,
    bg_override: Option<Hsla>,
    combined: &mut String,
    text_runs: &mut Vec<TextRun>,
) {
    if text.is_empty() {
        return;
    }
    combined.push_str(text);
    text_runs.push(make_run(theme, r, size_px, base_weight, text.len(), bg_override));
}

/// Look up the byte index inside the combined-line string under
/// `position`. `index_for_position` returns `Ok` for an exact hit and
/// `Err` for anything past the end / above the start; both carry a
/// best-effort byte index so we accept either.
pub(crate) fn layout_index_at(
    layout: &gpui::TextLayout,
    position: gpui::Point<gpui::Pixels>,
) -> Option<usize> {
    match layout.index_for_position(position) {
        Ok(i) => Some(i),
        Err(i) => Some(i),
    }
}

/// Find the link href (if any) that covers the given rendered byte
/// inside the combined-line string. Returns the run's stored link
/// destination so cmd/ctrl-click on a link opens it externally.
fn link_at(hits: &[HitRun], rendered_byte: usize) -> Option<SharedString> {
    let mut cumulative = 0usize;
    for hit in hits {
        let next = cumulative + hit.rendered_len;
        if rendered_byte < next {
            return hit.link.clone();
        }
        cumulative = next;
    }
    None
}

/// Translate a byte index inside the rendered combined string back to
/// the matching source byte offset. Walks the parallel `HitRun` table
/// built during render.
pub(crate) fn render_byte_to_source(hits: &[HitRun], rendered_byte: usize) -> usize {
    let mut cumulative = 0usize;
    for hit in hits {
        let next = cumulative + hit.rendered_len;
        if rendered_byte < next {
            let local = rendered_byte - cumulative;
            let source_len = hit.source_end.saturating_sub(hit.source_start);
            // 1:1 source/render — direct add.
            if hit.rendered_len == source_len {
                return hit.source_start + local;
            }
            // Mismatched lengths (smart punctuation, hard breaks). Snap to
            // the start of the run; fine-grained mid-run mapping isn't
            // available without storing per-character source offsets.
            return hit.source_start;
        }
        cumulative = next;
    }
    // Past the end of the combined string — return the last run's
    // source_end (or 0 if there are no runs).
    hits.last().map(|h| h.source_end).unwrap_or(0)
}

/// Source byte → rendered byte (inverse of `render_byte_to_source`).
/// Returns `None` when the source byte falls outside every hit run.
pub(crate) fn source_byte_to_render(hits: &[HitRun], source_byte: usize) -> Option<usize> {
    let mut cumulative = 0usize;
    for hit in hits {
        if source_byte >= hit.source_start && source_byte <= hit.source_end {
            let source_len = hit.source_end.saturating_sub(hit.source_start);
            if hit.rendered_len == source_len {
                return Some(cumulative + (source_byte - hit.source_start));
            }
            // Non-1:1 mapping — snap caret to the run's start in render
            // coords. Matches the snap behaviour of click hit-testing.
            return Some(cumulative);
        }
        cumulative += hit.rendered_len;
    }
    None
}

fn make_run(
    theme: &Theme,
    r: &InlineRun,
    size_px: f32,
    base_weight: FontWeight,
    len: usize,
    bg_override: Option<Hsla>,
) -> TextRun {
    let weight = if r.bold { FontWeight::BOLD } else { base_weight };
    let style = if r.italic {
        gpui::FontStyle::Italic
    } else {
        gpui::FontStyle::Normal
    };
    let family: SharedString = crate::fonts::mono();
    // Color priority: link > syntax highlight override > inline `code`
    // chrome > body text. Links win because cmd-click affordances need to
    // read as links even inside a highlighted fence. Highlight overrides
    // beat the flat `chrome_text` so fenced-code tokens get per-kind
    // colors instead of the plain monospace fallback.
    let color = if r.link.is_some() {
        hsla(theme.link)
    } else if let Some(c) = r.color_override {
        c
    } else if r.code {
        hsla(theme.chrome_text)
    } else {
        hsla(theme.text)
    };
    // Selection > <mark> highlight > inline `code` chrome. Selection has
    // to win so the user can always see what's selected; mark wins over
    // code so a `<mark><code>x</code></mark>` shows the highlight tint
    // instead of the gray code chrome.
    let background = bg_override
        .or_else(|| {
            if r.mark {
                Some(rgba(theme.mark_highlight).into())
            } else {
                None
            }
        })
        .or_else(|| if r.code { Some(hsla(theme.chrome)) } else { None });
    // Link underline (link colour) wins over `<u>` / `<ins>` underline
    // (body text colour) when a run has both — links are the more
    // information-dense styling.
    let underline = if r.link.is_some() {
        Some(UnderlineStyle {
            color: Some(hsla(theme.link)),
            thickness: px(1.),
            wavy: false,
        })
    } else if r.underline {
        Some(UnderlineStyle {
            color: Some(hsla(theme.text)),
            thickness: px(1.),
            wavy: false,
        })
    } else {
        None
    };
    let strikethrough = if r.strike {
        Some(StrikethroughStyle {
            color: Some(hsla(theme.text)),
            thickness: px(1.),
        })
    } else {
        None
    };
    let _ = size_px;
    TextRun {
        len,
        font: gpui::Font {
            family,
            features: Default::default(),
            fallbacks: None,
            weight,
            style,
        },
        color,
        background_color: background,
        underline,
        strikethrough,
    }
}

/// True when `p` looks like an image we'd want to embed in a markdown
/// document. Extension match only — we don't sniff the file content.
/// Anything not on this list falls through to the workspace-level drop
/// handler that opens dropped files as tabs.
fn is_image_path(p: &std::path::Path) -> bool {
    let Some(ext) = p.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tiff" | "tif" | "heic"
    )
}

/// Build the markdown image snippet for a drop. Multiple files become
/// multiple `![alt](url)` lines separated by blank lines so each lands
/// as its own block. The alt text is the file stem (filename minus
/// extension); the URL is a `file://` reference to the absolute path so
/// later image-rendering can resolve it without the source file's
/// working directory.
fn build_image_snippet(paths: &[&PathBuf]) -> String {
    let mut s = String::new();
    for (i, p) in paths.iter().enumerate() {
        if i > 0 {
            s.push_str("\n\n");
        }
        let alt = p
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("image");
        let abs = std::fs::canonicalize(p).unwrap_or_else(|_| (*p).clone());
        let url = format!("file://{}", abs.display());
        s.push_str(&format!("![{}]({})", alt, url));
    }
    s
}

fn hsla(rgba_u32: u32) -> Hsla {
    // Treat all theme color values as 24-bit RGB (no alpha). Theme fields
    // that carry alpha (selection, mark_highlight) come through `rgba`
    // separately so we don't lose the high byte here.
    let r = ((rgba_u32 >> 16) & 0xFF) as u8;
    let g = ((rgba_u32 >> 8) & 0xFF) as u8;
    let b = (rgba_u32 & 0xFF) as u8;
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

// --------------------------------------------------------------------------
// CaretOverlay — Element that paints a thin vertical caret inside a
// `build_selectable_line` div, anchored to the source-byte position of
// `Tab.selection.head`. Sits as a sibling of the StyledText (the parent
// div is `relative()`), so its paint pass runs after the text layout
// has been finalised and can call `TextLayout::position_for_index` to
// look up the caret's pixel coords inside the wrapped layout.
//
// Caret is painted only when:
//   * The workspace focus handle is the focused handle (so an
//     unfocused preview pane shows no caret — matches the source
//     editor's behavior).
//   * `Tab.selection.head` falls inside this line's source-byte range
//     (one of the HitRuns covers it). Lines that don't contain the
//     caret are silent.
//   * `selection.head == selection.anchor` (no caret while a range is
//     selected — selection highlight already shows position).
//
// Blinking is driven by `workspace.caret_visible`, which the
// `spawn_caret_blink` timer in `app/window.rs` toggles on/off. The
// overlay simply reads that flag and skips painting when off.

struct CaretOverlay {
    workspace: Entity<Workspace>,
    layout: gpui::TextLayout,
    hits: Arc<Vec<HitRun>>,
    color: Hsla,
}

impl IntoElement for CaretOverlay {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for CaretOverlay {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        // Zero-size absolute overlay — we paint into the parent's bounds
        // using window-relative coords from `position_for_index`. GPUI
        // requires an Element to claim layout space; a zero-size
        // absolute element is the cheapest "no layout impact" option.
        let mut style = gpui::Style::default();
        style.position = gpui::Position::Absolute;
        style.size.width = px(0.).into();
        style.size.height = px(0.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        // Register this block as a pixel-hit target for the workspace's
        // window-level drag listener and the outer-pane mouse-down
        // handler. The vec is cleared each render frame in
        // `workspace/render.rs` before `markdown_preview_pane` runs;
        // every block then pushes once per paint.
        let target = PreviewBlockTarget {
            layout: self.layout.clone(),
            hits: self.hits.clone(),
        };
        self.workspace.update(cx, |w, _| {
            w.preview_block_targets.push(target);
        });

        let Some((focused, sel_anchor, sel_head, blink_on, scroll_pending)) = ({
            let w = self.workspace.read(cx);
            let focus_handle = w.focus_handle.clone();
            let focused = focus_handle.is_focused(window);
            w.tabs.get(w.active).map(|tab| {
                let sel = tab.selection;
                (
                    focused,
                    sel.anchor,
                    sel.head,
                    w.caret_visible,
                    w.preview_caret_scroll_pending,
                )
            })
        }) else {
            return;
        };
        if !focused {
            return;
        }
        // Resolve the caret's pixel position from `sel_head` against this
        // block's layout. Done before the no-selection / blink guards
        // because scroll-into-view still applies while the user is
        // extending a selection (shift+arrow keys, find-next that lands
        // inside a wider selection, etc.).
        let Some(rendered_byte) = source_byte_to_render(&self.hits, sel_head) else {
            // The caret is in a different block. That block's paint will
            // do the scroll work — we just leave the flag alone.
            return;
        };
        let Some(origin) = self.layout.position_for_index(rendered_byte) else {
            return;
        };
        // Caret height: use the line height from the StyledText layout.
        // `line_layout_for_index` is the only public surface for it; fall
        // back to a hardcoded 18px if the line lookup misses (which
        // shouldn't happen for a byte we just resolved via
        // position_for_index).
        let line_height = self
            .layout
            .line_layout_for_index(rendered_byte)
            .map(|_| window.line_height())
            .unwrap_or(px(18.));

        // If a keyboard / edit op asked for the caret to be brought into
        // view, apply the scroll adjustment now while we have the actual
        // caret window-coord pos. The flag is cleared either way so the
        // next user-driven scroll (wheel, scrollbar, mouse drag) doesn't
        // keep getting yanked back.
        if scroll_pending {
            self.workspace.update(cx, |w, cx| {
                let viewport = w.md_preview_scroll_handle.bounds();
                let viewport_h = viewport.size.height;
                if viewport_h > px(0.) {
                    let margin = line_height;
                    let viewport_top = viewport.origin.y;
                    let viewport_bottom = viewport_top + viewport_h;
                    let caret_top = origin.y;
                    let caret_bottom = origin.y + line_height;
                    let mut offset = w.md_preview_scroll_handle.offset();
                    let max_y = w.md_preview_scroll_handle.max_offset().height;
                    let mut changed = false;
                    if caret_top < viewport_top + margin {
                        // Caret above viewport — scroll content down by
                        // moving offset.y toward 0 (less negative).
                        let delta = (viewport_top + margin) - caret_top;
                        offset.y = (offset.y + delta).min(px(0.));
                        changed = true;
                    } else if caret_bottom > viewport_bottom - margin {
                        // Caret below viewport — scroll content up by
                        // moving offset.y toward -max_y (more negative).
                        let delta = caret_bottom - (viewport_bottom - margin);
                        offset.y = (offset.y - delta).max(-max_y);
                        changed = true;
                    }
                    if changed {
                        w.md_preview_scroll_handle.set_offset(offset);
                        cx.notify();
                    }
                }
                w.preview_caret_scroll_pending = false;
            });
        }

        if sel_anchor != sel_head {
            return;
        }
        if !blink_on {
            return;
        }
        let caret_bounds = Bounds::new(origin, size(px(1.5), line_height));
        window.paint_quad(fill(caret_bounds, self.color));
    }
}

#[cfg(test)]
mod hostile_input_tests {
    use super::*;

    /// Every selectable line the renderer would build, depth-first.
    fn collect_lines<'a>(blocks: &'a [Block], out: &mut Vec<(&'a [InlineRun], &'a [EmphasisTag])>) {
        for b in blocks {
            match b {
                Block::Heading { runs, tags, .. } | Block::Paragraph { runs, tags } => {
                    out.push((runs, tags))
                }
                Block::CodeBlock { runs, .. } => out.push((runs, &[])),
                Block::BlockQuote(inner)
                | Block::Aligned { blocks: inner, .. }
                | Block::FootnoteDefinition { blocks: inner, .. } => collect_lines(inner, out),
                Block::List { items, .. } => {
                    for item in items {
                        collect_lines(&item.blocks, out);
                    }
                }
                Block::Table { header, rows, .. } => {
                    for cell in header.iter().chain(rows.iter().flatten()) {
                        out.push((&cell.runs, &cell.tags));
                    }
                }
                Block::ParagraphWithImages { segments } => {
                    for s in segments {
                        if let ParagraphSegment::Text { runs, tags } = s {
                            out.push((runs, tags));
                        }
                    }
                }
                Block::Rule | Block::Image { .. } => {}
            }
        }
    }

    /// Parse `source` and push every line through the same run-building
    /// and hit-mapping code the renderer and click handlers use, with
    /// caret / selection positions at the start, the middle, inside
    /// multi-byte characters, at the end and past the end.
    fn exercise(source: &str) -> Vec<Block> {
        let blocks = parse(source);
        let mut lines = Vec::new();
        collect_lines(&blocks, &mut lines);
        let len = source.len();
        let probes = [0, 1, len / 2, len.saturating_sub(1), len, len + 7, usize::MAX];
        let theme = Theme::for_mode(crate::theme::ThemeMode::Dark);
        let sel_bg: Hsla = rgba(theme.selection).into();
        let marker = hsla(theme.disabled_text);
        for (runs, tags) in lines {
            for &a in &probes {
                for &b in &probes {
                    let sel = a.min(b)..a.max(b);
                    let (combined, text_runs, hits) = build_runs_with_selection(
                        &theme,
                        runs,
                        tags,
                        13.5,
                        FontWeight::NORMAL,
                        Some(&sel),
                        Some(b),
                        sel_bg,
                        marker,
                    );
                    // gpui shapes `combined` run by run: the run lengths
                    // must tile the string exactly, on char boundaries.
                    let mut at = 0usize;
                    for run in &text_runs {
                        at += run.len;
                        assert!(combined.is_char_boundary(at));
                    }
                    assert_eq!(at, combined.len());
                    // Each lookup walks every hit run, so sample long lines.
                    let step = (combined.len() / 256).max(1);
                    for i in (0..=combined.len() + 1).step_by(step).chain([combined.len() + 1]) {
                        render_byte_to_source(&hits, i);
                        let _ = link_at(&hits, i);
                    }
                    for &p in &probes {
                        if let Some(r) = source_byte_to_render(&hits, p) {
                            assert!(r <= combined.len());
                        }
                    }
                }
            }
        }
        blocks
    }

    #[test]
    fn empty_and_whitespace_documents() {
        assert!(exercise("").is_empty());
        exercise("\n");
        exercise("\r\n\r\n");
        exercise("\r\r\r");
        exercise("   \t  ");
    }

    #[test]
    fn crlf_cr_and_multibyte_content() {
        let body = "# é 中 👩\u{200d}👩\u{200d}👧\u{200d}👦\n\n- 中文 *é*\n- [x] 👍🏽 **ok**\n\n\
                    | é | 中 |\n|---|:-:|\n| 👩\u{200d}💻 | `中` |\n\n\
                    [链接 é](http://例え.jp) and https://example.com/é)\n\n> «quote» --- ...\n";
        exercise(body);
        exercise(&body.replace('\n', "\r\n"));
        exercise(&body.replace('\n', "\r"));
    }

    #[test]
    fn tables_with_mismatched_columns() {
        exercise("| a | b | c |\n|---|---|---|\n| 1 |\n| 1 | 2 | 3 | 4 | 5 |\n|\n");
        exercise("| header | only |\n|:--|--:|\n");
        exercise("| a |\n|---|\n||\n|||\n");
        exercise("a|b\n-|-\n");
        let wide = format!("{}\n{}\n{}", "|x".repeat(2_000), "|-".repeat(2_000), "|\n".repeat(200));
        exercise(&wide);
    }

    #[test]
    fn unterminated_constructs_and_missing_references() {
        exercise("```rust\nfn main() {\n    let é = \"中\";\n");
        exercise("~~~\nunterminated tilde fence");
        exercise("[missing][nope] and [also missing] and ![img][gone] and [^nofootnote]");
        exercise("**unclosed *emphasis ~~strike `code");
        exercise("<div align=\"center\">\n\nunclosed container\n");
        exercise("[^1]: definition only\n\n[^1]\n");
    }

    /// Levels of nested block lists, the top level counting as 1.
    fn nesting_depth(blocks: &[Block]) -> usize {
        blocks
            .iter()
            .map(|b| match b {
                Block::BlockQuote(inner)
                | Block::Aligned { blocks: inner, .. }
                | Block::FootnoteDefinition { blocks: inner, .. } => nesting_depth(inner),
                Block::List { items, .. } => {
                    items.iter().map(|i| nesting_depth(&i.blocks)).max().unwrap_or(0)
                }
                _ => 0,
            })
            .max()
            .map_or(0, |d| d + 1)
    }

    /// Parse a deeply nested document and check the tree stays within
    /// `MAX_BLOCK_NESTING` while the innermost text still renders.
    fn assert_nesting_bounded(source: &str) {
        let blocks = exercise(source);
        assert!(nesting_depth(&blocks) <= MAX_BLOCK_NESTING + 1);
        let mut lines = Vec::new();
        collect_lines(&blocks, &mut lines);
        assert!(lines.iter().any(|(runs, _)| runs.iter().any(|r| r.text.contains("deep"))));
    }

    #[test]
    fn deeply_nested_blockquotes() {
        assert_nesting_bounded(&format!("{}deep", "> ".repeat(10_000)));
        assert_nesting_bounded(&format!("{}deep\n", ">".repeat(10_000)));
    }

    #[test]
    fn deeply_nested_lists() {
        assert_nesting_bounded(&format!("{}deep", "- ".repeat(10_000)));
        assert_nesting_bounded(&format!("{}deep", "1. ".repeat(10_000)));
        assert_nesting_bounded(&format!("{}deep", "* > ".repeat(5_000)));
        // One item per line, each indented one level deeper.
        let stairs: String = (0..2_000).map(|i| format!("{}- deep\n", "  ".repeat(i))).collect();
        assert_nesting_bounded(&stairs);
    }

    #[test]
    fn deeply_nested_footnote_definitions() {
        assert_nesting_bounded(&format!("{}deep\n", "[^a]: > ".repeat(5_000)));
    }

    #[test]
    fn footnote_refs_inside_many_open_html_links() {
        // Each reference used to copy the whole stack of open `<a>` links.
        let source = format!(
            "{}{}\n\n[^1]: note\n",
            "<a href=\"u\">".repeat(60_000),
            "[^1] ".repeat(60_000)
        );
        let blocks = parse(&source);
        let mut lines = Vec::new();
        collect_lines(&blocks, &mut lines);
        let refs = lines
            .iter()
            .flat_map(|(runs, _)| runs.iter())
            .filter(|r| r.link.as_deref() == Some("notepad-fn:def:1"))
            .count();
        assert_eq!(refs, 60_000);
    }

    #[test]
    fn image_reads_only_regular_files() {
        let dir = std::env::temp_dir().join(format!("notepad-studio-md-image-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("a.png");
        std::fs::write(&png, b"\x89PNG bytes").unwrap();
        assert_eq!(read_image_file(&png), Ok(b"\x89PNG bytes".to_vec()));
        assert_eq!(read_image_file(&dir), Err("not a file"));
        assert_eq!(read_image_file(&dir.join("missing.png")), Err("file not found"));
        #[cfg(unix)]
        {
            // Reading this to the end used to fill memory until the
            // process died.
            let zero = dir.join("zero.png");
            let _ = std::fs::remove_file(&zero);
            std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
            assert_eq!(read_image_file(&zero), Err("not a file"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overlapping_highlight_spans() {
        // `syntax::highlight` returns disjoint spans; a nested span ending
        // before the current run used to underflow the run-local end.
        use crate::syntax::{HighlightKind, HighlightSpan};
        let run = |text: &str, start: usize| InlineRun {
            text: text.to_string(),
            source_range: start..start + text.len(),
            bold: false,
            italic: false,
            strike: false,
            underline: false,
            mark: false,
            code: false,
            link: None,
            color_override: None,
        };
        let runs = [run("abc", 10), run("def", 13), run("é", 16)];
        let spans = [
            HighlightSpan { range: 0..5, kind: HighlightKind::String },
            HighlightSpan { range: 1..2, kind: HighlightKind::Keyword },
            HighlightSpan { range: 5..8, kind: HighlightKind::Comment },
        ];
        let theme = Theme::for_mode(crate::theme::ThemeMode::Dark);
        let out = split_runs_with_highlights(&runs, &spans, &theme.syntax);
        let text: String = out.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "abcdefé");
        let mut at = 10;
        for r in &out {
            assert_eq!(r.source_range, at..at + r.text.len());
            at = r.source_range.end;
        }
    }

    /// Emphasis tags of a one-paragraph document, ordered by open marker.
    fn paragraph_tags(source: &str) -> Vec<EmphasisTag> {
        let blocks = parse(source);
        let [Block::Paragraph { tags, .. }] = blocks.as_slice() else {
            panic!("expected one paragraph");
        };
        let mut tags = tags.clone();
        tags.sort_by_key(|t| t.open.start);
        tags
    }

    fn markers(source: &str) -> Vec<(Range<usize>, String, Range<usize>, String)> {
        paragraph_tags(source)
            .into_iter()
            .map(|t| (t.open, t.open_text.to_string(), t.close, t.close_text.to_string()))
            .collect()
    }

    #[test]
    fn nested_emphasis_markers_stay_their_own_size() {
        // An open marker used to run on to the first text, swallowing the
        // markers of every tag nested inside it, so N nested tags held
        // marker text quadratic in N.
        assert_eq!(
            markers("***a***"),
            vec![
                (0..1, "*".to_string(), 6..7, "*".to_string()),
                (1..3, "**".to_string(), 4..6, "**".to_string()),
            ]
        );
        let n = 20_000;
        let tags = paragraph_tags(&format!("{}x{}", "*".repeat(n), "*".repeat(n)));
        let marker_bytes: usize = tags.iter().map(|t| t.open_text.len() + t.close_text.len()).sum();
        assert_eq!(marker_bytes, 2 * n);
    }

    #[test]
    fn emphasis_marker_ranges() {
        assert_eq!(
            markers("**a *b* c**"),
            vec![
                (0..2, "**".to_string(), 9..11, "**".to_string()),
                (4..5, "*".to_string(), 6..7, "*".to_string()),
            ]
        );
        assert_eq!(
            markers("*a* and ~~b~~"),
            vec![
                (0..1, "*".to_string(), 2..3, "*".to_string()),
                (8..10, "~~".to_string(), 11..13, "~~".to_string()),
            ]
        );
    }

    #[test]
    fn nested_emphasis_spanning_many_lines() {
        // Each text event used to update every open emphasis tag.
        let n = 200_000;
        let source = format!("{}{}x{}", "*".repeat(n), "x\n".repeat(n), "*".repeat(n));
        let blocks = parse(&source);
        let [Block::Paragraph { tags, .. }] = blocks.as_slice() else {
            panic!("expected one paragraph");
        };
        assert_eq!(tags.len(), n / 2);
        // The innermost tag's markers hug the text on both sides.
        let inner = tags.iter().min_by_key(|t| t.close.start - t.open.end).unwrap();
        assert_eq!(&source[inner.open.end..inner.close.start], format!("{}x", "x\n".repeat(n)));
        // The outermost tag spans the whole paragraph.
        assert!(tags.iter().any(|t| t.open.start == 0 && t.close.end == source.len()));
    }

    #[test]
    fn deeply_nested_emphasis() {
        exercise(&format!("{}x{}", "*".repeat(10_000), "*".repeat(10_000)));
        exercise(&format!("{}x{}", "_**~~".repeat(2_000), "~~**_".repeat(2_000)));
    }

    #[test]
    fn deeply_nested_links_and_brackets() {
        exercise(&format!("{}x{}", "[".repeat(10_000), "](u)".repeat(10_000)));
        exercise(&format!("{}x{}", "[".repeat(10_000), "]".repeat(10_000)));
    }

    #[test]
    fn deeply_nested_images() {
        exercise(&format!("{}x{}", "![".repeat(5_000), "](i.png)".repeat(5_000)));
    }

    #[test]
    fn deeply_nested_inline_html() {
        exercise(&format!("{}x{}", "<b><i><a href=\"u\">".repeat(3_000), "</a></i></b>".repeat(3_000)));
    }

    #[test]
    fn long_backtick_runs() {
        exercise(&"`".repeat(10_000));
    }

    #[test]
    fn deeply_nested_html_blocks() {
        exercise(&format!("{}x\n\n{}", "<div>\n\n".repeat(10_000), "</div>\n\n".repeat(10_000)));
        exercise(&format!("{}x\n\n{}", "<center>\n\n".repeat(10_000), "</center>\n\n".repeat(10_000)));
        exercise(&format!(
            "{}x\n\n{}",
            "<div align=\"right\">\n\n".repeat(10_000),
            "</div>\n\n".repeat(10_000)
        ));
    }
}

