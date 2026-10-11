//! Legacy Microsoft Word `.doc` (Word 97-2003) reader.
//!
//! Best-effort text extraction. The `.doc` binary format stores text in the
//! `WordDocument` OLE2 stream, referenced by a piece table in a `0Table` or
//! `1Table` stream. We walk the piece table to recover the document body and
//! split paragraphs on the Word paragraph mark (\x0D), then map each
//! paragraph onto our `ast::Block::Paragraph` with a single plain `InlineRun`.
//!
//! What we explicitly DON'T do:
//! - Decode character properties (no bold/italic/underline from `.doc` — would
//!   require resolving CHPX FKPs against the style sheet).
//! - Resolve paragraph properties (no heading detection — would require PAPX
//!   FKPs + STSH lookup).
//! - Extract tables, images, footnotes, comments, headers/footers.
//!
//! The user can still apply Bold/Italic/Underline in Doc View after open;
//! those edits land on the in-memory AST and persist through Save As .docx.
//!
//! Save semantics: regular Save is rejected (`RichDocError::SaveAsRequired`).
//! The only persistence path is `save_as_docx`, which writes a fresh `.docx`
//! zip via the existing DOCX serializer — the original `.doc` is never
//! modified.

use super::{ast, docx, DocFormat, RichDocError, RichDocPayload, Result};
use std::io::{Cursor, Read, Write};
use std::path::Path;

const FIB_MAGIC: u16 = 0xA5EC;
const WORD_DOCUMENT_STREAM: &str = "WordDocument";
const TABLE_STREAM_0: &str = "0Table";
const TABLE_STREAM_1: &str = "1Table";

/// Open a `.doc` and return a `RichDocPayload` with best-effort plain text.
///
/// `catch_unwind` only catches dependency panics in debug and test builds.
/// Release builds abort on panic, so input validation remains necessary.
pub fn open(path: &Path) -> Result<RichDocPayload> {
    let original_bytes = std::fs::read(path)?;
    let parsed = std::panic::catch_unwind(|| parse_doc_bytes(&original_bytes))
        .map_err(|_| RichDocError::Doc("parser panicked on input".into()))??;
    let raw_view_text = parsed.text.clone();
    let blocks: Vec<ast::Block> = split_paragraphs(&parsed.text)
        .into_iter()
        .map(|s| ast::Block::Paragraph {
            runs: vec![ast::InlineRun::plain(&s)],
            props: ast::BlockProps::default(),
        })
        .collect();
    Ok(RichDocPayload {
        format: DocFormat::Doc,
        ast: ast::Document { blocks },
        raw_view_text,
        original_bytes: original_bytes.into(),
        edit_log: Vec::new(),
        active_block: None,
        selection: None,
        drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        headers: Vec::new(),
        footers: Vec::new(),
    })
}

/// Write `payload`'s current AST to `target_path` as a fresh `.docx` file.
/// The original `.doc` is never touched. Reuses `docx::serialize_document_xml`
/// to emit the body; we synthesise the rest of the minimal-valid OOXML
/// package by hand (Content Types, rels, styles).
pub fn save_as_docx(payload: &RichDocPayload, target_path: &Path) -> Result<()> {
    super::ensure_text_only(&payload.ast)?;
    let document_xml = docx_body_for(&payload.ast);
    let bytes = build_minimal_docx(&document_xml)?;
    super::write_document_bytes(target_path, &bytes)?;
    Ok(())
}

/// Public for `save_as_docx` and tests — produces a `word/document.xml`
/// payload from a `Document` with no `original_xml` to merge with.
fn docx_body_for(doc: &ast::Document) -> String {
    // Pass an empty doc shell so `serialize_document_xml` finds no namespace
    // attrs or `<w:sectPr>` in the source and falls back to its safe defaults
    // (the canonical `xmlns:w=…` and a self-closed `<w:sectPr/>`).
    docx::serialize_document_xml(doc, "")
}

/// Internal text-extraction result. Kept private — the public API speaks in
/// `RichDocPayload` only.
struct ExtractedDoc {
    text: String,
}

