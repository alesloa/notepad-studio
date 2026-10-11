//! Zip container helpers shared across formats. Phase 1: DOCX only.

use super::{Result, RichDocError, MAX_NESTING_DEPTH};
use std::io::{BufReader, Cursor, Read, Write};
use std::path::Path;

use quick_xml::events::Event;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// Most bytes a .docx / .odt may decompress to, per entry and summed over
/// the archive. This is the editor's existing huge-file line
/// (`io_stream::HUGE_FILE_WARNING_BYTES`), past which a plain-text open asks
/// before reading; a rich-doc open has no prompt, so it refuses instead.
const MAX_UNCOMPRESSED_BYTES: u64 = crate::io_stream::HUGE_FILE_WARNING_BYTES;

/// Vet a .docx / .odt archive before any parser reads it:
/// - the uncompressed sizes the entries declare must sum to no more than
///   `MAX_UNCOMPRESSED_BYTES`;
/// - no entry may inflate past the size it declares (zip 4 keeps inflating,
///   so a small file can expand without limit);
/// - every `.xml` and `.rels` part must nest no deeper than
///   `MAX_NESTING_DEPTH`. docx-rust recurses per element level and the ODT
///   Raw View pretty-printer emits indentation that grows with depth
///   squared, so hostile nesting would overflow the stack or exhaust memory.
///
/// Corrupt entries and malformed XML return an error before parsing.
pub(crate) fn check_archive(bytes: &[u8]) -> Result<()> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| RichDocError::Zip(e.to_string()))?;
    let mut declared_total = 0u64;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|err| RichDocError::Zip(err.to_string()))?;
        let name = entry.name().to_owned();
        let declared = entry.size();
        declared_total = declared_total
            .checked_add(declared)
            .filter(|total| *total <= MAX_UNCOMPRESSED_BYTES)
            .ok_or_else(|| {
                RichDocError::Zip(format!(
                    "archive expands past the {MAX_UNCOMPRESSED_BYTES}-byte limit (at {name})"
                ))
            })?;
        let mut counted = CountingReader {
            inner: entry.take(declared.saturating_add(1)),
            count: 0,
        };
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let mut buffered = BufReader::new(&mut counted);
            check_xml(&mut buffered)?;
            std::io::copy(&mut buffered, &mut std::io::sink())?;
        } else {
            std::io::copy(&mut counted, &mut std::io::sink())?;
        }
        if counted.count > declared {
            return Err(inflates_past_declared(&name, declared));
        }
    }
    Ok(())
}

/// `Read` adapter that tallies the bytes it hands out.
struct CountingReader<R> {
    inner: R,
    count: u64,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count = self.count.saturating_add(n as u64);
        Ok(n)
    }
}

fn inflates_past_declared(name: &str, declared: u64) -> RichDocError {
    RichDocError::Zip(format!(
        "entry {name} inflates past the {declared} bytes its header declares"
    ))
}

/// Read a whole entry without trusting its header: refuse a declared size
/// past `MAX_UNCOMPRESSED_BYTES`, and stop one byte past the declared size
/// so an entry that inflates further is an error rather than unbounded.
fn read_entry_capped<R: Read>(entry: &mut zip::read::ZipFile<'_, R>) -> Result<Vec<u8>> {
    let name = entry.name().to_owned();
    let declared = entry.size();
    if declared > MAX_UNCOMPRESSED_BYTES {
        return Err(RichDocError::Zip(format!(
            "entry {name} declares {declared} bytes, past the {MAX_UNCOMPRESSED_BYTES}-byte limit"
        )));
    }
    let mut buf = Vec::new();
    entry.take(declared.saturating_add(1)).read_to_end(&mut buf)?;
    if buf.len() as u64 > declared {
        return Err(inflates_past_declared(&name, declared));
    }
    Ok(buf)
}

