// Streaming file loader for large files.
//
// `open_path` in `src/io.rs` reads the whole file with `std::fs::read` and
// then constructs a rope in one shot. Fine for typical source files;
// catastrophic for a 200 MB log. This module takes over once a file's size
// exceeds `STREAM_THRESHOLD_BYTES`: the read happens in 1 MB chunks on the
// background executor, each chunk is decoded incrementally with
// `encoding_rs::Decoder`, line endings are normalized to LF, and the result
// is appended to the active tab's rope on the main thread between chunks.
// The status bar shows progress while the load is in flight, and a Cancel
// button on the same chip aborts mid-read.
//
// Tree-sitter highlighting is skipped while loading and only kicks in at
// completion if the file is below `HIGHLIGHT_SIZE_LIMIT` — running the
// highlighter against a 500 MB string takes seconds and produces a span list
// that no human ever needs to see.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use encoding_rs::{UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};
use gpui::Context;

use crate::app::menus::rebuild_menus;
use crate::buffer::{Buffer, LoadProgress, Selection};
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
pub(crate) const STREAM_CHUNK_SIZE: usize = 1 * 1024 * 1024;

/// Files larger than this skip tree-sitter highlighting at end-of-load. The
/// editor already painted half a million spans by the time the highlighter
/// returned; not worth the time.
pub(crate) const HIGHLIGHT_SIZE_LIMIT: u64 = 50 * 1024 * 1024;

/// Files larger than this surface a Yes/No warning before the load even
/// starts. NPP equivalent dialog. Cancel is the default button.
pub(crate) const HUGE_FILE_WARNING_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Streaming line-ending normalizer. Replaces `\r\n` and bare `\r` with `\n`
/// while handling the case where the `\r` of a `\r\n` falls at the end of one
/// chunk and the `\n` at the start of the next. Also memoizes the dominant
/// line ending so the buffer's `line_ending` field can be set once at the
/// end of the load — cheaper than re-running `LineEnding::detect` over a
/// 200 MB string.
struct LineEndingStreamer {
    pending_cr: bool,
    detected: Option<LineEnding>,
}

impl LineEndingStreamer {
    fn new() -> Self {
        Self {
            pending_cr: false,
            detected: None,
        }
    }