fn parse_doc_bytes(bytes: &[u8]) -> Result<ExtractedDoc> {
    // Try the cfb crate first (handles every well-formed Word 97-2003 doc).
    // macOS textutil-produced .doc files have a MiniFAT layout cfb 0.14
    // rejects ("mini sector N pointed to twice") even though the regular
    // streams we actually need (WordDocument + 0/1Table, both ≥4KB) are
    // intact. Fall back to a native reader that walks only the regular FAT
    // when cfb refuses to open the file.
    let use_native = match cfb::CompoundFile::open(Cursor::new(bytes)) {
        Ok(_) => false,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Malformed MiniFAT") {
                true
            } else {
                return Err(RichDocError::Doc(format!("cfb open: {e}")));
            }
        }
    };

    let read_stream = |name: &str| -> Result<Vec<u8>> {
        if use_native {
            native_read_stream(bytes, name).ok_or_else(|| {
                RichDocError::Doc(format!("native fallback could not safely read {name} stream"))
            })
        } else {
            let mut comp = cfb::CompoundFile::open(Cursor::new(bytes))
                .map_err(|e| RichDocError::Doc(format!("cfb open: {e}")))?;
            let expected_size = comp.entry(name)
                .map_err(|e| RichDocError::Doc(format!("{name} stream missing: {e}")))?.len();
            if expected_size > bytes.len() as u64 {
                return Err(RichDocError::Doc(format!("{name} stream size exceeds the container")));
            }
            let mut buf = Vec::new();
            buf.try_reserve(expected_size as usize)
                .map_err(|e| RichDocError::Doc(format!("cannot allocate {name} stream: {e}")))?;
            comp.open_stream(name)
                .map_err(|e| RichDocError::Doc(format!("{name} stream missing: {e}")))?
                .take(expected_size.saturating_add(1))
                .read_to_end(&mut buf)
                .map_err(|e| RichDocError::Doc(format!("{name} read: {e}")))?;
            if buf.len() as u64 != expected_size {
                return Err(RichDocError::Doc(format!("{name} stream length does not match its directory entry")));
            }
            Ok(buf)
        }
    };

    let word_bytes = read_stream(WORD_DOCUMENT_STREAM)?;
    let fib = parse_fib(&word_bytes)?;
    let table_name = if fib.f_which_tbl_stm {
        TABLE_STREAM_1
    } else {
        TABLE_STREAM_0
    };
    let table_bytes = read_stream(table_name)?;
    let text = extract_text(&word_bytes, &table_bytes, &fib)?;
    Ok(ExtractedDoc { text })
}

/// Minimal FIB (File Information Block) view. We only pull the fields the
/// text extractor needs; the full FIB is hundreds of bytes of partition
/// counts, page layout settings, and revision data we don't use.
struct Fib {
    /// Total characters in main document body (paragraphs).
    ccp_text: u32,
    /// Byte offset of the Clx (piece table block) in the table stream.
    fc_clx: u32,
    /// Length in bytes of the Clx.
    lcb_clx: u32,
    /// `false` → table stream is `0Table`. `true` → `1Table`. Picked at
    /// save time by Word based on which one is the "previous" stream.
    f_which_tbl_stm: bool,
}

fn parse_fib(word_bytes: &[u8]) -> Result<Fib> {
    if word_bytes.len() < 0x500 {
        return Err(RichDocError::Doc(format!(
            "WordDocument too short for FIB ({} bytes)",
            word_bytes.len()
        )));
    }
    let magic = u16_le(word_bytes, 0)?;
    if magic != FIB_MAGIC {
        return Err(RichDocError::Doc(format!(
            "bad FIB magic 0x{magic:04X}, expected 0x{FIB_MAGIC:04X} — not a Word 97-2003 file?"
        )));
    }
    // FIB.fFlags is a packed 16-bit word at offset 0x0A. Bit 9 (mask 0x0200)
    // is fWhichTblStm — 0 selects 0Table, 1 selects 1Table.
    let f_flags = u16_le(word_bytes, 0x0A)?;
    let f_which_tbl_stm = (f_flags & 0x0200) != 0;
    // ccpText lives in FIB.fibRgLw97 at offset 0x004C.
    let ccp_text = u32_le(word_bytes, 0x004C)?;
    // The fibRgFcLcb97 partition starts at offset 0x009A. fcClx/lcbClx are
    // entries 33 and 34 in the FcLcb array. Each entry is 8 bytes (fc then
    // lcb), so fcClx sits at 0x009A + 33*8 = 0x01A2.
    let fc_clx = u32_le(word_bytes, 0x01A2)?;
    let lcb_clx = u32_le(word_bytes, 0x01A6)?;
    Ok(Fib {
        ccp_text,
        fc_clx,
        lcb_clx,
        f_which_tbl_stm,
    })
}

/// Decode body text using the piece table. Returns text containing paragraph
/// marks (\x0D) as separators; downstream `split_paragraphs` does the split.
fn extract_text(word_bytes: &[u8], table_bytes: &[u8], fib: &Fib) -> Result<String> {
    if fib.ccp_text == 0 {
        return Ok(String::new());
    }
    // Limit expansion before allocating. Excessive repeated references are
    // unsupported; reporting an error keeps the source intact and avoids
    // presenting a truncated body as a successfully opened document.
    let total = fib.ccp_text as usize;
    if total > word_bytes.len() {
        return Err(RichDocError::Doc("body text exceeds the WordDocument stream-size limit".into()));
    }
    let clx_start = fib.fc_clx as usize;
    let clx_end = clx_start.saturating_add(fib.lcb_clx as usize);
    if clx_end > table_bytes.len() {
        return Err(RichDocError::Doc(format!(
            "Clx out of bounds (fc={} lcb={} table_len={})",
            fib.fc_clx,
            fib.lcb_clx,
            table_bytes.len()
        )));
    }
    let clx = table_bytes.get(clx_start..clx_end)
        .ok_or_else(|| RichDocError::Doc("invalid Clx range".into()))?;
    let pcdt = find_pcdt(clx)?;
    let pieces = parse_plcfpcd(pcdt)?;
    let mut out = String::new();
    let mut produced_chars = 0usize;
    for piece in &pieces {
        if produced_chars >= total {
            break;
        }
        let remaining = total - produced_chars;
        let take = piece.char_count.min(remaining);
        decode_piece(word_bytes, piece, take, &mut out)?;
        produced_chars += take;
    }
    if produced_chars != total {
        return Err(RichDocError::Doc("piece table does not cover the entire document body".into()));
    }
    Ok(out)
}

