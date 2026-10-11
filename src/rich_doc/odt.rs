//! ODT parser + writer. Phase 3b: open + render + whole-block bold/italic/underline
//! + container-replay save. Mirrors `docx.rs` 1:1 — same zip preservation strategy,
//! same `original_bytes` byte-perfect no-edit save, same hand-rolled XML emitter.
//!
//! Library choice: hand-rolled `quick-xml` (already a direct dep). ODFerrous is
//! unpublished + brand new (design risk §9); falling back avoids depending on an
//! immature library for a v1 feature.
//!
//! Subset implemented:
//! - `<text:p>` → `Block::Paragraph`
//! - `<text:h text:outline-level="N">` → `Block::Heading { level: N }` (clamped 1..=6)
//! - `<text:span>` resolved against pre-scanned automatic-styles map for bold,
//!   italic, underline
//! - Bare text nodes inside paragraphs/headings inherit default (no formatting)
//!
//! Out of scope for Phase 3b: tables, lists, frames/images, font color/size,
//! sub-range selection, header/footer pages.
//!
//! Save modes:
//! - empty `edit_log` → write `original_bytes` byte-for-byte
//! - non-empty `edit_log` → re-serialize `content.xml` from AST, preserve the
//!   original `<office:document-content>` opening tag (namespaces) and
//!   `<office:automatic-styles>` block (we append our own auto-style entries
//!   for runs that need bold/italic/underline), swap into the original zip via
//!   `container::replace_zip_entry`

use super::{ast, container, DocFormat, RichDocError, RichDocPayload, Result};
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

/// Auto-style names we emit for body runs that need formatting. Deterministic
/// so re-serializing the same AST twice produces identical XML.
const STYLE_BOLD: &str = "__rd_bold";
const STYLE_ITALIC: &str = "__rd_italic";
const STYLE_UNDERLINE: &str = "__rd_underline";
const STYLE_BOLD_ITALIC: &str = "__rd_bold_italic";
const STYLE_BOLD_UNDERLINE: &str = "__rd_bold_underline";
const STYLE_ITALIC_UNDERLINE: &str = "__rd_italic_underline";
const STYLE_ALL: &str = "__rd_bold_italic_underline";

/// Auto-style names for our minted bullet / numbered list-style entries. Same
/// rationale as the run-style constants above — deterministic round-trip.
const LIST_STYLE_BULLET: &str = "__rd_list_bullet";
const LIST_STYLE_NUMBER: &str = "__rd_list_number";

/// Bound expanded space runs before allocating their text.
const MAX_SPACE_COUNT: usize = u16::MAX as usize;

pub fn open(path: &Path) -> Result<RichDocPayload> {
    let original_bytes = std::fs::read(path)?;
    let content_xml = container::read_zip_entry_text(path, "content.xml")?;
    let styles_xml = container::read_zip_entry_text(path, "styles.xml").unwrap_or_default();
    let ast = parse_with_path(&content_xml, &styles_xml, Some(path))?;
    let raw_view_text = pretty_print_xml(&content_xml);
    let (headers, footers) = parse_master_pages(&styles_xml, &content_xml, Some(path));
    Ok(RichDocPayload {
        format: DocFormat::Odt,
        ast,
        raw_view_text,
        original_bytes,
        edit_log: Vec::new(),
        active_block: None,
        selection: None,
        drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        headers,
        footers,
    })
}

pub fn save(payload: &RichDocPayload, path: &Path) -> Result<()> {
    if payload.edit_log.is_empty() {
        super::write_document_bytes(path, &payload.original_bytes)?;
        return Ok(());
    }
    super::ensure_text_only(&payload.ast)?;
    let new_content = serialize_content_xml(&payload.ast, &payload.raw_view_text);
    let new_zip = container::replace_zip_entry(
        &payload.original_bytes,
        "content.xml",
        new_content.as_bytes(),
    )?;
    super::write_document_bytes(path, &new_zip)?;
    Ok(())
}

/// Parse content.xml + styles.xml into an AST.
///
/// Two-pass strategy:
/// 1. Pre-scan both `<office:automatic-styles>` blocks (in content.xml and
///    styles.xml) and collect every `<style:style style:name="X" ...>` into a
///    map: style-name → InlineStyle.
/// 2. Walk `<office:body><office:text>` emitting Paragraphs / Headings /
///    Tables / Images. `<text:span text:style-name="X">` runs resolve their
///    style via the map.
fn parse_content_xml(content_xml: &str, styles_xml: &str) -> Result<ast::Document> {
    parse_with_path(content_xml, styles_xml, None)
}

/// Same as `parse_content_xml` but also extracts inline image bytes when a
/// `path` to the .odt zip is supplied. Tests call with `None` to keep parsing
/// pure (no file I/O); production goes through `open()` which always passes
/// the path.
fn parse_with_path(
    content_xml: &str,
    styles_xml: &str,
    zip_path: Option<&Path>,
) -> Result<ast::Document> {
    let mut raw_styles: HashMap<String, RawOdtStyle> = HashMap::new();
    let mut list_style_map: HashMap<String, bool> = HashMap::new();
    collect_raw_styles_into(&mut raw_styles, content_xml)?;
    collect_list_styles_into(&mut list_style_map, content_xml)?;
    if !styles_xml.is_empty() {
        collect_raw_styles_into(&mut raw_styles, styles_xml)?;
        collect_list_styles_into(&mut list_style_map, styles_xml)?;
    }
    let (style_map, para_style_map) = resolve_raw_to_maps(&raw_styles);
    let blocks = parse_body_blocks(
        content_xml,
        &style_map,
        &para_style_map,
        &list_style_map,
        zip_path,
    )?;
    Ok(ast::Document { blocks })
}

/// Per-style partial properties harvested from a single `<style:style>` entry.
/// `None` on a field means "not declared on this style" — the resolver will
/// inherit from the parent chain. `Some` means "declared in XML" (including
/// `Some(false)` for an explicit `font-weight="normal"`, which must override an
/// inherited bold). `parent` carries the value of `style:parent-style-name`
/// when present; the resolver walks parents transitively until a missing
/// parent, an empty name, or a cycle stops it.
#[derive(Debug, Clone, Default)]
struct RawOdtStyle {
    parent: Option<String>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    color: Option<u32>,
    alignment: Option<ast::Alignment>,
    indent_level: Option<u8>,
}

