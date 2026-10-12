// Streaming file loader for large files.
//
// `open_path` in `src/io.rs` reads the whole file with `std::fs::read` and
// then constructs a rope in one shot. Fine for typical source files;
// catastrophic for a 200 MB log. This module takes over once a file's size
// exceeds `STREAM_THRESHOLD_BYTES`: the read happens in 1 MB chunks on the
// background executor, each chunk is decoded incrementally by
// `StreamDecoder`, and the result is appended to the tab's rope on the main
// thread between chunks. The status bar shows progress while the load is in
// flight, and a Cancel button on the same chip aborts mid-read.
//
// `StreamDecoder` gives the same encoding, line ending and rope text as the
// whole-file path (`encoding::decode_bytes` + `split_line_endings`) for any
// split of the input, so a streamed file also saves back byte-identical.
// Two facts only show up part-way through a file: invalid UTF-8 (the file is
// Latin-1 after all) and a line ending that breaks the mode picked from the
// first one. Either restarts the read from byte 0 in the corrected mode; each
// can happen at most once.
//
// Tree-sitter highlighting is skipped while loading and only kicks in at
// completion if the file is below `HIGHLIGHT_SIZE_LIMIT` — running the
// highlighter against a 500 MB string takes seconds and produces a span list
// that no human ever needs to see.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use encoding_rs::{CoderResult, DecoderResult, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};
use gpui::Context;

use crate::app::menus::rebuild_menus;
use crate::buffer::{Buffer, LoadProgress};
use crate::consts::APP_NAME;
use crate::recent;
use crate::tab::Tab;
use crate::text::encoding::Encoding;
use crate::text::line_ending::LineEnding;
use crate::workspace::Workspace;

/// Files at or below this size go through the existing read-all-at-once
/// path in `src/io.rs`. Anything larger streams.
pub(crate) const STREAM_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;

/// Per-iteration read size. Big enough that the per-chunk overhead
/// (bg-executor hop + main-thread update) doesn't dominate, small enough
/// that progress updates feel live.
pub(crate) const STREAM_CHUNK_SIZE: usize = 1024 * 1024;

/// Files larger than this skip tree-sitter highlighting at end-of-load. The
/// editor already painted half a million spans by the time the highlighter
/// returned; not worth the time.
pub(crate) const HIGHLIGHT_SIZE_LIMIT: u64 = 50 * 1024 * 1024;

/// Files larger than this surface a Yes/No warning before the load even
/// starts. NPP equivalent dialog. Cancel is the default button.
pub(crate) const HUGE_FILE_WARNING_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Largest file a rope can hold: Rust caps every allocation at `isize::MAX`
/// bytes. Only reachable on 32-bit builds, where it is 2 GiB - 1 and a
/// bigger file would abort the process on allocation failure.
const MAX_IN_MEMORY_BYTES: u64 = isize::MAX as u64;

/// Why a streaming pass has to start over from byte 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Restart {
    /// The file isn't valid UTF-8: read it as Latin-1, BOM bytes included,
    /// as `encoding::decode_bytes` does.
    Latin1,
    /// A later line break doesn't fit the mode taken from the first one,
    /// so normalizing would not round-trip: keep line endings raw.
    RawEol,
}

/// How a streaming pass decodes the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamMode {
    pub(crate) encoding: Encoding,
    /// BOM bytes to skip at the start of the file.
    bom_len: usize,
    /// Keep line endings exactly as they are (the buffer opens as LF).
    raw_eol: bool,
}

impl StreamMode {
    /// Pick the first pass's mode from the file's leading bytes. `prefix`
    /// must be the first chunk read in full, so a BOM is never cut short.
    pub(crate) fn sniff(prefix: &[u8]) -> Self {
        let (encoding, bom_len) = if prefix.starts_with(&[0xEF, 0xBB, 0xBF]) {
            (Encoding::Utf8Bom, 3)
        } else if prefix.starts_with(&[0xFF, 0xFE]) {
            (Encoding::Utf16Le, 2)
        } else if prefix.starts_with(&[0xFE, 0xFF]) {
            (Encoding::Utf16Be, 2)
        } else {
            (Encoding::Utf8, 0)
        };
        Self {
            encoding,
            bom_len,
            raw_eol: false,
        }
    }

    pub(crate) fn after(self, restart: Restart) -> Self {
        match restart {
            Restart::Latin1 => Self {
                encoding: Encoding::Latin1,
                bom_len: 0,
                raw_eol: self.raw_eol,
            },
            Restart::RawEol => Self {
                raw_eol: true,
                ..self
            },
        }
    }
}