/// Walk a Clx looking for the `Pcdt` entry (the actual piece table). The Clx
/// is a sequence of `Prc` data preceding optional Pcdt; we skip Prcs by
/// reading their length prefix and advance until we hit byte 0x02 (the Pcdt
/// marker) which is then followed by `lcb` (4 bytes) and `lcb` bytes of
/// piece-table data.
fn find_pcdt(clx: &[u8]) -> Result<&[u8]> {
    let mut i = 0;
    while let Some(&kind) = clx.get(i) {
        match kind {
            // clxtGrpPrl: skip length-prefixed Prc data.
            0x01 => {
                i += 1;
                if i + 2 > clx.len() {
                    return Err(RichDocError::Doc("truncated Prc length in Clx".into()));
                }
                let cb = u16_le(clx, i)? as usize;
                i = i.checked_add(2).and_then(|start| start.checked_add(cb))
                    .filter(|end| *end <= clx.len())
                    .ok_or_else(|| RichDocError::Doc("Prc extends past Clx".into()))?;
            }
            // clxtPlcfpcd: piece table marker. Followed by lcb (u32) then data.
            0x02 => {
                if i + 5 > clx.len() {
                    return Err(RichDocError::Doc("truncated Pcdt length in Clx".into()));
                }
                let lcb = u32_le(clx, i + 1)? as usize;
                let start = i + 5;
                let end = start.checked_add(lcb).ok_or_else(|| {
                    RichDocError::Doc("Pcdt lcb overflows".into())
                })?;
                if end > clx.len() {
                    return Err(RichDocError::Doc(format!(
                        "Pcdt extends past Clx (need {end} have {})",
                        clx.len()
                    )));
                }
                return clx.get(start..end)
                    .ok_or_else(|| RichDocError::Doc("invalid Pcdt range".into()));
            }
            other => {
                return Err(RichDocError::Doc(format!(
                    "unknown Clx entry type 0x{other:02X} at offset {i}"
                )));
            }
        }
    }
    Err(RichDocError::Doc("no Pcdt found in Clx".into()))
}

/// A single piece. Word stores text in either UTF-16LE or windows-1252 — the
/// piece descriptor's fc field's bit 30 encodes which (set = compressed
/// cp1252, clear = wide UTF-16LE).
struct Piece {
    /// Number of characters (NOT bytes) this piece contributes.
    char_count: usize,
    /// Byte offset in the WordDocument stream where this piece's encoded
    /// bytes begin.
    byte_offset: usize,
    /// Whether to decode as compressed cp1252 (1 byte/char) vs UTF-16LE (2).
    is_compressed: bool,
}

/// A Plcfpcd is `n+1` CPs (32-bit char positions, 4 bytes each) followed by
/// `n` PCDs (8 bytes each). Solve `cp_count*4 + n*8 = data_len` → derive `n`.
fn parse_plcfpcd(data: &[u8]) -> Result<Vec<Piece>> {
    if data.len() < 4 + 8 {
        return Err(RichDocError::Doc(format!(
            "Plcfpcd too short ({} bytes)",
            data.len()
        )));
    }
    let n = (data.len() - 4) / 12;
    if 4 * (n + 1) + 8 * n != data.len() {
        return Err(RichDocError::Doc(format!(
            "Plcfpcd size {} not a valid (n+1)*4 + n*8 layout",
            data.len()
        )));
    }
    // MS-DOC 2.8.35: CPs increase strictly; the body starts at CP zero.
    if u32_le(data, 0)? != 0 {
        return Err(RichDocError::Doc("piece table does not start at character zero".into()));
    }
    let mut pieces = Vec::new();
    pieces.try_reserve(n)
        .map_err(|e| RichDocError::Doc(format!("cannot allocate piece table: {e}")))?;
    let pcd_base = (n + 1) * 4;
    for i in 0..n {
        let pcd_off = pcd_base + i * 8;
        // PCD layout: u16 flags, u32 fc, u16 prm.
        let fc_raw = u32_le(data, pcd_off + 2)?;
        // Bit 30 (mask 0x40000000) is the "fCompressed" flag.
        let is_compressed = (fc_raw & 0x4000_0000) != 0;
        // MS-DOC 2.9.73: bit 31 is reserved and ignored, not an offset bit.
        let fc = (fc_raw & 0x3FFF_FFFF) as usize;
        // When compressed, real byte offset is fc / 2.
        let byte_offset = if is_compressed { fc / 2 } else { fc };
        let cp = u32_le(data, i * 4)?;
        let next_cp = u32_le(data, (i + 1) * 4)?;
        if next_cp >= 0x7FFF_FFFF {
            return Err(RichDocError::Doc("piece character position exceeds the supported range".into()));
        }
        let char_count = next_cp.checked_sub(cp).filter(|count| *count > 0)
            .ok_or_else(|| RichDocError::Doc("piece character positions do not increase strictly".into()))? as usize;
        pieces.push(Piece {
            char_count,
            byte_offset,
            is_compressed,
        });
    }
    Ok(pieces)
}

