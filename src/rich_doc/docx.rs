//! DOCX parser. Phase 1: read-only. Maps docx-rust's body tree into our `ast::Document`.
//!
//! Subset implemented: paragraphs, headings (Heading1..Heading6 via paragraph style id),
//! and inline bold/italic/underline. Lists, tables, images, color, and font size are
//! deferred to Phase 1b.

use super::{ast, container, DocFormat, RichDocError, RichDocPayload, Result};
use docx_rust::{
    document::{
        BodyContent, BreakType, ParagraphContent, RunContent, Table, TableCellContent, TableRowContent,
    },
    formatting::{CharacterProperty, ParagraphProperty},
    DocxFile,
};
use quick_xml::events::Event as QxEvent;
use quick_xml::Reader as QxReader;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub fn open(path: &Path) -> Result<RichDocPayload> {
    let original_bytes = std::fs::read(path)?;
    container::check_archive(&original_bytes)?;
    let raw_view_text = container::read_zip_entry_text(path, "word/document.xml")?;
    validate_xml_characters(&raw_view_text)?;
    let rels_xml =
        container::read_zip_entry_text(path, "word/_rels/document.xml.rels").unwrap_or_default();
    let rels = parse_rels(&rels_xml);
    let numbering_xml =
        container::read_zip_entry_text(path, "word/numbering.xml").unwrap_or_default();
    let num_map = parse_numbering_xml(&numbering_xml);
    let ast = parse(path, &original_bytes, &rels, &num_map)?;
    let headers = read_header_footer_blocks(path, "word/header", ".xml")?;
    let footers = read_header_footer_blocks(path, "word/footer", ".xml")?;
    Ok(RichDocPayload {
        format: DocFormat::Docx,
        ast,
        raw_view_text,
        original_bytes: original_bytes.into(),
        edit_log: Vec::new(),
        active_block: None,
        selection: None,
        drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        edit_group_start: None,
        headers,
        footers,
    })
}

/// Write `payload` back to `path`.
///
/// - `edit_log` empty → write `original_bytes` byte-for-byte (round-trip stays
///   identical to the source file, no XML re-serialization risk).
/// - `edit_log` non-empty → re-serialize `word/document.xml` from `ast`,
///   preserving the original document's namespace header and `<w:sectPr>`
///   block, then swap it into the original zip container (all other entries —
///   media, headers/footers, styles, comments — copy through unchanged).
pub fn save(payload: &RichDocPayload, path: &Path) -> Result<()> {
    if payload.edit_log.is_empty() {
        super::write_document_bytes(path, &payload.original_bytes)?;
        return Ok(());
    }
    super::ensure_text_only(&payload.ast)?;
    // Read the existing rels first so the LinkAllocator can dedupe new
    // hyperlinks against URLs Word already had. Without this, every save
    // appends a fresh `<Relationship Id="rIdRDN" Target="…">` row even when
    // the rels file already had `Id="rId7" Target="…"` for the same URL,
    // bloating the file with duplicate Targets.
    let original_rels = read_rels_from_bytes(&payload.original_bytes).unwrap_or_default();
    let existing_hyperlinks = parse_existing_hyperlinks(&original_rels);
    let mut links = LinkAllocator::with_existing(existing_hyperlinks);
    let new_doc_xml = serialize_document_xml_with_links(&payload.ast, &payload.raw_view_text, &mut links)?;

    // If any block in the AST carries list membership, the saved file must
    // include matching numbering definitions or Word/LibreOffice will
    // silently drop the `<w:numPr>` references and the bullets/numbers
    // vanish on reopen. Build the augmented numbering.xml / Content_Types
    // / rels eagerly; the dedup logic in the inject_* helpers makes
    // these no-ops when the entries are already present.
    let has_list = doc_has_any_list(&payload.ast);
    let mut new_rels_owned: Option<String> = None;
    let mut new_numbering: Option<String> = None;
    let mut new_content_types: Option<String> = None;
    if has_list {
        let existing_numbering = read_named_entry_from_bytes(&payload.original_bytes, "word/numbering.xml")
            .unwrap_or_default();
        let updated_numbering = if existing_numbering.trim().is_empty() {
            build_minted_numbering_xml()
        } else {
            inject_minted_numbering(&existing_numbering)
        };
        if updated_numbering != existing_numbering {
            new_numbering = Some(updated_numbering);
        }
        let existing_ct = read_named_entry_from_bytes(&payload.original_bytes, "[Content_Types].xml")
            .unwrap_or_default();
        let updated_ct = inject_numbering_content_type(&existing_ct);
        if updated_ct != existing_ct {
            new_content_types = Some(updated_ct);
        }
        let updated_rels = inject_numbering_relationship(&original_rels);
        if updated_rels != original_rels {
            new_rels_owned = Some(updated_rels);
        }
    }

    // Compose the replacement list. document.xml always replaced; rels
    // replaced if either hyperlinks added new entries OR numbering added a
    // new Relationship row; numbering.xml + Content_Types only when they
    // actually changed.
    let mut hyperlink_rels: Option<String> = None;
    if !links.is_empty() {
        // Start from whatever rels we already mutated for numbering (so
        // both edits land in the same final document), or the original.
        let base = new_rels_owned.as_deref().unwrap_or(&original_rels);
        hyperlink_rels = Some(inject_hyperlink_relationships(base, &links));
    }
    // Hyperlink rels supersede numbering-only rels (it's a superset edit).
    let final_rels: Option<String> = hyperlink_rels.or(new_rels_owned);

    let mut replacements: Vec<(&str, Vec<u8>)> = Vec::new();
    replacements.push(("word/document.xml", new_doc_xml.into_bytes()));
    if let Some(r) = final_rels {
        validate_xml_characters(&r)?;
        replacements.push(("word/_rels/document.xml.rels", r.into_bytes()));
    }
    if let Some(n) = new_numbering {
        validate_xml_characters(&n)?;
        replacements.push(("word/numbering.xml", n.into_bytes()));
    }
    if let Some(ct) = new_content_types {
        validate_xml_characters(&ct)?;
        replacements.push(("[Content_Types].xml", ct.into_bytes()));
    }
    let ref_replacements: Vec<(&str, &[u8])> = replacements
        .iter()
        .map(|(n, b)| (*n, b.as_slice()))
        .collect();
    let new_zip = container::replace_zip_entries(&payload.original_bytes, &ref_replacements)?;
    super::write_document_bytes(path, &new_zip)?;
    Ok(())
}

/// Read a single text entry from raw zip bytes (no filesystem hop). Used
/// during save to peek at `word/numbering.xml` and `[Content_Types].xml`
/// without re-opening the file. Returns `None` if the entry is missing
/// or the zip is corrupt.
fn read_named_entry_from_bytes(zip_bytes: &[u8], entry_name: &str) -> Option<String> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).ok()?;
    let mut entry = archive.by_name(entry_name).ok()?;
    let mut s = String::new();
    entry.read_to_string(&mut s).ok()?;
    Some(s)
}

/// True if any block (recursively into table cells) carries a list marker.
/// Drives the "do we need to inject numbering definitions on save?" branch.
fn doc_has_any_list(doc: &ast::Document) -> bool {
    fn block_has_list(b: &ast::Block) -> bool {
        match b {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => {
                props.list.is_some()
            }
            ast::Block::Table { rows } => rows
                .iter()
                .any(|r| r.cells.iter().any(|c| c.blocks.iter().any(block_has_list))),
            ast::Block::Image { .. } => false,
        }
    }
    doc.blocks.iter().any(block_has_list)
}