/// The streaming twin of `LineEnding::detect` + `normalize_to_lf`, fed the
/// decoded text a piece at a time.
///
/// The mode comes from the first line break: LF keeps the text as-is, CR LF
/// collapses pairs to LF, a lone CR becomes LF. A CR LF file must not hold
/// a lone LF or CR CR LF, and a CR file must not hold any LF; meeting one
/// returns `Restart::RawEol`. CR and LF are ASCII, so working on the bytes
/// of a `str` never splits a character.
struct EolNormalizer {
    mode: Option<LineEnding>,
    /// The input so far ended in a CR that hasn't been written out yet,
    /// because whether it is half of a CR LF depends on the next piece.
    held_cr: bool,
    /// Last two input bytes seen, for checking CR CR LF across pieces.
    tail: [u8; 2],
}

impl EolNormalizer {
    fn new(raw: bool) -> Self {
        Self {
            mode: raw.then_some(LineEnding::Lf),
            held_cr: false,
            tail: [0; 2],
        }
    }

    fn line_ending(&self) -> LineEnding {
        self.mode.unwrap_or_else(LineEnding::platform_default)
    }

    fn process(&mut self, text: String) -> Result<String, Restart> {
        if text.is_empty() {
            return Ok(text);
        }
        let new_tail = match text.as_bytes() {
            [] => self.tail,
            [x] => [self.tail[1], *x],
            [.., x, y] => [*x, *y],
        };
        if self.mode.is_none() {
            self.mode = self.decide(text.as_bytes());
        }
        let out = match self.mode {
            // No line break settled yet: either `text` has none, or its
            // last byte is the first one, a CR whose partner may follow.
            None => {
                let mut text = text;
                if text.ends_with('\r') {
                    text.pop();
                    self.held_cr = true;
                }
                text
            }
            Some(LineEnding::Lf) => text,
            Some(LineEnding::Cr) => {
                if text.contains('\n') {
                    return Err(Restart::RawEol);
                }
                let mut out = String::with_capacity(text.len().saturating_add(1));
                if std::mem::take(&mut self.held_cr) {
                    out.push('\n');
                }
                out.push_str(&text.replace('\r', "\n"));
                out
            }
            Some(LineEnding::Crlf) => self.crlf(&text)?,
        };
        self.tail = new_tail;
        Ok(out)
    }

    fn decide(&self, b: &[u8]) -> Option<LineEnding> {
        if self.held_cr {
            return match b.first() {
                None => None,
                Some(b'\n') => Some(LineEnding::Crlf),
                Some(_) => Some(LineEnding::Cr),
            };
        }
        let i = b.iter().position(|&c| c == b'\r' || c == b'\n')?;
        if b.get(i) == Some(&b'\n') {
            return Some(LineEnding::Lf);
        }
        match b.get(i.saturating_add(1)) {
            Some(b'\n') => Some(LineEnding::Crlf),
            Some(_) => Some(LineEnding::Cr),
            None => None,
        }
    }

    fn crlf(&mut self, text: &str) -> Result<String, Restart> {
        let b = text.as_bytes();
        // Byte `j` of the virtual stream `tail ++ b`.
        let at = |j: usize| -> u8 {
            match j.checked_sub(2) {
                None => self.tail.get(j).copied().unwrap_or(0),
                Some(k) => b.get(k).copied().unwrap_or(0),
            }
        };
        for (i, _) in text.match_indices('\n') {
            let v = i.saturating_add(2);
            if at(v - 1) != b'\r' || at(v - 2) == b'\r' {
                return Err(Restart::RawEol);
            }
        }
        let mut out = String::with_capacity(text.len().saturating_add(1));
        let mut rest = text;
        if std::mem::take(&mut self.held_cr) {
            if let Some(after) = text.strip_prefix('\n') {
                out.push('\n');
                rest = after;
            } else {
                out.push('\r');
            }
        }
        let mut seg = 0usize;
        for (i, _) in rest.match_indices("\r\n") {
            out.push_str(rest.get(seg..i).unwrap_or_default());
            out.push('\n');
            seg = i.saturating_add(2);
        }
        let remainder = rest.get(seg..).unwrap_or_default();
        match remainder.strip_suffix('\r') {
            Some(head) => {
                out.push_str(head);
                self.held_cr = true;
            }
            None => out.push_str(remainder),
        }
        Ok(out)
    }

    /// End of input: write out a CR still being held.
    fn finish(&mut self) -> String {
        if !std::mem::take(&mut self.held_cr) {
            return String::new();
        }
        match self.mode {
            // The file's only line break is a CR at the very end.
            None => {
                self.mode = Some(LineEnding::Cr);
                "\n".to_string()
            }
            Some(LineEnding::Cr) => "\n".to_string(),
            Some(LineEnding::Crlf | LineEnding::Lf) => "\r".to_string(),
        }
    }
}

