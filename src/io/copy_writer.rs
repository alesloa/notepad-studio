// Existing Save and Save a Copy As use the same encoding and native writers.
use super::*;

// Capture only cheap buffer metadata. Native content is captured in bounded UI turns.
pub(super) fn snapshot(buffer: &Buffer) -> Buffer {
    Buffer { rope: buffer.rope.clone(), file_path: None, dirty: buffer.dirty,
        raw_revision: buffer.raw_revision.clone(), savepoint: buffer.savepoint.clone(),
        line_ending: buffer.line_ending, encoding: buffer.encoding, bookmarks: Default::default(),
        marks: Vec::new(), language: None, highlights: Vec::new(), loading: None,
        new_label: None, read_only: buffer.read_only, tab_color: None,
        is_changelog: false, rich_doc: None, flat_text_cache: None }
}

pub(super) fn write_buffer(buffer: &Buffer, path: &Path) -> Result<(), String> {
    if buffer.loading.is_some() { return Err("document is still loading".into()); }
    if let Some(payload) = buffer.rich_doc.as_ref() {
        let raw_override = buffer.dirty.then(|| buffer.rope.to_string());
        crate::rich_doc::validate_raw_view(payload, raw_override.as_deref())
            .map_err(|error| format!("rich-doc save failed: {error}"))?;
        if path.is_file() { run_save_backup(path); }
        return crate::rich_doc::save(payload, path, raw_override.as_deref())
            .map_err(|error| format!("rich-doc save failed: {error}"));
    }
    let eol_bytes = rope_to_bytes(&buffer.rope, buffer.line_ending);
    let bytes = if buffer.encoding == Encoding::Utf8 {
        eol_bytes
    } else {
        let utf8 = std::str::from_utf8(&eol_bytes)
            .map_err(|error| format!("internal: rope was not valid UTF-8: {error}"))?;
        encoding::encode_checked(utf8, buffer.encoding).map_err(|error| error.to_string())?
    };
    save_bytes_with_backup(path, &bytes, run_save_backup).map_err(|error| error.to_string())
}

pub(super) fn write_copy(buffer: &Buffer, path: &Path) -> Result<(), String> {
    if let Some(payload) = buffer.rich_doc.as_ref().filter(|payload| payload.format == crate::rich_doc::DocFormat::Doc) {
        if buffer.loading.is_some() { return Err("document is still loading".into()); }
        if buffer.dirty || !payload.edit_log.is_empty() {
            return Err("edited .doc documents require Save As .docx".into());
        }
        return save_bytes_with_backup(path, &payload.original_bytes, run_save_backup).map_err(|error| error.to_string());
    }
    write_buffer(buffer, path)
}
