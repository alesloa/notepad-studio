// Doc View edits live in the AST, not in the raw-view rope. Keep both together
// with the native container so a restart never needs the original disk file.
use crate::rich_doc::{ast, edits::Edit, DocFormat, RichDocPayload};
use serde::{Deserialize, Serialize};
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 32 * 1024 * 1024;
const MAX_BLOCKS: usize = 100_000;
const MAX_DEPTH: usize = 64;
const MAX_IMAGES: usize = 10_000;

pub(super) fn path_in(dir: &Path, id: Uuid) -> PathBuf { dir.join(format!("{id}.rich")) }

pub(crate) struct Capture {
    format: DocFormat,
    ast: ast::Document,
    headers: Vec<Vec<ast::Block>>,
    footers: Vec<Vec<ast::Block>>,
    edits: Vec<Edit>,
    original: Arc<Vec<u8>>,
    raw: String,
}

impl Capture {
    pub(super) fn new(payload: &RichDocPayload) -> Self {
        Self {
            format: payload.format, ast: payload.ast.clone(), headers: payload.headers.clone(),
            footers: payload.footers.clone(), edits: payload.edit_log.clone(),
            original: payload.original_bytes.clone(), raw: payload.raw_view_text.clone(),
        }
    }

    pub(super) fn write(&self, path: &Path, source: &str) -> io::Result<()> {
        if self.original.len() as u64 > MAX_BYTES || source.len() as u64 > MAX_BYTES {
            return Err(invalid("rich document recovery exceeds 256 MiB; save the document"));
        }
        let mut images = Vec::new();
        let state = State {
            version: VERSION, format: self.format,
            ast: FlatDocument::encode(&self.ast.blocks, &mut images)?,
            headers: self.headers.iter().map(|blocks| FlatDocument::encode(blocks, &mut images)).collect::<io::Result<_>>()?,
            footers: self.footers.iter().map(|blocks| FlatDocument::encode(blocks, &mut images)).collect::<io::Result<_>>()?,
            edits: self.edits.iter().map(|edit| StoredEdit::encode(edit, &mut images)).collect::<io::Result<_>>()?,
            raw: self.raw.clone(),
        };
        let mut json = LimitedBytes(Vec::new());
        serde_json::to_writer(&mut json, &state).map_err(io::Error::other)?;
        let total = images.iter().try_fold(self.original.len() as u64, |total, image: &Arc<Vec<u8>>| {
            total.checked_add(image.len() as u64).ok_or_else(|| invalid("image size overflow"))
        })?.checked_add(json.0.len() as u64).and_then(|n| n.checked_add(source.len() as u64))
            .ok_or_else(|| invalid("backup size overflow"))?;
        if total > MAX_BYTES { return Err(invalid("rich document recovery exceeds 256 MiB; save the document")); }
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in [("state.json", json.0.as_slice()), ("original.bin", self.original.as_slice()), ("source.txt", source.as_bytes())] {
            zip.start_file(name, options).map_err(io::Error::other)?;
            zip.write_all(bytes)?;
        }
        for (index, image) in images.iter().enumerate() {
            zip.start_file(format!("image-{index}"), options).map_err(io::Error::other)?;
            zip.write_all(image)?;
        }
        let bytes = zip.finish().map_err(io::Error::other)?.into_inner();
        if bytes.len() as u64 > MAX_BYTES { return Err(invalid("rich document recovery exceeds 256 MiB; save the document")); }
        crate::persist::atomic_write(path, &bytes)
    }
}

#[derive(Serialize, Deserialize)]
struct State {
    version: u32,
    format: DocFormat,
    ast: FlatDocument,
    headers: Vec<FlatDocument>,
    footers: Vec<FlatDocument>,
    edits: Vec<StoredEdit>,
    raw: String,
}

#[derive(Serialize, Deserialize)]
struct FlatDocument {
    roots: Vec<usize>,
    nodes: Vec<FlatBlock>,
}

#[derive(Serialize, Deserialize)]
enum FlatBlock {
    Paragraph { runs: Vec<ast::InlineRun>, props: ast::BlockProps },
    Heading { level: u8, runs: Vec<ast::InlineRun>, props: ast::BlockProps },
    Table { rows: Vec<Vec<Vec<usize>>> },
    Image { image: usize, mime: String, width_px: Option<u32>, height_px: Option<u32>, alt: Option<String> },
}