/// Bytes in, rope text out, one chunk at a time.
pub(crate) struct StreamDecoder {
    mode: StreamMode,
    decoder: encoding_rs::Decoder,
    eol: EolNormalizer,
    /// BOM bytes still to drop.
    skip: usize,
    /// Malformed UTF-16 was replaced with U+FFFD (see `Decoded::lossy`).
    pub(crate) lossy: bool,
}

impl StreamDecoder {
    pub(crate) fn new(mode: StreamMode) -> Self {
        let decoder = match mode.encoding {
            Encoding::Utf8 | Encoding::Utf8Bom => UTF_8.new_decoder_without_bom_handling(),
            Encoding::Utf16Le => UTF_16LE.new_decoder_without_bom_handling(),
            Encoding::Utf16Be => UTF_16BE.new_decoder_without_bom_handling(),
            Encoding::Latin1 => WINDOWS_1252.new_decoder_without_bom_handling(),
        };
        Self {
            mode,
            decoder,
            eol: EolNormalizer::new(mode.raw_eol),
            skip: mode.bom_len,
            lossy: false,
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<String, Restart> {
        let text = self.decode(bytes, false)?;
        self.eol.process(text)
    }

    pub(crate) fn finish(&mut self) -> Result<String, Restart> {
        let text = self.decode(&[], true)?;
        let mut out = self.eol.process(text)?;
        out.push_str(&self.eol.finish());
        Ok(out)
    }

    pub(crate) fn line_ending(&self) -> LineEnding {
        self.eol.line_ending()
    }

    fn decode(&mut self, bytes: &[u8], last: bool) -> Result<String, Restart> {
        let mut bytes = bytes;
        if self.skip > 0 {
            let n = self.skip.min(bytes.len());
            bytes = bytes.get(n..).unwrap_or_default();
            self.skip -= n;
        }
        // No BOM or a UTF-8 BOM: the file must be valid UTF-8 all the way
        // through, or it is Latin-1. Replacing bad bytes with U+FFFD would
        // rewrite them on the next save.
        let strict = matches!(self.mode.encoding, Encoding::Utf8 | Encoding::Utf8Bom);
        let mut out = String::new();
        loop {
            // `decode_to_string*` only writes into spare capacity and stops
            // with `OutputFull` when it runs out, so reserve the worst case.
            let need = if strict {
                self.decoder
                    .max_utf8_buffer_length_without_replacement(bytes.len())
            } else {
                self.decoder.max_utf8_buffer_length(bytes.len())
            };
            out.reserve(need.unwrap_or_else(|| bytes.len().saturating_mul(3)).max(4));
            let (full, read) = if strict {
                let (result, read) =
                    self.decoder
                        .decode_to_string_without_replacement(bytes, &mut out, last);
                match result {
                    DecoderResult::InputEmpty => (false, read),
                    DecoderResult::OutputFull => (true, read),
                    DecoderResult::Malformed(..) => return Err(Restart::Latin1),
                }
            } else {
                let (result, read, replaced) = self.decoder.decode_to_string(bytes, &mut out, last);
                self.lossy |= replaced;
                (result == CoderResult::OutputFull, read)
            };
            if !full {
                return Ok(out);
            }
            bytes = bytes.get(read..).unwrap_or_default();
        }
    }
}

/// Read up to `STREAM_CHUNK_SIZE` bytes, fewer only at end of file. Empty
/// means end of file.
fn read_chunk(file: &mut std::fs::File) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(STREAM_CHUNK_SIZE);
    file.by_ref()
        .take(STREAM_CHUNK_SIZE as u64)
        .read_to_end(&mut buf)?;
    Ok(buf)
}

/// Format a byte count with a single fractional digit. `0` → "0 B", `1500`
/// → "1.5 KB", and so on through TB. Used for the status-bar progress chip.
pub(crate) fn format_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut idx = 0usize;
    while value >= 1024.0 && idx + 1 < UNITS.len() {
        value /= 1024.0;
        idx += 1;
    }
    match UNITS.get(idx) {
        Some(unit) if idx > 0 => format!("{:.1} {}", value, unit),
        _ => format!("{} B", n),
    }
}

/// Public formatter used by the status bar to render a `LoadProgress`
/// snapshot. Lives here so the format string only has one canonical home.
pub(crate) fn format_progress(p: &LoadProgress) -> String {
    let pct = if p.total_bytes > 0 {
        ((p.bytes_read as f64 / p.total_bytes as f64) * 100.0).min(100.0)
    } else {
        0.0
    };
    format!(
        "Loading {} / {} ({:.1}%)",
        format_bytes(p.bytes_read),
        format_bytes(p.total_bytes),
        pct
    )
}

