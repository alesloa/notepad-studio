// Original atomic Reload worker; NppIO.cpp:588 confirms discarding the old document.
use std::{io::{Read, Seek, SeekFrom, Write}, path::PathBuf,
    sync::{Arc, atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering}}};
use crate::{buffer::Buffer, io_stream::{StreamDecoder, StreamMode}, settings::Settings};

pub(crate) struct Progress {
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) parser_cancel: Arc<AtomicUsize>,
    pub(crate) bytes: AtomicU64,
    pub(crate) total: AtomicU64,
    #[cfg(test)] pub(crate) elapsed_nanos: AtomicU64,
}
impl Default for Progress {
    fn default() -> Self { Self { cancel: Arc::new(AtomicBool::new(false)),
        parser_cancel: Arc::new(AtomicUsize::new(0)), bytes: AtomicU64::new(0), total: AtomicU64::new(0),
        #[cfg(test)] elapsed_nanos: AtomicU64::new(0) } }
}
impl Progress {
    pub(crate) fn stop(&self) { self.cancel.store(true, Ordering::Relaxed); self.parser_cancel.store(1, Ordering::Relaxed); }
    fn check(&self) -> Result<(), Error> { if self.cancel.load(Ordering::Relaxed) { self.stop(); Err(Error::Cancelled) } else { Ok(()) } }
}
pub(crate) enum Error { Cancelled, Huge(u64), Failed(String) }
impl From<std::io::Error> for Error { fn from(e: std::io::Error) -> Self { Self::Failed(e.to_string()) } }
pub(crate) struct Request { pub(crate) path: PathBuf, pub(crate) settings: Settings,
    pub(crate) progress: Arc<Progress>, pub(crate) allow_huge: bool }

fn chunk(file: &mut std::fs::File, progress: &Progress) -> Result<Vec<u8>, Error> {
    progress.check()?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(crate::io_stream::STREAM_CHUNK_SIZE).map_err(|_| Error::Failed("Not enough memory to reload this document".into()))?;
    bytes.resize(crate::io_stream::STREAM_CHUNK_SIZE, 0);
    let mut filled = 0usize;
    while filled < bytes.len() {
        progress.check()?;
        let Some(remaining) = bytes.get_mut(filled..) else { break; };
        match file.read(remaining) {
            Ok(0) => break,
            Ok(count) => filled = filled.saturating_add(count),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bytes.truncate(filled);
    progress.check()?;
    Ok(bytes)
}
fn limits(bytes: u64, allow_huge: bool) -> Result<(), Error> {
    if bytes > isize::MAX as u64 { return Err(Error::Failed("Document exceeds this build's memory limit".into())); }
    let warning = crate::io_stream::HUGE_FILE_WARNING_BYTES;
    #[cfg(test)] let warning = TEST_WARNING.with(|limit| limit.get().unwrap_or(warning));
    if !allow_huge && bytes >= warning { return Err(Error::Huge(bytes)); }
    Ok(())
}

pub(crate) fn read(request: Request) -> Result<Buffer, Error> {
    let progress = &request.progress;
    progress.check()?;
    let mut file = std::fs::File::open(&request.path)?;
    let initial = file.metadata()?;
    if !initial.is_file() { return Err(Error::Failed("Reload requires a regular file".into())); }
    limits(initial.len(), request.allow_huge)?;
    progress.total.store(initial.len(), Ordering::Relaxed);
    let mut buffer = if crate::rich_doc::detect_format(&request.path).is_some() {
        // Existing parsers reopen parts by path. Keep every reread on one private snapshot.
        let directory = tempfile::Builder::new().prefix("notepad-studio-reload-").tempdir()?;
        let name = format!("document.{}", request.path.extension().and_then(|x| x.to_str()).unwrap_or("bin"));
        let snapshot = directory.path().join(name);
        let mut output = std::fs::File::create(&snapshot)?;
        let mut count = 0u64;
        loop {
            let bytes = chunk(&mut file, progress)?;
            if bytes.is_empty() { break; }
            count = count.checked_add(bytes.len() as u64).ok_or_else(|| Error::Failed("Document size overflow".into()))?;
            limits(count, request.allow_huge)?;
            output.write_all(&bytes)?;
            progress.bytes.store(count, Ordering::Relaxed);
        }
        output.flush()?; drop(output);
        unchanged(&file, &initial, count)?;
        progress.check()?;
        // This joins only on the background executor. Cancel blocks publication while parsing finishes.
        let payload = crate::rich_doc::open(&snapshot).map_err(|e| Error::Failed(e.to_string()))?;
        progress.check()?;
        limits(payload.raw_view_text.len() as u64, request.allow_huge)?;
        let (eol, text) = crate::text::line_ending::split_line_endings(payload.raw_view_text.clone());
        let mut buffer = Buffer::empty();
        buffer.rope = ropey::Rope::from_str(&text);
        buffer.encoding = crate::text::encoding::Encoding::Utf8;
        buffer.line_ending = eol; buffer.rich_doc = Some(payload);
        buffer
    } else {
        let mut mode = None;
        let mut completed = None;
        for _ in 0..3 {
            progress.check()?; file.seek(SeekFrom::Start(0))?;
            let mut bytes = chunk(&mut file, progress)?;
            let pass = *mode.get_or_insert_with(|| StreamMode::sniff(&bytes));
            let mut decoder = StreamDecoder::new(pass);
            let mut builder = ropey::RopeBuilder::new();
            let mut count = 0u64; let mut decoded = 0u64;
            loop {
                progress.check()?;
                let eof = bytes.is_empty();
                count = count.checked_add(bytes.len() as u64).ok_or_else(|| Error::Failed("Document size overflow".into()))?;
                limits(count, request.allow_huge)?;
                let text = match if eof { decoder.finish() } else { decoder.feed(&bytes) } {
                    Ok(text) => text,
                    Err(restart) => { mode = Some(pass.after(restart)); break; }
                };
                decoded = decoded.checked_add(text.len() as u64)
                    .ok_or_else(|| Error::Failed("Decoded document exceeds this build's memory limit".into()))?;
                limits(decoded, request.allow_huge)?;
                builder.append(&text); progress.bytes.store(count, Ordering::Relaxed);
                if eof {
                    unchanged(&file, &initial, count)?;
                    let mut buffer = Buffer::empty(); buffer.rope = builder.finish();
                    buffer.encoding = pass.encoding; buffer.line_ending = decoder.line_ending(); buffer.read_only = decoder.lossy;
                    completed = Some(buffer); break;
                }
                bytes = chunk(&mut file, progress)?;
            }
            if completed.is_some() { break; }
        }
        completed.ok_or_else(|| Error::Failed("Reload decoding did not settle".into()))?
    };
    buffer.file_path = Some(request.path.clone());
    buffer.language = crate::syntax::language_for_path(&request.path).or_else(|| {
        if buffer.rich_doc.is_some() { return None; }
        let first = buffer.rope.get_line(0)?;
        // A shebang cannot require flattening a megabyte-long first line.
        let prefix: String = first.chars().take(4096).collect();
        crate::syntax::language_for_shebang(&prefix)
    });
    if let Some(language) = buffer.language {
        if !crate::syntax::should_skip_syntax(buffer.rope.len_bytes() as u64, &request.settings) {
            progress.check()?;
            let source = buffer.rope.to_string();
            buffer.highlights = crate::syntax::highlight_cancellable(language, &source, Some(&progress.parser_cancel))
                .map_err(|e| Error::Failed(e.to_string()))?;
        }
    }
    // The original handle must still identify the same length/mtime after every parser finishes.
    // An external writer preserving both timestamp and length cannot be detected by metadata.
    #[cfg(test)] BEFORE_FINAL_CHECK.with(|hook| { if let Some(hook) = hook.borrow_mut().take() { hook(); } });
    unchanged(&file, &initial, initial.len())?;
    let current_path = std::fs::metadata(&request.path)?;
    if !current_path.is_file() || current_path.len() != initial.len()
        || current_path.modified().ok() != initial.modified().ok() {
        return Err(Error::Failed("File changed while reloading; reload again".into()));
    }
    progress.check()?; buffer.mark_saved(); Ok(buffer)
}

#[cfg(test)]
thread_local! {
    static TEST_WARNING: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static BEFORE_FINAL_CHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
fn unchanged(file: &std::fs::File, before: &std::fs::Metadata, count: u64) -> Result<(), Error> {
    let after = file.metadata()?;
    if count != before.len() || before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(Error::Failed("File changed while reloading; reload again".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reload_worker_chunk_reads_never_grow_the_reserved_buffer() {
        let fixture = crate::persist::TempDir::new("reload-chunk-bound"); let path = fixture.join("neutral.txt");
        let size = crate::io_stream::STREAM_CHUNK_SIZE;
        std::fs::write(&path,vec![b'x';size+7]).expect("fixture");
        let mut file = std::fs::File::open(path).expect("open"); let progress = Progress::default();
        for length in [size,7,0] {
            let bytes = chunk(&mut file,&progress).ok().expect("chunk");
            assert_eq!(bytes.len(),length); assert_eq!(bytes.capacity(),size);
            assert!(bytes.iter().all(|byte| *byte == b'x'));
        }
    }
    #[test]
    fn reload_worker_fidelity_matches_whole_file_decoding() {
        let fixture = crate::persist::TempDir::new("reload-fidelity");
        for encoding in [crate::text::encoding::Encoding::Utf8,crate::text::encoding::Encoding::Utf8Bom,
            crate::text::encoding::Encoding::Utf16Le,crate::text::encoding::Encoding::Utf16Be,crate::text::encoding::Encoding::Latin1] {
            for text in ["café\r\nsecond\r\n","café\nsecond\n","café\rsecond\r","café\r\nsecond\nthird\r"] {
                let path = fixture.join("neutral.txt");
                let bytes = crate::text::encoding::encode_checked(text,encoding).expect("fixture");
                std::fs::write(&path,&bytes).expect("write");
                let buffer = read(Request { path,settings:Settings::default(),progress:Arc::new(Progress::default()),allow_huge:false }).ok().expect("read");
                let saved = crate::text::encoding::encode_checked(&buffer.rope.to_string().replace("\n",buffer.line_ending.as_str()),buffer.encoding);
                // Mixed endings remain literal and are emitted with LF mode.
                assert_eq!(saved.expect("encode"),bytes);
            }
        }
    }
    #[test]
    fn reload_worker_cancel_and_memory_limits_refuse_without_io() {
        let progress = Arc::new(Progress::default()); progress.stop();
        assert!(matches!(read(Request {path:PathBuf::from("unused"),settings:Settings::default(),progress,allow_huge:false}),Err(Error::Cancelled)));
        assert!(matches!(limits(crate::io_stream::HUGE_FILE_WARNING_BYTES,false),Err(Error::Huge(_))));
        assert!(limits(u64::MAX,true).is_err());
    }
    #[test]
    fn reload_worker_late_encoding_and_eol_restarts_preserve_bytes() {
        let fixture = crate::persist::TempDir::new("reload-late-fidelity"); let path = fixture.join("neutral.txt");
        let mut bytes = vec![b'a';crate::io_stream::STREAM_CHUNK_SIZE-1]; bytes.extend_from_slice(b"\r\nlate\n"); bytes.push(0xe9);
        std::fs::write(&path,&bytes).expect("fixture");
        let buffer = read(Request {path,settings:Settings::default(),progress:Arc::new(Progress::default()),allow_huge:false}).ok().expect("read");
        let output = crate::text::encoding::encode_checked(&buffer.rope.to_string(),buffer.encoding).expect("encode");
        assert_eq!(output,bytes); assert_eq!(buffer.line_ending,crate::text::line_ending::LineEnding::Lf);
    }
    #[test]
    fn reload_worker_rich_snapshot_preserves_rtf_payload_and_original_path() {
        let fixture = crate::persist::TempDir::new("reload-rich-snapshot"); let path = fixture.join("neutral.rtf");
        let bytes = br"{\rtf1\ansi original}"; std::fs::write(&path,bytes).expect("fixture");
        let buffer = read(Request {path:path.clone(),settings:Settings::default(),progress:Arc::new(Progress::default()),allow_huge:false}).ok().expect("read");
        assert_eq!(buffer.file_path,Some(path)); let payload = buffer.rich_doc.expect("payload");
        assert_eq!(payload.original_bytes.as_slice(),bytes); assert_eq!(payload.block_text(0),"original");
    }
    #[test]
    fn reload_worker_decoded_expansion_requires_confirmation_before_rope_append() {
        struct Reset; impl Drop for Reset {fn drop(&mut self) {TEST_WARNING.with(|limit| limit.set(None));}}
        let _reset = Reset; TEST_WARNING.with(|limit| limit.set(Some(1536*1024)));
        let fixture = crate::persist::TempDir::new("reload-decoded-growth"); let path = fixture.join("neutral.txt");
        std::fs::write(&path,vec![0xe9;1024*1024]).expect("fixture");
        let result = read(Request {path,settings:Settings::default(),progress:Arc::new(Progress::default()),allow_huge:false});
        assert!(matches!(result,Err(Error::Huge(size)) if size == 2*1024*1024));
    }
    #[test]
    fn reload_worker_checks_original_handle_after_rich_and_syntax_parsing() {
        for (name,bytes) in [("neutral.rs",b"fn main() {}".as_slice()),("neutral.rtf",br"{\rtf1 original}".as_slice())] {
            let fixture = crate::persist::TempDir::new("reload-final-identity"); let path = fixture.join(name);
            std::fs::write(&path,bytes).expect("fixture"); let changed = path.clone();
            BEFORE_FINAL_CHECK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {std::fs::write(changed,b"different length").expect("external write");})));
            let result = read(Request {path,settings:Settings::default(),progress:Arc::new(Progress::default()),allow_huge:false});
            assert!(matches!(result,Err(Error::Failed(error)) if error.contains("File changed")));
        }
    }
}

#[cfg(test)]
mod path_replacement_tests {
    use super::*;
    #[test]
    fn reload_worker_refuses_path_replacement_after_parsing() {
        let fixture = crate::persist::TempDir::new("reload-path-replacement"); let path = fixture.join("neutral.rs");
        std::fs::write(&path,b"fn original() {}").expect("fixture");
        let changed = path.clone(); let replacement = fixture.join("replacement.rs");
        std::fs::write(&replacement,b"fn new_disk_contents() {}").expect("replacement");
        BEFORE_FINAL_CHECK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
            std::fs::rename(replacement,changed).expect("atomic path replacement");
        })));
        let result = read(Request {path,settings:Settings::default(),progress:Arc::new(Progress::default()),allow_huge:false});
        assert!(matches!(result,Err(Error::Failed(error)) if error.contains("File changed")));
    }
}