/// Bound XML nesting before recursive readers or pretty-printers use it.
fn check_xml<R: std::io::BufRead>(xml: R) -> Result<()> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(_)) => {
                depth = depth.saturating_add(1);
                if depth > MAX_NESTING_DEPTH {
                    return Err(RichDocError::Zip(format!("XML nests more than {MAX_NESTING_DEPTH} elements deep at byte {}", reader.buffer_position())));
                }
            }
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::Eof) => {
                return if depth == 0 { Ok(()) } else { Err(RichDocError::Zip("unclosed XML elements".into())) };
            }
            Err(err) => return Err(RichDocError::Zip(format!("XML at byte {}: {err}", reader.buffer_position()))),
            Ok(_) => {}
        }
        buf.clear();
    }
}

/// Extract a single text entry from a zip. Used to pull `word/document.xml`
/// out of a `.docx` for Raw View.
pub fn read_zip_entry_text(zip_path: &Path, entry_name: &str) -> Result<String> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| RichDocError::Zip(e.to_string()))?;
    let mut entry = archive
        .by_name(entry_name)
        .map_err(|e| RichDocError::Zip(format!("entry {entry_name}: {e}")))?;
    let bytes = read_entry_capped(&mut entry)?;
    if entry_name.ends_with(".xml") || entry_name.ends_with(".rels") {
        check_xml(Cursor::new(&bytes))?;
    }
    String::from_utf8(bytes)
        .map_err(|e| RichDocError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
}

/// Extract a single binary entry from a zip. Used for image media parts that
/// aren't valid UTF-8 (PNG/JPEG/etc bytes).
pub fn read_zip_entry_bytes(zip_path: &Path, entry_name: &str) -> Result<Vec<u8>> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| RichDocError::Zip(e.to_string()))?;
    let mut entry = archive
        .by_name(entry_name)
        .map_err(|e| RichDocError::Zip(format!("entry {entry_name}: {e}")))?;
    read_entry_capped(&mut entry)
}

/// List zip entry names matching a prefix + suffix filter. Used to discover
/// `word/header*.xml`, `word/footer*.xml`, `Pictures/*` etc. without knowing
/// the exact filenames ahead of time. Both filters are case-sensitive byte
/// comparisons against the entry name. Returns names sorted for deterministic
/// header/footer ordering on re-open.
pub fn list_zip_entries(zip_path: &Path, prefix: &str, suffix: &str) -> Result<Vec<String>> {
    let file = std::fs::File::open(zip_path)?;
    let archive = zip::ZipArchive::new(file).map_err(|e| RichDocError::Zip(e.to_string()))?;
    let mut out: Vec<String> = archive
        .file_names()
        .filter(|n| n.starts_with(prefix) && n.ends_with(suffix))
        .map(|n| n.to_string())
        .collect();
    out.sort();
    Ok(out)
}

/// Re-pack a zip by copying every entry from `source_bytes` except
/// `replace_entry`, which is substituted with `replacement`. Returns the new
/// zip bytes. Preserves entry names, comments, and the compression method of
/// each copied entry. The replacement entry uses DEFLATE.
///
/// Used by `docx::save` to swap a freshly-serialized `word/document.xml` into
/// the original DOCX without touching media, styles, headers/footers, etc.
pub fn replace_zip_entry(
    source_bytes: &[u8],
    replace_entry: &str,
    replacement: &[u8],
) -> Result<Vec<u8>> {
    replace_zip_entries(source_bytes, &[(replace_entry, replacement)])
}