/// Entry point. Spawns the streaming task and returns immediately. The
/// active or new tab is configured up front (path, encoding, empty rope,
/// `LoadProgress::Some`) so the user sees the loading UI on the very next
/// frame even before the first chunk arrives.
///
/// Caller is `Workspace::open_path` in `src/io.rs`, which has already
/// confirmed `total_bytes > STREAM_THRESHOLD_BYTES`.
pub(crate) fn open_path_streaming(
    workspace: &mut Workspace,
    path: PathBuf,
    total_bytes: u64,
    cx: &mut Context<Workspace>,
) {
    let display = path.display().to_string();

    if total_bytes > MAX_IN_MEMORY_BYTES {
        workspace.set_status(format!(
            "Can't open {display}: at {} it is larger than this build of {APP_NAME} can hold in memory",
            format_bytes(total_bytes)
        ));
        return;
    }

    // Files past the warning threshold pop a Yes/No dialog before any I/O.
    // Cancel is the default — accidental opens of multi-GB log files
    // shouldn't punish the user with a half-minute load.
    if total_bytes >= HUGE_FILE_WARNING_BYTES {
        let total_human = format_bytes(total_bytes);
        let path_clone = path.clone();
        cx.spawn(async move |this, cx| {
            let result = rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title(APP_NAME)
                .set_description(format!(
                    "{display} is {total_human}. Open anyway?"
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                .await;
            if !matches!(result, rfd::MessageDialogResult::Yes) {
                return;
            }
            this.update(cx, |w, cx| {
                start_streaming(w, path_clone, total_bytes, cx);
            })
            .ok();
        })
        .detach();
        return;
    }

    start_streaming(workspace, path, total_bytes, cx);
}

/// Index of the tab this load feeds, found by its cancel flag rather than
/// a remembered index: closing or moving another tab mid-load shifts the
/// indices, and a stale index would pour the file into the wrong tab.
fn loading_tab(workspace: &Workspace, cancel: &Arc<AtomicBool>) -> Option<usize> {
    if workspace.window_closed { return None; }
    workspace.tabs.iter().position(|t| {
        t.buffer
            .loading
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(&p.cancel, cancel))
    })
}

fn start_streaming(
    workspace: &mut Workspace,
    path: PathBuf,
    total_bytes: u64,
    cx: &mut Context<Workspace>,
) {
    if workspace.window_closed { return; }
    let cancel = Arc::new(AtomicBool::new(false));

    // Seed the tab with an empty buffer carrying a `LoadProgress`. Routes
    // into the active pristine empty tab if available, otherwise opens a
    // new tab. Mirrors `open_path` so a casual user sees the same one-tab
    // behaviour for both small and large files.
    let empty_buffer = Buffer {
        rope: ropey::Rope::new(),
        file_path: Some(path.clone()),
        dirty: false,
        line_ending: LineEnding::platform_default(),
        encoding: Encoding::Utf8,
        bookmarks: std::collections::BTreeSet::new(),
        marks: Vec::new(),
        // Skip tree-sitter while streaming. Re-enabled (per file size) at
        // end-of-load.
        language: None,
        highlights: std::sync::Arc::new(Vec::new()),
        loading: Some(LoadProgress {
            bytes_read: 0,
            total_bytes,
            cancel: cancel.clone(),
        }),
        new_label: None,
        new_label_is_custom: false,
        // No edits until the whole file is in: a restart replaces the rope,
        // and text typed into a half-read file would be lost or misplaced.
        read_only: true,
        tab_color: None,
        is_changelog: false,
        rich_doc: None,
        flat_text_cache: None,
        raw_revision: crate::buffer::ContentRevision::default(),
        savepoint: None,
    };

    let pristine = active_is_pristine_empty(workspace);
    match workspace.tabs.get_mut(workspace.active).filter(|_| pristine) {
        Some(tab) => crate::io::reset_tab_with(tab, empty_buffer),
        None => {
            let mut tab = Tab::empty();
            tab.buffer = empty_buffer;
            workspace.tabs.push(tab);
            workspace.active = workspace.tabs.len().saturating_sub(1);
        }
    }

    workspace.set_status(format!("Loading {}…", path.display()));
    cx.notify();

    cx.spawn(async move |this, cx| {
        let executor = cx.background_executor().clone();

        let path_open = path.clone();
        let opened = executor
            .spawn(async move { std::fs::File::open(&path_open) })
            .await;
        let mut file = match opened {
            Ok(f) => f,
            Err(e) => {
                let msg = format!("Failed to open {}: {}", path.display(), e);
                this.update(cx, |w, cx| {
                    if let Some(idx) = loading_tab(w, &cancel) {
                        abort_load(w, idx, msg);
                    }
                    cx.notify();
                })
                .ok();
                return;
            }
        };

        let mut mode: Option<StreamMode> = None;
        let mut restarts = 0u8;
        let (decoder, encoding) = 'pass: loop {
            // Start (or restart) from byte 0.
            let read = executor
                .spawn(async move {
                    let r = file.seek(SeekFrom::Start(0)).and_then(|_| read_chunk(&mut file));
                    (file, r)
                })
                .await;
            file = read.0;
            let mut chunk = match read.1 {
                Ok(c) => c,
                Err(e) => {
                    fail_load(&this, cx, &cancel, format!("Failed to read {}: {e}", path.display()));
                    return;
                }
            };
            let pass_mode = *mode.get_or_insert_with(|| StreamMode::sniff(&chunk));
            let mut decoder = StreamDecoder::new(pass_mode);
            let mut bytes_read = 0u64;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    fail_load(&this, cx, &cancel, format!("Cancelled loading {}", path.display()));
                    return;
                }
                let at_eof = chunk.is_empty();
                bytes_read = bytes_read.saturating_add(chunk.len() as u64);
                let fed = if at_eof {
                    decoder.finish()
                } else {
                    decoder.feed(&chunk)
                };
                let text = match fed {
                    Ok(t) => t,
                    Err(why) => {
                        restarts = restarts.saturating_add(1);
                        if restarts > 2 {
                            fail_load(
                                &this,
                                cx,
                                &cancel,
                                format!("Failed to read {}: decoding did not settle", path.display()),
                            );
                            return;
                        }
                        let next = pass_mode.after(why);
                        mode = Some(next);
                        let reason = match why {
                            Restart::Latin1 => "it isn't valid UTF-8; reading it as Latin-1",
                            Restart::RawEol => "it mixes line endings; keeping them as they are",
                        };
                        let status = format!("Loading {}: {reason}…", path.display());
                        let alive = this
                            .update(cx, |w, cx| {
                                let Some(idx) = loading_tab(w, &cancel) else {
                                    return false;
                                };
                                let Some(tab) = w.tabs.get_mut(idx) else {
                                    return false;
                                };
                                tab.buffer.rope = ropey::Rope::new();
                                tab.buffer.invalidate_flat_text();
                                tab.wrap_cache = None;
                                tab.buffer.encoding = next.encoding;
                                if let Some(p) = tab.buffer.loading.as_mut() {
                                    p.bytes_read = 0;
                                }
                                w.set_status(status);
                                cx.notify();
                                true
                            })
                            .unwrap_or(false);
                        if !alive {
                            return;
                        }
                        continue 'pass;
                    }
                };

                let alive = this
                    .update(cx, |w, cx| {
                        let Some(idx) = loading_tab(w, &cancel) else {
                            return false;
                        };
                        let Some(tab) = w.tabs.get_mut(idx) else {
                            return false;
                        };
                        tab.buffer.encoding = pass_mode.encoding;
                        if !text.is_empty() {
                            let len = tab.buffer.rope.len_chars();
                            tab.buffer.rope.insert(len, &text);
                            tab.buffer.invalidate_flat_text();
                            tab.wrap_cache = None;
                        }
                        if let Some(p) = tab.buffer.loading.as_mut() {
                            p.bytes_read = bytes_read;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    return;
                }
                if at_eof {
                    break 'pass (decoder, pass_mode.encoding);
                }

                let read = executor
                    .spawn(async move {
                        let r = read_chunk(&mut file);
                        (file, r)
                    })
                    .await;
                file = read.0;
                chunk = match read.1 {
                    Ok(c) => c,
                    Err(e) => {
                        // Never keep a partial read bound to the path: a
                        // save would cut the file short.
                        fail_load(&this, cx, &cancel, format!("Failed to read {}: {e}", path.display()));
                        return;
                    }
                };
            }
        };
        drop(file);

        let line_ending = decoder.line_ending();
        let lossy = decoder.lossy;
        this.update(cx, |w, cx| {
            finish_streaming(w, StreamCompletion {
                path, total_bytes, cancel, line_ending, encoding, lossy,
            }, cx);
        })
        .ok();
    })
    .detach();
}

