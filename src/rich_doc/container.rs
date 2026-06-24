//! Zip container helpers shared across formats. Phase 1: DOCX only.

use super::{Result, RichDocError};
use std::io::{Cursor, Read, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// Extract a single text entry from a zip. Used to pull `word/document.xml`
/// out of a `.docx` for Raw View.
pub fn read_zip_entry_text(zip_path: &Path, entry_name: &str) -> Result<String> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| RichDocError::Zip(e.to_string()))?;
    let mut entry = archive
        .by_name(entry_name)
        .map_err(|e| RichDocError::Zip(format!("entry {entry_name}: {e}")))?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf)?;
    Ok(buf)
}

/// Extract a single binary entry from a zip. Used for image media parts that
/// aren't valid UTF-8 (PNG/JPEG/etc bytes).
pub fn read_zip_entry_bytes(zip_path: &Path, entry_name: &str) -> Result<Vec<u8>> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| RichDocError::Zip(e.to_string()))?;
    let mut entry = archive
        .by_name(entry_name)
        .map_err(|e| RichDocError::Zip(format!("entry {entry_name}: {e}")))?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
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
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
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

    #[test]
    fn missing_file_errors() {
        let err = read_zip_entry_text(&fixture("does-not-exist.docx"), "word/document.xml")
            .expect_err("missing file should error");
        assert!(matches!(err, RichDocError::Io(_)));
    }
}