impl FlatDocument {
    fn encode(blocks: &[ast::Block], images: &mut Vec<Arc<Vec<u8>>>) -> io::Result<Self> {
        if blocks.len() > MAX_BLOCKS { return Err(invalid("rich recovery has too many blocks")); }
        let mut pending: Vec<_> = blocks.iter().map(|block| (block, 0usize)).collect();
        let roots = (0..pending.len()).collect();
        let mut nodes = Vec::new();
        let mut index = 0usize;
        while let Some(&(block, depth)) = pending.get(index) {
            if pending.len() > MAX_BLOCKS || depth > MAX_DEPTH { return Err(invalid("rich recovery has too many or too deeply nested blocks")); }
            let node = match block {
                ast::Block::Paragraph { runs, props } => FlatBlock::Paragraph { runs: runs.clone(), props: props.clone() },
                ast::Block::Heading { level, runs, props } => FlatBlock::Heading { level: *level, runs: runs.clone(), props: props.clone() },
                ast::Block::Table { rows } => {
                    let mut stored_rows = Vec::new();
                    for row in rows {
                        let mut cells = Vec::new();
                        for cell in &row.cells {
                            if cell.blocks.len() > MAX_BLOCKS.saturating_sub(pending.len()) { return Err(invalid("rich recovery has too many blocks")); }
                            let start = pending.len();
                            pending.extend(cell.blocks.iter().map(|block| (block, depth.saturating_add(1))));
                            cells.push((start..pending.len()).collect());
                        }
                        stored_rows.push(cells);
                    }
                    FlatBlock::Table { rows: stored_rows }
                }
                ast::Block::Image { source: ast::ImageSource::Inline { bytes, mime }, width_px, height_px, alt } => {
                    let image = if let Some(index) = images.iter().position(|old| Arc::ptr_eq(old, bytes)) { index } else {
                        if images.len() >= MAX_IMAGES { return Err(invalid("rich recovery has too many images")); }
                        images.push(bytes.clone()); images.len().saturating_sub(1)
                    };
                    FlatBlock::Image { image, mime: mime.clone(), width_px: *width_px, height_px: *height_px, alt: alt.clone() }
                }
            };
            nodes.push(node);
            index = index.saturating_add(1);
        }
        Ok(Self { roots, nodes })
    }

    fn decode(self, images: &[Arc<Vec<u8>>]) -> io::Result<Vec<ast::Block>> {
        if self.nodes.len() > MAX_BLOCKS { return Err(invalid("too many recovery blocks")); }
        let mut decoded: Vec<Option<(ast::Block, usize)>> = (0..self.nodes.len()).map(|_| None).collect();
        for (index, node) in self.nodes.into_iter().enumerate().rev() {
            let mut depth = 0usize;
            let block = match node {
                FlatBlock::Paragraph { runs, props } => ast::Block::Paragraph { runs, props },
                FlatBlock::Heading { level, runs, props } => ast::Block::Heading { level, runs, props },
                FlatBlock::Image { image, mime, width_px, height_px, alt } => ast::Block::Image {
                    source: ast::ImageSource::Inline { bytes: images.get(image).ok_or_else(|| invalid("missing recovery image"))?.clone(), mime },
                    width_px, height_px, alt,
                },
                FlatBlock::Table { rows } => {
                    let mut restored_rows = Vec::new();
                    for row in rows {
                        let mut cells = Vec::new();
                        for cell in row {
                            let mut blocks = Vec::new();
                            for child in cell {
                                if child <= index { return Err(invalid("cyclic recovery blocks")); }
                                let (block, child_depth) = decoded.get_mut(child).and_then(Option::take)
                                    .ok_or_else(|| invalid("missing or shared recovery block"))?;
                                depth = depth.max(child_depth.saturating_add(1));
                                if depth > MAX_DEPTH { return Err(invalid("recovery blocks nest too deeply")); }
                                blocks.push(block);
                            }
                            cells.push(ast::TableCell { blocks });
                        }
                        restored_rows.push(ast::TableRow { cells });
                    }
                    ast::Block::Table { rows: restored_rows }
                }
            };
            let Some(slot) = decoded.get_mut(index) else { return Err(invalid("invalid recovery index")); };
            *slot = Some((block, depth));
        }
        let mut blocks = Vec::new();
        for root in self.roots {
            let (block, _) = decoded.get_mut(root).and_then(Option::take).ok_or_else(|| invalid("missing or shared recovery root"))?;
            blocks.push(block);
        }
        if decoded.iter().any(Option::is_some) { return Err(invalid("orphan recovery blocks")); }
        Ok(blocks)
    }
}