/// Build a `url → rId` map of the hyperlink-type relationships already
/// present in `document.xml.rels`. Drives the dedup path in
/// `LinkAllocator::with_existing` so a save that re-stamps an existing
/// hyperlink reuses Word's original rId.
fn parse_existing_hyperlinks(xml: &str) -> HashMap<String, String> {
    const HYPERLINK_TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    let mut out = HashMap::new();
    let mut reader = QxReader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(QxEvent::Eof) => break,
            Err(_) => break,
            Ok(QxEvent::Start(ref e)) | Ok(QxEvent::Empty(ref e)) => {
                let name = e.name();
                let local = name.as_ref().rsplit(|b| *b == b':').next().unwrap_or(name.as_ref());
                if local != b"Relationship" {
                    continue;
                }
                let mut id: Option<String> = None;
                let mut target: Option<String> = None;
                let mut ty: Option<String> = None;
                for attr in e.attributes().flatten() {
                    let k = attr.key.as_ref();
                    let s = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map(|c| c.into_owned())
                        .unwrap_or_default();
                    match k {
                        b"Id" => id = Some(s),
                        b"Target" => target = Some(s),
                        b"Type" => ty = Some(s),
                        _ => {}
                    }
                }
                if matches!(ty.as_deref(), Some(HYPERLINK_TYPE)) {
                    if let (Some(i), Some(t)) = (id, target) {
                        out.insert(t, i);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    out
}

fn read_rels_from_bytes(zip_bytes: &[u8]) -> Option<String> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).ok()?;
    let mut entry = archive.by_name("word/_rels/document.xml.rels").ok()?;
    let mut s = String::new();
    entry.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Insert new `<Relationship .../>` rows for every URL the allocator collected.
/// If the rels XML already contains an rId pointing at the same URL+hyperlink
/// type, we reuse it (no duplicate insertion). Falls back to a minimal valid
/// rels document if `original_rels` is empty.
fn inject_hyperlink_relationships(original_rels: &str, links: &LinkAllocator) -> String {
    const HYPERLINK_TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    let mut base = if original_rels.trim().is_empty() {
        String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>",
        )
    } else {
        original_rels.to_string()
    };
    let close_tag = "</Relationships>";
    let Some(close_idx) = base.rfind(close_tag) else {
        return base;
    };
    let mut insertions = String::new();
    for (url, rid) in links.entries.iter() {
        insertions.push_str("<Relationship Id=\"");
        push_xml_escaped(&mut insertions, rid);
        insertions.push_str("\" Type=\"");
        insertions.push_str(HYPERLINK_TYPE);
        insertions.push_str("\" Target=\"");
        push_xml_escaped(&mut insertions, url);
        insertions.push_str("\" TargetMode=\"External\"/>");
    }
    base.insert_str(close_idx, &insertions);
    base
}

fn push_xml_escaped(out: &mut String, s: &str) {
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

/// Allocates stable rIds for hyperlink URLs encountered during serialization.
/// Two pools:
/// - `existing`: hyperlink rels already present in the source's
///   `document.xml.rels` (`Target → Id`). Reused so saving a document
///   that already had `rId7 -> https://x.example` doesn't append a
///   duplicate `rIdRDN -> https://x.example` row.
/// - `entries`: NEW (url, rid) pairs for URLs the user added that aren't
///   in `existing`. Only these get injected into the rels file.
///
/// New ids use the `rIdRD{N}` namespace so they never collide with Word's
/// own `rId1`, `rId2`, … . The N counter starts above the highest
/// `rIdRD{K}` already in `existing` so re-saves don't reuse a now-taken
/// id either.
pub(super) struct LinkAllocator {
    entries: Vec<(String, String)>, // (url, rid) — NEW rels only
    existing: HashMap<String, String>, // url -> rid
    /// Seed for the new-id counter: max(N) over `rIdRD{N}` already in
    /// `existing`, or 0 if none. New ids are
    /// `rIdRD{rd_counter_seed + entries.len() + 1}`.
    rd_counter_seed: usize,
}

impl LinkAllocator {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            existing: HashMap::new(),
            rd_counter_seed: 0,
        }
    }

    pub(super) fn with_existing(existing: HashMap<String, String>) -> Self {
        let seed = existing
            .values()
            .filter_map(|rid| rid.strip_prefix("rIdRD").and_then(|s| s.parse::<usize>().ok()))
            .max()
            .unwrap_or(0);
        Self {
            entries: Vec::new(),
            existing,
            rd_counter_seed: seed,
        }
    }

    pub(super) fn intern(&mut self, url: &str) -> String {
        if let Some(idx) = self.entries.iter().position(|(u, _)| u == url) {
            return self.entries[idx].1.clone();
        }
        if let Some(rid) = self.existing.get(url) {
            return rid.clone();
        }
        // u128 so a seed parsed from the file as usize::MAX still counts up.
        let next = self.rd_counter_seed as u128 + self.entries.len() as u128 + 1;
        let rid = format!("rIdRD{next}");
        self.entries.push((url.to_string(), rid.clone()));
        rid
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Re-serialize an AST `Document` as `word/document.xml`. `original_xml` is the
/// source document.xml; we pull the namespace decls (so Word doesn't choke on a
/// missing prefix) and the `<w:sectPr>` block (so page/section settings survive)
/// out of it. Everything else in the body is regenerated from `ast` — formatting
/// we don't model (alignment, indentation, font color, lists, tables, images,
/// custom run properties) is *dropped*. That's Phase 2 scope: bold/italic/underline
/// round-trip without claiming to preserve arbitrary Word formatting.
pub(super) fn serialize_document_xml(ast: &ast::Document, original_xml: &str) -> Result<String> {
    // External callers (Phase 4 Save As .docx) don't care about link rels —
    // they build fresh documents from scratch with no hyperlinks. Drop the
    // allocator on the floor for them.
    let mut links = LinkAllocator::new();
    serialize_document_xml_with_links(ast, original_xml, &mut links)
}

pub(super) fn serialize_document_xml_with_links(
    ast: &ast::Document,
    original_xml: &str,
    links: &mut LinkAllocator,
) -> Result<String> {
    let ns_attrs = extract_namespace_attrs(original_xml);
    let sect_pr = extract_sect_pr(original_xml).unwrap_or_else(|| "<w:sectPr/>".to_string());

    let mut out = String::with_capacity(original_xml.len());
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
    out.push_str("<w:document");
    if !ns_attrs.is_empty() {
        out.push(' ');
        out.push_str(&ns_attrs);
    }
    out.push_str("><w:body>");
    for b in &ast.blocks {
        if let ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } = b {
            for run in runs {
                if let Some(url) = run.style.link.as_deref() {
                    validate_characters(url)?;
                }
            }
        }
        serialize_block(&mut out, b, links);
    }
    out.push_str(&sect_pr);
    out.push_str("</w:body></w:document>");
    validate_xml_characters(&out)?;
    Ok(out)
}

// XML 1.0 section 2.2: numeric character references must obey Char as well.
// https://www.w3.org/TR/xml/#charsets
fn validate_characters(text: &str) -> Result<()> {
    if let Some(ch) = text.chars().find(|ch| !matches!(*ch,
        '\t' | '\n' | '\r' | '\u{0020}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')) {
        return Err(RichDocError::Docx(format!("forbidden XML character U+{:04X}", ch as u32)));
    }
    Ok(())
}

fn validate_xml_characters(xml: &str) -> Result<()> {
    validate_characters(xml)?;
    let mut reader = QxReader::from_str(xml);
    loop {
        match reader.read_event().map_err(|err| RichDocError::Docx(err.to_string()))? {
            QxEvent::Eof => return Ok(()),
            QxEvent::Text(text) => {
                let decoded = text.unescape().map_err(|err| RichDocError::Docx(err.to_string()))?;
                validate_characters(&decoded)?;
            }
            QxEvent::Start(element) | QxEvent::Empty(element) => {
                for attr in element.attributes() {
                    let attr = attr.map_err(|err| RichDocError::Docx(err.to_string()))?;
                    let decoded = attr.decode_and_unescape_value(reader.decoder())
                        .map_err(|err| RichDocError::Docx(err.to_string()))?;
                    validate_characters(&decoded)?;
                }
            }
            _ => {}
        }
    }
}

/// Pull the attribute list from the opening `<w:document …>` element of the
/// source XML. Returns the substring between `<w:document` and the next `>`
/// (trimmed of leading whitespace). Falls back to a hardcoded minimal set if
/// the source can't be parsed — Word requires at least the `w:` namespace to
/// open the file at all.
fn extract_namespace_attrs(xml: &str) -> String {
    if let Some(start) = xml.find("<w:document") {
        let after = &xml[start + "<w:document".len()..];
        if let Some(end) = after.find('>') {
            return after[..end].trim().trim_end_matches('/').trim().to_string();
        }
    }
    "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"".to_string()
}

/// Pull `<w:sectPr…>…</w:sectPr>` (or self-closed `<w:sectPr/>`) out of the
/// source XML so we can re-attach it to the regenerated body. Without this
/// Word treats the document as having no section properties and reverts to
/// defaults (US Letter, no headers/footers binding) which surprises users.
fn extract_sect_pr(xml: &str) -> Option<String> {
    if let Some(start) = xml.find("<w:sectPr") {
        let after = &xml[start..];
        // Self-closed: <w:sectPr ... />
        if let Some(close) = after.find("/>") {
            let open_end = after.find('>')?;
            if close <= open_end {
                return Some(after[..close + 2].to_string());
            }
        }
        // Open/close pair: <w:sectPr ...>…</w:sectPr>
        if let Some(close) = after.find("</w:sectPr>") {
            return Some(after[..close + "</w:sectPr>".len()].to_string());
        }
    }
    None
}

fn serialize_block(out: &mut String, block: &ast::Block, links: &mut LinkAllocator) {
    match block {
        ast::Block::Paragraph { .. } | ast::Block::Heading { .. } => {}
        ast::Block::Table { .. } | ast::Block::Image { .. } => {
            // Save rejects these blocks before calling the text serializer.
            return;
        }
    }
    out.push_str("<w:p>");
    let (heading_level, props) = match block {
        ast::Block::Heading { level, props, .. } => (Some(*level), props),
        ast::Block::Paragraph { props, .. } => (None, props),
        ast::Block::Table { .. } | ast::Block::Image { .. } => return,
    };
    let needs_ppr = heading_level.is_some()
        || props.alignment != ast::Alignment::Left
        || props.indent_level > 0
        || props.list.is_some();
    if needs_ppr {
        out.push_str("<w:pPr>");
        if let Some(level) = heading_level {
            out.push_str("<w:pStyle w:val=\"Heading");
            out.push_str(&level.to_string());
            out.push_str("\"/>");
        }
        if let Some(marker) = props.list {
            let num_id = if marker.ordered {
                MINTED_NUM_ID_ORDERED
            } else {
                MINTED_NUM_ID_BULLET
            };
            out.push_str("<w:numPr>");
            out.push_str("<w:ilvl w:val=\"");
            out.push_str(&marker.level.to_string());
            out.push_str("\"/>");
            out.push_str("<w:numId w:val=\"");
            out.push_str(&num_id.to_string());
            out.push_str("\"/>");
            out.push_str("</w:numPr>");
        }
        if props.alignment != ast::Alignment::Left {
            out.push_str("<w:jc w:val=\"");
            out.push_str(alignment_to_docx_val(props.alignment));
            out.push_str("\"/>");
        }
        if props.indent_level > 0 {
            let twips = (props.indent_level as u32) * 720;
            out.push_str("<w:ind w:left=\"");
            out.push_str(&twips.to_string());
            out.push_str("\"/>");
        }
        out.push_str("</w:pPr>");
    }
    let runs = match block {
        ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
        ast::Block::Table { .. } | ast::Block::Image { .. } => return,
    };
    let mut previous_was_cr = false;
    for r in runs {
        if let Some(url) = r.style.link.as_deref() {
            let rid = links.intern(url);
            out.push_str("<w:hyperlink r:id=\"");
            push_xml_escaped(out, &rid);
            out.push_str("\">");
            serialize_run(out, r, &mut previous_was_cr);
            out.push_str("</w:hyperlink>");
        } else {
            serialize_run(out, r, &mut previous_was_cr);
        }
    }
    out.push_str("</w:p>");
}

fn serialize_run(out: &mut String, run: &ast::InlineRun, previous_was_cr: &mut bool) {
    out.push_str("<w:r>");
    let has_rpr = run.style.bold
        || run.style.italic
        || run.style.underline
        || run.style.strike
        || run.style.color.is_some();
    if has_rpr {
        out.push_str("<w:rPr>");
        if run.style.bold {
            out.push_str("<w:b/>");
        }
        if run.style.italic {
            out.push_str("<w:i/>");
        }
        if run.style.underline {
            // `single` is the standard Word underline style; matches what the
            // reader detects via `cp.underline.is_some()`.
            out.push_str("<w:u w:val=\"single\"/>");
        }
        if run.style.strike {
            out.push_str("<w:strike/>");
        }
        if let Some(rgb) = run.style.color {
            out.push_str("<w:color w:val=\"");
            push_hex6(out, rgb);
            out.push_str("\"/>");
        }
        out.push_str("</w:rPr>");
    }
    let mut text_open = false;
    for ch in run.text.chars() {
        let follows_cr = *previous_was_cr;
        *previous_was_cr = ch == '\r';
        if ch == '\n' && follows_cr {
            continue;
        }
        if matches!(ch, '\t' | '\r' | '\n' | '\u{000B}') {
            if text_open {
                out.push_str("</w:t>");
                text_open = false;
            }
            out.push_str(if ch == '\t' { "<w:tab/>" } else { "<w:br/>" });
        } else {
            if !text_open {
                out.push_str("<w:t xml:space=\"preserve\">");
                text_open = true;
            }
            let mut utf8 = [0; 4];
            xml_escape_into(out, ch.encode_utf8(&mut utf8));
        }
    }
    if text_open {
        out.push_str("</w:t>");
    }
    out.push_str("</w:r>");
}

fn push_hex6(out: &mut String, rgb: u32) {
    use std::fmt::Write;
    let _ = write!(out, "{:06X}", rgb & 0xFF_FFFF);
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

/// `bytes` is the archive `container::check_archive` vetted; docx-rust reads
/// it rather than re-opening `path`, so it parses exactly what was checked.
fn parse(
    path: &Path,
    bytes: &[u8],
    rels: &HashMap<String, String>,
    num_map: &HashMap<isize, bool>,
) -> Result<ast::Document> {
    let file = DocxFile::from_reader(std::io::Cursor::new(bytes))
        .map_err(|e| RichDocError::Docx(format!("{e:?}")))?;
    let docx = file.parse().map_err(|e| RichDocError::Docx(format!("{e:?}")))?;
    let mut blocks = Vec::new();
    for elem in &docx.document.body.content {
        match elem {
            BodyContent::Paragraph(p) => {
                // If the paragraph contains an inline picture, emit a
                // standalone Image block — Phase 5a treats images as block-
                // level so the renderer can size them independently. Adjacent
                // text in the same paragraph is dropped; real Word docs
                // typically isolate images in their own paragraph anyway.
                if let Some(img) = extract_paragraph_image(p, rels, path) {
                    blocks.push(img);
                } else {
                    blocks.push(paragraph_to_block_with_rels(p, rels, num_map));
                }
            }
            BodyContent::Table(t) => {
                blocks.push(table_to_block(t, rels, path, num_map));
            }
            // SDTs and other content elements stay unrepresented in the AST
            // — they survive in `original_bytes` and round-trip via the
            // byte-perfect save path. Editing them is out of v1 scope.
            _ => {}
        }
    }
    Ok(ast::Document { blocks })
}

/// Parse `word/_rels/document.xml.rels` into an `Id → Target` map. The Target
/// is a path relative to the rels file's directory (typically `word/`).
fn parse_rels(xml: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut reader = QxReader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(QxEvent::Eof) => break,
            Err(_) => break,
            Ok(QxEvent::Start(ref e)) | Ok(QxEvent::Empty(ref e)) => {
                let name = e.name();
                let local = name.as_ref().rsplit(|b| *b == b':').next().unwrap_or(name.as_ref());
                if local != b"Relationship" {
                    continue;
                }
                let mut id: Option<String> = None;
                let mut target: Option<String> = None;
                for attr in e.attributes().flatten() {
                    let k = attr.key.as_ref();
                    // Unescape XML entities (`&amp;` → `&`) so URLs like
                    // `https://x.example?a=1&amp;b=2` resolve to the source URL.
                    let s = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map(|c| c.into_owned())
                        .unwrap_or_default();
                    match k {
                        b"Id" => id = Some(s),
                        b"Target" => target = Some(s),
                        _ => {}
                    }
                }
                if let (Some(i), Some(t)) = (id, target) {
                    out.insert(i, t);
                }
            }
            _ => {}
        }
        buf.clear();
    }
    out
}