fn decode_piece(word_bytes: &[u8], piece: &Piece, take: usize, out: &mut String) -> Result<()> {
    if take == 0 {
        return Ok(());
    }
    let bytes_per_char = if piece.is_compressed { 1 } else { 2 };
    let byte_len = take.checked_mul(bytes_per_char)
        .ok_or_else(|| RichDocError::Doc("piece byte length overflows".into()))?;
    let end = piece.byte_offset.checked_add(byte_len)
        .ok_or_else(|| RichDocError::Doc("piece byte range overflows".into()))?;
    let slice = word_bytes.get(piece.byte_offset..end)
        .ok_or_else(|| RichDocError::Doc("piece text extends past the WordDocument stream".into()))?;
    let capacity = take.checked_mul(3)
        .ok_or_else(|| RichDocError::Doc("decoded piece length overflows".into()))?;
    out.try_reserve(capacity)
        .map_err(|e| RichDocError::Doc(format!("cannot allocate decoded text: {e}")))?;
    if piece.is_compressed {
        // Word uses cp1252 for compressed pieces. encoding_rs handles all the
        // edge cases (Euro at 0x80, etc.).
        for chunk in slice.chunks(4096) {
            let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(chunk);
            push_clean(out, &decoded);
        }
    } else {
        let (pairs, _) = slice.as_chunks::<2>();
        let units = pairs.iter().copied().map(u16::from_le_bytes);
        for ch in char::decode_utf16(units) {
            let mut utf8 = [0; 4];
            push_clean(out, ch.unwrap_or(char::REPLACEMENT_CHARACTER).encode_utf8(&mut utf8));
        }
    }
    Ok(())
}

/// Strip Word's body-internal control characters that aren't part of normal
/// text (cell marks, field codes, picture placeholders, etc.) but keep
/// paragraph marks (\x0D), tabs, and line breaks (\x0B).
fn push_clean(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '\r' | '\t' | '\u{000B}' => out.push(ch),
            // Field characters (0x13/0x14/0x15), cell marks (0x07), section
            // marks (0x0C), picture placeholder (0x01), footnote/annotation
            // marks (0x02/0x05). All dropped from the user-visible text.
            '\u{0001}' | '\u{0002}' | '\u{0005}' | '\u{0007}' | '\u{000C}'
            | '\u{0013}' | '\u{0014}' | '\u{0015}' => {}
            c if (c as u32) < 0x20 && c != '\n' => {}
            c => out.push(c),
        }
    }
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split('\r') {
        let trimmed_nul = raw.trim_end_matches('\0');
        // Skip empty trailing paragraph that the last \r produces.
        if trimmed_nul.is_empty() {
            continue;
        }
        out.push(trimmed_nul.to_string());
    }
    if out.is_empty() {
        // Even a zero-paragraph doc gets one empty paragraph so the renderer
        // has something to draw — otherwise the Doc View is blank and looks
        // broken.
        out.push(String::new());
    }
    out
}

// ---------- minimal-docx package builder ----------

/// Build a minimal valid `.docx` zip in-memory. Five entries:
/// - `[Content_Types].xml`
/// - `_rels/.rels`
/// - `word/_rels/document.xml.rels`
/// - `word/document.xml` (caller supplies)
/// - `word/styles.xml` (built-in Heading1..6 styles so headings render)
pub(super) fn build_minimal_docx(document_xml: &str) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let cursor = Cursor::new(&mut buf);
        let mut zw = zip::ZipWriter::new(cursor);
        let opts: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        zw.start_file("[Content_Types].xml", opts)
            .map_err(|e| RichDocError::Zip(format!("start [Content_Types]: {e}")))?;
        zw.write_all(CONTENT_TYPES_XML.as_bytes())
            .map_err(|e| RichDocError::Io(e))?;

        zw.start_file("_rels/.rels", opts)
            .map_err(|e| RichDocError::Zip(format!("start _rels/.rels: {e}")))?;
        zw.write_all(ROOT_RELS_XML.as_bytes())
            .map_err(|e| RichDocError::Io(e))?;

        zw.start_file("word/_rels/document.xml.rels", opts)
            .map_err(|e| RichDocError::Zip(format!("start document rels: {e}")))?;
        zw.write_all(DOCUMENT_RELS_XML.as_bytes())
            .map_err(|e| RichDocError::Io(e))?;

        zw.start_file("word/document.xml", opts)
            .map_err(|e| RichDocError::Zip(format!("start document.xml: {e}")))?;
        zw.write_all(document_xml.as_bytes())
            .map_err(|e| RichDocError::Io(e))?;

        zw.start_file("word/styles.xml", opts)
            .map_err(|e| RichDocError::Zip(format!("start styles.xml: {e}")))?;
        zw.write_all(STYLES_XML.as_bytes())
            .map_err(|e| RichDocError::Io(e))?;

        zw.finish()
            .map_err(|e| RichDocError::Zip(format!("zip finish: {e}")))?;
    }
    Ok(buf)
}

const CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>"#;

const ROOT_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

const DOCUMENT_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

// Minimal styles.xml. Defines Heading1..Heading6 and Normal so that headings
// emitted by docx::serialize_block (`<w:pStyle w:val="HeadingN"/>`) actually
// render with hierarchy in Word/LibreOffice. Without this the styles render
// as default body text and our "heading" is indistinguishable from a
// paragraph.
const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="56"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="48"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="40"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading4"><w:name w:val="heading 4"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading5"><w:name w:val="heading 5"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="28"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading6"><w:name w:val="heading 6"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="24"/></w:rPr></w:style>
</w:styles>"#;

// ---------- low-level helpers ----------

fn u16_le(b: &[u8], offset: usize) -> Result<u16> {
    let pair = offset.checked_add(2).and_then(|end| b.get(offset..end))
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| RichDocError::Doc(format!("truncated 16-bit integer at byte {offset}")))?;
    Ok(u16::from_le_bytes(pair))
}

fn u32_le(b: &[u8], offset: usize) -> Result<u32> {
    let quad = offset.checked_add(4).and_then(|end| b.get(offset..end))
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| RichDocError::Doc(format!("truncated 32-bit integer at byte {offset}")))?;
    Ok(u32::from_le_bytes(quad))
}

// ---------- native CFB fallback ----------
//
// Minimal OLE2 / Compound File Binary reader covering only what the .doc
// text extractor needs: locate a named stream and return its bytes via the
// regular FAT chain. Mini streams (size < 4096) and DIFAT sectors past the
// header's first 109 slots are intentionally NOT implemented — the
// WordDocument and 0/1Table streams in real .doc files are always regular
// streams and the doc file is small enough to fit DIFAT in the header. This
// is a fallback for the cfb crate's strict MiniFAT validation, not a
// general-purpose CFB library.

const OLE2_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const END_OF_CHAIN: u32 = 0xFFFFFFFE;
const FREE_SECTOR: u32 = 0xFFFFFFFF;
const ENTRY_TYPE_STREAM: u8 = 2;

/// Locate a named stream in the OLE2 container and return its full bytes
/// via the regular FAT chain. Returns `None` if the header is malformed,
/// the stream is missing, the stream uses the mini FAT, or any chain walk
/// would read past the end of `bytes`.
fn native_read_stream(bytes: &[u8], target_name: &str) -> Option<Vec<u8>> {
    if bytes.len() < 512 || bytes.get(..8)? != OLE2_MAGIC {
        return None;
    }
    // MS-CFB 2.2: only version 3/512-byte and version 4/4096-byte sectors.
    let sector_shift = u16_le(bytes, 30).ok()?;
    match (u16_le(bytes, 26).ok()?, sector_shift) {
        (3, 9) | (4, 12) => {}
        _ => return None,
    }
    let sector_size: usize = 1usize << sector_shift;
    let num_fat_sectors = u32_le(bytes, 44).ok()? as usize;
    let first_dir_sect = u32_le(bytes, 48).ok()?;
    if num_fat_sectors > 109 {
        return None; // Additional DIFAT sectors are outside the fallback's scope.
    }

    let header_difat_count = num_fat_sectors.min(109);
    let mut fat_sectors: Vec<u32> = Vec::with_capacity(header_difat_count);
    for i in 0..header_difat_count {
        let s = u32_le(bytes, 76 + i * 4).ok()?;
        if s == FREE_SECTOR || fat_sectors.contains(&s) {
            return None;
        }
        fat_sectors.push(s);
    }

    let mut fat: Vec<u32> = Vec::with_capacity(fat_sectors.len() * (sector_size / 4));
    for &fs in &fat_sectors {
        let start = sector_to_offset(fs, sector_size)?;
        let end = start.checked_add(sector_size)?;
        if end > bytes.len() {
            return None;
        }
        for &chunk in bytes.get(start..end)?.as_chunks::<4>().0 {
            fat.push(u32::from_le_bytes(chunk));
        }
    }

    let dir_data = follow_chain(bytes, &fat, first_dir_sect, sector_size, usize::MAX)?;

    let target_utf16: Vec<u16> = target_name.encode_utf16().collect();
    for entry in dir_data.as_chunks::<128>().0 {
        let name_len = u16_le(entry, 64).ok()? as usize;
        if !(2..=64).contains(&name_len) || !name_len.is_multiple_of(2) {
            continue;
        }
        let name_bytes = entry.get(..name_len - 2)?;
        let name_utf16: Vec<u16> = name_bytes
            .as_chunks::<2>().0.iter().copied()
            .map(u16::from_le_bytes)
            .collect();
        if name_utf16 != target_utf16 {
            continue;
        }
        let entry_type = *entry.get(66)?;
        if entry_type != ENTRY_TYPE_STREAM {
            continue;
        }
        let start_sect = u32_le(entry, 116).ok()?;
        let size_bytes = entry.get(120..128)?.try_into().ok()?;
        let size = usize::try_from(u64::from_le_bytes(size_bytes)).ok()?;
        // Mini streams aren't supported in fallback. The streams we need
        // (WordDocument, 0/1Table) are large enough to live on regular FAT.
        if size < 4096 || size > bytes.len() {
            return None;
        }
        return follow_chain(bytes, &fat, start_sect, sector_size, size);
    }
    None
}