// The actual end-of-load UI callback. Identity comes from the original load flag.
pub(crate) struct StreamCompletion {
    pub(crate) path: PathBuf,
    pub(crate) total_bytes: u64,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) line_ending: LineEnding,
    pub(crate) encoding: Encoding,
    pub(crate) lossy: bool,
}

pub(crate) fn finish_streaming(w: &mut Workspace, completion: StreamCompletion, cx: &mut Context<Workspace>) {
    let StreamCompletion { path, total_bytes, cancel, line_ending, encoding, lossy } = completion;
    let Some(idx) = loading_tab(w, &cancel) else {
        return;
    };
    let Some(tab) = w.tabs.get_mut(idx) else {
        return;
    };
    tab.buffer.invalidate_flat_text();
    tab.wrap_cache = None;
    tab.buffer.line_ending = line_ending;
    tab.buffer.encoding = encoding;
    tab.buffer.loading = None;
    tab.buffer.mark_saved();
    // Malformed UTF-16 can't be saved back as it was read; open
    // read-only, as the whole-file path does.
    tab.buffer.read_only = lossy;

    // Keep inference for eligible streamed files; parsing uses the bounded
    // background queue after the load metadata and saved state settle.
    if total_bytes <= HIGHLIGHT_SIZE_LIMIT {
        let inferred = crate::syntax::language_for_path(&path);
        tab.buffer.language = inferred;
    }
    let id = tab.session_id;

    let basename = tab.buffer.display_short_name().to_string();
    if lossy {
        w.set_status(crate::io::lossy_decode_status(&basename, encoding));
    } else {
        w.set_status(format!("Opened {}", basename));
    }
    w.request_syntax_refresh_for_tab(id, std::time::Duration::ZERO, cx);
    w.last_lines.clear();
    w.caret_visible = true;
    w.save_session(cx);
    recent::record_opened(path.clone());
    rebuild_menus(&mut *cx);
    cx.notify();
}