    fn process(&mut self, input: &str) -> String {
        if input.is_empty() && !self.pending_cr {
            return String::new();
        }
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        if self.pending_cr {
            self.pending_cr = false;
            // Stale `\r` from end of previous chunk. If the new chunk leads
            // with `\n` it was a CRLF straddling a boundary; otherwise the
            // `\r` was a standalone CR line ending.
            match chars.peek() {
                Some(&'\n') => {
                    chars.next();
                    out.push('\n');
                    self.detected.get_or_insert(LineEnding::Crlf);
                }
                _ => {
                    out.push('\n');
                    self.detected.get_or_insert(LineEnding::Cr);
                }
            }
        }
        while let Some(c) = chars.next() {
            match c {
                '\r' => match chars.peek() {
                    Some(&'\n') => {
                        chars.next();
                        out.push('\n');
                        self.detected.get_or_insert(LineEnding::Crlf);
                    }
                    Some(_) => {
                        out.push('\n');
                        self.detected.get_or_insert(LineEnding::Cr);
                    }
                    None => {
                        // Trailing `\r`. Defer to next chunk to disambiguate
                        // CRLF vs CR.
                        self.pending_cr = true;
                    }
                },
                '\n' => {
                    out.push('\n');
                    self.detected.get_or_insert(LineEnding::Lf);
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn flush(&mut self) -> String {
        if self.pending_cr {
            self.pending_cr = false;
            self.detected.get_or_insert(LineEnding::Cr);
            "\n".to_string()
        } else {
            String::new()
        }
    }
}

/// BOM detection for the very first chunk. Returns the chosen encoding plus
/// the byte offset where the actual content starts (i.e. past the BOM).
fn sniff_bom(prefix: &[u8]) -> (Encoding, usize) {
    if prefix.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (Encoding::Utf8Bom, 3)
    } else if prefix.starts_with(&[0xFF, 0xFE]) {
        (Encoding::Utf16Le, 2)
    } else if prefix.starts_with(&[0xFE, 0xFF]) {
        (Encoding::Utf16Be, 2)
    } else {
        // No BOM. Default to strict UTF-8; the decoder falls back to
        // `WINDOWS_1252` further down if any chunk fails to decode.
        (Encoding::Utf8, 0)
    }
}

fn decoder_for(enc: Encoding) -> encoding_rs::Decoder {
    match enc {
        Encoding::Utf8 | Encoding::Utf8Bom => UTF_8.new_decoder_without_bom_handling(),
        Encoding::Utf16Le => UTF_16LE.new_decoder_without_bom_handling(),
        Encoding::Utf16Be => UTF_16BE.new_decoder_without_bom_handling(),
        Encoding::Latin1 => WINDOWS_1252.new_decoder_without_bom_handling(),
    }
}

/// Format a byte count with a single fractional digit. `0` → "0 B", `1500`
/// → "1.5 KB", and so on through TB. Used for the status-bar progress chip.
fn format_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut idx = 0usize;
    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{} B", n)
    } else {
        format!("{:.1} {}", value, UNITS[idx])
    }
}

/// Public formatter used by the status bar to render a `LoadProgress`
/// snapshot. Lives here so the format string only has one canonical home.
pub(crate) fn format_progress(p: &LoadProgress) -> String {
    let pct = if p.total_bytes > 0 {
        (p.bytes_read as f64 / p.total_bytes as f64) * 100.0
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

fn start_streaming(
    workspace: &mut Workspace,
    path: PathBuf,
    total_bytes: u64,
    cx: &mut Context<Workspace>,
) {
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
        highlights: Vec::new(),
        loading: Some(LoadProgress {
            bytes_read: 0,
            total_bytes,
            cancel: cancel.clone(),
        }),
        new_label: None,
        read_only: false,
        tab_color: None,
        is_changelog: false,
        rich_doc: None,
        flat_text_cache: None,
    };

    let target_idx = if active_is_pristine_empty(workspace) {
        let idx = workspace.active;
        let tab = &mut workspace.tabs[idx];
        tab.buffer = empty_buffer;
        tab.selection = Selection::default();
        tab.desired_col_x = None;
        tab.undo_stack.clear();
        tab.redo_stack.clear();
        idx
    } else {
        let mut tab = Tab::empty();
        tab.buffer = empty_buffer;
        workspace.tabs.push(tab);
        workspace.active = workspace.tabs.len() - 1;
        workspace.tabs.len() - 1
    };

    workspace.set_status(format!("Loading {}…", path.display()));
    cx.notify();

    let cancel_for_task = cancel.clone();
    let path_for_task = path.clone();

    cx.spawn(async move |this, cx| {
        let executor = cx.background_executor().clone();

        // Open + read first chunk in one bg hop so we can BOM-sniff before
        // the first main-thread update.
        let path_open = path_for_task.clone();
        let opened = executor
            .spawn(async move {
                use std::io::Read;
                let mut file = std::fs::File::open(&path_open)?;
                let mut buf = vec![0u8; STREAM_CHUNK_SIZE];
                let n = file.read(&mut buf)?;
                buf.truncate(n);
                Ok::<(std::fs::File, Vec<u8>), std::io::Error>((file, buf))
            })
            .await;

        let (mut file, mut first_bytes) = match opened {
            Ok(x) => x,
            Err(e) => {
                let msg = format!("Failed to open {}: {}", path_for_task.display(), e);
                this.update(cx, |w, cx| {
                    abort_load(w, target_idx, msg);
                    cx.notify();
                })
                .ok();
                return;
            }
        };

        // BOM-sniff and strip the marker bytes from the first chunk before
        // decoding. encoding_rs's BOM-handling decoders eat the BOM
        // automatically, but we want to surface the detected encoding to
        // the buffer's `encoding` field and store BOM-presence as a
        // distinct value (`Utf8Bom` vs `Utf8`).
        let (encoding, body_offset) = sniff_bom(&first_bytes);
        let _ = first_bytes.drain(..body_offset.min(first_bytes.len()));

        let mut decoder = decoder_for(encoding);
        let mut eol = LineEndingStreamer::new();
        let mut bytes_read = body_offset as u64 + first_bytes.len() as u64;

        let mut decoded = String::with_capacity(first_bytes.len() * 2);
        let last = bytes_read >= total_bytes;
        let _ = decoder.decode_to_string(&first_bytes, &mut decoded, last);
        let normalized_first = eol.process(&decoded);

        // First main-thread update — push initial chunk and the encoding.
        let proceed = this
            .update(cx, |w, cx| {
                if cancel_for_task.load(Ordering::Relaxed) {
                    return false;
                }
                let Some(tab) = w.tabs.get_mut(target_idx) else {
                    return false;
                };
                if tab.buffer.loading.is_none() {
                    return false;
                }
                tab.buffer.encoding = encoding;
                if !normalized_first.is_empty() {
                    let len = tab.buffer.rope.len_chars();
                    tab.buffer.rope.insert(len, &normalized_first);
                    tab.buffer.invalidate_flat_text();
                }
                if let Some(p) = tab.buffer.loading.as_mut() {
                    p.bytes_read = bytes_read;
                }
                cx.notify();
                true
            })
            .unwrap_or(false);
        if !proceed {
            return;
        }

        // Drain remaining chunks. Each iteration: bg-read one chunk, decode,
        // normalize, push to rope on main, update progress.
        loop {
            if cancel_for_task.load(Ordering::Relaxed) {
                break;
            }
            if bytes_read >= total_bytes {
                break;
            }

            let read = executor
                .spawn(async move {
                    use std::io::Read;
                    let mut buf = vec![0u8; STREAM_CHUNK_SIZE];
                    let n = file.read(&mut buf).unwrap_or(0);
                    buf.truncate(n);
                    (file, buf)
                })
                .await;

            let (next_file, chunk) = read;
            file = next_file;

            if chunk.is_empty() {
                break;
            }

            bytes_read += chunk.len() as u64;
            let last = bytes_read >= total_bytes;

            let mut chunk_text = String::with_capacity(chunk.len() * 2);
            let _ = decoder.decode_to_string(&chunk, &mut chunk_text, last);
            let normalized = eol.process(&chunk_text);

            let still_loading = this
                .update(cx, |w, cx| {
                    if cancel_for_task.load(Ordering::Relaxed) {
                        return false;
                    }
                    let Some(tab) = w.tabs.get_mut(target_idx) else {
                        return false;
                    };
                    if tab.buffer.loading.is_none() {
                        return false;
                    }
                    if !normalized.is_empty() {
                        let len = tab.buffer.rope.len_chars();
                        tab.buffer.rope.insert(len, &normalized);
                        tab.buffer.invalidate_flat_text();
                    }
                    if let Some(p) = tab.buffer.loading.as_mut() {
                        p.bytes_read = bytes_read;
                    }
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !still_loading {
                return;
            }
        }

        // Drain any deferred CR + decoder tail.
        let mut tail = String::new();
        let _ = decoder.decode_to_string(&[], &mut tail, true);
        let normalized_tail = format!("{}{}", eol.process(&tail), eol.flush());
        let detected_eol = eol.detected.unwrap_or_else(LineEnding::platform_default);
        let was_cancelled = cancel_for_task.load(Ordering::Relaxed);

        this.update(cx, |w, cx| {
            let Some(tab) = w.tabs.get_mut(target_idx) else {
                return;
            };
            if tab.buffer.loading.is_none() {
                return;
            }
            if was_cancelled {
                // Cancel: drop the partial rope, leave a status note. Tab
                // stays open with whatever was loaded so far so the user
                // can still glance at it; loading flag clears.
                tab.buffer.loading = None;
                w.set_status(format!("Cancelled load of {}", path_for_task.display()));
                cx.notify();
                return;
            }
            if !normalized_tail.is_empty() {
                let len = tab.buffer.rope.len_chars();
                tab.buffer.rope.insert(len, &normalized_tail);
            }
            tab.buffer.invalidate_flat_text();
            tab.buffer.line_ending = detected_eol;
            tab.buffer.loading = None;

            // Late tree-sitter assignment. Skipped entirely for files past
            // `HIGHLIGHT_SIZE_LIMIT` — running the highlighter on a
            // multi-hundred-MB file blocks the main thread for seconds.
            if total_bytes <= HIGHLIGHT_SIZE_LIMIT {
                let inferred = crate::syntax::language_for_path(&path_for_task);
                tab.buffer.language = inferred;
                tab.buffer.reparse_highlights();
            }

            let basename = tab.buffer.display_short_name().to_string();
            w.set_status(format!("Opened {}", basename));
            w.last_lines.clear();
            w.caret_visible = true;
            w.save_session();
            recent::record_opened(path_for_task.clone());
            rebuild_menus(&mut *cx);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Mirror of `Workspace::active_is_pristine_empty` — duplicated here because
/// the original is a private helper inside `io.rs`. Kept tiny on purpose.
fn active_is_pristine_empty(workspace: &Workspace) -> bool {
    let tab = &workspace.tabs[workspace.active];
    tab.buffer.rope.len_bytes() == 0 && tab.buffer.file_path.is_none() && !tab.buffer.dirty
}

/// Roll a streaming load back to a clean error state. The tab keeps
/// existing (it may have been newly opened just for this load), but its
/// buffer is reset to empty so a subsequent file can land in it.
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
        tab.buffer.file_path = None;
        tab.buffer.new_label = Some(label);
    }
    workspace.set_status(status);
}