#[derive(Serialize, Deserialize)]
enum StoredEdit {
    Plain(Edit),
    InsertBlocks { at_block: usize, at_run: usize, at_byte: usize, blocks: FlatDocument },
}

impl StoredEdit {
    fn encode(edit: &Edit, images: &mut Vec<Arc<Vec<u8>>>) -> io::Result<Self> {
        Ok(match edit {
            Edit::InsertBlocks { at_block, at_run, at_byte, blocks } => Self::InsertBlocks {
                at_block: *at_block, at_run: *at_run, at_byte: *at_byte, blocks: FlatDocument::encode(blocks, images)?,
            },
            other => Self::Plain(other.clone()),
        })
    }

    fn decode(self, images: &[Arc<Vec<u8>>]) -> io::Result<Edit> {
        match self {
            Self::Plain(Edit::InsertBlocks { .. }) => Err(invalid("unflattened recovery edit")),
            Self::Plain(edit) => Ok(edit),
            Self::InsertBlocks { at_block, at_run, at_byte, blocks } => Ok(Edit::InsertBlocks { at_block, at_run, at_byte, blocks: blocks.decode(images)? }),
        }
    }
}

struct LimitedBytes(Vec<u8>);

impl Write for LimitedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_JSON_BYTES.saturating_sub(self.0.len()) { return Err(invalid("rich recovery metadata exceeds 32 MiB")); }
        self.0.try_reserve(bytes.len()).map_err(io::Error::other)?;
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

pub(super) fn read(path: &Path) -> io::Result<(RichDocPayload, String)> {
    let path = path.to_owned();
    // Malicious nested JSON is refused by serde's recursion limit. The large
    // stack also covers the bounded RTF parser and recursive AST destruction.
    std::thread::Builder::new().name("rich-recovery".into()).stack_size(32 * 1024 * 1024)
        .spawn(move || read_inner(&path))?.join().map_err(|_| io::Error::other("rich recovery worker failed"))?
}

fn read_inner(path: &Path) -> io::Result<(RichDocPayload, String)> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > MAX_BYTES { return Err(invalid("rich recovery file exceeds 256 MiB")); }
    let mut zip = zip::ZipArchive::new(file).map_err(io::Error::other)?;
    if zip.len() > MAX_IMAGES.saturating_add(3) { return Err(invalid("too many recovery archive entries")); }
    let mut total = 0u64;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(io::Error::other)?;
        total = total.checked_add(entry.size()).ok_or_else(|| invalid("recovery archive size overflow"))?;
        if total > MAX_BYTES { return Err(invalid("expanded recovery archive exceeds 256 MiB")); }
        if index < 3 && entry.name() != ["state.json", "original.bin", "source.txt"].get(index).copied().unwrap_or("") {
            return Err(invalid("invalid recovery archive layout"));
        }
        if index >= 3 && entry.name() != format!("image-{}", index.saturating_sub(3)) { return Err(invalid("invalid recovery image name")); }
    }
    let json = read_entry(&mut zip, "state.json", MAX_JSON_BYTES as u64)?;
    let state: State = serde_json::from_slice(&json).map_err(io::Error::other)?;
    if state.version != VERSION { return Err(invalid("unsupported rich recovery version")); }
    let original = Arc::new(read_entry(&mut zip, "original.bin", MAX_BYTES)?);
    let source = String::from_utf8(read_entry(&mut zip, "source.txt", MAX_BYTES)?).map_err(io::Error::other)?;
    let mut images = Vec::new();
    for index in 0..zip.len().saturating_sub(3) {
        images.push(Arc::new(read_entry(&mut zip, &format!("image-{index}"), MAX_BYTES)?));
    }
    if state.format == DocFormat::Rtf && state.edits.is_empty()
        && crate::text::line_ending::split_line_endings(state.raw.clone()).1 != source {
        // Raw View edits replace the RTF document. Its stored AST predates
        // those edits; reparse the source contained in this same atomic file.
        let payload = crate::rich_doc::rtf::open_bytes(source.as_bytes().to_vec()).map_err(io::Error::other)?;
        return Ok((payload, source));
    }
    let ast = ast::Document { blocks: state.ast.decode(&images)? };
    let headers = state.headers.into_iter().map(|doc| doc.decode(&images)).collect::<io::Result<_>>()?;
    let footers = state.footers.into_iter().map(|doc| doc.decode(&images)).collect::<io::Result<_>>()?;
    let edit_log = state.edits.into_iter().map(|edit| edit.decode(&images)).collect::<io::Result<_>>()?;
    Ok((RichDocPayload {
        format: state.format, ast, raw_view_text: state.raw, original_bytes: original, edit_log,
        headers, footers, active_block: None, selection: None, drag_active: false,
        pending_drag_anchor: None, undo_stack: Vec::new(), redo_stack: Vec::new(), edit_group_start: None,
    }, source))
}