/// Translate a FAT sector index into a byte offset within the file. The
/// header occupies one complete sector, so sector N lives at
/// `(N + 1) * sector_size`.
fn sector_to_offset(sector: u32, sector_size: usize) -> Option<usize> {
    (sector as usize).checked_add(1)?.checked_mul(sector_size)
}

/// Walk a FAT chain starting at `start_sect`, concatenating each sector's
/// payload into the returned `Vec`. Rejects cycles and incorrect stream
/// lengths. `usize::MAX` reads a directory chain of unknown length.
fn follow_chain(
    bytes: &[u8],
    fat: &[u32],
    start_sect: u32,
    sector_size: usize,
    cap: usize,
) -> Option<Vec<u8>> {
    if !matches!(sector_size, 512 | 4096) {
        return None;
    }
    let file_sectors = (bytes.len() / sector_size).checked_sub(1)?;
    let mut seen = Vec::new();
    let tracked_sectors = file_sectors.min(fat.len());
    seen.try_reserve_exact(tracked_sectors).ok()?;
    seen.resize(tracked_sectors, false);
    let mut out: Vec<u8> = Vec::new();
    let mut sec = start_sect;
    while sec != END_OF_CHAIN {
        let visited = seen.get_mut(sec as usize)?;
        if *visited || out.len() >= cap {
            return None;
        }
        *visited = true;
        let off = sector_to_offset(sec, sector_size)?;
        let end = off.checked_add(sector_size)?;
        if end > bytes.len() {
            return None;
        }
        let sector = bytes.get(off..end)?;
        let take = sector.len().min(cap.saturating_sub(out.len()));
        out.try_reserve(take).ok()?;
        out.extend_from_slice(sector.get(..take)?);
        let next = *fat.get(sec as usize)?;
        sec = next;
    }
    if cap == usize::MAX || out.len() == cap { Some(out) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece_table(cps: &[u32], offsets: &[u32]) -> Vec<u8> {
        assert_eq!(cps.len(), offsets.len() + 1);
        let mut plc = Vec::new();
        for cp in cps {
            plc.extend_from_slice(&cp.to_le_bytes());
        }
        for fc in offsets {
            plc.extend_from_slice(&0u16.to_le_bytes());
            plc.extend_from_slice(&fc.to_le_bytes());
            plc.extend_from_slice(&0u16.to_le_bytes());
        }
        let mut clx = vec![0x02];
        clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
        clx.extend_from_slice(&plc);
        clx
    }

    fn extract_fixture(word: &[u8], cps: &[u32], offsets: &[u32], body_chars: u32) -> Result<String> {
        let table = piece_table(cps, offsets);
        extract_text(word, &table, &Fib {
            ccp_text: body_chars, fc_clx: 0, lcb_clx: table.len() as u32,
            f_which_tbl_stm: false,
        })
    }

    #[test]
    fn extract_rejects_a_body_count_that_outgrows_the_stream() {
        assert!(matches!(extract_fixture(b"abc", &[0, 3], &[0x4000_0000], 1024 * 1024),
            Err(RichDocError::Doc(_))));
    }

    #[test]
    fn extract_rejects_excessive_overlapping_piece_expansion() {
        assert!(matches!(extract_fixture(b"abcd", &[0, 4, 8], &[0x4000_0000; 2], 8),
            Err(RichDocError::Doc(_))));
    }

    #[test]
    fn extract_rejects_incomplete_or_invalid_piece_ranges() {
        for (cps, offsets, count) in [
            (vec![0, 2], vec![0x4000_0000], 3),
            (vec![0, 3], vec![0x4000_0008], 3),
            (vec![1, 4], vec![0x4000_0000], 3),
            (vec![0, 2, 1], vec![0x4000_0000; 2], 3),
            (vec![0, 2, 2], vec![0x4000_0000; 2], 3),
            (vec![0, 0x8000_0000], vec![0x4000_0000], 3),
        ] {
            assert!(matches!(extract_fixture(b"abcdef", &cps, &offsets, count),
                Err(RichDocError::Doc(_))), "CPs {cps:?}, offsets {offsets:?}");
        }
    }

    #[test]
    fn piece_offsets_ignore_the_reserved_high_bit() {
        assert_eq!(extract_fixture(b"abc", &[0, 3], &[0xC000_0000], 3)
            .expect("reserved bit ignored"), "abc");
    }

    #[test]
    fn malformed_integer_offsets_cannot_panic() {
        for offset in [0, 1, 3, usize::MAX] {
            assert!(matches!(u16_le(b"a", offset), Err(RichDocError::Doc(_))));
            assert!(matches!(u32_le(b"abc", offset), Err(RichDocError::Doc(_))));
        }
        assert_eq!(u16_le(&[0x34, 0x12], 0).expect("complete integer"), 0x1234);
        assert_eq!(u32_le(&[0x78, 0x56, 0x34, 0x12], 0).expect("complete integer"), 0x1234_5678);
    }

    #[test]
    fn wide_piece_length_overflow_cannot_panic() {
        let piece = Piece { char_count: usize::MAX, byte_offset: 0, is_compressed: false };
        let mut out = String::new();
        assert!(matches!(decode_piece(b"ab", &piece, usize::MAX, &mut out), Err(RichDocError::Doc(_))));
        assert!(out.is_empty());
    }

    #[test]
    fn looping_fat_chains_are_rejected_instead_of_repeating_text() {
        let bytes = vec![0u8; 512 * 3];
        let mut fat = vec![FREE_SECTOR; 128];
        fat[0] = 1;
        fat[1] = 1;
        assert!(follow_chain(&bytes, &fat, 0, 512, usize::MAX).is_none());
        assert!(follow_chain(&bytes, &fat, 0, 512, 3 * 512).is_none());
    }

    #[test]
    fn truncated_or_invalid_fat_chains_are_rejected() {
        let bytes = vec![0u8; 512 * 3];
        assert!(follow_chain(&bytes, &[END_OF_CHAIN], 0, 512, 1024).is_none());
        assert!(follow_chain(&bytes, &[FREE_SECTOR], 0, 512, usize::MAX).is_none());
    }

    #[test]
    fn valid_fat_chains_keep_exact_stream_lengths() {
        for sector_size in [512, 4096] {
            let mut bytes = vec![0u8; sector_size * 3];
            bytes[sector_size..sector_size * 2].fill(b'a');
            bytes[sector_size * 2..].fill(b'b');
            let size = sector_size + 3;
            let expected = [&vec![b'a'; sector_size][..], b"bbb"].concat();
            assert_eq!(follow_chain(&bytes, &[1, END_OF_CHAIN], 0, sector_size, size), Some(expected));
            assert_eq!(follow_chain(&bytes, &[1, END_OF_CHAIN], 0, sector_size, usize::MAX)
                .expect("directory chain").len(), sector_size * 2);
        }
    }

    #[test]
    fn supported_mixed_text_pieces_keep_their_contents() {
        let mut word = vec![0; 32];
        word[..3].copy_from_slice(b"A\x80\t");
        let wide: Vec<u8> = "\u{00e9}\u{1f600}\r".encode_utf16().flat_map(u16::to_le_bytes).collect();
        word[8..8 + wide.len()].copy_from_slice(&wide);
        // The final piece belongs to a non-body story and must not be extracted.
        assert_eq!(extract_fixture(&word, &[0, 3, 7, 8], &[0x4000_0000, 8, 0x4000_0020], 7)
            .expect("mixed pieces"), "A\u{20ac}\t\u{00e9}\u{1f600}\r");
        assert_eq!(extract_fixture(&word, &[0, 3, 6], &[0x4000_0000; 2], 6)
            .expect("supported overlap"), "A\u{20ac}\tA\u{20ac}\t");
    }

    fn make_doc_bytes(word_size: usize) -> Vec<u8> {
        make_doc_bytes_version(word_size, cfb::Version::V4)
    }

    fn make_doc_bytes_version(word_size: usize, version: cfb::Version) -> Vec<u8> {
        let body = b"Synthetic document\r";
        let mut word = vec![0; word_size.max(4096)];
        let table = piece_table(&[0, body.len() as u32], &[0x4000_0A00]);
        word[..2].copy_from_slice(&FIB_MAGIC.to_le_bytes());
        word[0x004C..0x0050].copy_from_slice(&(body.len() as u32).to_le_bytes());
        word[0x01A6..0x01AA].copy_from_slice(&(table.len() as u32).to_le_bytes());
        word[0x500..0x500 + body.len()].copy_from_slice(body);
        let mut compound = cfb::CompoundFile::create_with_version(version, Cursor::new(Vec::new())).expect("container");
        compound.create_stream(WORD_DOCUMENT_STREAM).expect("word stream").write_all(&word).expect("word data");
        compound.create_stream(TABLE_STREAM_0).expect("table stream").write_all(&table).expect("table data");
        compound.into_inner().into_inner()
    }

    #[test]
    fn native_fallback_reads_regular_streams_in_both_cfb_versions() {
        for version in [cfb::Version::V3, cfb::Version::V4] {
            let bytes = make_doc_bytes_version(4096, version);
            let word = native_read_stream(&bytes, WORD_DOCUMENT_STREAM).expect("native stream");
            assert_eq!(word.len(), 4096);
            assert_eq!(&word[0x500..0x500 + 19], b"Synthetic document\r");
        }
    }

    #[test]
    fn open_native_doc_keeps_text_and_original_container() {
        let dir = crate::persist::TempDir::new("legacy-doc-open");
        let path = dir.join("sample.doc");
        let bytes = make_doc_bytes(4096);
        std::fs::write(&path, &bytes).expect("fixture");
        let payload = open(&path).expect("open");
        assert_eq!(payload.format, DocFormat::Doc);
        assert_eq!(payload.raw_view_text, "Synthetic document\r");
        assert_eq!(payload.original_bytes.as_slice(), bytes.as_slice());
        assert_eq!(std::fs::read(&path).expect("original"), bytes);
    }

    #[test]
    #[ignore = "release timing on a legacy document above the streaming threshold"]
    fn large_legacy_document_open_timing() {
        let dir = crate::persist::TempDir::new("legacy-doc-timing");
        let path = dir.join("sample.doc");
        std::fs::write(&path, make_doc_bytes(11 * 1024 * 1024)).expect("fixture");
        assert!(std::fs::metadata(&path).expect("metadata").len() > crate::io_stream::STREAM_THRESHOLD_BYTES);
        let started = std::time::Instant::now();
        for _ in 0..5 {
            assert_eq!(open(&path).expect("open").raw_view_text, "Synthetic document\r");
        }
        println!("five opens of 11 MiB DOC: {:?}", started.elapsed());
    }

    #[test]
    fn split_paragraphs_handles_trailing_cr() {
        let r = split_paragraphs("alpha\rbravo\rcharlie\r");
        assert_eq!(r, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn split_paragraphs_empty_input_yields_one_empty() {
        let r = split_paragraphs("");
        assert_eq!(r, vec![String::new()]);
    }

    #[test]
    fn push_clean_drops_field_chars_keeps_tab_and_cr() {
        let mut out = String::new();
        push_clean(&mut out, "a\x13b\x14c\x15d\te\rf");
        assert_eq!(out, "abcd\te\rf");
    }

    #[test]
    fn open_rejects_non_ole_file() {
        let tmp = std::env::temp_dir().join("notepad-studio-doc-not-ole.doc");
        std::fs::write(&tmp, b"not a real .doc file").unwrap();
        let err = open(&tmp).expect_err("garbage bytes must not parse");
        assert!(matches!(err, RichDocError::Doc(_)));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn open_rejects_empty_file() {
        let tmp = std::env::temp_dir().join("notepad-studio-doc-empty.doc");
        std::fs::write(&tmp, b"").unwrap();
        let err = open(&tmp).expect_err("empty file must not parse");
        assert!(matches!(err, RichDocError::Doc(_)));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn build_minimal_docx_produces_a_valid_zip_with_document_xml() {
        let body = "<?xml version=\"1.0\"?><w:document xmlns:w=\"x\"><w:body></w:body></w:document>";
        let bytes = build_minimal_docx(body).expect("builds zip");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("zip parses");
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.contains(&"[Content_Types].xml".to_string()));
        assert!(names.contains(&"_rels/.rels".to_string()));
        assert!(names.contains(&"word/_rels/document.xml.rels".to_string()));
        assert!(names.contains(&"word/document.xml".to_string()));
        assert!(names.contains(&"word/styles.xml".to_string()));
        let mut document_entry = archive.by_name("word/document.xml").expect("document.xml");
        let mut got = String::new();
        document_entry.read_to_string(&mut got).expect("reads");
        assert_eq!(got, body);
    }

    #[test]
    fn save_as_docx_then_reopen_round_trips_paragraphs() {
        // Build a payload by hand — bypass the OLE2 parser since we don't
        // want test fixtures bundled. The Save-As path doesn't care how the
        // payload got built, only that the AST is correct.
        let payload = RichDocPayload {
            format: DocFormat::Doc,
            ast: ast::Document {
                blocks: vec![
                    ast::Block::Heading {
                        level: 1,
                        runs: vec![ast::InlineRun::plain("Title")],
                        props: ast::BlockProps::default(),
                    },
                    ast::Block::Paragraph {
                        runs: vec![ast::InlineRun {
                            text: "body text".into(),
                            style: ast::InlineStyle {
                                bold: true,
                                ..Default::default()
                            },
                        }],
                        props: ast::BlockProps::default(),
                    },
                ],
            },
            raw_view_text: String::new(),
            original_bytes: std::sync::Arc::new(Vec::new()),
            edit_log: vec![],
            active_block: None,
            selection: None,
            drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
            headers: Vec::new(),
            footers: Vec::new(),
        };

        let tmp = std::env::temp_dir().join("notepad-studio-doc-save-as.docx");
        save_as_docx(&payload, &tmp).expect("save_as_docx succeeds");

        let reopened = docx::open(&tmp).expect("reopens as docx");
        assert_eq!(reopened.ast.blocks.len(), 2, "two blocks survive round-trip");
        assert!(matches!(
            reopened.ast.blocks[0],
            ast::Block::Heading { level: 1, .. }
        ));
        match &reopened.ast.blocks[1] {
            ast::Block::Paragraph { runs, .. } => {
                assert!(runs.iter().any(|r| r.style.bold), "bold survives");
                let text: String = runs.iter().map(|r| r.text.as_str()).collect();
                assert!(text.contains("body text"));
            }
            _ => panic!("second block must be paragraph"),
        }
        std::fs::remove_file(&tmp).ok();
    }
}