/// End a load that can't finish (cancelled, or a read failed part-way).
fn fail_load(
    this: &gpui::WeakEntity<Workspace>,
    cx: &mut gpui::AsyncApp,
    cancel: &Arc<AtomicBool>,
    status: String,
) {
    this.update(cx, |w, cx| {
        if let Some(idx) = loading_tab(w, cancel) {
            abort_load(w, idx, status);
        }
        cx.notify();
    })
    .ok();
}

/// Mirror of `Workspace::active_is_pristine_empty` — duplicated here because
/// the original is a private helper inside `io.rs`. Kept tiny on purpose.
fn active_is_pristine_empty(workspace: &Workspace) -> bool {
    let Some(tab) = workspace.tabs.get(workspace.active) else {
        return false;
    };
    tab.buffer.rope.len_bytes() == 0 && tab.buffer.file_path.is_none() && !tab.buffer.dirty
}

/// Roll a streaming load back to a clean error state. The tab keeps
/// existing (it may have been newly opened just for this load), but its
/// buffer is reset to an empty untitled one, so nothing partial stays bound
/// to the file's path where a save could cut the file short.
fn abort_load(workspace: &mut Workspace, idx: usize, status: String) {
    let label = crate::buffer::next_new_label_for(
        workspace
            .tabs
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, t)| t.buffer.new_label.as_ref()),
    );
    if let Some(tab) = workspace.tabs.get_mut(idx) {
        tab.buffer.loading = None;
        tab.buffer.rope = ropey::Rope::new();
        tab.buffer.invalidate_flat_text();
        tab.wrap_cache = None;
        tab.buffer.file_path = None;
        tab.buffer.new_label = Some(label);
        tab.buffer.new_label_is_custom = false;
        tab.buffer.read_only = false;
        tab.buffer.mark_saved();
    }
    workspace.set_status(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::encoding;
    use crate::text::line_ending::{rope_to_bytes, split_line_endings};

    struct Streamed {
        encoding: Encoding,
        line_ending: LineEnding,
        text: String,
        lossy: bool,
    }

    /// Run the streaming decoder over `raw` in `chunk`-byte pieces, with
    /// the driver's restart loop.
    fn stream_decode_all(raw: &[u8], chunk: usize) -> Streamed {
        let first = raw.get(..STREAM_CHUNK_SIZE.min(raw.len())).unwrap_or_default();
        let mut mode = StreamMode::sniff(first);
        for _ in 0..3 {
            let mut d = StreamDecoder::new(mode);
            let mut text = String::new();
            let mut restart = None;
            for c in raw.chunks(chunk.max(1)) {
                match d.feed(c) {
                    Ok(t) => text.push_str(&t),
                    Err(r) => {
                        restart = Some(r);
                        break;
                    }
                }
            }
            if restart.is_none() {
                match d.finish() {
                    Ok(t) => text.push_str(&t),
                    Err(r) => restart = Some(r),
                }
            }
            match restart {
                Some(r) => mode = mode.after(r),
                None => {
                    return Streamed {
                        encoding: mode.encoding,
                        line_ending: d.line_ending(),
                        text,
                        lossy: d.lossy,
                    };
                }
            }
        }
        panic!("decoding did not settle for {raw:02X?}");
    }

    /// What saving the streamed buffer without edits writes.
    fn saved_bytes(s: &Streamed) -> Vec<u8> {
        let rope = ropey::Rope::from_str(&s.text);
        let utf8 = rope_to_bytes(&rope, s.line_ending);
        let utf8 = String::from_utf8(utf8).unwrap_or_default();
        encoding::encode_checked(&utf8, s.encoding).unwrap_or_default()
    }

    fn cases() -> Vec<Vec<u8>> {
        let bom8: &[u8] = &[0xEF, 0xBB, 0xBF];
        vec![
            b"".to_vec(),
            b"plain".to_vec(),
            b"a\nb\n".to_vec(),
            b"a\r\nb\r\n".to_vec(),
            b"a\rb\r".to_vec(),
            b"a\r\nb\nc\r\n".to_vec(),
            b"a\rb\nc\r\nd".to_vec(),
            b"a\nb\r\nc\rd".to_vec(),
            b"a\r\nb\rc\r\n".to_vec(),
            b"a\r\r\nb\r\r\n".to_vec(),
            b"\r".to_vec(),
            b"\r\n".to_vec(),
            b"x\r".to_vec(),
            "é\r\n中\n😀\r".as_bytes().to_vec(),
            b"caf\xE9\r\nna\xEFve\r\n".to_vec(),
            [bom8, "héllo\r\nwörld".as_bytes()].concat(),
            [bom8, b"caf\xE9 ok\n"].concat(),
            encoding::encode("a😀b\r\nc\nd\r", Encoding::Utf16Le),
            encoding::encode("a😀b\r\nc\r\n", Encoding::Utf16Be),
            (0u8..=255).collect(),
        ]
    }

    #[test]
    fn streaming_matches_the_whole_file_path_at_every_split() {
        for raw in cases() {
            let whole = encoding::decode_bytes(&raw);
            let (eol, text) = split_line_endings(whole.text);
            for chunk in 1..=raw.len().max(1) {
                let s = stream_decode_all(&raw, chunk);
                assert_eq!(s.encoding, whole.encoding, "{raw:02X?} split {chunk}");
                assert_eq!(s.line_ending, eol, "{raw:02X?} split {chunk}");
                assert_eq!(s.text, text, "{raw:02X?} split {chunk}");
                assert!(!s.lossy, "{raw:02X?} split {chunk}");
                assert_eq!(saved_bytes(&s), raw, "{raw:02X?} split {chunk}");
            }
        }
    }

    #[test]
    fn a_chunk_of_high_latin1_bytes_is_not_truncated() {
        // Regression: each output String had capacity `2 * chunk`, but a
        // byte the decoder replaces (or maps to U+20AC..) needs three
        // UTF-8 bytes, and `decode_to_string` stops at the capacity and
        // drops the rest of the chunk.
        let raw = vec![0x80u8; STREAM_CHUNK_SIZE];
        let s = stream_decode_all(&raw, STREAM_CHUNK_SIZE);
        assert_eq!(s.text.chars().count(), STREAM_CHUNK_SIZE);
        assert_eq!(saved_bytes(&s), raw);
    }

    #[test]
    fn malformed_utf16_is_flagged_at_every_split() {
        let raw = [&[0xFF, 0xFE][..], b"A\0", &[0x3D, 0xD8], b"B\0C"].concat();
        let whole = encoding::decode_bytes(&raw);
        assert!(whole.lossy);
        for chunk in 1..=raw.len() {
            let s = stream_decode_all(&raw, chunk);
            assert!(s.lossy, "split {chunk}");
            assert_eq!(s.text, whole.text, "split {chunk}");
        }
    }

    #[test]
    fn late_problems_restart_once_each() {
        // A CRLF file that turns out to hold a lone LF and a Latin-1 byte
        // past the first chunk: both corrections land and the file still
        // round-trips.
        let mut raw = b"line\r\n".repeat(STREAM_CHUNK_SIZE / 6 + 10);
        raw.extend_from_slice(b"lone lf\ncaf\xE9\r\n");
        let s = stream_decode_all(&raw, STREAM_CHUNK_SIZE);
        assert_eq!(s.encoding, Encoding::Latin1);
        assert_eq!(s.line_ending, LineEnding::Lf);
        assert_eq!(saved_bytes(&s), raw);
    }

    #[test]
    fn read_chunk_fills_whole_chunks() {
        let dir = crate::persist::TempDir::new("stream-chunk");
        let p = dir.join("f.bin");
        let len = STREAM_CHUNK_SIZE * 2 + 7;
        let _ = std::fs::write(&p, vec![b'x'; len]);
        let Ok(mut f) = std::fs::File::open(&p) else {
            panic!("open");
        };
        let sizes: Vec<usize> = std::iter::from_fn(|| match read_chunk(&mut f) {
            Ok(c) if !c.is_empty() => Some(c.len()),
            _ => None,
        })
        .collect();
        assert_eq!(sizes, vec![STREAM_CHUNK_SIZE, STREAM_CHUNK_SIZE, 7]);
    }

    /// Pathological files, opened through both load paths and saved back.
    /// Every file is above `HIGHLIGHT_SIZE_LIMIT`, so this is ignored by
    /// default. Run it in release and read the timings:
    /// `cargo test --release -p notepad-studio --bin notepad-studio big_files -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn big_files_open_and_save_in_release() {
        use std::time::Instant;

        fn xorshift_bytes(len: usize) -> Vec<u8> {
            let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
            let mut out = Vec::with_capacity(len);
            while out.len() < len {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                out.extend_from_slice(&x.to_le_bytes());
            }
            out.truncate(len);
            out
        }

        let mb = 1024 * 1024;
        let mut files: Vec<(&str, Vec<u8>)> = Vec::new();
        files.push(("single line, 50 MB", b"abcdefghijklmnopqrstuvwxyz".repeat(50 * mb / 26 + 1)));
        let mut lines = Vec::with_capacity(70 * mb);
        for i in 0..5_000_000u32 {
            lines.extend_from_slice(format!("line {i:07}\r\n").as_bytes());
        }
        files.push(("5,000,000 CRLF lines", lines));
        files.push(("binary, 64 MB", xorshift_bytes(64 * mb)));
        let tab_line = format!("{}\n", "\tword".repeat(64 * 1024 / 5));
        files.push(("64 KiB lines with tabs, 64 MB", tab_line.repeat(64 * mb / tab_line.len()).into_bytes()));
        let mut late = b"some text here\r\n".repeat(64 * mb / 16);
        late.extend_from_slice(b"one lone LF at the end\n");
        files.push(("64 MB CRLF, lone LF at the end", late));

        let dir = crate::persist::TempDir::new("big-files");
        for (name, raw) in files {
            let path = dir.join("big.bin");
            let _ = std::fs::remove_file(&path);
            assert!(std::fs::write(&path, &raw).is_ok());

            let t = Instant::now();
            let bytes = std::fs::read(&path).unwrap_or_default();
            let whole = encoding::decode_bytes(&bytes);
            drop(bytes);
            let (eol, text) = split_line_endings(whole.text);
            let rope = ropey::Rope::from_str(&text);
            drop(text);
            let whole_ms = t.elapsed().as_millis();

            let t = Instant::now();
            let Ok(mut f) = std::fs::File::open(&path) else {
                panic!("open");
            };
            let mut first = read_chunk(&mut f).unwrap_or_default();
            let mut mode = StreamMode::sniff(&first);
            let mut passes = 0;
            let (streamed, s_eol) = 'pass: loop {
                passes += 1;
                assert!(passes <= 3);
                if passes > 1 {
                    let _ = f.seek(SeekFrom::Start(0));
                    first = read_chunk(&mut f).unwrap_or_default();
                }
                let mut d = StreamDecoder::new(mode);
                let mut r = ropey::Rope::new();
                let mut chunk = std::mem::take(&mut first);
                loop {
                    let eof = chunk.is_empty();
                    let fed = if eof { d.finish() } else { d.feed(&chunk) };
                    match fed {
                        Ok(t) => {
                            let at = r.len_chars();
                            r.insert(at, &t);
                        }
                        Err(why) => {
                            mode = mode.after(why);
                            continue 'pass;
                        }
                    }
                    if eof {
                        break 'pass (r, d.line_ending());
                    }
                    chunk = read_chunk(&mut f).unwrap_or_default();
                }
            };
            let stream_ms = t.elapsed().as_millis();
            assert_eq!(s_eol, eol, "{name}");
            assert!(streamed == rope, "{name}: streamed rope differs");

            let t = Instant::now();
            let utf8 = rope_to_bytes(&rope, eol);
            let out = if whole.encoding == Encoding::Utf8 {
                utf8
            } else {
                let s = String::from_utf8(utf8).unwrap_or_default();
                encoding::encode_checked(&s, whole.encoding).unwrap_or_default()
            };
            assert!(crate::io::write_file_safely(&path, &out).is_ok());
            let save_ms = t.elapsed().as_millis();
            assert!(out == raw, "{name}: save changed the bytes");

            println!(
                "{name:<34} {:>4} MB  {:?}/{:?}  whole-file {whole_ms:>5} ms  streamed {stream_ms:>5} ms ({passes} pass)  save {save_ms:>5} ms",
                raw.len() / mb,
                whole.encoding,
                eol,
            );
        }
    }

    #[test]
    fn progress_never_reads_past_100_percent() {
        let p = LoadProgress {
            bytes_read: 30,
            total_bytes: 10,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        assert!(format_progress(&p).ends_with("(100.0%)"));
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(u64::MAX), "16777216.0 TB");
    }
}