fn read_entry<R: Read + io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str, limit: u64) -> io::Result<Vec<u8>> {
    let entry = zip.by_name(name).map_err(io::Error::other)?;
    let declared = entry.size();
    if declared > limit { return Err(invalid("recovery entry exceeds its size limit")); }
    let mut bytes = Vec::new();
    entry.take(declared.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != declared { return Err(invalid("recovery entry length does not match its declaration")); }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::TempDir;

    fn paragraph(text: &str) -> ast::Block {
        ast::Block::Paragraph { runs: vec![ast::InlineRun::plain(text)], props: ast::BlockProps::default() }
    }

    fn table(block: ast::Block) -> ast::Block {
        ast::Block::Table { rows: vec![ast::TableRow { cells: vec![ast::TableCell { blocks: vec![block] }] }] }
    }

    #[test]
    fn nested_tables_images_headers_and_pasted_blocks_round_trip() {
        let dir = TempDir::new("rich-recovery-structure");
        let path = dir.join("snapshot.rich");
        let mut payload = crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi original}".to_vec()).expect("rtf");
        let image = ast::Block::Image { source: ast::ImageSource::Inline { bytes: Arc::new(vec![1, 2, 3, 255]), mime: "image/png".into() }, width_px: Some(42), height_px: Some(55), alt: Some("中".into()) };
        let nested = (0..40).fold(paragraph("café 中"), |block, _| table(block));
        payload.ast.blocks = vec![nested, image.clone()];
        payload.headers = vec![vec![table(paragraph("header"))]];
        payload.footers = vec![vec![paragraph("footer")]];
        payload.edit_log.push(Edit::InsertBlocks { at_block: usize::MAX, at_run: 0, at_byte: 0, blocks: vec![table(image)] });
        Capture::new(&payload).write(&path, "source\n").expect("write");
        let (restored, source) = read(&path).expect("read");
        assert_eq!(restored.ast, payload.ast);
        assert_eq!(restored.headers, payload.headers);
        assert_eq!(restored.footers, payload.footers);
        assert_eq!(restored.edit_log, payload.edit_log);
        assert_eq!(restored.original_bytes, payload.original_bytes);
        assert_eq!(restored.raw_view_text, payload.raw_view_text);
        assert_eq!(source, "source\n");
        assert!(restored.undo_stack.is_empty());
    }

    #[test]
    fn raw_rtf_edits_are_reparsed_from_the_same_snapshot() {
        let dir = TempDir::new("rich-recovery-raw");
        let path = dir.join("snapshot.rich");
        let payload = crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi original}".to_vec()).expect("rtf");
        Capture::new(&payload).write(&path, r"{\rtf1\ansi changed}").expect("write");
        let (restored, source) = read(&path).expect("read");
        assert_eq!(restored.block_text(0), "changed");
        assert_eq!(restored.raw_view_text, source);
        assert_eq!(restored.original_bytes.as_slice(), source.as_bytes());
    }

    #[test]
    fn cycles_shared_children_orphans_missing_images_and_excessive_depth_are_errors() {
        let para = || FlatBlock::Paragraph { runs: vec![ast::InlineRun::plain("x")], props: ast::BlockProps::default() };
        for doc in [
            FlatDocument { roots: vec![0], nodes: vec![FlatBlock::Table { rows: vec![vec![vec![0]]] }] },
            FlatDocument { roots: vec![0], nodes: vec![FlatBlock::Table { rows: vec![vec![vec![1, 1]]] }, para()] },
            FlatDocument { roots: vec![0, 0], nodes: vec![para()] },
            FlatDocument { roots: vec![], nodes: vec![para()] },
            FlatDocument { roots: vec![usize::MAX], nodes: vec![para()] },
            FlatDocument { roots: vec![0], nodes: vec![FlatBlock::Image { image: usize::MAX, mime: String::new(), width_px: None, height_px: None, alt: None }] },
        ] {
            assert!(doc.decode(&[]).is_err());
        }
        let mut nodes: Vec<_> = (0..MAX_DEPTH + 1).map(|index| FlatBlock::Table { rows: vec![vec![vec![index + 1]]] }).collect();
        nodes.push(para());
        assert!(FlatDocument { roots: vec![0], nodes }.decode(&[]).is_err());
        let nested = (0..MAX_DEPTH + 1).fold(paragraph("x"), |block, _| table(block));
        assert!(FlatDocument::encode(&[nested], &mut Vec::new()).is_err());
    }

    fn archive(path: &Path, parts: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in parts { zip.start_file(*name, options).expect("entry"); zip.write_all(data).expect("bytes"); }
        std::fs::write(path, zip.finish().expect("zip").into_inner()).expect("archive");
    }

    #[test]
    fn damaged_archives_versions_and_recursive_json_are_refused() {
        let dir = TempDir::new("rich-recovery-hostile");
        let path = dir.join("snapshot.rich");
        let payload = crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi original}".to_vec()).expect("rtf");
        Capture::new(&payload).write(&path, &payload.raw_view_text).expect("write");
        let bytes = std::fs::read(&path).expect("bytes");
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let json = read_entry(&mut zip, "state.json", MAX_JSON_BYTES as u64).expect("state");
        let mut state: serde_json::Value = serde_json::from_slice(&json).expect("json");
        state["version"] = (VERSION + 1).into();
        archive(&path, &[("state.json", &serde_json::to_vec(&state).expect("json")), ("original.bin", b""), ("source.txt", b"")]);
        assert!(read(&path).is_err());
        let deep = format!("{}0{}", "[".repeat(1000), "]".repeat(1000));
        archive(&path, &[("state.json", deep.as_bytes()), ("original.bin", b""), ("source.txt", b"")]);
        assert!(read(&path).is_err());
        archive(&path, &[("wrong-name", &json), ("original.bin", b""), ("source.txt", b"")]);
        assert!(read(&path).is_err());
        std::fs::write(&path, b"broken").expect("write");
        assert!(read(&path).is_err());
    }

    #[test]
    fn declared_entry_limits_are_checked_before_allocation() {
        let dir = TempDir::new("rich-recovery-limits");
        let path = dir.join("snapshot.rich");
        archive(&path, &[("state.json", b"123456789")]);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).expect("file")).expect("zip");
        assert!(read_entry(&mut zip, "state.json", 8).is_err());
        drop(zip);
        let mut encoded = std::fs::read(&path).expect("archive bytes");
        let local = encoded.windows(4).position(|bytes| bytes == b"PK\x03\x04").expect("local header");
        let central = encoded.windows(4).position(|bytes| bytes == b"PK\x01\x02").expect("central header");
        encoded.get_mut(local + 22..local + 26).expect("local size").copy_from_slice(&8u32.to_le_bytes());
        encoded.get_mut(central + 24..central + 28).expect("central size").copy_from_slice(&8u32.to_le_bytes());
        std::fs::write(&path, encoded).expect("misdeclared entry");
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).expect("file")).expect("zip");
        assert!(read_entry(&mut zip, "state.json", 16).is_err());
        drop(zip);
        let mut bytes = LimitedBytes(vec![0; MAX_JSON_BYTES]);
        assert!(bytes.write_all(b"x").is_err());
        let oversized = std::fs::File::create(&path).expect("file");
        oversized.set_len(MAX_BYTES + 1).expect("sparse oversized file");
        drop(oversized);
        assert!(read(&path).is_err());
    }

    #[test]
    #[ignore = "release timing on a rich document above the streaming threshold"]
    fn large_rich_recovery_timing() {
        let dir = TempDir::new("rich-recovery-timing");
        let mut bytes = br"{\rtf1\ansi ".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', 11 * 1024 * 1024));
        bytes.push(b'}');
        let payload = crate::rich_doc::rtf::open_bytes(bytes).expect("large rtf");
        let started = std::time::Instant::now();
        let old: Vec<_> = (0..5).map(|_| payload.original_bytes.as_ref().clone()).collect();
        let old_copy = started.elapsed();
        drop(old);
        let started = std::time::Instant::now();
        let captures: Vec<_> = (0..5).map(|_| Capture::new(&payload)).collect();
        let capture = started.elapsed();
        let started = std::time::Instant::now();
        captures.first().expect("capture").write(&dir.join("snapshot.rich"), &payload.raw_view_text).expect("worker write");
        let write = started.elapsed();
        println!("11 MiB RTF: five original-byte Vec copies {old_copy:?}; five AST/raw captures with shared originals {capture:?}; one structured worker write {write:?}");
    }
}