/// Walk an XML doc and harvest every `<style:style style:name="X" ...>` entry
/// into `raw_map` as `name → RawOdtStyle`. Properties are stored as
/// `Option<T>` so the resolver can later distinguish "explicitly set" from
/// "inherit from parent". `style:parent-style-name` is captured for chain
/// resolution.
///
/// When the same style name is declared in both `content.xml` and
/// `styles.xml`, the latter call wins for every field — last declaration
/// replaces the entry wholesale. This matches LibreOffice's "common styles
/// shadow automatic styles of the same name" behavior.
fn collect_raw_styles_into(raw_map: &mut HashMap<String, RawOdtStyle>, xml: &str) -> Result<()> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut cur: RawOdtStyle = RawOdtStyle::default();
    let mut depth_in_style: u32 = 0;
    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(RichDocError::Odt(format!("style scan: {e}"))),
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if name == b"style" {
                    if let Some(n) = read_attr(e, "style:name") {
                        cur_name = Some(n);
                        cur = RawOdtStyle::default();
                        cur.parent = read_attr(e, "style:parent-style-name")
                            .filter(|s| !s.is_empty());
                        depth_in_style = 1;
                        continue;
                    }
                }
                if depth_in_style > 0 {
                    depth_in_style += 1;
                    if name == b"text-properties" {
                        absorb_text_properties_raw(e, &mut cur);
                    }
                    if name == b"paragraph-properties" {
                        absorb_paragraph_properties_raw(e, &mut cur);
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if depth_in_style > 0 && name == b"text-properties" {
                    absorb_text_properties_raw(e, &mut cur);
                }
                if depth_in_style > 0 && name == b"paragraph-properties" {
                    absorb_paragraph_properties_raw(e, &mut cur);
                }
            }
            Ok(Event::End(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if depth_in_style > 0 {
                    depth_in_style -= 1;
                    if depth_in_style == 0 && name == b"style" {
                        if let Some(n) = cur_name.take() {
                            raw_map.insert(n, std::mem::take(&mut cur));
                        }
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// Resolve every entry in `raw` into a final `(style_map, para_style_map)` by
/// walking each style's `parent-style-name` chain and merging child-over-parent
/// (child wins; parent only fills fields the child did NOT set).
///
/// * Chain stops on: empty / missing `parent`, parent name not in `raw`, or a
///   cycle (detected via a per-resolve `HashSet`). Cycles never panic and
///   never loop infinitely — the walk halts at the first repeated name and
///   returns whatever was accumulated up to that point.
/// * `style_map` always gets an entry for every style (even one with no
///   properties at all) — the renderer falls back to `unwrap_or_default()` on
///   the InlineStyle, and presence-in-map already governs "did we know this
///   style at all" for span resolution.
/// * `para_style_map` only gets an entry when the resolved style ended up
///   with at least one paragraph property set, matching the previous
///   "props_dirty" gate so the renderer can distinguish "no paragraph style"
///   from "paragraph style with all defaults".
fn resolve_raw_to_maps(
    raw: &HashMap<String, RawOdtStyle>,
) -> (
    HashMap<String, ast::InlineStyle>,
    HashMap<String, ast::BlockProps>,
) {
    let mut style_map: HashMap<String, ast::InlineStyle> = HashMap::new();
    let mut para_style_map: HashMap<String, ast::BlockProps> = HashMap::new();
    for name in raw.keys() {
        let effective = resolve_chain(name, raw);
        let inline = ast::InlineStyle {
            bold: effective.bold.unwrap_or(false),
            italic: effective.italic.unwrap_or(false),
            underline: effective.underline.unwrap_or(false),
            strike: effective.strike.unwrap_or(false),
            color: effective.color,
            link: None,
        };
        style_map.insert(name.clone(), inline);
        if effective.alignment.is_some() || effective.indent_level.is_some() {
            let props = ast::BlockProps {
                alignment: effective.alignment.unwrap_or_default(),
                indent_level: effective.indent_level.unwrap_or(0),
                list: None,
            };
            para_style_map.insert(name.clone(), props);
        }
    }
    (style_map, para_style_map)
}

/// Walk the parent chain starting at `start`, accumulating properties with
/// child-wins semantics. Cycle detection via owned-`String` HashSet (chains
/// are typically 1–3 deep so the per-step allocation is negligible).
fn resolve_chain(start: &str, raw: &HashMap<String, RawOdtStyle>) -> RawOdtStyle {
    let mut seen: HashSet<String> = HashSet::new();
    let mut acc = RawOdtStyle::default();
    let mut cur: String = start.to_string();
    loop {
        if !seen.insert(cur.clone()) {
            break;
        }
        let Some(entry) = raw.get(&cur) else {
            break;
        };
        if acc.bold.is_none() {
            acc.bold = entry.bold;
        }
        if acc.italic.is_none() {
            acc.italic = entry.italic;
        }
        if acc.underline.is_none() {
            acc.underline = entry.underline;
        }
        if acc.strike.is_none() {
            acc.strike = entry.strike;
        }
        if acc.color.is_none() {
            acc.color = entry.color;
        }
        if acc.alignment.is_none() {
            acc.alignment = entry.alignment;
        }
        if acc.indent_level.is_none() {
            acc.indent_level = entry.indent_level;
        }
        match entry.parent.as_deref() {
            Some(p) if !p.is_empty() => cur = p.to_string(),
            _ => break,
        }
    }
    acc.parent = None;
    acc
}

/// Walk an XML doc and harvest every `<text:list-style style:name="X">` into
/// `map` as `name → ordered-flag`. The flag is decided by the first level-style
/// child encountered: `<text:list-level-style-number>` → ordered (true),
/// `<text:list-level-style-bullet>` → unordered (false). Styles with no level
/// children default to bullet (false) since that matches LibreOffice's
/// behavior when a style is referenced but empty.
fn collect_list_styles_into(map: &mut HashMap<String, bool>, xml: &str) -> Result<()> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut first_kind: Option<bool> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(RichDocError::Odt(format!("list style scan: {e}"))),
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) => {
                let raw = e.name();
                let name = local_name(raw.as_ref());
                let prefix = local_prefix(raw.as_ref());
                if prefix == Some(b"text") && name == b"list-style" {
                    cur_name = read_attr(e, "style:name");
                    first_kind = None;
                } else if cur_name.is_some()
                    && prefix == Some(b"text")
                    && first_kind.is_none()
                {
                    match name {
                        b"list-level-style-bullet" => first_kind = Some(false),
                        b"list-level-style-number" => first_kind = Some(true),
                        _ => {}
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let raw = e.name();
                let name = local_name(raw.as_ref());
                let prefix = local_prefix(raw.as_ref());
                if cur_name.is_some()
                    && prefix == Some(b"text")
                    && first_kind.is_none()
                {
                    match name {
                        b"list-level-style-bullet" => first_kind = Some(false),
                        b"list-level-style-number" => first_kind = Some(true),
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let raw = e.name();
                let name = local_name(raw.as_ref());
                let prefix = local_prefix(raw.as_ref());
                if prefix == Some(b"text") && name == b"list-style" {
                    if let Some(n) = cur_name.take() {
                        map.insert(n, first_kind.unwrap_or(false));
                    }
                    first_kind = None;
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// Pull `fo:text-align` and `fo:margin-left` from a `<style:paragraph-properties>`
/// element into the raw style's Option fields. Only sets a field when the
/// corresponding attribute is present and well-formed, so the resolver can
/// later distinguish "child explicitly aligned left" from "child silent —
/// inherit parent alignment".
fn absorb_paragraph_properties_raw(e: &BytesStart, raw: &mut RawOdtStyle) {
    for attr in e.attributes().flatten() {
        let key = attr.key.as_ref();
        let local = local_name(key);
        let val = attr.value.as_ref();
        match local {
            b"text-align" => {
                let v = std::str::from_utf8(val).unwrap_or("");
                let align = match v {
                    "center" => ast::Alignment::Center,
                    "end" | "right" => ast::Alignment::Right,
                    "justify" => ast::Alignment::Justify,
                    "start" | "left" => ast::Alignment::Left,
                    _ => continue,
                };
                raw.alignment = Some(align);
            }
            b"margin-left" => {
                let v = std::str::from_utf8(val).unwrap_or("");
                if let Some(cm) = parse_odf_length_cm(v) {
                    // 0.5 inch = 1.27cm. Round to nearest level.
                    let level = (cm / 1.27).round().max(0.0) as u32;
                    raw.indent_level = Some(level.min(255) as u8);
                }
            }
            _ => {}
        }
    }
}

/// Parse a CSS-style length like `"1.27cm"`, `"36pt"`, `"0.5in"`, or `"720twip"`
/// into centimeters. Returns `None` for empty, zero, or unrecognized units.
fn parse_odf_length_cm(v: &str) -> Option<f32> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    let (num_part, unit) = v.split_at(v.find(|c: char| c.is_alphabetic()).unwrap_or(v.len()));
    let num: f32 = num_part.trim().parse().ok()?;
    let cm = match unit {
        "cm" => num,
        "mm" => num / 10.0,
        "in" => num * 2.54,
        "pt" => num * 2.54 / 72.0,
        "pc" => num * 12.0 * 2.54 / 72.0,
        "twip" => num * 2.54 / 1440.0,
        "" => return None,
        _ => return None,
    };
    Some(cm)
}

fn alignment_to_odt(a: ast::Alignment) -> &'static str {
    match a {
        ast::Alignment::Left => "start",
        ast::Alignment::Center => "center",
        ast::Alignment::Right => "end",
        ast::Alignment::Justify => "justify",
    }
}

/// Pull bold / italic / underline / strike / color from a
/// `<style:text-properties>` element into the raw style. Each attribute
/// produces `Some(_)` (including `Some(false)` for an explicit
/// `font-weight="normal"`, `text-underline-style="none"`, etc.) so the
/// resolver knows the child explicitly turned it off and the inherited
/// parent value must NOT leak through.
fn absorb_text_properties_raw(e: &BytesStart, raw: &mut RawOdtStyle) {
    for attr in e.attributes().flatten() {
        let key = attr.key.as_ref();
        let local = local_name(key);
        let val = attr.value.as_ref();
        match local {
            // ODF font-weight: "normal" | "bold" | numeric ≥ 700 = bold
            b"font-weight" => {
                let v = std::str::from_utf8(val).unwrap_or("");
                raw.bold = Some(is_bold_weight(v));
            }
            b"font-style" => {
                let v = std::str::from_utf8(val).unwrap_or("");
                raw.italic = Some(matches!(v, "italic" | "oblique"));
            }
            // `style:text-underline-style` — any value except `none` means underlined.
            b"text-underline-style" => {
                let v = std::str::from_utf8(val).unwrap_or("");
                raw.underline = Some(!v.is_empty() && v != "none");
            }
            // `style:text-line-through-style` — any value except `none` means struck-through.
            b"text-line-through-style" => {
                let v = std::str::from_utf8(val).unwrap_or("");
                raw.strike = Some(!v.is_empty() && v != "none");
            }
            // `fo:color` — `#RRGGBB` or the keyword `transparent`. "transparent"
            // resolves to `None`, which keeps the field unset on this child —
            // an inherited color from a parent style is then allowed through
            // (matches ODF's "reset to default" semantics for transparent).
            b"color" => {
                let v = std::str::from_utf8(val).unwrap_or("");
                if let Some(rgb) = parse_odf_color(v) {
                    raw.color = Some(rgb);
                }
            }
            _ => {}
        }
    }
}

/// Parse an ODF `fo:color` value (`#RRGGBB` or `transparent`) into 0xRRGGBB.
fn parse_odf_color(v: &str) -> Option<u32> {
    let v = v.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("transparent") {
        return None;
    }
    let hex = v.trim_start_matches('#');
    u32::from_str_radix(hex, 16).ok().filter(|&n| n <= 0xFF_FFFF)
}

fn is_bold_weight(v: &str) -> bool {
    if v == "bold" || v == "bolder" {
        return true;
    }
    if let Ok(n) = v.parse::<u32>() {
        return n >= 700;
    }
    false
}

/// Walk `<office:body><office:text>` and emit blocks. Span style-names are
/// resolved through `style_map`; styles not present in the map contribute no
/// formatting (matches reader leniency expectations — missing style = plain).
/// `zip_path` (when present) lets the walker read image bytes for `<draw:frame>`
/// elements; in tests it's None and images are skipped.
fn parse_body_blocks(
    content_xml: &str,
    style_map: &HashMap<String, ast::InlineStyle>,
    para_style_map: &HashMap<String, ast::BlockProps>,
    list_style_map: &HashMap<String, bool>,
    zip_path: Option<&Path>,
) -> Result<Vec<ast::Block>> {
    let mut reader = Reader::from_str(content_xml);
    reader.config_mut().trim_text(false);
    let mut walker = BodyWalker::new(style_map, para_style_map, list_style_map, zip_path);
    walker.advance_to_body(&mut reader)?;
    walker.walk_until_body_end(&mut reader)?;
    Ok(walker.blocks)
}

/// Same as `parse_body_blocks` but on a content fragment that doesn't have
/// an enclosing `<office:body><office:text>` wrapper — used for header/footer
/// XML (extracted from `<style:header>` / `<style:footer>`). Wraps the
/// fragment to satisfy the body walker's state machine.
fn parse_fragment_blocks(
    fragment: &str,
    style_map: &HashMap<String, ast::InlineStyle>,
    para_style_map: &HashMap<String, ast::BlockProps>,
    list_style_map: &HashMap<String, bool>,
    zip_path: Option<&Path>,
) -> Vec<ast::Block> {
    // Borrow the same namespace decls as a real content.xml so qualified names
    // (`text:p`, `text:span`, `table:table`, `draw:frame`) resolve correctly
    // during the walk.
    let wrapped = format!(
        "<office:document-content \
         xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
         xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
         xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
         xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" \
         xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\">\
         <office:body><office:text>{fragment}</office:text></office:body>\
         </office:document-content>"
    );
    parse_body_blocks(&wrapped, style_map, para_style_map, list_style_map, zip_path)
        .unwrap_or_default()
}

/// In-flight walker state — extracted so the recursive table-cell parser can
/// reuse the same span/run accumulation logic as the top-level body walker.
struct BodyWalker<'a> {
    style_map: &'a HashMap<String, ast::InlineStyle>,
    para_style_map: &'a HashMap<String, ast::BlockProps>,
    list_style_map: &'a HashMap<String, bool>,
    zip_path: Option<&'a Path>,
    blocks: Vec<ast::Block>,
    span_stack: Vec<ast::InlineStyle>,
    cur_runs: Vec<ast::InlineRun>,
    cur_outline: Option<u8>,
    cur_props: ast::BlockProps,
    in_block: bool,
    /// Nesting stack of currently-open `<text:list>` containers. `level` of a
    /// paragraph inside a list is the index of its enclosing list on the stack
    /// (0-based: outermost = 0). `ordered` is decided per list element by
    /// looking the `text:style-name` up in `list_style_map`.
    list_stack: Vec<ast::ListMarker>,
}

impl<'a> BodyWalker<'a> {
    fn new(
        style_map: &'a HashMap<String, ast::InlineStyle>,
        para_style_map: &'a HashMap<String, ast::BlockProps>,
        list_style_map: &'a HashMap<String, bool>,
        zip_path: Option<&'a Path>,
    ) -> Self {
        Self {
            style_map,
            para_style_map,
            list_style_map,
            zip_path,
            blocks: Vec::new(),
            span_stack: Vec::new(),
            cur_runs: Vec::new(),
            cur_outline: None,
            cur_props: ast::BlockProps::default(),
            in_block: false,
            list_stack: Vec::new(),
        }
    }

    /// Forward the reader to the `<office:text>` opening event so the main
    /// walker can start its loop in a known state.
    fn advance_to_body(&mut self, reader: &mut Reader<&[u8]>) -> Result<()> {
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Err(e) => return Err(RichDocError::Odt(format!("seek body: {e}"))),
                Ok(Event::Eof) => return Ok(()),
                Ok(Event::Start(ref e)) => {
                    let raw = e.name();
                    if local_name(raw.as_ref()) == b"text"
                        && local_prefix(raw.as_ref()) == Some(b"office")
                    {
                        buf.clear();
                        return Ok(());
                    }
                }
                _ => {}
            }
            buf.clear();
        }
    }

    fn walk_until_body_end(&mut self, reader: &mut Reader<&[u8]>) -> Result<()> {
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Err(e) => return Err(RichDocError::Odt(format!("body parse: {e}"))),
                Ok(Event::Eof) => break,
                Ok(Event::Start(ref e)) => {
                    let raw = e.name();
                    let name = local_name(raw.as_ref());
                    let prefix = local_prefix(raw.as_ref());
                    if name == b"text" && prefix == Some(b"office") {
                        break;
                    }
                    match name {
                        b"p" => {
                            self.in_block = true;
                            self.cur_outline = None;
                            self.cur_runs.clear();
                            self.cur_props = read_attr(e, "text:style-name")
                                .and_then(|n| self.para_style_map.get(&n).cloned())
                                .unwrap_or_default();
                            if let Some(marker) = self.list_stack.last().copied() {
                                self.cur_props.list = Some(marker);
                            }
                        }
                        b"h" => {
                            self.in_block = true;
                            self.cur_runs.clear();
                            self.cur_props = read_attr(e, "text:style-name")
                                .and_then(|n| self.para_style_map.get(&n).cloned())
                                .unwrap_or_default();
                            if let Some(marker) = self.list_stack.last().copied() {
                                self.cur_props.list = Some(marker);
                            }
                            let lvl = read_attr(e, "text:outline-level")
                                .and_then(|s| s.parse::<u8>().ok())
                                .unwrap_or(1);
                            self.cur_outline = Some(lvl.clamp(1, 6));
                        }
                        b"list" if prefix == Some(b"text") => {
                            // Outermost `<text:list>` = level 0; subsequent
                            // nestings increment. Style-name lookup picks
                            // bullet-vs-numbered; unknown styles default to
                            // bullet (matches list_style_map default).
                            let style_name =
                                read_attr(e, "text:style-name").unwrap_or_default();
                            let ordered = self
                                .list_style_map
                                .get(&style_name)
                                .copied()
                                .unwrap_or(false);
                            let level = self.list_stack.len().min(255) as u8;
                            self.list_stack
                                .push(ast::ListMarker { ordered, level });
                        }
                        b"span" if self.in_block => {
                            let style_name = read_attr(e, "text:style-name").unwrap_or_default();
                            let base = self.span_stack.last().cloned().unwrap_or_default();
                            let resolved =
                                self.style_map.get(&style_name).cloned().unwrap_or_default();
                            let merged = merge_styles(&base, &resolved);
                            self.span_stack.push(merged);
                        }
                        b"a" if self.in_block && prefix == Some(b"text") => {
                            // `<text:a xlink:href="...">` introduces an inline
                            // hyperlink scope. Treat it like a span that adds
                            // a link target without changing other style flags.
                            let href = read_attr(e, "xlink:href").unwrap_or_default();
                            let base = self.span_stack.last().cloned().unwrap_or_default();
                            let mut overlay = ast::InlineStyle::default();
                            if !href.is_empty() {
                                overlay.link = Some(href);
                            }
                            self.span_stack.push(merge_styles(&base, &overlay));
                        }
                        b"table" if prefix == Some(b"table") => {
                            // Tables sit between paragraphs in ODT — flush any
                            // in-progress paragraph first so the table block
                            // lands in document order.
                            self.flush_current_block();
                            let table = self.parse_table(reader)?;
                            self.blocks.push(table);
                        }
                        b"frame" if prefix == Some(b"draw") => {
                            // Frame appearing at the body level (not inside a
                            // paragraph) holds an image or shape. Skip the
                            // entire subtree, snatching the first image bytes
                            // we encounter inside it.
                            if let Some(img) = self.consume_frame(reader, e)? {
                                self.flush_current_block();
                                self.blocks.push(img);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let raw = e.name();
                    let name = local_name(raw.as_ref());
                    let prefix = local_prefix(raw.as_ref());
                    if self.in_block && name == b"tab" {
                        push_run(&mut self.cur_runs, &self.span_stack, "\t");
                    }
                    if self.in_block && name == b"line-break" {
                        push_run(&mut self.cur_runs, &self.span_stack, "\n");
                    }
                    if self.in_block && name == b"s" {
                        let count = read_attr(e, "text:c");
                        let n = count.as_deref().unwrap_or("1").parse::<usize>()
                            .map_err(|_| RichDocError::Odt("invalid text:s space count".into()))?;
                        if n > MAX_SPACE_COUNT {
                            return Err(RichDocError::Odt(format!("space run exceeds the {MAX_SPACE_COUNT}-space limit")));
                        }
                        let spaces = " ".repeat(n);
                        push_run(&mut self.cur_runs, &self.span_stack, &spaces);
                    }
                    if name == b"frame" && prefix == Some(b"draw") {
                        // Self-closing draw:frame is uncommon (frame usually
                        // wraps draw:image as a child) but technically valid.
                        // Nothing to extract since no inner href.
                    }
                }
                Ok(Event::Text(ref t)) => {
                    if !self.in_block {
                        continue;
                    }
                    let text = t
                        .unescape()
                        .map_err(|e| RichDocError::Odt(format!("text decode: {e}")))?
                        .into_owned();
                    if text.is_empty() {
                        continue;
                    }
                    push_run(&mut self.cur_runs, &self.span_stack, &text);
                }
                Ok(Event::End(ref e)) => {
                    let raw = e.name();
                    let name = local_name(raw.as_ref());
                    let prefix = local_prefix(raw.as_ref());
                    if name == b"text" && prefix == Some(b"office") {
                        break;
                    }
                    match name {
                        b"p" | b"h" => {
                            self.flush_current_block();
                            self.in_block = false;
                            self.cur_outline = None;
                            self.span_stack.clear();
                        }
                        b"span" if self.in_block => {
                            self.span_stack.pop();
                        }
                        b"a" if self.in_block && prefix == Some(b"text") => {
                            self.span_stack.pop();
                        }
                        b"list" if prefix == Some(b"text") => {
                            self.list_stack.pop();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }

    fn flush_current_block(&mut self) {
        if !self.in_block || self.cur_runs.is_empty() {
            return;
        }
        let runs = std::mem::take(&mut self.cur_runs);
        let props = std::mem::take(&mut self.cur_props);
        let block = match self.cur_outline {
            Some(level) => ast::Block::Heading { level, runs, props },
            None => ast::Block::Paragraph { runs, props },
        };
        self.blocks.push(block);
    }

    /// Drive the reader through `<table:table>...</table:table>`, gathering
    /// rows and cells. Cells contain paragraphs; we recursively parse each
    /// cell's content via a nested walker so styles + headings stay consistent.
    fn parse_table(&mut self, reader: &mut Reader<&[u8]>) -> Result<ast::Block> {
        let mut buf = Vec::new();
        let mut rows: Vec<ast::TableRow> = Vec::new();
        let mut cur_row: Vec<ast::TableCell> = Vec::new();
        let mut cell_xml: Option<String> = None;
        let mut depth_in_cell = 0u32;
        loop {
            match reader.read_event_into(&mut buf) {
                Err(e) => return Err(RichDocError::Odt(format!("table parse: {e}"))),
                Ok(Event::Eof) => break,
                Ok(Event::Start(ref e)) => {
                    let raw = e.name();
                    let name = local_name(raw.as_ref());
                    let prefix = local_prefix(raw.as_ref());
                    if prefix == Some(b"table") {
                        match name {
                            b"table-row" => {
                                cur_row.clear();
                            }
                            b"table-cell" => {
                                cell_xml = Some(String::new());
                                depth_in_cell = 1;
                            }
                            _ => {
                                if depth_in_cell > 0 {
                                    depth_in_cell += 1;
                                    if let Some(s) = cell_xml.as_mut() {
                                        append_xml_open(s, e);
                                    }
                                }
                            }
                        }
                    } else if depth_in_cell > 0 {
                        depth_in_cell += 1;
                        if let Some(s) = cell_xml.as_mut() {
                            append_xml_open(s, e);
                        }
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    if depth_in_cell > 0 {
                        if let Some(s) = cell_xml.as_mut() {
                            append_xml_empty(s, e);
                        }
                    }
                }
                Ok(Event::Text(ref t)) => {
                    if depth_in_cell > 0 {
                        if let Some(s) = cell_xml.as_mut() {
                            let txt = t.unescape().unwrap_or_default().into_owned();
                            append_xml_text(s, &txt);
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let raw = e.name();
                    let name = local_name(raw.as_ref());
                    let prefix = local_prefix(raw.as_ref());
                    if prefix == Some(b"table") {
                        match name {
                            b"table" => break,
                            b"table-row" => {
                                rows.push(ast::TableRow {
                                    cells: std::mem::take(&mut cur_row),
                                });
                            }
                            b"table-cell" => {
                                let xml = cell_xml.take().unwrap_or_default();
                                let cell_blocks = parse_fragment_blocks(
                                    &xml,
                                    self.style_map,
                                    self.para_style_map,
                                    self.list_style_map,
                                    self.zip_path,
                                );
                                cur_row.push(ast::TableCell { blocks: cell_blocks });
                                depth_in_cell = 0;
                            }
                            _ => {
                                if depth_in_cell > 0 {
                                    depth_in_cell -= 1;
                                    if let Some(s) = cell_xml.as_mut() {
                                        append_xml_close(s, e);
                                    }
                                }
                            }
                        }
                    } else if depth_in_cell > 0 {
                        depth_in_cell -= 1;
                        if let Some(s) = cell_xml.as_mut() {
                            append_xml_close(s, e);
                        }
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        Ok(ast::Block::Table { rows })
    }

    /// Consume a `<draw:frame>...</draw:frame>` subtree, returning an
    /// `Image` block if an inner `<draw:image xlink:href="..."/>` resolved to
    /// readable bytes. Otherwise returns None (caller drops the frame
    /// entirely — Phase 5a doesn't render arbitrary shapes).
    fn consume_frame(
        &mut self,
        reader: &mut Reader<&[u8]>,
        frame_start: &BytesStart,
    ) -> Result<Option<ast::Block>> {
        let svg_w = read_attr(frame_start, "svg:width");
        let svg_h = read_attr(frame_start, "svg:height");
        let mut buf = Vec::new();
        let mut href: Option<String> = None;
        let mut alt: Option<String> = None;
        let mut depth = 1u32;
        let mut in_desc = false;
        let mut desc_text = String::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Err(e) => return Err(RichDocError::Odt(format!("frame parse: {e}"))),
                Ok(Event::Eof) => break,
                Ok(Event::Start(ref e)) => {
                    depth += 1;
                    let raw = e.name();
                    let name = local_name(raw.as_ref());
                    let prefix = local_prefix(raw.as_ref());
                    if prefix == Some(b"svg") && name == b"desc" {
                        in_desc = true;
                        desc_text.clear();
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let raw = e.name();
                    let name = local_name(raw.as_ref());
                    let prefix = local_prefix(raw.as_ref());
                    if prefix == Some(b"draw") && name == b"image" && href.is_none() {
                        href = read_attr(e, "xlink:href");
                    }
                }
                Ok(Event::Text(ref t)) => {
                    if in_desc {
                        let s = t.unescape().unwrap_or_default().into_owned();
                        desc_text.push_str(&s);
                    }
                }
                Ok(Event::End(ref e)) => {
                    let raw = e.name();
                    let name = local_name(raw.as_ref());
                    let prefix = local_prefix(raw.as_ref());
                    if prefix == Some(b"draw") && name == b"image" {
                        // already captured via Empty
                    }
                    if prefix == Some(b"svg") && name == b"desc" {
                        if !desc_text.is_empty() {
                            alt = Some(std::mem::take(&mut desc_text));
                        }
                        in_desc = false;
                    }
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        let Some(href) = href else { return Ok(None) };
        let Some(zip_path) = self.zip_path else {
            return Ok(None);
        };
        let entry = href.trim_start_matches('/').to_string();
        let bytes = match container::read_zip_entry_bytes(zip_path, &entry) {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        let mime = match mime_from_filename(&entry) {
            Some(m) => m,
            None => return Ok(None),
        };
        let width_px = svg_w.as_deref().and_then(odf_length_to_px);
        let height_px = svg_h.as_deref().and_then(odf_length_to_px);
        Ok(Some(ast::Block::Image {
            source: ast::ImageSource::Inline {
                bytes: Arc::new(bytes),
                mime: mime.into(),
            },
            width_px,
            height_px,
            alt,
        }))
    }
}

/// Re-emit a Start event into the buffered cell XML — quick-xml gave us back
/// the raw bytes between < and > but tag content needs reconstruction.
fn append_xml_open(out: &mut String, e: &BytesStart) {
    out.push('<');
    out.push_str(std::str::from_utf8(e.name().as_ref()).unwrap_or(""));
    for attr in e.attributes().flatten() {
        out.push(' ');
        out.push_str(std::str::from_utf8(attr.key.as_ref()).unwrap_or(""));
        out.push_str("=\"");
        for ch in std::str::from_utf8(attr.value.as_ref()).unwrap_or("").chars() {
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    out.push('>');
}

fn append_xml_empty(out: &mut String, e: &BytesStart) {
    out.push('<');
    out.push_str(std::str::from_utf8(e.name().as_ref()).unwrap_or(""));
    for attr in e.attributes().flatten() {
        out.push(' ');
        out.push_str(std::str::from_utf8(attr.key.as_ref()).unwrap_or(""));
        out.push_str("=\"");
        for ch in std::str::from_utf8(attr.value.as_ref()).unwrap_or("").chars() {
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    out.push_str("/>");
}

fn append_xml_close(out: &mut String, e: &quick_xml::events::BytesEnd) {
    out.push_str("</");
    out.push_str(std::str::from_utf8(e.name().as_ref()).unwrap_or(""));
    out.push('>');
}

fn append_xml_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// Parse an ODF length string (e.g. `2cm`, `1in`, `96px`, `36pt`) into pixels
/// at 96 DPI. Returns None for unknown units or unparseable input.
fn odf_length_to_px(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Split numeric prefix from unit suffix.
    let split = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '.' && *c != '-')
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let (num_str, unit) = s.split_at(split);
    let num: f64 = num_str.parse().ok()?;
    let px = match unit {
        "px" | "" => num,
        "in" => num * 96.0,
        "cm" => num * (96.0 / 2.54),
        "mm" => num * (96.0 / 25.4),
        "pt" => num * (96.0 / 72.0),
        "pc" => num * 16.0,
        _ => return None,
    };
    if px.is_finite() && px >= 0.0 {
        Some(px.round() as u32)
    } else {
        None
    }
}

fn mime_from_filename(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        _ => return None,
    })
}

/// Scan styles.xml `<style:master-page>` for header/footer content. Returns
/// `(headers, footers)` where each is a vec of block sequences (one per
/// master page). Falls back to empty vecs if styles.xml is malformed or has
/// no master pages — quietly: header/footer parsing failure shouldn't block
/// opening an otherwise-valid ODT.
fn parse_master_pages(
    styles_xml: &str,
    content_xml: &str,
    zip_path: Option<&Path>,
) -> (Vec<Vec<ast::Block>>, Vec<Vec<ast::Block>>) {
    if styles_xml.is_empty() {
        return (Vec::new(), Vec::new());
    }
    // Style map for span resolution inside header/footer paragraphs.
    let mut raw_styles: HashMap<String, RawOdtStyle> = HashMap::new();
    let mut list_style_map: HashMap<String, bool> = HashMap::new();
    let _ = collect_raw_styles_into(&mut raw_styles, content_xml);
    let _ = collect_raw_styles_into(&mut raw_styles, styles_xml);
    let _ = collect_list_styles_into(&mut list_style_map, content_xml);
    let _ = collect_list_styles_into(&mut list_style_map, styles_xml);
    let (style_map, para_style_map) = resolve_raw_to_maps(&raw_styles);

    let headers = extract_master_page_sections(styles_xml, "header")
        .into_iter()
        .map(|frag| {
            parse_fragment_blocks(&frag, &style_map, &para_style_map, &list_style_map, zip_path)
        })
        .collect();
    let footers = extract_master_page_sections(styles_xml, "footer")
        .into_iter()
        .map(|frag| {
            parse_fragment_blocks(&frag, &style_map, &para_style_map, &list_style_map, zip_path)
        })
        .collect();
    (headers, footers)
}

/// Substring-extract `<style:header>...</style:header>` (or `…footer…`)
/// payloads from styles.xml. Returns the inner XML of each match, in document
/// order. Self-closed `<style:header/>` is ignored (no content).
fn extract_master_page_sections(xml: &str, kind: &str) -> Vec<String> {
    let open = format!("<style:{kind}");
    let close = format!("</style:{kind}>");
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(start) = xml[cursor..].find(&open) {
        let abs_start = cursor + start;
        let after_open = &xml[abs_start + open.len()..];
        let Some(gt) = after_open.find('>') else {
            break;
        };
        // Self-closed `<style:header/>` — no payload.
        if after_open[..gt].trim_end().ends_with('/') {
            cursor = abs_start + open.len() + gt + 1;
            continue;
        }
        let body_start = abs_start + open.len() + gt + 1;
        let Some(end) = xml[body_start..].find(&close) else {
            break;
        };
        let body_end = body_start + end;
        out.push(xml[body_start..body_end].to_string());
        cursor = body_end + close.len();
    }
    out
}

fn merge_styles(base: &ast::InlineStyle, overlay: &ast::InlineStyle) -> ast::InlineStyle {
    ast::InlineStyle {
        bold: base.bold || overlay.bold,
        italic: base.italic || overlay.italic,
        underline: base.underline || overlay.underline,
        strike: base.strike || overlay.strike,
        // overlay wins for color + link (innermost span / wrapping `<text:a>`)
        color: overlay.color.or(base.color),
        link: overlay.link.clone().or_else(|| base.link.clone()),
    }
}

fn push_run(runs: &mut Vec<ast::InlineRun>, span_stack: &[ast::InlineStyle], text: &str) {
    let style = span_stack.last().cloned().unwrap_or_default();
    runs.push(ast::InlineRun {
        text: text.to_string(),
        style,
    });
}

fn local_name(qname: &[u8]) -> &[u8] {
    match qname.iter().position(|b| *b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    }
}

fn local_prefix(qname: &[u8]) -> Option<&[u8]> {
    qname
        .iter()
        .position(|b| *b == b':')
        .map(|i| &qname[..i])
}

fn read_attr(e: &BytesStart, qname: &str) -> Option<String> {
    let target_local = local_name(qname.as_bytes());
    for attr in e.attributes().flatten() {
        if local_name(attr.key.as_ref()) == target_local {
            // Decode XML entities so `https://x?a=1&amp;b=2` round-trips to its
            // unescaped form rather than landing in the AST with `&amp;`.
            return attr
                .decode_and_unescape_value(quick_xml::Reader::from_str("").decoder())
                .ok()
                .map(|c| c.into_owned());
        }
    }
    None
}

/// Re-serialize an AST as a fresh `content.xml`. Strategy mirrors `docx::serialize_document_xml`:
/// 1. Pull the namespace declarations from the original `<office:document-content>` opening tag.
/// 2. Preserve the original `<office:automatic-styles>` block verbatim — user-defined styles
///    referenced by parts of the document we don't model (lists, tables) must survive.
/// 3. Append our own `<style:style>` entries for the seven combinations of B/I/U we may emit.
/// 4. Emit `<office:body><office:text>` with paragraphs/headings, wrapping styled runs in
///    `<text:span text:style-name="__rd_*">`.
fn serialize_content_xml(ast: &ast::Document, original_xml: &str) -> String {
    let ns_attrs = extract_namespace_attrs(original_xml);
    let original_auto_styles = extract_automatic_styles(original_xml).unwrap_or_default();

    // Walk the AST once to collect every distinct non-default InlineStyle. The
    // catalog assigns each a deterministic style name (static for pure B/I/U,
    // dynamic `__rd_dyn_<n>` for anything with color or other extensions).
    let mut catalog = StyleCatalog::new();
    for b in &ast.blocks {
        collect_styles_in_block(&mut catalog, b);
    }

    let mut out = String::with_capacity(original_xml.len() + 256);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    out.push_str("<office:document-content");
    if !ns_attrs.is_empty() {
        out.push(' ');
        out.push_str(&ns_attrs);
    }
    out.push('>');

    // Auto-styles: preserve original content + append our own.
    out.push_str("<office:automatic-styles>");
    if !original_auto_styles.is_empty() {
        out.push_str(&original_auto_styles);
    }
    catalog.emit_styles(&mut out);
    out.push_str("</office:automatic-styles>");

    out.push_str("<office:body><office:text>");
    serialize_blocks(&mut out, &ast.blocks, &catalog);
    out.push_str("</office:text></office:body></office:document-content>");
    out
}

/// Walk `blocks` left-to-right, grouping consecutive paragraphs/headings with
/// the same `props.list.ordered` kind into a single `<text:list>` wrapper.
/// Tables, images, and any kind change (or non-list block) break the run.
///
/// Level is intentionally NOT used to nest — Phase 5d.2 only exposes level 0
/// via the toolbar, so consecutive paragraphs at any level flatten into one
/// flat list per kind on save. Real nested lists (multi-level parsed from
/// elsewhere) survive open + reload because the parser preserves kind, only
/// level collapses to 0 on the round-trip. Mismatches are visible in the AST
/// before save but disappear post-save — explicit limitation, not a bug.
fn serialize_blocks(out: &mut String, blocks: &[ast::Block], catalog: &StyleCatalog) {
    let mut current_list_kind: Option<bool> = None;
    for b in blocks {
        let kind = block_list_kind(b);
        if kind != current_list_kind {
            if current_list_kind.is_some() {
                out.push_str("</text:list>");
            }
            if let Some(ordered) = kind {
                out.push_str("<text:list text:style-name=\"");
                out.push_str(if ordered {
                    LIST_STYLE_NUMBER
                } else {
                    LIST_STYLE_BULLET
                });
                out.push_str("\">");
            }
            current_list_kind = kind;
        }
        if kind.is_some() {
            out.push_str("<text:list-item>");
            serialize_block(out, b, catalog);
            out.push_str("</text:list-item>");
        } else {
            serialize_block(out, b, catalog);
        }
    }
    if current_list_kind.is_some() {
        out.push_str("</text:list>");
    }
}

fn block_list_kind(b: &ast::Block) -> Option<bool> {
    match b {
        ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => {
            props.list.map(|m| m.ordered)
        }
        ast::Block::Table { .. } | ast::Block::Image { .. } => None,
    }
}

fn collect_styles_in_block(catalog: &mut StyleCatalog, block: &ast::Block) {
    match block {
        ast::Block::Paragraph { runs, props } | ast::Block::Heading { runs, props, .. } => {
            catalog.intern_para(props);
            if let Some(marker) = props.list {
                if marker.ordered {
                    catalog.needs_number_list = true;
                } else {
                    catalog.needs_bullet_list = true;
                }
            }
            for r in runs {
                catalog.intern(&r.style);
            }
        }
        ast::Block::Table { rows } => {
            for row in rows {
                for cell in &row.cells {
                    for b in &cell.blocks {
                        collect_styles_in_block(catalog, b);
                    }
                }
            }
        }
        ast::Block::Image { .. } => {}
    }
}

/// Maps every distinct non-default `InlineStyle` to a stable style name.
/// Pure B/I/U combos reuse the seven static `__rd_*` names; anything else
/// (color, links once we lift them through here, etc.) gets a `__rd_dyn_N`
/// name assigned in encounter order, so re-saves produce identical XML.
struct StyleCatalog {
    entries: Vec<(ast::InlineStyle, String)>,
    para_entries: Vec<(ast::BlockProps, String)>,
    needs_bullet_list: bool,
    needs_number_list: bool,
}

impl StyleCatalog {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            para_entries: Vec::new(),
            needs_bullet_list: false,
            needs_number_list: false,
        }
    }

    /// Look up the style name for `s` (interning it if first seen).
    /// Returns `None` for the default style (no span wrapping needed).
    fn intern(&mut self, s: &ast::InlineStyle) -> Option<&str> {
        if *s == ast::InlineStyle::default() {
            return None;
        }
        if let Some(idx) = self.entries.iter().position(|(k, _)| k == s) {
            return Some(self.entries[idx].1.as_str());
        }
        let name = if let Some(static_name) = pure_biu_name(s) {
            static_name.to_string()
        } else {
            format!("__rd_dyn_{}", self.entries.len())
        };
        self.entries.push((s.clone(), name));
        Some(self.entries.last().unwrap().1.as_str())
    }

    fn lookup(&self, s: &ast::InlineStyle) -> Option<&str> {
        if *s == ast::InlineStyle::default() {
            return None;
        }
        self.entries.iter().find(|(k, _)| k == s).map(|(_, n)| n.as_str())
    }

    /// Intern a paragraph-level BlockProps. Returns the synthesized
    /// `__rd_para_N` style name (or `None` if `props` is the default and no
    /// `<text:style-name>` attribute is needed on the paragraph).
    fn intern_para(&mut self, p: &ast::BlockProps) -> Option<&str> {
        if *p == ast::BlockProps::default() {
            return None;
        }
        if let Some(idx) = self.para_entries.iter().position(|(k, _)| k == p) {
            return Some(self.para_entries[idx].1.as_str());
        }
        let name = format!("__rd_para_{}", self.para_entries.len());
        self.para_entries.push((p.clone(), name));
        Some(self.para_entries.last().unwrap().1.as_str())
    }

    fn lookup_para(&self, p: &ast::BlockProps) -> Option<&str> {
        if *p == ast::BlockProps::default() {
            return None;
        }
        self.para_entries.iter().find(|(k, _)| k == p).map(|(_, n)| n.as_str())
    }

    fn emit_styles(&self, out: &mut String) {
        if self.needs_bullet_list {
            out.push_str("<text:list-style style:name=\"");
            out.push_str(LIST_STYLE_BULLET);
            out.push_str("\"><text:list-level-style-bullet text:level=\"1\" text:bullet-char=\"\u{2022}\">\
<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
<style:list-level-label-alignment text:label-followed-by=\"listtab\" \
text:list-tab-stop-position=\"0.5in\" fo:text-indent=\"-0.25in\" fo:margin-left=\"0.5in\"/>\
</style:list-level-properties></text:list-level-style-bullet></text:list-style>");
        }
        if self.needs_number_list {
            out.push_str("<text:list-style style:name=\"");
            out.push_str(LIST_STYLE_NUMBER);
            out.push_str("\"><text:list-level-style-number text:level=\"1\" style:num-format=\"1\" style:num-suffix=\".\">\
<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
<style:list-level-label-alignment text:label-followed-by=\"listtab\" \
text:list-tab-stop-position=\"0.5in\" fo:text-indent=\"-0.25in\" fo:margin-left=\"0.5in\"/>\
</style:list-level-properties></text:list-level-style-number></text:list-style>");
        }
        for (style, name) in &self.entries {
            out.push_str("<style:style style:name=\"");
            out.push_str(name);
            out.push_str("\" style:family=\"text\"><style:text-properties");
            if style.bold {
                out.push_str(" fo:font-weight=\"bold\"");
            }
            if style.italic {
                out.push_str(" fo:font-style=\"italic\"");
            }
            if style.underline {
                out.push_str(" style:text-underline-style=\"solid\"");
            }
            if style.strike {
                out.push_str(" style:text-line-through-style=\"solid\"");
            }
            if let Some(rgb) = style.color {
                out.push_str(" fo:color=\"#");
                push_hex6_odt(out, rgb);
                out.push('"');
            }
            out.push_str("/></style:style>");
        }
        for (props, name) in &self.para_entries {
            out.push_str("<style:style style:name=\"");
            out.push_str(name);
            out.push_str("\" style:family=\"paragraph\"><style:paragraph-properties fo:text-align=\"");
            out.push_str(alignment_to_odt(props.alignment));
            out.push('"');
            if props.indent_level > 0 {
                // 1 level = 0.5 inch = 1.27 cm. Emit cm to match LibreOffice
                // round-trip; readers normalize anyway.
                let cm = (props.indent_level as f32) * 1.27;
                out.push_str(" fo:margin-left=\"");
                use std::fmt::Write;
                let _ = write!(out, "{:.4}cm", cm);
                out.push('"');
            }
            out.push_str("/></style:style>");
        }
    }
}

fn pure_biu_name(s: &ast::InlineStyle) -> Option<&'static str> {
    if s.color.is_some() || s.link.is_some() || s.strike {
        return None;
    }
    match (s.bold, s.italic, s.underline) {
        (false, false, false) => None,
        (true, false, false) => Some(STYLE_BOLD),
        (false, true, false) => Some(STYLE_ITALIC),
        (false, false, true) => Some(STYLE_UNDERLINE),
        (true, true, false) => Some(STYLE_BOLD_ITALIC),
        (true, false, true) => Some(STYLE_BOLD_UNDERLINE),
        (false, true, true) => Some(STYLE_ITALIC_UNDERLINE),
        (true, true, true) => Some(STYLE_ALL),
    }
}

fn push_hex6_odt(out: &mut String, rgb: u32) {
    use std::fmt::Write;
    let _ = write!(out, "{:06X}", rgb & 0xFF_FFFF);
}

fn extract_namespace_attrs(xml: &str) -> String {
    if let Some(start) = xml.find("<office:document-content") {
        let after = &xml[start + "<office:document-content".len()..];
        if let Some(end) = after.find('>') {
            return after[..end].trim().trim_end_matches('/').trim().to_string();
        }
    }
    // Minimal fallback: enough namespaces to declare a valid ODF text doc.
    [
        "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"",
        "xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"",
        "xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\"",
        "xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\"",
        "office:version=\"1.2\"",
    ]
    .join(" ")
}

/// Extract the inner XML of `<office:automatic-styles>` (children only, without
/// the wrapping element). Empty string if absent or self-closed.
fn extract_automatic_styles(xml: &str) -> Option<String> {
    let tag_open = "<office:automatic-styles";
    let start = xml.find(tag_open)?;
    let after_open = &xml[start + tag_open.len()..];
    let gt = after_open.find('>')?;
    // Self-closed form: `<office:automatic-styles/>`
    if after_open[..gt].trim_end().ends_with('/') {
        return Some(String::new());
    }
    let body_start = start + tag_open.len() + gt + 1;
    let close = "</office:automatic-styles>";
    let body_end = xml[body_start..].find(close)? + body_start;
    Some(xml[body_start..body_end].to_string())
}

fn serialize_block(out: &mut String, block: &ast::Block, catalog: &StyleCatalog) {
    match block {
        ast::Block::Paragraph { runs, props } => {
            out.push_str("<text:p");
            if let Some(name) = catalog.lookup_para(props) {
                out.push_str(" text:style-name=\"");
                out.push_str(name);
                out.push('"');
            }
            out.push('>');
            for r in runs {
                serialize_run(out, r, catalog);
            }
            out.push_str("</text:p>");
        }
        ast::Block::Heading { level, runs, props } => {
            out.push_str("<text:h text:outline-level=\"");
            out.push_str(&level.to_string());
            out.push('"');
            if let Some(name) = catalog.lookup_para(props) {
                out.push_str(" text:style-name=\"");
                out.push_str(name);
                out.push('"');
            }
            out.push('>');
            for r in runs {
                serialize_run(out, r, catalog);
            }
            out.push_str("</text:h>");
        }
        // Save rejects these blocks before calling the text serializer.
        ast::Block::Table { .. } | ast::Block::Image { .. } => {}
    }
}

fn serialize_run(out: &mut String, run: &ast::InlineRun, catalog: &StyleCatalog) {
    let style_name = catalog.lookup(&run.style);
    let has_link = run.style.link.is_some();
    if let Some(url) = &run.style.link {
        out.push_str("<text:a xlink:type=\"simple\" xlink:href=\"");
        xml_escape_into(out, url);
        out.push_str("\">");
    }
    if let Some(name) = style_name {
        out.push_str("<text:span text:style-name=\"");
        out.push_str(name);
        out.push_str("\">");
        xml_escape_into(out, &run.text);
        out.push_str("</text:span>");
    } else {
        xml_escape_into(out, &run.text);
    }
    if has_link {
        out.push_str("</text:a>");
    }
}

fn xml_escape_into(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
}

/// Pretty-print XML for the Raw View. quick-xml re-emits with two-space indents
/// and a trailing newline — matches what users expect when peeking at content.xml.
/// Falls back to the original text on any reader error so the view never crashes.
fn pretty_print_xml(src: &str) -> String {
    let mut reader = Reader::from_str(src);
    reader.config_mut().trim_text(true);
    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Err(_) => return src.to_string(),
            Ok(Event::Text(t)) => {
                let s = t.unescape().unwrap_or_default().into_owned();
                if writer
                    .write_event(Event::Text(BytesText::new(&s)))
                    .is_err()
                {
                    return src.to_string();
                }
            }
            Ok(other) => {
                if writer.write_event(other).is_err() {
                    return src.to_string();
                }
            }
        }
        buf.clear();
    }
    let bytes = writer.into_inner().into_inner();
    String::from_utf8(bytes).unwrap_or_else(|_| src.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal in-memory `.odt` (zip) containing `mimetype`,
    /// `META-INF/manifest.xml`, `content.xml`, and `styles.xml`. Returns the
    /// zip bytes ready for `parse_content_xml` or for writing to a temp file.
    fn build_minimal_odt(content_xml: &str, styles_xml: &str) -> Vec<u8> {
        build_minimal_odt_with_extras(content_xml, styles_xml, &[])
    }

    /// Variant that also adds arbitrary extra entries (path + bytes), e.g.
    /// `Pictures/image1.png` to test image extraction.
    fn build_minimal_odt_with_extras(
        content_xml: &str,
        styles_xml: &str,
        extras: &[(&str, &[u8])],
    ) -> Vec<u8> {
        use zip::write::SimpleFileOptions;
        use zip::CompressionMethod;
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let deflated = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated);
            z.start_file("mimetype", stored).unwrap();
            z.write_all(b"application/vnd.oasis.opendocument.text").unwrap();
            z.start_file("META-INF/manifest.xml", deflated).unwrap();
            z.write_all(b"<?xml version=\"1.0\"?><manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"></manifest:manifest>").unwrap();
            z.start_file("content.xml", deflated).unwrap();
            z.write_all(content_xml.as_bytes()).unwrap();
            z.start_file("styles.xml", deflated).unwrap();
            z.write_all(styles_xml.as_bytes()).unwrap();
            for (name, bytes) in extras {
                z.start_file(*name, deflated).unwrap();
                z.write_all(bytes).unwrap();
            }
            z.finish().unwrap();
        }
        buf
    }

    /// Minimal valid 1x1 PNG (89 50 4E 47 ...). Used to test image extraction
    /// — bytes pulled from the canonical "smallest valid PNG" reference.
    const ONE_PX_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn minimal_content(body_inner: &str, auto_styles_inner: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<office:document-content \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
office:version=\"1.2\">\
<office:automatic-styles>{auto}</office:automatic-styles>\
<office:body><office:text>{body}</office:text></office:body>\
</office:document-content>",
            auto = auto_styles_inner,
            body = body_inner,
        )
    }

    fn minimal_styles_xml() -> &'static str {
        "<?xml version=\"1.0\"?>\
<office:document-styles \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
office:version=\"1.2\">\
<office:styles></office:styles>\
<office:automatic-styles></office:automatic-styles>\
</office:document-styles>"
    }

    #[test]
    fn edited_document_with_a_table_keeps_the_original_file() {
        let dir = crate::persist::TempDir::new("odt-table-save");
        let path = dir.join("document.odt");
        let body = "<text:p>text</text:p><table:table><table:table-row><table:table-cell><text:p>keep table</text:p></table:table-cell></table:table-row></table:table>";
        let content = minimal_content(body, "").replacen("office:version=", "xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" office:version=", 1);
        let original = build_minimal_odt(&content, minimal_styles_xml());
        std::fs::write(&path, &original).expect("write fixture");
        let mut payload = open(&path).expect("open");
        assert!(payload.ast.blocks.iter().any(|block| matches!(block, ast::Block::Table { .. })));
        save(&payload, &path).expect("unchanged table document");
        assert_eq!(std::fs::read(&path).expect("read"), original);
        payload.apply_edit(super::super::edits::Edit::SetBlockBold { block: 0, on: true });
        assert!(matches!(save(&payload, &path), Err(RichDocError::UnsupportedContent)));
        assert_eq!(std::fs::read(&path).expect("read"), original);
    }

    #[test]
    fn parses_plain_paragraph() {
        let c = minimal_content("<text:p>Hello</text:p>", "");
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        assert_eq!(d.blocks.len(), 1);
        match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => {
                assert_eq!(runs.len(), 1);
                assert_eq!(runs[0].text, "Hello");
                assert!(!runs[0].style.bold);
            }
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn parses_bold_span_via_style_resolution() {
        let auto = "<style:style style:name=\"B1\" style:family=\"text\">\
<style:text-properties fo:font-weight=\"bold\"/></style:style>";
        let body = "<text:p>Hi <text:span text:style-name=\"B1\">bold</text:span> end</text:p>";
        let c = minimal_content(body, auto);
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        let runs = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs,
            _ => panic!(),
        };
        // 3 runs: "Hi " (plain), "bold" (bold), " end" (plain)
        let bold_run = runs.iter().find(|r| r.text == "bold").expect("has bold run");
        assert!(bold_run.style.bold);
        let plain_run = runs.iter().find(|r| r.text == "Hi ").expect("has prefix");
        assert!(!plain_run.style.bold);
    }

    #[test]
    fn parses_italic_and_underline() {
        let auto = "<style:style style:name=\"I\" style:family=\"text\">\
<style:text-properties fo:font-style=\"italic\"/></style:style>\
<style:style style:name=\"U\" style:family=\"text\">\
<style:text-properties style:text-underline-style=\"solid\"/></style:style>";
        let body = "<text:p><text:span text:style-name=\"I\">it</text:span>\
<text:span text:style-name=\"U\">un</text:span></text:p>";
        let c = minimal_content(body, auto);
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        let runs = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs,
            _ => panic!(),
        };
        let it = runs.iter().find(|r| r.text == "it").unwrap();
        assert!(it.style.italic);
        assert!(!it.style.bold);
        let un = runs.iter().find(|r| r.text == "un").unwrap();
        assert!(un.style.underline);
        assert!(!un.style.italic);
    }

    #[test]
    fn parses_heading_with_outline_level() {
        let body = "<text:h text:outline-level=\"2\">Section</text:h>\
<text:p>body</text:p>";
        let c = minimal_content(body, "");
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        assert_eq!(d.blocks.len(), 2);
        match &d.blocks[0] {
            ast::Block::Heading { level, runs, .. } => {
                assert_eq!(*level, 2);
                assert_eq!(runs[0].text, "Section");
            }
            _ => panic!("expected heading"),
        }
        match &d.blocks[1] {
            ast::Block::Paragraph { runs, .. } => assert_eq!(runs[0].text, "body"),
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn font_weight_numeric_700_is_bold() {
        let auto = "<style:style style:name=\"B700\" style:family=\"text\">\
<style:text-properties fo:font-weight=\"700\"/></style:style>";
        let body = "<text:p><text:span text:style-name=\"B700\">heavy</text:span></text:p>";
        let c = minimal_content(body, auto);
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        let runs = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs,
            _ => panic!(),
        };
        assert!(runs[0].style.bold);
    }

    #[test]
    fn span_outside_known_style_falls_back_to_plain() {
        let body = "<text:p><text:span text:style-name=\"Unknown\">x</text:span></text:p>";
        let c = minimal_content(body, "");
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        let runs = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs,
            _ => panic!(),
        };
        assert_eq!(runs[0].text, "x");
        assert!(!runs[0].style.bold);
    }

    #[test]
    fn tab_and_line_break_emit_text() {
        let body = "<text:p>a<text:tab/>b<text:line-break/>c</text:p>";
        let c = minimal_content(body, "");
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        let combined: String = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
            _ => panic!(),
        };
        assert_eq!(combined, "a\tb\nc");
    }

    #[test]
    fn text_s_expands_n_spaces() {
        let body = "<text:p>a<text:s text:c=\"3\"/>b</text:p>";
        let c = minimal_content(body, "");
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        let combined: String = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
            _ => panic!(),
        };
        assert_eq!(combined, "a   b");
    }

    #[test]
    fn oversized_or_invalid_space_counts_are_refused() {
        for count in [usize::MAX.to_string(), "9999999999999999999999999999999999".into(), "-1".into(), "wrong".into()] {
            let body = format!("<text:p>a<text:s text:c=\"{count}\"/>b</text:p>");
            let content = minimal_content(&body, "");
            assert!(matches!(parse_content_xml(&content, minimal_styles_xml()), Err(RichDocError::Odt(_))), "count {count}");
        }
    }

    #[test]
    fn ordinary_space_counts_keep_every_space() {
        for count in [0, 1, 3, 65_535] {
            let body = format!("<text:p>a<text:s text:c=\"{count}\"/>b</text:p>");
            let content = minimal_content(&body, "");
            let doc = parse_content_xml(&content, minimal_styles_xml()).expect("parse");
            let ast::Block::Paragraph { runs, .. } = &doc.blocks[0] else { panic!("paragraph") };
            let text: String = runs.iter().map(|run| run.text.as_str()).collect();
            assert_eq!(text, format!("a{}b", " ".repeat(count)));
        }
    }

    #[test]
    fn nested_spans_merge_styles() {
        let auto = "<style:style style:name=\"B\" style:family=\"text\">\
<style:text-properties fo:font-weight=\"bold\"/></style:style>\
<style:style style:name=\"I\" style:family=\"text\">\
<style:text-properties fo:font-style=\"italic\"/></style:style>";
        let body = "<text:p><text:span text:style-name=\"B\">b<text:span text:style-name=\"I\">bi</text:span></text:span></text:p>";
        let c = minimal_content(body, auto);
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        let runs = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs,
            _ => panic!(),
        };
        let bi = runs.iter().find(|r| r.text == "bi").unwrap();
        assert!(bi.style.bold);
        assert!(bi.style.italic);
    }

    #[test]
    fn full_open_round_trip_via_zip_extracts_payload() {
        let body = "<text:p>Hello</text:p><text:h text:outline-level=\"1\">Title</text:h>";
        let content = minimal_content(body, "");
        let bytes = build_minimal_odt(&content, minimal_styles_xml());
        let tmp = std::env::temp_dir().join("notepad-studio-odt-open.odt");
        std::fs::write(&tmp, &bytes).expect("write tmp odt");
        let payload = open(&tmp).expect("opens");
        assert_eq!(payload.format, DocFormat::Odt);
        assert_eq!(payload.ast.blocks.len(), 2);
        assert!(!payload.raw_view_text.is_empty());
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn save_with_no_edits_is_byte_perfect() {
        let body = "<text:p>Hello</text:p>";
        let content = minimal_content(body, "");
        let bytes = build_minimal_odt(&content, minimal_styles_xml());
        let src = std::env::temp_dir().join("notepad-studio-odt-noedit-src.odt");
        let dst = std::env::temp_dir().join("notepad-studio-odt-noedit-dst.odt");
        std::fs::write(&src, &bytes).expect("write src");
        let payload = open(&src).expect("opens");
        save(&payload, &dst).expect("saves");
        let src_b = std::fs::read(&src).unwrap();
        let dst_b = std::fs::read(&dst).unwrap();
        assert_eq!(src_b, dst_b, "no-edit save must be byte-perfect");
        std::fs::remove_file(&src).ok();
        std::fs::remove_file(&dst).ok();
    }

    #[test]
    fn round_trip_with_bold_edit_keeps_bold_after_reopen() {
        let body = "<text:p>Hello world</text:p>";
        let content = minimal_content(body, "");
        let bytes = build_minimal_odt(&content, minimal_styles_xml());
        let src = std::env::temp_dir().join("notepad-studio-odt-bold-src.odt");
        let dst = std::env::temp_dir().join("notepad-studio-odt-bold-dst.odt");
        std::fs::write(&src, &bytes).expect("write src");
        let mut payload = open(&src).expect("opens");
        payload.apply_edit(super::super::edits::Edit::SetBlockBold { block: 0, on: true });
        save(&payload, &dst).expect("saves");
        let reopened = open(&dst).expect("reopens");
        let runs = match &reopened.ast.blocks[0] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => unreachable!(),
        };
        assert!(
            !runs.is_empty() && runs.iter().all(|r| r.style.bold),
            "every run in the bolded block should be bold after round-trip"
        );
        std::fs::remove_file(&src).ok();
        std::fs::remove_file(&dst).ok();
    }

    #[test]
    fn save_preserves_other_zip_entries() {
        let body = "<text:p>Hi</text:p>";
        let content = minimal_content(body, "");
        let bytes = build_minimal_odt(&content, minimal_styles_xml());
        let src = std::env::temp_dir().join("notepad-studio-odt-preserve-src.odt");
        let dst = std::env::temp_dir().join("notepad-studio-odt-preserve-dst.odt");
        std::fs::write(&src, &bytes).expect("write src");
        let mut payload = open(&src).expect("opens");
        payload.apply_edit(super::super::edits::Edit::SetBlockBold { block: 0, on: true });
        save(&payload, &dst).expect("saves");
        // mimetype and styles.xml must survive
        let mimetype = container::read_zip_entry_text(&dst, "mimetype").expect("mimetype");
        assert_eq!(mimetype, "application/vnd.oasis.opendocument.text");
        let styles = container::read_zip_entry_text(&dst, "styles.xml").expect("styles");
        assert!(styles.contains("office:document-styles"));
        std::fs::remove_file(&src).ok();
        std::fs::remove_file(&dst).ok();
    }

    #[test]
    fn extract_automatic_styles_handles_self_closed() {
        let xml = "<office:document-content><office:automatic-styles/></office:document-content>";
        assert_eq!(extract_automatic_styles(xml).as_deref(), Some(""));
    }

    #[test]
    fn extract_automatic_styles_returns_inner() {
        let xml = "<office:document-content><office:automatic-styles>\
<style:style style:name=\"P1\" style:family=\"paragraph\"/></office:automatic-styles>\
<office:body/></office:document-content>";
        let inner = extract_automatic_styles(xml).unwrap();
        assert!(inner.contains("style:name=\"P1\""));
    }

    #[test]
    fn serialize_emits_text_span_for_bold_run() {
        let doc = ast::Document {
            blocks: vec![ast::Block::Paragraph {
                runs: vec![ast::InlineRun {
                    text: "hi".into(),
                    style: ast::InlineStyle {
                        bold: true,
                        ..Default::default()
                    },
                }],
                props: ast::BlockProps::default(),
            }],
        };
        let xml = serialize_content_xml(
            &doc,
            "<office:document-content xmlns:office=\"o\" xmlns:text=\"t\" xmlns:style=\"s\" xmlns:fo=\"f\"></office:document-content>",
        );
        assert!(xml.contains("<text:p>"));
        assert!(xml.contains("text:style-name=\"__rd_bold\""));
        assert!(xml.contains(">hi</text:span>"));
    }

    #[test]
    fn serialize_heading_emits_outline_level() {
        let doc = ast::Document {
            blocks: vec![ast::Block::Heading {
                level: 2,
                runs: vec![ast::InlineRun::plain("Section")],
                props: ast::BlockProps::default(),
            }],
        };
        let xml = serialize_content_xml(
            &doc,
            "<office:document-content xmlns:office=\"o\"></office:document-content>",
        );
        assert!(xml.contains("<text:h text:outline-level=\"2\">"));
        assert!(xml.contains(">Section</text:h>"));
    }

    #[test]
    fn xml_escape_handles_entities() {
        let mut out = String::new();
        xml_escape_into(&mut out, "a & b < c > d \" e ' f");
        assert_eq!(out, "a &amp; b &lt; c &gt; d &quot; e &apos; f");
    }

    #[test]
    fn round_trip_with_color_edit_keeps_color_after_reopen() {
        let content = minimal_content("<text:p>hello</text:p>", "");
        let zip_bytes = build_minimal_odt(&content, minimal_styles_xml());
        let tmp_src = std::env::temp_dir().join("notepad-studio-odt-color-src.odt");
        std::fs::write(&tmp_src, &zip_bytes).expect("writes fixture");
        let mut payload = open(&tmp_src).expect("opens");
        let target_block = payload
            .ast
            .blocks
            .iter()
            .position(|b| matches!(b, ast::Block::Paragraph { runs, .. } if !runs.is_empty()))
            .expect("fixture has at least one paragraph");
        payload.apply_edit(super::super::edits::Edit::SetBlockColor {
            block: target_block,
            color: Some(0x12_34_56),
        });
        let tmp_out = std::env::temp_dir().join("notepad-studio-odt-color-out.odt");
        save(&payload, &tmp_out).expect("saves");
        let reopened = open(&tmp_out).expect("reopens");
        let runs = match &reopened.ast.blocks[target_block] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => unreachable!(),
        };
        assert!(
            runs.iter().all(|r| r.style.color == Some(0x12_34_56)),
            "every run in the colored block should keep the color after round-trip"
        );
        std::fs::remove_file(&tmp_src).ok();
        std::fs::remove_file(&tmp_out).ok();
    }

    #[test]
    fn round_trip_with_alignment_and_indent_persist() {
        let content = minimal_content("<text:p>hello</text:p>", "");
        let zip_bytes = build_minimal_odt(&content, minimal_styles_xml());
        let tmp_src = std::env::temp_dir().join("notepad-studio-odt-props-src.odt");
        std::fs::write(&tmp_src, &zip_bytes).expect("writes fixture");
        let mut payload = open(&tmp_src).expect("opens");
        let target_block = payload
            .ast
            .blocks
            .iter()
            .position(|b| matches!(b, ast::Block::Paragraph { runs, .. } if !runs.is_empty()))
            .expect("has paragraph");
        payload.apply_edit(super::super::edits::Edit::SetBlockAlignment {
            block: target_block,
            alignment: ast::Alignment::Justify,
        });
        payload.apply_edit(super::super::edits::Edit::SetBlockIndent {
            block: target_block,
            delta: 1,
        });
        let tmp_out = std::env::temp_dir().join("notepad-studio-odt-props-out.odt");
        save(&payload, &tmp_out).expect("saves");
        let reopened = open(&tmp_out).expect("reopens");
        let props = match &reopened.ast.blocks[target_block] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => props.clone(),
            _ => unreachable!(),
        };
        assert_eq!(props.alignment, ast::Alignment::Justify);
        assert_eq!(props.indent_level, 1);
        std::fs::remove_file(&tmp_src).ok();
        std::fs::remove_file(&tmp_out).ok();
    }

    #[test]
    fn round_trip_with_link_edit_keeps_link_after_reopen() {
        let content = minimal_content("<text:p>hello</text:p>", "");
        let zip_bytes = build_minimal_odt(&content, minimal_styles_xml());
        let tmp_src = std::env::temp_dir().join("notepad-studio-odt-link-src.odt");
        std::fs::write(&tmp_src, &zip_bytes).expect("writes fixture");
        let mut payload = open(&tmp_src).expect("opens");
        let target_block = payload
            .ast
            .blocks
            .iter()
            .position(|b| matches!(b, ast::Block::Paragraph { runs, .. } if !runs.is_empty()))
            .expect("has a paragraph");
        payload.apply_edit(super::super::edits::Edit::SetBlockLink {
            block: target_block,
            url: Some("https://example.org/x?y=1&z=2".into()),
        });
        let tmp_out = std::env::temp_dir().join("notepad-studio-odt-link-out.odt");
        save(&payload, &tmp_out).expect("saves");
        let reopened = open(&tmp_out).expect("reopens");
        let runs = match &reopened.ast.blocks[target_block] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => unreachable!(),
        };
        assert!(
            runs.iter()
                .all(|r| r.style.link.as_deref() == Some("https://example.org/x?y=1&z=2")),
            "every run in the linked block should resolve to the same URL"
        );
        std::fs::remove_file(&tmp_src).ok();
        std::fs::remove_file(&tmp_out).ok();
    }

    #[test]
    fn parse_odf_color_accepts_hash_hex_and_rejects_transparent() {
        assert_eq!(parse_odf_color("#FFAABB"), Some(0xFF_AA_BB));
        assert_eq!(parse_odf_color("ffaabb"), Some(0xFF_AA_BB));
        assert_eq!(parse_odf_color("transparent"), None);
        assert_eq!(parse_odf_color(""), None);
    }

    #[test]
    fn style_catalog_uses_static_names_for_pure_biu() {
        let mut cat = StyleCatalog::new();
        let bi = ast::InlineStyle {
            bold: true,
            italic: true,
            ..Default::default()
        };
        assert_eq!(cat.intern(&bi), Some(STYLE_BOLD_ITALIC));
        let all = ast::InlineStyle {
            bold: true,
            italic: true,
            underline: true,
            ..Default::default()
        };
        assert_eq!(cat.intern(&all), Some(STYLE_ALL));
        assert_eq!(cat.intern(&ast::InlineStyle::default()), None);
    }

    #[test]
    fn style_catalog_assigns_dynamic_name_for_color() {
        let mut cat = StyleCatalog::new();
        let red = ast::InlineStyle {
            color: Some(0xFF_00_00),
            ..Default::default()
        };
        assert_eq!(cat.intern(&red), Some("__rd_dyn_0"));
        // re-interning the same style returns the same name
        assert_eq!(cat.intern(&red), Some("__rd_dyn_0"));
    }

    #[test]
    fn is_bold_weight_recognises_numeric_and_keywords() {
        assert!(is_bold_weight("bold"));
        assert!(is_bold_weight("bolder"));
        assert!(is_bold_weight("700"));
        assert!(is_bold_weight("900"));
        assert!(!is_bold_weight("normal"));
        assert!(!is_bold_weight("400"));
        assert!(!is_bold_weight(""));
    }

    #[test]
    fn parses_simple_two_by_two_table() {
        let body = "<table:table><table:table-row>\
<table:table-cell><text:p>r0c0</text:p></table:table-cell>\
<table:table-cell><text:p>r0c1</text:p></table:table-cell>\
</table:table-row><table:table-row>\
<table:table-cell><text:p>r1c0</text:p></table:table-cell>\
<table:table-cell><text:p>r1c1</text:p></table:table-cell>\
</table:table-row></table:table>";
        let c = format!(
            "<?xml version=\"1.0\"?>\
<office:document-content \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\">\
<office:body><office:text>{body}</office:text></office:body>\
</office:document-content>"
        );
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        assert_eq!(d.blocks.len(), 1);
        let rows = match &d.blocks[0] {
            ast::Block::Table { rows } => rows,
            _ => panic!("expected table"),
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].cells.len(), 2);
        let text_of = |row: usize, col: usize| -> String {
            match &rows[row].cells[col].blocks[0] {
                ast::Block::Paragraph { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
                _ => panic!("cell expected paragraph"),
            }
        };
        assert_eq!(text_of(0, 0), "r0c0");
        assert_eq!(text_of(1, 1), "r1c1");
    }

    #[test]
    fn table_preserves_heading_inside_cell() {
        let body = "<table:table><table:table-row><table:table-cell>\
<text:h text:outline-level=\"2\">Title</text:h>\
<text:p>body</text:p>\
</table:table-cell></table:table-row></table:table>";
        let c = format!(
            "<?xml version=\"1.0\"?>\
<office:document-content \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\">\
<office:body><office:text>{body}</office:text></office:body>\
</office:document-content>"
        );
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        let cell = match &d.blocks[0] {
            ast::Block::Table { rows } => &rows[0].cells[0],
            _ => panic!(),
        };
        assert_eq!(cell.blocks.len(), 2);
        match &cell.blocks[0] {
            ast::Block::Heading { level, runs, .. } => {
                assert_eq!(*level, 2);
                assert_eq!(runs[0].text, "Title");
            }
            _ => panic!("first cell block must be heading"),
        }
        match &cell.blocks[1] {
            ast::Block::Paragraph { runs, .. } => assert_eq!(runs[0].text, "body"),
            _ => panic!("second cell block must be paragraph"),
        }
    }

    #[test]
    fn parses_image_from_draw_frame() {
        let body = "<text:p>Before</text:p>\
<draw:frame svg:width=\"2in\" svg:height=\"1in\">\
<draw:image xlink:href=\"Pictures/image1.png\"/>\
<svg:desc>my picture</svg:desc>\
</draw:frame>\
<text:p>After</text:p>";
        let content = format!(
            "<?xml version=\"1.0\"?>\
<office:document-content \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" \
xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\">\
<office:body><office:text>{body}</office:text></office:body>\
</office:document-content>"
        );
        let bytes = build_minimal_odt_with_extras(
            &content,
            minimal_styles_xml(),
            &[("Pictures/image1.png", ONE_PX_PNG)],
        );
        let tmp = std::env::temp_dir().join("notepad-studio-odt-image.odt");
        std::fs::write(&tmp, &bytes).unwrap();
        let payload = open(&tmp).expect("opens");
        // Expect: Paragraph "Before", Image, Paragraph "After"
        assert_eq!(payload.ast.blocks.len(), 3);
        match &payload.ast.blocks[1] {
            ast::Block::Image { source, width_px, height_px, alt } => {
                let ast::ImageSource::Inline { bytes: b, mime } = source;
                assert_eq!(mime, "image/png");
                assert_eq!(b.as_slice(), ONE_PX_PNG);
                assert_eq!(*width_px, Some(192)); // 2in * 96 DPI
                assert_eq!(*height_px, Some(96));
                assert_eq!(alt.as_deref(), Some("my picture"));
            }
            _ => panic!("middle block must be image"),
        }
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn parses_header_from_master_page() {
        let styles = "<?xml version=\"1.0\"?>\
<office:document-styles \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
office:version=\"1.2\">\
<office:styles></office:styles>\
<office:automatic-styles></office:automatic-styles>\
<office:master-styles>\
<style:master-page style:name=\"Standard\">\
<style:header><text:p>Page header</text:p></style:header>\
<style:footer><text:p>Page footer</text:p></style:footer>\
</style:master-page>\
</office:master-styles>\
</office:document-styles>";
        let body = "<text:p>body</text:p>";
        let content = minimal_content(body, "");
        let bytes = build_minimal_odt(&content, styles);
        let tmp = std::env::temp_dir().join("notepad-studio-odt-headers.odt");
        std::fs::write(&tmp, &bytes).unwrap();
        let payload = open(&tmp).expect("opens");
        assert_eq!(payload.headers.len(), 1, "one master page → one header");
        assert_eq!(payload.footers.len(), 1);
        let header_text = match &payload.headers[0][0] {
            ast::Block::Paragraph { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect::<String>(),
            _ => panic!("header block must be paragraph"),
        };
        assert_eq!(header_text, "Page header");
        let footer_text = match &payload.footers[0][0] {
            ast::Block::Paragraph { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect::<String>(),
            _ => panic!(),
        };
        assert_eq!(footer_text, "Page footer");
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn empty_styles_xml_yields_no_headers_footers() {
        let body = "<text:p>only body</text:p>";
        let content = minimal_content(body, "");
        let bytes = build_minimal_odt(&content, minimal_styles_xml());
        let tmp = std::env::temp_dir().join("notepad-studio-odt-no-master.odt");
        std::fs::write(&tmp, &bytes).unwrap();
        let payload = open(&tmp).expect("opens");
        assert!(payload.headers.is_empty());
        assert!(payload.footers.is_empty());
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn odf_length_to_px_handles_common_units() {
        assert_eq!(odf_length_to_px("96px"), Some(96));
        assert_eq!(odf_length_to_px("1in"), Some(96));
        assert_eq!(odf_length_to_px("2cm"), Some(76));
        assert_eq!(odf_length_to_px("36pt"), Some(48));
        assert_eq!(odf_length_to_px("0"), Some(0));
        assert_eq!(odf_length_to_px(""), None);
        assert_eq!(odf_length_to_px("bogus"), None);
    }

    #[test]
    fn extract_master_page_sections_finds_multiple() {
        let xml = "<style:master-page><style:header>A</style:header></style:master-page>\
<style:master-page><style:header>B</style:header></style:master-page>";
        let hs = extract_master_page_sections(xml, "header");
        assert_eq!(hs, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn extract_master_page_sections_skips_self_closed() {
        let xml = "<style:master-page><style:header/><style:footer>F</style:footer></style:master-page>";
        let hs = extract_master_page_sections(xml, "header");
        assert!(hs.is_empty());
        let fs = extract_master_page_sections(xml, "footer");
        assert_eq!(fs, vec!["F".to_string()]);
    }

    #[test]
    fn collect_list_styles_detects_bullet_vs_number() {
        let xml = "<office:document-content xmlns:text=\"t\" xmlns:style=\"s\">\
<office:automatic-styles>\
<text:list-style style:name=\"LB\">\
<text:list-level-style-bullet text:level=\"1\" text:bullet-char=\"\u{2022}\"/>\
</text:list-style>\
<text:list-style style:name=\"LN\">\
<text:list-level-style-number text:level=\"1\" style:num-format=\"1\"/>\
</text:list-style>\
</office:automatic-styles></office:document-content>";
        let mut map: HashMap<String, bool> = HashMap::new();
        collect_list_styles_into(&mut map, xml).expect("scans");
        assert_eq!(map.get("LB"), Some(&false));
        assert_eq!(map.get("LN"), Some(&true));
    }

    #[test]
    fn parses_text_list_into_paragraph_props() {
        let auto = "<text:list-style style:name=\"LB\">\
<text:list-level-style-bullet text:level=\"1\" text:bullet-char=\"\u{2022}\"/>\
</text:list-style>\
<text:list-style style:name=\"LN\">\
<text:list-level-style-number text:level=\"1\" style:num-format=\"1\"/>\
</text:list-style>";
        let body = "<text:list text:style-name=\"LB\">\
<text:list-item><text:p>b1</text:p></text:list-item>\
<text:list-item><text:p>b2</text:p></text:list-item>\
</text:list>\
<text:list text:style-name=\"LN\">\
<text:list-item><text:p>n1</text:p></text:list-item>\
</text:list>";
        let c = minimal_content(body, auto);
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        assert_eq!(d.blocks.len(), 3);
        let kind_of = |idx: usize| match &d.blocks[idx] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => {
                props.list.map(|m| m.ordered)
            }
            _ => None,
        };
        assert_eq!(kind_of(0), Some(false));
        assert_eq!(kind_of(1), Some(false));
        assert_eq!(kind_of(2), Some(true));
    }

    #[test]
    fn parses_nested_text_list_tracks_level() {
        let auto = "<text:list-style style:name=\"LB\">\
<text:list-level-style-bullet text:level=\"1\" text:bullet-char=\"\u{2022}\"/>\
</text:list-style>";
        let body = "<text:list text:style-name=\"LB\">\
<text:list-item><text:p>outer</text:p>\
<text:list text:style-name=\"LB\">\
<text:list-item><text:p>inner</text:p></text:list-item>\
</text:list>\
</text:list-item></text:list>";
        let c = minimal_content(body, auto);
        let d = parse_content_xml(&c, minimal_styles_xml()).expect("parses");
        let level_of = |idx: usize| match &d.blocks[idx] {
            ast::Block::Paragraph { props, .. } => props.list.map(|m| m.level),
            _ => None,
        };
        assert_eq!(level_of(0), Some(0));
        assert_eq!(level_of(1), Some(1));
    }

    #[test]
    fn serialize_wraps_list_paragraphs_in_text_list() {
        let mut props_b = ast::BlockProps::default();
        props_b.list = Some(ast::ListMarker { ordered: false, level: 0 });
        let mut props_n = ast::BlockProps::default();
        props_n.list = Some(ast::ListMarker { ordered: true, level: 0 });
        let doc = ast::Document {
            blocks: vec![
                ast::Block::Paragraph {
                    runs: vec![ast::InlineRun::plain("a")],
                    props: props_b.clone(),
                },
                ast::Block::Paragraph {
                    runs: vec![ast::InlineRun::plain("b")],
                    props: props_b,
                },
                ast::Block::Paragraph {
                    runs: vec![ast::InlineRun::plain("c")],
                    props: props_n,
                },
            ],
        };
        let xml = serialize_content_xml(
            &doc,
            "<office:document-content xmlns:office=\"o\" xmlns:text=\"t\" xmlns:style=\"s\" xmlns:fo=\"f\"></office:document-content>",
        );
        // Bullet group: one <text:list text:style-name="__rd_list_bullet"> with two items
        assert!(xml.contains("<text:list text:style-name=\"__rd_list_bullet\">"));
        assert!(xml.contains("<text:list text:style-name=\"__rd_list_number\">"));
        // Both list-style definitions land in automatic-styles
        assert!(xml.contains("<text:list-style style:name=\"__rd_list_bullet\""));
        assert!(xml.contains("<text:list-style style:name=\"__rd_list_number\""));
        // Items wrap paragraphs
        assert!(xml.contains("<text:list-item><text:p"));
    }

    #[test]
    fn serialize_breaks_list_at_kind_change() {
        let mut bullet = ast::BlockProps::default();
        bullet.list = Some(ast::ListMarker { ordered: false, level: 0 });
        let mut number = ast::BlockProps::default();
        number.list = Some(ast::ListMarker { ordered: true, level: 0 });
        let doc = ast::Document {
            blocks: vec![
                ast::Block::Paragraph {
                    runs: vec![ast::InlineRun::plain("a")],
                    props: bullet.clone(),
                },
                ast::Block::Paragraph {
                    runs: vec![ast::InlineRun::plain("b")],
                    props: number,
                },
                ast::Block::Paragraph {
                    runs: vec![ast::InlineRun::plain("c")],
                    props: bullet,
                },
            ],
        };
        let xml = serialize_content_xml(
            &doc,
            "<office:document-content xmlns:office=\"o\" xmlns:text=\"t\" xmlns:style=\"s\" xmlns:fo=\"f\"></office:document-content>",
        );
        // Two bullet wrappers (one before number, one after) + one number wrapper
        let bullet_opens = xml.matches("<text:list text:style-name=\"__rd_list_bullet\">").count();
        let number_opens = xml.matches("<text:list text:style-name=\"__rd_list_number\">").count();
        assert_eq!(bullet_opens, 2, "kind change must close + reopen the wrapper");
        assert_eq!(number_opens, 1);
    }

    #[test]
    fn round_trip_with_list_edits_preserves_kinds() {
        let body = "<text:p>one</text:p><text:p>two</text:p><text:p>three</text:p>";
        let content = minimal_content(body, "");
        let bytes = build_minimal_odt(&content, minimal_styles_xml());
        let src = std::env::temp_dir().join("notepad-studio-odt-lists-src.odt");
        let dst = std::env::temp_dir().join("notepad-studio-odt-lists-dst.odt");
        std::fs::write(&src, &bytes).expect("writes src");
        let mut payload = open(&src).expect("opens");
        payload.apply_edit(super::super::edits::Edit::ToggleBlockList {
            block: 0,
            ordered: false,
        });
        payload.apply_edit(super::super::edits::Edit::ToggleBlockList {
            block: 1,
            ordered: false,
        });
        payload.apply_edit(super::super::edits::Edit::ToggleBlockList {
            block: 2,
            ordered: true,
        });
        save(&payload, &dst).expect("saves");
        let reopened = open(&dst).expect("reopens");
        let kind_of = |idx: usize| match &reopened.ast.blocks[idx] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => {
                props.list.map(|m| m.ordered)
            }
            _ => None,
        };
        assert_eq!(kind_of(0), Some(false), "first paragraph should be bullet");
        assert_eq!(kind_of(1), Some(false), "second paragraph should be bullet");
        assert_eq!(kind_of(2), Some(true), "third paragraph should be numbered");
        std::fs::remove_file(&src).ok();
        std::fs::remove_file(&dst).ok();
    }

    #[test]
    fn round_trip_list_then_toggle_off_clears_marker() {
        let body = "<text:p>one</text:p>";
        let content = minimal_content(body, "");
        let bytes = build_minimal_odt(&content, minimal_styles_xml());
        let src = std::env::temp_dir().join("notepad-studio-odt-list-clear-src.odt");
        let dst = std::env::temp_dir().join("notepad-studio-odt-list-clear-dst.odt");
        std::fs::write(&src, &bytes).expect("writes src");
        let mut payload = open(&src).expect("opens");
        payload.apply_edit(super::super::edits::Edit::ToggleBlockList {
            block: 0,
            ordered: false,
        });
        payload.apply_edit(super::super::edits::Edit::ToggleBlockList {
            block: 0,
            ordered: false,
        });
        save(&payload, &dst).expect("saves");
        let reopened = open(&dst).expect("reopens");
        match &reopened.ast.blocks[0] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => {
                assert_eq!(props.list, None, "second toggle of same kind must clear");
            }
            _ => panic!("expected paragraph"),
        }
        std::fs::remove_file(&src).ok();
        std::fs::remove_file(&dst).ok();
    }

    // ----- parent-style chain resolution (Phase 3b) -----------------------

    fn raw_with(parent: Option<&str>, mut props: RawOdtStyle) -> RawOdtStyle {
        props.parent = parent.map(|s| s.to_string());
        props
    }

    #[test]
    fn chain_single_level_inherits_from_parent() {
        let mut raw: HashMap<String, RawOdtStyle> = HashMap::new();
        raw.insert(
            "Heading".into(),
            raw_with(None, RawOdtStyle { bold: Some(true), ..Default::default() }),
        );
        raw.insert(
            "Heading 1".into(),
            raw_with("Heading".into(), RawOdtStyle::default()),
        );
        let resolved = resolve_chain("Heading 1", &raw);
        assert_eq!(resolved.bold, Some(true), "child inherits bold from parent");
    }

    #[test]
    fn chain_two_levels_inherits_transitively() {
        let mut raw: HashMap<String, RawOdtStyle> = HashMap::new();
        raw.insert(
            "Default".into(),
            raw_with(None, RawOdtStyle { color: Some(0x00_FF00), ..Default::default() }),
        );
        raw.insert(
            "Heading".into(),
            raw_with("Default".into(), RawOdtStyle { italic: Some(true), ..Default::default() }),
        );
        raw.insert(
            "Heading 1".into(),
            raw_with("Heading".into(), RawOdtStyle { bold: Some(true), ..Default::default() }),
        );
        let resolved = resolve_chain("Heading 1", &raw);
        assert_eq!(resolved.bold, Some(true), "own bold");
        assert_eq!(resolved.italic, Some(true), "inherits italic from Heading");
        assert_eq!(resolved.color, Some(0x00_FF00), "inherits color from Default (transitive)");
    }

    #[test]
    fn chain_child_overrides_parent_explicit_false() {
        let mut raw: HashMap<String, RawOdtStyle> = HashMap::new();
        raw.insert(
            "P".into(),
            raw_with(None, RawOdtStyle { bold: Some(true), ..Default::default() }),
        );
        raw.insert(
            "C".into(),
            // Child explicitly sets font-weight="normal" → Some(false).
            raw_with("P".into(), RawOdtStyle { bold: Some(false), ..Default::default() }),
        );
        let resolved = resolve_chain("C", &raw);
        assert_eq!(
            resolved.bold,
            Some(false),
            "child's explicit Some(false) must override parent's Some(true)"
        );
    }

    #[test]
    fn chain_cycle_does_not_infinite_loop() {
        let mut raw: HashMap<String, RawOdtStyle> = HashMap::new();
        raw.insert(
            "A".into(),
            raw_with("B".into(), RawOdtStyle { bold: Some(true), ..Default::default() }),
        );
        raw.insert(
            "B".into(),
            raw_with("A".into(), RawOdtStyle { italic: Some(true), ..Default::default() }),
        );
        // Must terminate. Partial chain is fine — what matters is no panic, no hang.
        let resolved = resolve_chain("A", &raw);
        assert_eq!(resolved.bold, Some(true), "A's own bold accumulates");
        assert_eq!(resolved.italic, Some(true), "B's italic accumulates before the cycle stops");
    }

    #[test]
    fn chain_dangling_parent_treated_as_root() {
        let mut raw: HashMap<String, RawOdtStyle> = HashMap::new();
        raw.insert(
            "Leaf".into(),
            raw_with(
                "DoesNotExist".into(),
                RawOdtStyle { bold: Some(true), ..Default::default() },
            ),
        );
        let resolved = resolve_chain("Leaf", &raw);
        assert_eq!(resolved.bold, Some(true));
        assert!(resolved.italic.is_none(), "no parent → nothing else to inherit");
    }

    #[test]
    fn chain_empty_parent_name_terminates() {
        let mut raw: HashMap<String, RawOdtStyle> = HashMap::new();
        // collect_raw_styles_into filters empty parent to None, but defensively
        // verify resolve_chain itself stops on empty string parent too.
        raw.insert(
            "X".into(),
            RawOdtStyle {
                parent: Some(String::new()),
                bold: Some(true),
                ..Default::default()
            },
        );
        let resolved = resolve_chain("X", &raw);
        assert_eq!(resolved.bold, Some(true));
    }

    #[test]
    fn end_to_end_parent_chain_resolves_through_parse_content_xml() {
        // styles.xml declares Heading (bold) → child Heading 1 (italic) inherits bold.
        let styles_xml = "<?xml version=\"1.0\"?>\
<office:document-styles \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
office:version=\"1.2\">\
<office:styles>\
<style:style style:name=\"Heading\" style:family=\"paragraph\">\
<style:text-properties fo:font-weight=\"bold\"/>\
</style:style>\
<style:style style:name=\"Heading 1\" style:family=\"paragraph\" style:parent-style-name=\"Heading\">\
<style:text-properties fo:font-style=\"italic\"/>\
</style:style>\
</office:styles>\
<office:automatic-styles></office:automatic-styles>\
</office:document-styles>";
        // Use Heading 1 directly on a text span (style-name resolution path).
        let body = "<text:p><text:span text:style-name=\"Heading 1\">x</text:span></text:p>";
        let c = minimal_content(body, "");
        let d = parse_content_xml(&c, styles_xml).expect("parses");
        let runs = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs,
            _ => panic!(),
        };
        let r = runs.iter().find(|r| r.text == "x").unwrap();
        assert!(r.style.bold, "inherited bold from parent 'Heading'");
        assert!(r.style.italic, "own italic on Heading 1");
    }

    #[test]
    fn end_to_end_child_normal_overrides_parent_bold() {
        // Parent bold, child explicitly font-weight="normal" → child renders not bold.
        let styles_xml = "<?xml version=\"1.0\"?>\
<office:document-styles \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
office:version=\"1.2\">\
<office:styles>\
<style:style style:name=\"BoldParent\" style:family=\"text\">\
<style:text-properties fo:font-weight=\"bold\"/>\
</style:style>\
<style:style style:name=\"NormalChild\" style:family=\"text\" style:parent-style-name=\"BoldParent\">\
<style:text-properties fo:font-weight=\"normal\"/>\
</style:style>\
</office:styles>\
<office:automatic-styles></office:automatic-styles>\
</office:document-styles>";
        let body = "<text:p><text:span text:style-name=\"NormalChild\">x</text:span></text:p>";
        let c = minimal_content(body, "");
        let d = parse_content_xml(&c, styles_xml).expect("parses");
        let runs = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs,
            _ => panic!(),
        };
        let r = runs.iter().find(|r| r.text == "x").unwrap();
        assert!(!r.style.bold, "explicit font-weight=normal on child wins over inherited bold");
    }

    #[test]
    fn end_to_end_paragraph_properties_inherit_through_chain() {
        // Parent sets text-align=center, child inherits — child paragraph renders centered.
        let styles_xml = "<?xml version=\"1.0\"?>\
<office:document-styles \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
office:version=\"1.2\">\
<office:styles>\
<style:style style:name=\"Centered\" style:family=\"paragraph\">\
<style:paragraph-properties fo:text-align=\"center\"/>\
</style:style>\
<style:style style:name=\"CenteredChild\" style:family=\"paragraph\" style:parent-style-name=\"Centered\">\
</style:style>\
</office:styles>\
<office:automatic-styles></office:automatic-styles>\
</office:document-styles>";
        let body = "<text:p text:style-name=\"CenteredChild\">hi</text:p>";
        let c = minimal_content(body, "");
        let d = parse_content_xml(&c, styles_xml).expect("parses");
        match &d.blocks[0] {
            ast::Block::Paragraph { props, .. } => {
                assert_eq!(
                    props.alignment,
                    ast::Alignment::Center,
                    "child inherits text-align=center from parent style"
                );
            }
            _ => panic!("expected paragraph"),
        }
    }
}