/// Same as `replace_zip_entry` but swaps multiple entries in a single pass.
/// Used by DOCX save when both `word/document.xml` and `word/_rels/document.xml.rels`
/// need updating together (hyperlink insertion appends new <Relationship/> rows).
/// Entries not in `replacements` copy through with their original compression.
/// Replacement entries are written DEFLATE.
pub fn replace_zip_entries(
    source_bytes: &[u8],
    replacements: &[(&str, &[u8])],
) -> Result<Vec<u8>> {
    let cursor = Cursor::new(source_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| RichDocError::Zip(e.to_string()))?;
    let mut out_buf: Vec<u8> = Vec::with_capacity(source_bytes.len());
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut out_buf));
        let mut written: Vec<bool> = vec![false; replacements.len()];
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| RichDocError::Zip(e.to_string()))?;
            let name = entry.name().to_owned();
            if let Some(idx) = replacements.iter().position(|(n, _)| *n == name) {
                let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
                writer
                    .start_file(&name, opts)
                    .map_err(|e| RichDocError::Zip(e.to_string()))?;
                writer
                    .write_all(replacements[idx].1)
                    .map_err(RichDocError::Io)?;
                written[idx] = true;
                continue;
            }
            let opts = SimpleFileOptions::default().compression_method(entry.compression());
            writer
                .start_file(&name, opts)
                .map_err(|e| RichDocError::Zip(e.to_string()))?;
            let buf = read_entry_capped(&mut entry)?;
            writer.write_all(&buf).map_err(RichDocError::Io)?;
        }
        for (idx, (name, bytes)) in replacements.iter().enumerate() {
            if !written[idx] {
                let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
                writer
                    .start_file(*name, opts)
                    .map_err(|e| RichDocError::Zip(e.to_string()))?;
                writer.write_all(bytes).map_err(RichDocError::Io)?;
            }
        }
        writer
            .finish()
            .map_err(|e| RichDocError::Zip(e.to_string()))?;
    }
    Ok(out_buf)
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

    #[test]
    fn extracts_document_xml_from_docx() {
        let xml = read_zip_entry_text(&fixture("hello.docx"), "word/document.xml")
            .expect("document.xml should extract");
        assert!(xml.contains("<w:document"), "extracted xml should contain <w:document, got start: {}", &xml[..xml.len().min(200)]);
    }

    #[test]
    fn missing_entry_errors() {
        let err = read_zip_entry_text(&fixture("hello.docx"), "nope/missing.xml")
            .expect_err("missing entry should error");
        match err {
            RichDocError::Zip(_) => {}
            other => panic!("expected Zip error, got {other:?}"),
        }
    }

    #[test]
    fn replace_zip_entry_swaps_target_keeps_others() {
        let original = std::fs::read(fixture("hello.docx")).expect("read fixture");
        let replacement = b"<?xml version=\"1.0\"?><stub/>".to_vec();
        let new_zip = replace_zip_entry(&original, "word/document.xml", &replacement)
            .expect("replace works");
        // Round trip: new document.xml is our replacement
        let cursor = std::io::Cursor::new(&new_zip);
        let mut archive = zip::ZipArchive::new(cursor).expect("re-open new zip");
        let mut got = String::new();
        archive
            .by_name("word/document.xml")
            .expect("entry exists")
            .read_to_string(&mut got)
            .expect("read string");
        assert_eq!(got.as_bytes(), replacement.as_slice());
        // And: another entry survives unchanged
        let mut styles = String::new();
        archive
            .by_name("word/styles.xml")
            .expect("styles survived")
            .read_to_string(&mut styles)
            .expect("read styles");
        assert!(styles.contains("<w:styles"));
    }

    /// Zip with the given (name, contents) entries, deflated.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            for (name, data) in entries {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    /// Overwrite the uncompressed size an entry declares, in both its local
    /// header and its central-directory record.
    fn patch_declared_size(zip: &mut [u8], name: &str, size: u32) {
        let mut patched = 0;
        for at in 0..zip.len().saturating_sub(46) {
            let sig = u32::from_le_bytes(zip[at..at + 4].try_into().unwrap());
            let (size_at, name_len_at, name_at) = match sig {
                0x0403_4b50 => (at + 22, at + 26, at + 30),
                0x0201_4b50 => (at + 24, at + 28, at + 46),
                _ => continue,
            };
            let len = u16::from_le_bytes([zip[name_len_at], zip[name_len_at + 1]]) as usize;
            if zip.get(name_at..name_at + len) == Some(name.as_bytes()) {
                zip[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
                patched += 1;
            }
        }
        assert_eq!(patched, 2, "local header + central record for {name}");
    }

    fn temp_zip(file: &str, bytes: &[u8]) -> (crate::persist::TempDir, PathBuf) {
        let dir = crate::persist::TempDir::new("rich-archive");
        let p = dir.join(file);
        std::fs::write(&p, bytes).expect("fixture");
        (dir, p)
    }

    #[test]
    fn entry_declared_past_the_cap_is_refused() {
        let mut z = zip_of(&[("word/document.xml", b"<w:document/>")]);
        patch_declared_size(&mut z, "word/document.xml", 0xFFFF_FFF0);
        assert!(check_archive(&z).is_err());
        let (_dir, p) = temp_zip("notepad-studio-zip-declared-huge.docx", &z);
        assert!(read_zip_entry_text(&p, "word/document.xml").is_err());
        assert!(read_zip_entry_bytes(&p, "word/document.xml").is_err());
    }

    #[test]
    fn entries_declared_past_the_cap_together_are_refused() {
        let gib = 1024 * 1024 * 1024;
        let mut z = zip_of(&[("a.bin", b"a"), ("b.bin", b"b"), ("c.bin", b"c")]);
        for name in ["a.bin", "b.bin", "c.bin"] {
            patch_declared_size(&mut z, name, gib);
        }
        assert!(check_archive(&z).is_err());
    }

    #[test]
    fn entry_inflating_past_its_declared_size_is_refused() {
        // zip 4 keeps inflating past the declared size; a 1 MB entry that
        // claims 10 bytes is a bomb in miniature.
        let zeros = vec![0u8; 1024 * 1024];
        let mut z = zip_of(&[("word/document.xml", &zeros), ("word/media/x.png", &zeros)]);
        patch_declared_size(&mut z, "word/document.xml", 10);
        assert!(check_archive(&z).is_err());
        let (_dir, p) = temp_zip("notepad-studio-zip-lying-size.docx", &z);
        assert!(read_zip_entry_text(&p, "word/document.xml").is_err());
        let mut z = zip_of(&[("word/media/x.png", &zeros)]);
        patch_declared_size(&mut z, "word/media/x.png", 10);
        assert!(check_archive(&z).is_err());
        let (_dir, p) = temp_zip("notepad-studio-zip-lying-size-media.docx", &z);
        assert!(read_zip_entry_bytes(&p, "word/media/x.png").is_err());
        assert!(replace_zip_entry(&z, "other.xml", b"<x/>").is_err());
    }

    #[test]
    fn honest_archive_passes_the_check() {
        let original = std::fs::read(fixture("hello.docx")).expect("read fixture");
        check_archive(&original).expect("fixture is fine");
    }

    #[test]
    fn xml_depth_is_bounded_and_text_does_not_count_as_elements() {
        let limit = format!("{}x{}", "<x>".repeat(MAX_NESTING_DEPTH), "</x>".repeat(MAX_NESTING_DEPTH));
        check_xml(Cursor::new(limit)).expect("limit");
        let too_deep = format!("{}x{}", "<x>".repeat(MAX_NESTING_DEPTH + 1), "</x>".repeat(MAX_NESTING_DEPTH + 1));
        assert!(check_xml(Cursor::new(too_deep)).is_err());
        let text = format!("<x><![CDATA[{}]]>&lt;x&gt;</x>", "<x>".repeat(100_000));
        check_xml(Cursor::new(text)).expect("text");
    }

    #[test]
    fn optional_xml_parts_and_corrupt_entries_are_checked() {
        let deep = format!("{}{}", "<x>".repeat(MAX_NESTING_DEPTH + 1), "</x>".repeat(MAX_NESTING_DEPTH + 1));
        let archive = zip_of(&[("word/document.xml", b"<x/>"), ("word/_rels/document.xml.rels", deep.as_bytes())]);
        assert!(check_archive(&archive).is_err());
        for xml in ["<a></b>", "<a>"] {
            let archive = zip_of(&[("word/document.xml", xml.as_bytes())]);
            assert!(check_archive(&archive).is_err());
        }
    }

    #[test]
    fn entry_inflating_past_declared_size_is_rejected_by_reader() {
        let zeros = vec![0; 1024 * 1024];
        let mut archive = zip_of(&[("word/media/x.png", &zeros)]);
        patch_declared_size(&mut archive, "word/media/x.png", 10);
        let (_dir, path) = temp_zip("document.docx", &archive);
        assert!(read_zip_entry_bytes(&path, "word/media/x.png").is_err());
    }

    #[test]
    fn missing_file_errors() {
        let err = read_zip_entry_text(&fixture("does-not-exist.docx"), "word/document.xml")
            .expect_err("missing file should error");
        assert!(matches!(err, RichDocError::Io(_)));
    }
}