/// numId we mint for bulleted lists when injecting numbering definitions
/// into a saved document. Fixed-but-high value to avoid colliding with
/// numIds the source document already uses (Word typically allocates
/// rId/numId starting at 1).
const MINTED_NUM_ID_BULLET: isize = 9001;
/// numId we mint for ordered (decimal) lists. Companion to the bullet id.
const MINTED_NUM_ID_ORDERED: isize = 9002;
/// abstractNumId companions to the minted numIds — we use a 1:1 mapping
/// so the abstract id matches the numId for easy bookkeeping.
const MINTED_ABSTRACT_NUM_ID_BULLET: isize = 9001;
const MINTED_ABSTRACT_NUM_ID_ORDERED: isize = 9002;

/// Parse `word/numbering.xml` into a `numId → ordered` map. Walks every
/// `<w:abstractNum>` first to build `abstractNumId → ordered` (where
/// `ordered` is `numFmt != "bullet"` at level 0), then walks every
/// `<w:num>` to resolve its `<w:abstractNumId w:val="..."/>` reference.
/// Returns an empty map if the XML is empty or malformed — the caller
/// already treats missing entries as "default to bullet" for list
/// paragraphs that survived parsing.
fn parse_numbering_xml(xml: &str) -> HashMap<isize, bool> {
    let mut abstract_ordered: HashMap<isize, bool> = HashMap::new();
    let mut num_to_abstract: HashMap<isize, isize> = HashMap::new();

    let mut reader = QxReader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    // Walk state. We only care about level-0 numFmt for each abstractNum.
    let mut cur_abstract_id: Option<isize> = None;
    let mut cur_lvl_value: Option<isize> = None;
    let mut cur_abstract_level0_fmt: Option<String> = None;
    let mut in_lvl = false;
    let mut cur_num_id: Option<isize> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(QxEvent::Eof) => break,
            Err(_) => break,
            Ok(QxEvent::Start(ref e)) => {
                let name = e.name();
                let local = name.as_ref().rsplit(|b| *b == b':').next().unwrap_or(name.as_ref());
                match local {
                    b"abstractNum" => {
                        cur_abstract_id = read_xml_attr(e, "abstractNumId")
                            .and_then(|s| s.parse::<isize>().ok());
                        cur_abstract_level0_fmt = None;
                    }
                    b"lvl" => {
                        cur_lvl_value = read_xml_attr(e, "ilvl")
                            .and_then(|s| s.parse::<isize>().ok());
                        in_lvl = true;
                    }
                    b"num" => {
                        cur_num_id = read_xml_attr(e, "numId")
                            .and_then(|s| s.parse::<isize>().ok());
                    }
                    _ => {}
                }
            }
            Ok(QxEvent::Empty(ref e)) => {
                let name = e.name();
                let local = name.as_ref().rsplit(|b| *b == b':').next().unwrap_or(name.as_ref());
                if local == b"numFmt" && in_lvl && cur_lvl_value == Some(0) {
                    if let Some(val) = read_xml_attr(e, "val") {
                        cur_abstract_level0_fmt = Some(val);
                    }
                }
                if local == b"abstractNumId" {
                    if let (Some(num_id), Some(val)) = (
                        cur_num_id,
                        read_xml_attr(e, "val").and_then(|s| s.parse::<isize>().ok()),
                    ) {
                        num_to_abstract.insert(num_id, val);
                    }
                }
            }
            Ok(QxEvent::End(ref e)) => {
                let name = e.name();
                let local = name.as_ref().rsplit(|b| *b == b':').next().unwrap_or(name.as_ref());
                match local {
                    b"lvl" => {
                        in_lvl = false;
                        cur_lvl_value = None;
                    }
                    b"abstractNum" => {
                        if let Some(aid) = cur_abstract_id {
                            let ordered = match cur_abstract_level0_fmt.as_deref() {
                                // "bullet" is the only OOXML numFmt value
                                // that means an unordered list; everything
                                // else (decimal, lowerLetter, upperRoman, …)
                                // is an ordered presentation.
                                Some("bullet") => false,
                                Some(_) => true,
                                // Missing level-0 numFmt: treat as bullet.
                                // Real-world docs without a numFmt at level 0
                                // are typically broken; bullet is the safer
                                // fallback because the renderer can always
                                // produce a glyph without a counter.
                                None => false,
                            };
                            abstract_ordered.insert(aid, ordered);
                        }
                        cur_abstract_id = None;
                        cur_abstract_level0_fmt = None;
                    }
                    b"num" => {
                        cur_num_id = None;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        buf.clear();
    }

    let mut out = HashMap::new();
    for (num_id, abs_id) in num_to_abstract {
        if let Some(ordered) = abstract_ordered.get(&abs_id) {
            out.insert(num_id, *ordered);
        }
    }
    out
}

/// Build a fresh `word/numbering.xml` body that defines exactly our two
/// minted entries (bullet + ordered). Used when the source document had
/// no numbering part — emitted at save time alongside Content_Types and
/// rels updates.
fn build_minted_numbering_xml() -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
    out.push_str(
        "<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
    );
    push_minted_abstract_num_entries(&mut out);
    push_minted_num_entries(&mut out);
    out.push_str("</w:numbering>");
    out
}

/// Append our minted `<w:abstractNum>` definitions for bullet + ordered.
fn push_minted_abstract_num_entries(out: &mut String) {
    use std::fmt::Write;
    let _ = write!(
        out,
        "<w:abstractNum w:abstractNumId=\"{}\"><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"\u{2022}\"/><w:lvlJc w:val=\"left\"/></w:lvl></w:abstractNum>",
        MINTED_ABSTRACT_NUM_ID_BULLET
    );
    let _ = write!(
        out,
        "<w:abstractNum w:abstractNumId=\"{}\"><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/><w:lvlJc w:val=\"left\"/></w:lvl></w:abstractNum>",
        MINTED_ABSTRACT_NUM_ID_ORDERED
    );
}

/// Append our minted `<w:num>` instances that reference the abstractNums.
fn push_minted_num_entries(out: &mut String) {
    use std::fmt::Write;
    let _ = write!(
        out,
        "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/></w:num>",
        MINTED_NUM_ID_BULLET, MINTED_ABSTRACT_NUM_ID_BULLET
    );
    let _ = write!(
        out,
        "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/></w:num>",
        MINTED_NUM_ID_ORDERED, MINTED_ABSTRACT_NUM_ID_ORDERED
    );
}

/// Append our minted abstractNum + num entries to an existing
/// `word/numbering.xml` body, skipping any that are already present
/// (matches by `w:numId="9001"` / `w:abstractNumId="9001"` etc). Returns
/// the body if there's nothing to insert; otherwise returns the
/// augmented body with our entries inserted before `</w:numbering>`.
fn inject_minted_numbering(existing: &str) -> String {
    let needs_bullet_abstract = !existing.contains(&format!(
        "w:abstractNumId=\"{}\"",
        MINTED_ABSTRACT_NUM_ID_BULLET
    ));
    let needs_ordered_abstract = !existing.contains(&format!(
        "w:abstractNumId=\"{}\"",
        MINTED_ABSTRACT_NUM_ID_ORDERED
    ));
    let needs_bullet_num =
        !existing.contains(&format!("w:numId=\"{}\"", MINTED_NUM_ID_BULLET));
    let needs_ordered_num =
        !existing.contains(&format!("w:numId=\"{}\"", MINTED_NUM_ID_ORDERED));
    if !needs_bullet_abstract
        && !needs_ordered_abstract
        && !needs_bullet_num
        && !needs_ordered_num
    {
        return existing.to_string();
    }
    let close_tag = "</w:numbering>";
    let Some(close_idx) = existing.rfind(close_tag) else {
        // Malformed source numbering.xml: rebuild from scratch rather
        // than risk producing invalid XML by splicing into garbage.
        return build_minted_numbering_xml();
    };
    let mut out = String::with_capacity(existing.len() + 512);
    out.push_str(&existing[..close_idx]);
    // abstractNum entries must appear before num entries in numbering.xml
    // — the schema requires it. Insert in that order.
    let mut insertions = String::new();
    if needs_bullet_abstract || needs_ordered_abstract {
        let mut abs_buf = String::new();
        if needs_bullet_abstract {
            push_minted_bullet_abstract(&mut abs_buf);
        }
        if needs_ordered_abstract {
            push_minted_ordered_abstract(&mut abs_buf);
        }
        // Inserting just before the first <w:num> is the safest spot:
        // it keeps abstracts contiguous with the existing abstracts and
        // num instances contiguous with the existing nums.
        if let Some(first_num_idx) = out.find("<w:num ") {
            out.insert_str(first_num_idx, &abs_buf);
        } else {
            insertions.push_str(&abs_buf);
        }
    }
    if needs_bullet_num {
        let mut num_buf = String::new();
        use std::fmt::Write;
        let _ = write!(
            num_buf,
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/></w:num>",
            MINTED_NUM_ID_BULLET, MINTED_ABSTRACT_NUM_ID_BULLET
        );
        insertions.push_str(&num_buf);
    }
    if needs_ordered_num {
        let mut num_buf = String::new();
        use std::fmt::Write;
        let _ = write!(
            num_buf,
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/></w:num>",
            MINTED_NUM_ID_ORDERED, MINTED_ABSTRACT_NUM_ID_ORDERED
        );
        insertions.push_str(&num_buf);
    }
    out.push_str(&insertions);
    out.push_str(close_tag);
    out
}

fn push_minted_bullet_abstract(out: &mut String) {
    use std::fmt::Write;
    let _ = write!(
        out,
        "<w:abstractNum w:abstractNumId=\"{}\"><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"\u{2022}\"/><w:lvlJc w:val=\"left\"/></w:lvl></w:abstractNum>",
        MINTED_ABSTRACT_NUM_ID_BULLET
    );
}

fn push_minted_ordered_abstract(out: &mut String) {
    use std::fmt::Write;
    let _ = write!(
        out,
        "<w:abstractNum w:abstractNumId=\"{}\"><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/><w:lvlJc w:val=\"left\"/></w:lvl></w:abstractNum>",
        MINTED_ABSTRACT_NUM_ID_ORDERED
    );
}

/// Ensure `[Content_Types].xml` declares an Override for
/// `/word/numbering.xml`. If the override is already present, returns the
/// XML unchanged; otherwise inserts one before `</Types>`. Required when
/// a document didn't originally have a numbering part — Word rejects
/// numbering.xml entries that lack a Content_Types Override.
fn inject_numbering_content_type(content_types_xml: &str) -> String {
    const NUMBERING_PART: &str = "/word/numbering.xml";
    const NUMBERING_CT: &str =
        "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
    if content_types_xml.contains(&format!("PartName=\"{}\"", NUMBERING_PART)) {
        return content_types_xml.to_string();
    }
    let close_tag = "</Types>";
    let Some(close_idx) = content_types_xml.rfind(close_tag) else {
        return content_types_xml.to_string();
    };
    let mut out = String::with_capacity(content_types_xml.len() + 128);
    out.push_str(&content_types_xml[..close_idx]);
    out.push_str(&format!(
        "<Override PartName=\"{}\" ContentType=\"{}\"/>",
        NUMBERING_PART, NUMBERING_CT
    ));
    out.push_str(close_tag);
    out
}

/// Ensure `word/_rels/document.xml.rels` declares a relationship of type
/// `…/numbering` targeting `numbering.xml`. If a relationship of that
/// type is already present, returns the XML unchanged; otherwise mints
/// a fresh rId in the `rIdRDNUM` namespace (separate from the
/// hyperlink `rIdRD{N}` allocator so they never collide) and inserts a
/// `<Relationship/>` row before `</Relationships>`.
fn inject_numbering_relationship(rels_xml: &str) -> String {
    const NUMBERING_TYPE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
    if rels_xml.contains(NUMBERING_TYPE) {
        return rels_xml.to_string();
    }
    let mut base = if rels_xml.trim().is_empty() {
        String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>",
        )
    } else {
        rels_xml.to_string()
    };
    let close_tag = "</Relationships>";
    let Some(close_idx) = base.rfind(close_tag) else {
        return base;
    };
    let row = format!(
        "<Relationship Id=\"rIdRDNUM\" Type=\"{}\" Target=\"numbering.xml\"/>",
        NUMBERING_TYPE
    );
    base.insert_str(close_idx, &row);
    base
}

fn table_to_block(
    t: &Table,
    rels: &HashMap<String, String>,
    zip_path: &Path,
    num_map: &HashMap<isize, bool>,
) -> ast::Block {
    let mut rows = Vec::new();
    for tr in &t.rows {
        let mut cells = Vec::new();
        for trc in &tr.cells {
            if let TableRowContent::TableCell(tc) = trc {
                let mut cell_blocks = Vec::new();
                for cc in &tc.content {
                    let TableCellContent::Paragraph(p) = cc;
                    if let Some(img) = extract_paragraph_image(p, rels, zip_path) {
                        cell_blocks.push(img);
                    } else {
                        cell_blocks.push(paragraph_to_block_with_rels(p, rels, num_map));
                    }
                }
                cells.push(ast::TableCell { blocks: cell_blocks });
            }
            // Nested SDTs inside table rows aren't extracted.
        }
        rows.push(ast::TableRow { cells });
    }
    ast::Block::Table { rows }
}

/// Walk a paragraph's runs looking for a `<w:drawing>` containing an inline or
/// anchored picture. Returns `Some(Block::Image)` if found and the rId resolves
/// to a readable media entry; `None` otherwise. The first picture wins —
/// multi-image paragraphs are rare and supporting them would require splitting
/// the paragraph, which the renderer isn't ready for.
fn extract_paragraph_image(
    p: &docx_rust::document::Paragraph,
    rels: &HashMap<String, String>,
    zip_path: &Path,
) -> Option<ast::Block> {
    for pc in &p.content {
        let ParagraphContent::Run(r) = pc else {
            continue;
        };
        for rc in &r.content {
            let RunContent::Drawing(d) = rc else {
                continue;
            };
            // Inline (in-flow with text) and Anchor (floating) both wrap a
            // Graphic with a Picture. Prefer Inline since that's what
            // "inline image in document.xml" actually means; fall back to
            // Anchor so we still render floats.
            let (graphic_opt, extent_opt) = if let Some(inline) = d.inline.as_ref() {
                (inline.graphic.as_ref(), inline.extent.as_ref())
            } else if let Some(anchor) = d.anchor.as_ref() {
                (anchor.graphic.as_ref(), anchor.extent.as_ref())
            } else {
                continue;
            };
            let graphic = graphic_opt?;
            let pic = graphic.data.children.first()?;
            let embed = pic.fill.blip.embed.as_ref();
            if embed.is_empty() {
                continue;
            }
            let target = rels.get(embed)?;
            // `Target` in document.xml.rels is relative to the rels file's
            // own folder. document.xml.rels lives in `word/_rels/`, so a
            // Target like `media/image1.png` resolves to `word/media/image1.png`.
            let entry = if target.starts_with('/') {
                target.trim_start_matches('/').to_string()
            } else {
                format!("word/{}", target)
            };
            let bytes = container::read_zip_entry_bytes(zip_path, &entry).ok()?;
            let mime = mime_from_filename(&entry)?;
            let (w_px, h_px) = extent_opt
                .map(|e| (emu_to_px(e.cx), emu_to_px(e.cy)))
                .unwrap_or((None, None));
            return Some(ast::Block::Image {
                source: ast::ImageSource::Inline {
                    bytes: Arc::new(bytes),
                    mime: mime.into(),
                },
                width_px: w_px,
                height_px: h_px,
                alt: None,
            });
        }
    }
    None
}

/// Convert English Metric Units to pixels at 96 DPI. 914400 EMUs per inch.
/// Computed in u128 because the extent comes from the file; results past
/// `u32::MAX` saturate, as the ODT length conversion does.
fn emu_to_px(emu: u64) -> Option<u32> {
    if emu == 0 {
        return None;
    }
    let px = u128::from(emu) * 96 / 914_400;
    Some(u32::try_from(px).unwrap_or(u32::MAX))
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

/// Discover all header/footer XML entries in the zip and parse each into a
/// block sequence. Ignores the companion `_rels/*.rels` files — those aren't
/// header content, just rId tables we don't need for read-only display.
fn read_header_footer_blocks(
    path: &Path,
    prefix: &str,
    suffix: &str,
) -> Result<Vec<Vec<ast::Block>>> {
    let names = container::list_zip_entries(path, prefix, suffix)?;
    let mut out = Vec::new();
    for name in names {
        if name.contains("/_rels/") {
            continue;
        }
        let xml = container::read_zip_entry_text(path, &name)?;
        validate_xml_characters(&xml)?;
        out.push(parse_paragraphs_from_xml(&xml));
    }
    Ok(out)
}

/// Lightweight DOCX paragraph walker — handles `<w:p>` with optional `<w:pStyle
/// w:val="HeadingN"/>`, `<w:r>` runs whose `<w:rPr>` may carry `<w:b/>`,
/// `<w:i/>`, `<w:u w:val="..."/>`, `<w:t>` text, soft breaks and tabs. Drops everything else
/// (drawings, fields, comments, math). Used for headers and footers where
/// docx-rust doesn't give us a typed entry point without re-exporting hard_xml.
fn parse_paragraphs_from_xml(xml: &str) -> Vec<ast::Block> {
    let mut reader = QxReader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut blocks: Vec<ast::Block> = Vec::new();
    let mut in_p = false;
    let mut in_r = false;
    let mut in_rpr = false;
    let mut in_ppr = false;
    let mut in_t = false;
    let mut cur_heading: Option<u8> = None;
    let mut cur_runs: Vec<ast::InlineRun> = Vec::new();
    let mut cur_props = ast::BlockProps::default();
    let mut run_style = ast::InlineStyle::default();
    let mut run_text = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(QxEvent::Eof) => break,
            Err(_) => break,
            Ok(QxEvent::Start(ref e)) => {
                let name = e.name();
                let local = name.as_ref().rsplit(|b| *b == b':').next().unwrap_or(name.as_ref());
                match local {
                    b"p" => {
                        in_p = true;
                        cur_runs.clear();
                        cur_heading = None;
                        cur_props = ast::BlockProps::default();
                    }
                    b"pPr" if in_p => in_ppr = true,
                    b"r" if in_p => {
                        in_r = true;
                        run_style = ast::InlineStyle::default();
                    }
                    b"rPr" if in_r => in_rpr = true,
                    b"t" if in_r => {
                        in_t = true;
                        run_text.clear();
                    }
                    b"br" | b"cr" | b"tab" if in_r && !in_rpr => {
                        append_xml_control(&mut cur_runs, local, e, &run_style);
                    }
                    _ => {}
                }
            }
            Ok(QxEvent::Empty(ref e)) => {
                let name = e.name();
                let local = name.as_ref().rsplit(|b| *b == b':').next().unwrap_or(name.as_ref());
                if in_r && !in_rpr {
                    append_xml_control(&mut cur_runs, local, e, &run_style);
                }
                if in_rpr {
                    match local {
                        b"b" => run_style.bold = true,
                        b"i" => run_style.italic = true,
                        b"u" => run_style.underline = true,
                        b"strike" => run_style.strike = true,
                        _ => {}
                    }
                }
                if in_ppr && local == b"pStyle" {
                    if let Some(val) = read_xml_attr(e, "val") {
                        if let Some(suffix) = val.strip_prefix("Heading") {
                            if let Ok(n) = suffix.parse::<u8>() {
                                if (1..=6).contains(&n) {
                                    cur_heading = Some(n);
                                }
                            }
                        }
                    }
                }
                if in_ppr && local == b"jc" {
                    if let Some(val) = read_xml_attr(e, "val") {
                        cur_props.alignment = parse_docx_alignment(&val);
                    }
                }
                if in_ppr && local == b"ind" {
                    // w:left (DOCX) / w:start (DOCX 2010+) both express the
                    // leading indent in twentieths of a point (twips). A
                    // 720-twip step = 0.5 inch, our "level 1".
                    let val = read_xml_attr(e, "left").or_else(|| read_xml_attr(e, "start"));
                    if let Some(s) = val {
                        if let Ok(twips) = s.parse::<i32>() {
                            cur_props.indent_level = ((twips.max(0) as u32) / 720).min(255) as u8;
                        }
                    }
                }
            }
            Ok(QxEvent::Text(t)) => {
                if in_t {
                    let s = t.unescape().unwrap_or_default().into_owned();
                    run_text.push_str(&s);
                }
            }
            Ok(QxEvent::End(ref e)) => {
                let name = e.name();
                let local = name.as_ref().rsplit(|b| *b == b':').next().unwrap_or(name.as_ref());
                match local {
                    b"t" => {
                        if in_t && !run_text.is_empty() {
                            cur_runs.push(ast::InlineRun {
                                text: std::mem::take(&mut run_text),
                                style: run_style.clone(),
                            });
                        }
                        in_t = false;
                    }
                    b"rPr" => in_rpr = false,
                    b"r" => in_r = false,
                    b"pPr" => in_ppr = false,
                    b"p" => {
                        if !cur_runs.is_empty() {
                            let runs = std::mem::take(&mut cur_runs);
                            let props = std::mem::take(&mut cur_props);
                            let block = match cur_heading {
                                Some(level) => ast::Block::Heading { level, runs, props },
                                None => ast::Block::Paragraph { runs, props },
                            };
                            blocks.push(block);
                        }
                        in_p = false;
                        cur_heading = None;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        buf.clear();
    }
    blocks
}

fn append_xml_control(runs: &mut Vec<ast::InlineRun>, name: &[u8], element: &quick_xml::events::BytesStart, style: &ast::InlineStyle) {
    let text = match name {
        b"tab" => "\t",
        b"cr" => "\n",
        b"br" if read_xml_attr(element, "type").is_none_or(|ty| ty == "textWrapping") => "\n",
        _ => return,
    };
    runs.push(ast::InlineRun { text: text.into(), style: style.clone() });
}

fn read_xml_attr(e: &quick_xml::events::BytesStart, target_local: &str) -> Option<String> {
    let target = target_local.as_bytes();
    for attr in e.attributes().flatten() {
        let k = attr.key.as_ref();
        let local = k.rsplit(|b| *b == b':').next().unwrap_or(k);
        if local == target {
            return std::str::from_utf8(attr.value.as_ref()).ok().map(|s| s.to_string());
        }
    }
    None
}

fn paragraph_to_block_with_rels(
    p: &docx_rust::document::Paragraph,
    rels: &HashMap<String, String>,
    num_map: &HashMap<isize, bool>,
) -> ast::Block {
    let runs = collect_runs_with_rels(p, rels);
    let props = block_props_from_pp(p.property.as_ref(), num_map);
    if let Some(level) = heading_level(p.property.as_ref()) {
        ast::Block::Heading { level, runs, props }
    } else {
        ast::Block::Paragraph { runs, props }
    }
}

/// Parse `<w:jc w:val="..."/>` value into our Alignment enum. Defaults to
/// Left for unknown / empty / "start" (RTL handling deferred — we treat
/// "start" as Left and "end" as Right, which matches LTR readers).
fn parse_docx_alignment(s: &str) -> ast::Alignment {
    match s {
        "center" => ast::Alignment::Center,
        "right" | "end" => ast::Alignment::Right,
        "both" | "distribute" => ast::Alignment::Justify,
        _ => ast::Alignment::Left,
    }
}

fn alignment_to_docx_val(a: ast::Alignment) -> &'static str {
    match a {
        ast::Alignment::Left => "left",
        ast::Alignment::Center => "center",
        ast::Alignment::Right => "right",
        ast::Alignment::Justify => "both",
    }
}

/// Pull alignment + left indent + list membership from a docx-rust
/// ParagraphProperty. Falls back to defaults when missing — Word treats
/// missing entries as "use the inherited paragraph style" but we don't
/// model inherited styles in the AST, so we promote to "Left + 0 + none"
/// which is correct for the vast majority of bodies.
///
/// `num_map` is the parsed `word/numbering.xml` map (numId → ordered);
/// when `pp.numbering.id` references an id missing from the map the
/// fallback is `ordered: false` (bullet) since that's the safer guess
/// when the numbering definition is missing.
fn block_props_from_pp(
    pp: Option<&ParagraphProperty>,
    num_map: &HashMap<isize, bool>,
) -> ast::BlockProps {
    let mut props = ast::BlockProps::default();
    if let Some(pp) = pp {
        if let Some(j) = pp.justification.as_ref() {
            let val = match j.value {
                docx_rust::formatting::JustificationVal::Center => "center",
                docx_rust::formatting::JustificationVal::Right
                | docx_rust::formatting::JustificationVal::End => "right",
                docx_rust::formatting::JustificationVal::Both
                | docx_rust::formatting::JustificationVal::Distribute => "both",
                _ => "left",
            };
            props.alignment = parse_docx_alignment(val);
        }
        if let Some(ind) = pp.indent.as_ref() {
            if let Some(twips) = ind.left {
                let twips_u = twips.max(0) as u32;
                props.indent_level = (twips_u / 720).min(255) as u8;
            }
        }
        if let Some(np) = pp.numbering.as_ref() {
            if let Some(id) = np.id.as_ref() {
                let ordered = num_map.get(&id.value).copied().unwrap_or(false);
                let level = np
                    .level
                    .as_ref()
                    .map(|l| l.value.max(0).min(255) as u8)
                    .unwrap_or(0);
                props.list = Some(ast::ListMarker { ordered, level });
            }
        }
    }
    props
}

fn heading_level(pp: Option<&ParagraphProperty>) -> Option<u8> {
    let style_id = pp?.style_id.as_ref()?.value.as_ref();
    // Word style IDs for the built-in heading styles: Heading1..Heading9.
    let suffix = style_id.strip_prefix("Heading")?;
    let n: u8 = suffix.parse().ok()?;
    if (1..=6).contains(&n) {
        Some(n)
    } else {
        None
    }
}

fn collect_runs_with_rels(
    p: &docx_rust::document::Paragraph,
    rels: &HashMap<String, String>,
) -> Vec<ast::InlineRun> {
    let mut out = Vec::new();
    for pc in &p.content {
        match pc {
            ParagraphContent::Run(r) => {
                let style = inline_style(r.property.as_ref());
                emit_run_text(&mut out, r, &style);
            }
            ParagraphContent::Link(h) => {
                let url = h
                    .id
                    .as_ref()
                    .and_then(|rid| rels.get(rid.as_ref()))
                    .cloned();
                if let Some(r) = h.content.as_ref() {
                    let mut style = inline_style(r.property.as_ref());
                    style.link = url;
                    emit_run_text(&mut out, r, &style);
                }
            }
            _ => {}
        }
    }
    out
}

fn emit_run_text(out: &mut Vec<ast::InlineRun>, r: &docx_rust::document::Run, style: &ast::InlineStyle) {
    for rc in &r.content {
        let text = match rc {
            RunContent::Text(text) => text.text.as_ref(),
            RunContent::Tab(_) => "\t",
            RunContent::CarriageReturn(_) => "\n",
            RunContent::Break(br) if matches!(br.ty, None | Some(BreakType::TextWrapping)) => "\n",
            _ => continue,
        };
        if !text.is_empty() {
            out.push(ast::InlineRun { text: text.into(), style: style.clone() });
        }
    }
}

fn inline_style(cp: Option<&CharacterProperty>) -> ast::InlineStyle {
    let Some(cp) = cp else {
        return ast::InlineStyle::default();
    };
    ast::InlineStyle {
        bold: cp.bold.is_some(),
        italic: cp.italics.is_some(),
        underline: cp.underline.is_some(),
        strike: cp.strike.is_some(),
        color: parse_color_hex(cp.color.as_ref().map(|c| c.value.as_ref())),
        link: None,
    }
}

/// Parse a DOCX `<w:color w:val="FFAABB"/>` hex string into 0xRRGGBB.
/// "auto" and unparseable values return None.
fn parse_color_hex(val: Option<&str>) -> Option<u32> {
    let val = val?;
    if val.is_empty() || val.eq_ignore_ascii_case("auto") {
        return None;
    }
    u32::from_str_radix(val.trim_start_matches('#'), 16).ok().filter(|&n| n <= 0xFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("src/rich_doc/fixtures");
        p.push(name);
        p
    }

    const CONTROL_PARAGRAPHS: &str = "<w:p><w:r><w:rPr><w:b/><w:i/><w:u w:val=\"single\"/></w:rPr><w:t>A\u{00E9}</w:t><w:br/><w:t>B</w:t><w:br w:type=\"textWrapping\"></w:br><w:t>C</w:t><w:cr/><w:t>D</w:t><w:tab/><w:t>E\u{1F600}</w:t></w:r><w:r><w:t>plain</w:t></w:r></w:p><w:p><w:r><w:tab/><w:cr></w:cr></w:r></w:p>";

    fn control_fixture(path: &Path) -> Vec<u8> {
        let original = std::fs::read(fixture("hello.docx")).expect("fixture");
        let document = format!("<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><w:body>{CONTROL_PARAGRAPHS}</w:body></w:document>");
        let header = format!("<w:hdr xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{CONTROL_PARAGRAPHS}</w:hdr>");
        let footer = header.replace("w:hdr", "w:ftr");
        let bytes = container::replace_zip_entries(&original, &[
            ("word/document.xml", document.as_bytes()), ("word/header1.xml", header.as_bytes()),
            ("word/footer1.xml", footer.as_bytes()),
        ]).expect("parts");
        std::fs::write(path, &bytes).expect("fixture");
        bytes
    }

    fn block_runs(block: &ast::Block) -> &[ast::InlineRun] {
        match block {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => panic!("text fixture"),
        }
    }

    fn block_text(block: &ast::Block) -> String {
        block_runs(block).iter().map(|run| run.text.as_str()).collect()
    }

    fn check_control_blocks(blocks: &[ast::Block]) {
        assert_eq!(blocks.len(), 2);
        assert_eq!(block_text(&blocks[0]), "A\u{00E9}\nB\nC\nD\tE\u{1F600}plain");
        assert_eq!(block_text(&blocks[1]), "\t\n");
        let runs = block_runs(&blocks[0]);
        for run in runs.iter().filter(|run| run.text != "plain") {
            assert!(run.style.bold && run.style.italic && run.style.underline, "{run:?}");
        }
        assert_eq!(runs.last().expect("plain").style, ast::InlineStyle::default());
    }

    #[test]
    fn body_breaks_and_tabs_keep_their_text_and_style() {
        let dir = crate::persist::TempDir::new("docx-body-controls");
        let path = dir.join("controls.docx");
        let bytes = control_fixture(&path);
        let payload = open(&path).expect("open");
        check_control_blocks(&payload.ast.blocks);
        save(&payload, &path).expect("unchanged save");
        assert_eq!(std::fs::read(path).expect("unchanged bytes"), bytes);
    }

    #[test]
    fn header_and_footer_breaks_and_tabs_keep_their_text_and_style() {
        let dir = crate::persist::TempDir::new("docx-header-controls");
        let path = dir.join("controls.docx");
        control_fixture(&path);
        let payload = open(&path).expect("open");
        assert_eq!(payload.headers.len(), 1);
        assert_eq!(payload.footers.len(), 1);
        check_control_blocks(&payload.headers[0]);
        check_control_blocks(&payload.footers[0]);
    }

    #[test]
    fn edited_soft_breaks_and_tabs_reopen_without_doubling_crlf() {
        let dir = crate::persist::TempDir::new("docx-edited-controls");
        let path = dir.join("controls.docx");
        control_fixture(&path);
        let mut payload = open(&path).expect("open");
        let bold = ast::InlineStyle { bold: true, ..Default::default() };
        let italic = ast::InlineStyle { italic: true, color: Some(0x123456), link: Some("https://example.test/".into()), ..Default::default() };
        payload.ast.blocks = vec![ast::Block::Paragraph { runs: vec![
            ast::InlineRun { text: "\u{00E9}\r".into(), style: bold.clone() },
            ast::InlineRun { text: "\n\u{1F600}\tX\u{000B} Y\r\nZ\n".into(), style: italic.clone() },
        ], props: Default::default() }];
        payload.apply_edit(super::super::edits::Edit::InsertText { block: 0, run: 0, byte: 0, text: "!".into() });
        let header = container::read_zip_entry_text(&path, "word/header1.xml").expect("header");
        let footer = container::read_zip_entry_text(&path, "word/footer1.xml").expect("footer");
        save(&payload, &path).expect("save");
        let xml = container::read_zip_entry_text(&path, "word/document.xml").expect("XML");
        assert!(!xml.contains('\u{000B}'));
        assert_eq!(xml.matches("<w:br/>").count(), 4);
        assert_eq!(xml.matches("<w:tab/>").count(), 1);
        let reopened = open(&path).expect("reopen");
        assert_eq!(block_text(&reopened.ast.blocks[0]), "!\u{00E9}\n\u{1F600}\tX\n Y\nZ\n");
        let actual: Vec<_> = block_runs(&reopened.ast.blocks[0]).iter()
            .flat_map(|run| run.text.chars().map(|ch| (ch, run.style.clone()))).collect();
        let expected: Vec<_> = "!\u{00E9}\n".chars().map(|ch| (ch, bold.clone()))
            .chain("\u{1F600}\tX\n Y\nZ\n".chars().map(|ch| (ch, italic.clone()))).collect();
        assert_eq!(actual, expected);
        assert_eq!(container::read_zip_entry_text(&path, "word/header1.xml").expect("header"), header);
        assert_eq!(container::read_zip_entry_text(&path, "word/footer1.xml").expect("footer"), footer);
    }

    #[test]
    fn forbidden_xml_text_and_links_refuse_save_without_changing_file() {
        let dir = crate::persist::TempDir::new("docx-forbidden-controls");
        let path = dir.join("controls.docx");
        let original = control_fixture(&path);
        for ch in ['\0', '\u{0001}', '\u{000C}', '\u{001F}', '\u{FFFE}', '\u{FFFF}'] {
            let mut payload = open(&path).expect("open");
            payload.apply_edit(super::super::edits::Edit::InsertText { block: 0, run: 0, byte: 0, text: ch.to_string() });
            assert!(matches!(save(&payload, &path), Err(RichDocError::Docx(_))), "U+{:04X}", ch as u32);
            assert_eq!(std::fs::read(&path).expect("original bytes"), original);
        }
        let mut payload = open(&path).expect("open");
        payload.apply_edit(super::super::edits::Edit::SetBlockLink { block: 0, url: Some("https://example.test/\u{000B}".into()) });
        assert!(matches!(save(&payload, &path), Err(RichDocError::Docx(_))));
        assert_eq!(std::fs::read(path).expect("original bytes"), original);
    }

    #[test]
    fn forbidden_source_xml_and_character_references_are_rejected() {
        let dir = crate::persist::TempDir::new("docx-forbidden-input");
        let path = dir.join("controls.docx");
        let original = std::fs::read(fixture("hello.docx")).expect("fixture");
        for (part, root) in [("word/document.xml", "document"), ("word/header1.xml", "hdr"), ("word/footer1.xml", "ftr")] {
            for text in ["<w:t>\u{000B}</w:t>", "<w:t>&#xB;</w:t>", "<w:t data-test=\"&#xB;\">valid</w:t>"] {
                let paragraph = format!("<w:p><w:r>{text}</w:r></w:p>");
                let content = if root == "document" { format!("<w:body>{paragraph}</w:body>") } else { paragraph };
                let xml = format!("<w:{root} xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{content}</w:{root}>");
                let bytes = container::replace_zip_entry(&original, part, xml.as_bytes()).expect("parts");
                std::fs::write(&path, bytes).expect("fixture");
                assert!(open(&path).is_err(), "{part}: {text:?}");
            }
        }
    }

    #[test]
    fn edited_document_with_a_table_is_not_silently_truncated() {
        let dir = crate::persist::TempDir::new("docx-table-save");
        let path = dir.join("document.docx");
        let original = std::fs::read(fixture("hello.docx")).expect("fixture");
        let xml = container::read_zip_entry_text(&fixture("hello.docx"), "word/document.xml").expect("XML");
        let table = "<w:tbl><w:tblGrid><w:gridCol w:w=\"2400\"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>keep this table</w:t></w:r></w:p></w:tc></w:tr></w:tbl>";
        let xml = xml.replace("</w:body>", &format!("{table}</w:body>"));
        let original = container::replace_zip_entry(&original, "word/document.xml", xml.as_bytes()).expect("container");
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
    fn opens_hello_docx_into_payload() {
        let payload = open(&fixture("hello.docx")).expect("opens");
        assert_eq!(payload.format, DocFormat::Docx);
        assert!(!payload.raw_view_text.is_empty());
        assert!(
            !payload.ast.blocks.is_empty(),
            "fixture should have at least one block"
        );
    }

    #[test]
    fn extracts_at_least_one_text_run() {
        let payload = open(&fixture("hello.docx")).expect("opens");
        let any_text: String = payload
            .ast
            .blocks
            .iter()
            .flat_map(|b| match b {
                ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs.iter(),
                _ => [].iter(),
            })
            .map(|r| r.text.as_str())
            .collect();
        assert!(!any_text.is_empty(), "should pull some text from the fixture");
    }

    #[test]
    fn heading_level_parses_heading1_through_6() {
        for n in 1u8..=6 {
            let pp = ParagraphProperty {
                style_id: Some(docx_rust::formatting::ParagraphStyleId {
                    value: format!("Heading{n}").into(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert_eq!(heading_level(Some(&pp)), Some(n));
        }
    }

    #[test]
    fn serialize_paragraph_emits_w_p_with_runs() {
        let doc = ast::Document {
            blocks: vec![ast::Block::Paragraph {
                runs: vec![ast::InlineRun {
                    text: "hello".into(),
                    style: ast::InlineStyle {
                        bold: true,
                        ..Default::default()
                    },
                }],
                props: ast::BlockProps::default(),
            }],
        };
        let xml = serialize_document_xml(&doc, "<w:document xmlns:w=\"ns\"><w:body></w:body></w:document>").expect("serialize");
        assert!(xml.contains("<w:body>"));
        assert!(xml.contains("<w:p>"));
        assert!(xml.contains("<w:b/>"));
        assert!(xml.contains(">hello</w:t>"));
    }

    #[test]
    fn serialize_heading_emits_pstyle_heading_n() {
        let doc = ast::Document {
            blocks: vec![ast::Block::Heading {
                level: 2,
                runs: vec![ast::InlineRun::plain("Title")],
                props: ast::BlockProps::default(),
            }],
        };
        let xml = serialize_document_xml(&doc, "<w:document xmlns:w=\"ns\"></w:document>").expect("serialize");
        assert!(xml.contains("<w:pStyle w:val=\"Heading2\"/>"));
        assert!(xml.contains(">Title</w:t>"));
    }

    #[test]
    fn extract_sect_pr_handles_self_closed_form() {
        let src = "<w:body><w:p></w:p><w:sectPr /></w:body>";
        assert_eq!(extract_sect_pr(src).as_deref(), Some("<w:sectPr />"));
    }

    #[test]
    fn extract_sect_pr_handles_open_close_pair() {
        let src = "<w:body><w:p></w:p><w:sectPr><w:pgSz w:w=\"12240\"/></w:sectPr></w:body>";
        assert_eq!(
            extract_sect_pr(src).as_deref(),
            Some("<w:sectPr><w:pgSz w:w=\"12240\"/></w:sectPr>")
        );
    }

    #[test]
    fn xml_escape_handles_amp_lt_gt_quote_apos() {
        let mut out = String::new();
        xml_escape_into(&mut out, "a & b < c > d \" e ' f");
        assert_eq!(out, "a &amp; b &lt; c &gt; d &quot; e &apos; f");
    }

    #[test]
    fn round_trip_with_bold_edit_keeps_bold_after_reopen() {
        let src = fixture("hello.docx");
        let mut payload = open(&src).expect("opens");
        let target_block = payload
            .ast
            .blocks
            .iter()
            .position(|b| matches!(b, ast::Block::Paragraph { runs, .. } if !runs.is_empty()))
            .expect("fixture has at least one paragraph");
        payload.apply_edit(super::super::edits::Edit::SetBlockBold {
            block: target_block,
            on: true,
        });

        let tmp = std::env::temp_dir().join("notepad-studio-rich-doc-roundtrip-bold.docx");
        save(&payload, &tmp).expect("saves");

        let reopened = open(&tmp).expect("reopens edited file");
        let runs = match &reopened.ast.blocks[target_block] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => unreachable!(),
        };
        assert!(
            runs.iter().all(|r| r.style.bold),
            "every run in the bolded block should be bold after round-trip"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn round_trip_with_color_edit_keeps_color_after_reopen() {
        let src = fixture("hello.docx");
        let mut payload = open(&src).expect("opens");
        let target_block = payload
            .ast
            .blocks
            .iter()
            .position(|b| matches!(b, ast::Block::Paragraph { runs, .. } if !runs.is_empty()))
            .expect("fixture has at least one paragraph");
        payload.apply_edit(super::super::edits::Edit::SetBlockColor {
            block: target_block,
            color: Some(0xFF_00_AA),
        });

        let tmp = std::env::temp_dir().join("notepad-studio-rich-doc-roundtrip-color.docx");
        save(&payload, &tmp).expect("saves");

        let reopened = open(&tmp).expect("reopens");
        let runs = match &reopened.ast.blocks[target_block] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => unreachable!(),
        };
        assert!(
            runs.iter().all(|r| r.style.color == Some(0xFF_00_AA)),
            "every run in the colored block should keep the color after round-trip"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn round_trip_with_alignment_and_indent_persist() {
        let src = fixture("hello.docx");
        let mut payload = open(&src).expect("opens");
        let target_block = payload
            .ast
            .blocks
            .iter()
            .position(|b| matches!(b, ast::Block::Paragraph { runs, .. } if !runs.is_empty()))
            .expect("fixture has at least one paragraph");
        payload.apply_edit(super::super::edits::Edit::SetBlockAlignment {
            block: target_block,
            alignment: ast::Alignment::Right,
        });
        payload.apply_edit(super::super::edits::Edit::SetBlockIndent {
            block: target_block,
            delta: 3,
        });

        let tmp = std::env::temp_dir().join("notepad-studio-rich-doc-roundtrip-props.docx");
        save(&payload, &tmp).expect("saves");

        let reopened = open(&tmp).expect("reopens");
        let props = match &reopened.ast.blocks[target_block] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => props.clone(),
            _ => unreachable!(),
        };
        assert_eq!(props.alignment, ast::Alignment::Right);
        assert_eq!(props.indent_level, 3);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn round_trip_with_list_edits_persist_bullet_and_numbered() {
        use super::super::edits::Edit;
        let src = fixture("hello.docx");
        let mut payload = open(&src).expect("opens");
        // Pick the first two paragraph-bearing blocks. If the fixture only
        // has one, duplicate the test by toggling bullet → unbullet so
        // round-trip still covers the parse/serialize path.
        let para_indices: Vec<usize> = payload
            .ast
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b, ast::Block::Paragraph { runs, .. } if !runs.is_empty()))
            .map(|(i, _)| i)
            .collect();
        assert!(!para_indices.is_empty(), "fixture has at least one paragraph");

        let bullet_block = para_indices[0];
        payload.apply_edit(Edit::ToggleBlockList {
            block: bullet_block,
            ordered: false,
        });
        let numbered_block = para_indices.get(1).copied();
        if let Some(idx) = numbered_block {
            payload.apply_edit(Edit::ToggleBlockList {
                block: idx,
                ordered: true,
            });
        }

        let tmp = std::env::temp_dir().join("notepad-studio-rich-doc-roundtrip-lists.docx");
        save(&payload, &tmp).expect("saves");

        let reopened = open(&tmp).expect("reopens edited file");
        let bullet_props = match &reopened.ast.blocks[bullet_block] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => props.clone(),
            _ => unreachable!(),
        };
        assert_eq!(
            bullet_props.list,
            Some(ast::ListMarker { ordered: false, level: 0 }),
            "bulleted block must round-trip as ordered=false, level=0"
        );
        if let Some(idx) = numbered_block {
            let numbered_props = match &reopened.ast.blocks[idx] {
                ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => props.clone(),
                _ => unreachable!(),
            };
            assert_eq!(
                numbered_props.list,
                Some(ast::ListMarker { ordered: true, level: 0 }),
                "numbered block must round-trip as ordered=true, level=0"
            );
        }

        // Saved file must contain the injected numbering part + Content_Types override.
        let saved = std::fs::read(&tmp).expect("reads saved");
        let numbering = read_named_entry_from_bytes(&saved, "word/numbering.xml")
            .expect("numbering.xml present in saved zip");
        assert!(
            numbering.contains(&format!("w:numId=\"{}\"", MINTED_NUM_ID_BULLET)),
            "numbering.xml should contain minted bullet numId"
        );
        if numbered_block.is_some() {
            assert!(
                numbering.contains(&format!("w:numId=\"{}\"", MINTED_NUM_ID_ORDERED)),
                "numbering.xml should contain minted ordered numId"
            );
        }
        let ct = read_named_entry_from_bytes(&saved, "[Content_Types].xml")
            .expect("Content_Types present in saved zip");
        assert!(
            ct.contains("PartName=\"/word/numbering.xml\""),
            "Content_Types must declare the numbering part"
        );
        let rels = read_named_entry_from_bytes(&saved, "word/_rels/document.xml.rels")
            .expect("rels present");
        assert!(
            rels.contains("relationships/numbering"),
            "rels must include a numbering-type Relationship"
        );

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn parse_numbering_xml_distinguishes_bullet_from_decimal() {
        let xml = r#"<?xml version="1.0"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="0">
    <w:lvl w:ilvl="0">
      <w:numFmt w:val="bullet"/>
      <w:lvlText w:val="\u{2022}"/>
    </w:lvl>
  </w:abstractNum>
  <w:abstractNum w:abstractNumId="1">
    <w:lvl w:ilvl="0">
      <w:numFmt w:val="decimal"/>
      <w:lvlText w:val="%1."/>
    </w:lvl>
  </w:abstractNum>
  <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
  <w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
</w:numbering>"#;
        let m = parse_numbering_xml(xml);
        assert_eq!(m.get(&1), Some(&false), "numId 1 → abstract 0 (bullet) → unordered");
        assert_eq!(m.get(&2), Some(&true), "numId 2 → abstract 1 (decimal) → ordered");
    }

    #[test]
    fn round_trip_with_link_edit_keeps_link_after_reopen() {
        let src = fixture("hello.docx");
        let mut payload = open(&src).expect("opens");
        let target_block = payload
            .ast
            .blocks
            .iter()
            .position(|b| matches!(b, ast::Block::Paragraph { runs, .. } if !runs.is_empty()))
            .expect("fixture has at least one paragraph");
        payload.apply_edit(super::super::edits::Edit::SetBlockLink {
            block: target_block,
            url: Some("https://example.com/a?b=1&c=2".into()),
        });

        let tmp = std::env::temp_dir().join("notepad-studio-rich-doc-roundtrip-link.docx");
        save(&payload, &tmp).expect("saves");

        let reopened = open(&tmp).expect("reopens");
        let runs = match &reopened.ast.blocks[target_block] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => unreachable!(),
        };
        assert!(
            runs.iter().all(|r| r.style.link.as_deref() == Some("https://example.com/a?b=1&c=2")),
            "every run in the linked block should resolve to the same URL"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn link_allocator_assigns_stable_rids() {
        let mut a = LinkAllocator::new();
        assert_eq!(a.intern("https://a.example"), "rIdRD1");
        assert_eq!(a.intern("https://b.example"), "rIdRD2");
        // re-interning the same url returns the same rid
        assert_eq!(a.intern("https://a.example"), "rIdRD1");
        assert!(!a.is_empty());
    }

    #[test]
    fn link_allocator_reuses_existing_rid_for_known_url() {
        // Simulate the rels file already containing rId7 → known.example
        // (e.g. inserted by Word). Saving with that URL on a run must
        // reuse rId7, NOT allocate a fresh rIdRD1.
        let mut existing = HashMap::new();
        existing.insert("https://known.example".to_string(), "rId7".to_string());
        let mut a = LinkAllocator::with_existing(existing);
        assert_eq!(a.intern("https://known.example"), "rId7");
        assert!(a.is_empty(), "no NEW entries should be allocated when the URL already exists");
        // Unknown URL still gets a fresh rIdRD id.
        assert_eq!(a.intern("https://new.example"), "rIdRD1");
        assert!(!a.is_empty());
    }

    #[test]
    fn link_allocator_seeds_counter_above_existing_rd_ids() {
        // Existing rels contains rIdRD3 (e.g. from a prior save) — the
        // next new id must skip past 3 to avoid collision.
        let mut existing = HashMap::new();
        existing.insert("https://old.example".to_string(), "rIdRD3".to_string());
        let mut a = LinkAllocator::with_existing(existing);
        assert_eq!(a.intern("https://fresh.example"), "rIdRD4");
    }

    #[test]
    fn parse_existing_hyperlinks_filters_by_type() {
        let rels = r#"<?xml version="1.0"?>
            <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
              <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
              <Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com" TargetMode="External"/>
              <Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/>
            </Relationships>"#;
        let map = parse_existing_hyperlinks(rels);
        assert_eq!(map.get("https://example.com").map(String::as_str), Some("rId7"));
        // Non-hyperlink rels are excluded.
        assert!(map.get("styles.xml").is_none());
        assert!(map.get("media/image1.png").is_none());
    }

    #[test]
    fn parse_color_hex_accepts_six_hex_and_rejects_auto() {
        assert_eq!(parse_color_hex(Some("FFAABB")), Some(0xFF_AA_BB));
        assert_eq!(parse_color_hex(Some("ffaabb")), Some(0xFF_AA_BB));
        assert_eq!(parse_color_hex(Some("auto")), None);
        assert_eq!(parse_color_hex(Some("")), None);
        assert_eq!(parse_color_hex(None), None);
    }

    #[test]
    fn save_with_no_edits_is_byte_perfect_round_trip() {
        let src = fixture("hello.docx");
        let payload = open(&src).expect("opens");
        let tmp = std::env::temp_dir().join("notepad-studio-rich-doc-roundtrip.docx");
        save(&payload, &tmp).expect("saves");
        let original = std::fs::read(&src).expect("reads src");
        let saved = std::fs::read(&tmp).expect("reads saved");
        assert_eq!(
            saved, original,
            "no edits → save must produce byte-identical file"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn heading_level_rejects_other_styles() {
        let pp = ParagraphProperty {
            style_id: Some(docx_rust::formatting::ParagraphStyleId {
                value: "BodyText".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(heading_level(Some(&pp)), None);
    }

    #[test]
    fn parse_paragraphs_from_xml_extracts_text_and_styles() {
        let xml = r#"<?xml version="1.0"?>
<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r></w:p>
<w:p><w:r><w:t>plain</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Section</w:t></w:r></w:p>
</w:hdr>"#;
        let blocks = parse_paragraphs_from_xml(xml);
        assert_eq!(blocks.len(), 3);
        match &blocks[0] {
            ast::Block::Paragraph { runs, .. } => {
                assert_eq!(runs[0].text, "bold");
                assert!(runs[0].style.bold);
            }
            _ => panic!(),
        }
        match &blocks[1] {
            ast::Block::Paragraph { runs, .. } => {
                assert_eq!(runs[0].text, "plain");
                assert!(!runs[0].style.bold);
            }
            _ => panic!(),
        }
        match &blocks[2] {
            ast::Block::Heading { level, runs, .. } => {
                assert_eq!(*level, 2);
                assert_eq!(runs[0].text, "Section");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_rels_builds_id_target_map() {
        let xml = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="t1" Target="styles.xml"/>
<Relationship Id="rId7" Type="t2" Target="media/image1.png"/>
</Relationships>"#;
        let m = parse_rels(xml);
        assert_eq!(m.get("rId1").map(|s| s.as_str()), Some("styles.xml"));
        assert_eq!(m.get("rId7").map(|s| s.as_str()), Some("media/image1.png"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn emu_to_px_converts_one_inch() {
        // 914400 EMUs = 1 inch = 96 px at 96 DPI.
        assert_eq!(emu_to_px(914400), Some(96));
        assert_eq!(emu_to_px(0), None);
        assert_eq!(emu_to_px(914400 * 2), Some(192));
    }

    #[test]
    fn emu_to_px_saturates_huge_extents() {
        // `<wp:extent cx="18446744073709551615">` comes straight from the file.
        assert_eq!(emu_to_px(u64::MAX), Some(u32::MAX));
        assert_eq!(emu_to_px(u64::MAX / 96 + 1), Some(u32::MAX));
    }

    #[test]
    fn link_ids_past_usize_max_do_not_overflow() {
        // An existing rels file can carry any rIdRD number; the next id is
        // one past the largest, even when that is usize::MAX.
        let existing = HashMap::from([(
            "http://a.example/".to_string(),
            format!("rIdRD{}", usize::MAX),
        )]);
        let mut links = LinkAllocator::with_existing(existing);
        assert_eq!(links.intern("http://b.example/"), format!("rIdRD{}", usize::MAX as u128 + 1));
        assert_eq!(links.intern("http://c.example/"), format!("rIdRD{}", usize::MAX as u128 + 2));
        assert_eq!(links.intern("http://a.example/"), format!("rIdRD{}", usize::MAX));
    }

    #[test]
    fn hello_docx_fixture_has_no_headers_or_footers() {
        let payload = open(&fixture("hello.docx")).expect("opens");
        // Confirms the headers/footers fields are populated (default empty)
        // and the fixture doesn't surprise us with stray header parts.
        assert!(payload.headers.is_empty());
        assert!(payload.footers.is_empty());
    }
}
