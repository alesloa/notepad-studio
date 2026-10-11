// Session persistence — saves every tab, every dirty buffer, every cursor /
// scroll / bookmark, plus the active theme and closed-tab restore stack.
// Lives at `<config>/Notepad Studio/session.toml` (manifest) and
// `<config>/Notepad Studio/session/<uuid>.bak` (one per dirty / untitled tab).
//
// Atomic write: every save goes to `<path>.tmp` first, is synced, then
// renames over the old file (`persist::atomic_write`). A crash or a full
// disk mid-write leaves the prior session intact.
//
// Crash-recovery falls out for free: clean exits with no remaining dirty
// tabs clear the session dir; otherwise the manifest survives and the next
// launch restores from it. A manifest that is damaged keeps every tab entry
// that still parses, and any autosave it no longer points at comes back as
// an untitled tab instead of being reaped by the next save.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use gpui::SharedString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::buffer::{
    Buffer, Selection, deserialize_undo_stack, floor_char_boundary, retain_replayable_undo,
    serialize_undo_stack,
};
use crate::consts::APP_NAME;
use crate::persist;
use crate::syntax::LanguageId;
use crate::tab::{ClosedTab, Tab};
use crate::text::encoding::Encoding;
use crate::text::line_ending::LineEnding;
use crate::theme::ThemeMode;

mod windows;
pub(crate) use windows::{register_workspace, release_workspace};

// ---- Manifest types --------------------------------------------------------

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub(crate) struct SessionManifest {
    #[serde(default)]
    pub(crate) theme: Option<ThemeMode>,
    /// View ▸ Show Whitespace toggle. `#[serde(default)]` so manifests
    /// written before this field existed deserialize cleanly to `false`.
    #[serde(default)]
    pub(crate) show_whitespace: bool,
    /// View ▸ Word Wrap toggle. Defaults to `false` for compatibility with
    /// pre-feature manifests.
    #[serde(default)]
    pub(crate) word_wrap: bool,
    #[serde(default)]
    pub(crate) active: usize,
    #[serde(default)]
    pub(crate) tabs: Vec<SessionTab>,
    #[serde(default)]
    pub(crate) closed_tabs: Vec<SessionClosedTab>,
    /// Document-switcher MRU (Ctrl+Tab cycle order). Persisted across
    /// launches so the most-recently-used tab is reachable with a single
    /// Ctrl+Tab right after restart. `#[serde(default)]` for backward
    /// compat with pre-feature manifests — they restore with an empty
    /// MRU and the workspace seeds it from the restored active tab.
    #[serde(default)]
    pub(crate) mru: Vec<Uuid>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct SessionTab {
    /// UUID — names the `.bak` file for dirty / untitled buffers.
    pub(crate) id: Uuid,
    /// Original disk path (None for untitled buffers).
    #[serde(default)]
    pub(crate) path: Option<PathBuf>,
    /// True if the in-memory buffer has unsaved changes (or untitled).
    /// When true, `<id>.bak` holds the in-memory contents.
    #[serde(default)]
    pub(crate) dirty: bool,
    #[serde(default)]
    pub(crate) line_ending: LineEndingTag,
    #[serde(default)]
    pub(crate) encoding: EncodingTag,
    #[serde(default)]
    pub(crate) language: Option<LanguageId>,
    #[serde(default)]
    pub(crate) cursor_anchor: usize,
    #[serde(default)]
    pub(crate) cursor_head: usize,
    /// Scroll position in raw pixels at the time of save. Approximate on
    /// restore — the editor reshapes lines from scratch and scroll might
    /// land a hair off if zoom or font metrics changed.
    #[serde(default)]
    pub(crate) scroll_top: f32,
    #[serde(default)]
    pub(crate) scroll_left: f32,
    /// Bookmark byte offsets (line-start bytes, same form as
    /// `Buffer::bookmarks`).
    #[serde(default)]
    pub(crate) bookmarks: Vec<usize>,
    /// Original on-disk mtime as Unix seconds at the time of save. Used on
    /// restore to detect external edits — currently informational.
    #[serde(default)]
    pub(crate) original_mtime: Option<u64>,
    /// Persisted "new N" display label for untitled buffers. `None` for
    /// disk-backed tabs. Survives across launches so a restored untitled tab
    /// keeps its original number.
    #[serde(default)]
    pub(crate) new_label: Option<String>,
    /// Read-only flag — survives launches so a tab marked Read-Only stays
    /// edit-blocked next session.
    #[serde(default)]
    pub(crate) read_only: bool,
    /// Apply Color to Tab — `Some(1..=5)` maps to the same NPP-canonical
    /// palette painted by the tab strip. Persists across launches.
    #[serde(default)]
    pub(crate) tab_color: Option<u8>,
    /// True when the persisted tab is the auto-opened `change.log`. Survives
    /// across launches so closing the window with the changelog visible and
    /// re-opening via the dock keeps it on screen — the version-marker file
    /// in `crate::changelog` still owns the post-update auto-open.
    #[serde(default)]
    pub(crate) is_changelog: bool,
    /// Whether the tab was in `TabViewMode::MarkdownPreview` at snapshot time.
    /// Only meaningful for markdown paths — restore guards on the file being
    /// markdown before flipping the mode. Defaults to false so legacy session
    /// manifests don't suddenly all flip into preview.
    #[serde(default)]
    pub(crate) markdown_preview: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct SessionClosedTab {
    pub(crate) id: Uuid,
    #[serde(default)]
    pub(crate) path: Option<PathBuf>,
    #[serde(default)]
    // Older manifests omitted this flag; only those need a disk comparison.
    pub(crate) dirty: Option<bool>,
    #[serde(default)]
    pub(crate) read_only: bool,
    #[serde(default)]
    pub(crate) line_ending: LineEndingTag,
    #[serde(default)]
    pub(crate) encoding: EncodingTag,
    #[serde(default)]
    pub(crate) language: Option<LanguageId>,
    #[serde(default)]
    pub(crate) new_label: Option<String>,
    #[serde(default)]
    pub(crate) tab_color: Option<u8>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LineEndingTag {
    #[default]
    Lf,
    Crlf,
    Cr,
}

impl LineEndingTag {
    fn from_line_ending(le: LineEnding) -> Self {
        match le {
            LineEnding::Lf => Self::Lf,
            LineEnding::Crlf => Self::Crlf,
            LineEnding::Cr => Self::Cr,
        }
    }

    pub(crate) fn into_line_ending(self) -> LineEnding {
        match self {
            Self::Lf => LineEnding::Lf,
            Self::Crlf => LineEnding::Crlf,
            Self::Cr => LineEnding::Cr,
        }
    }
}

#[derive(Default, Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EncodingTag {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Latin1,
}

impl EncodingTag {
    fn from_encoding(enc: Encoding) -> Self {
        match enc {
            Encoding::Utf8 => Self::Utf8,
            Encoding::Utf8Bom => Self::Utf8Bom,
            Encoding::Utf16Le => Self::Utf16Le,
            Encoding::Utf16Be => Self::Utf16Be,
            Encoding::Latin1 => Self::Latin1,
        }
    }

    pub(crate) fn into_encoding(self) -> Encoding {
        match self {
            Self::Utf8 => Encoding::Utf8,
            Self::Utf8Bom => Encoding::Utf8Bom,
            Self::Utf16Le => Encoding::Utf16Le,
            Self::Utf16Be => Encoding::Utf16Be,
            Self::Latin1 => Encoding::Latin1,
        }
    }
}

// ---- Path helpers ----------------------------------------------------------

fn session_root() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME))
}

pub(crate) fn manifest_path() -> Option<PathBuf> {
    session_root().map(|d| d.join("session.toml"))
}

fn bak_dir() -> Option<PathBuf> {
    session_root().map(|d| d.join("session"))
}

fn bak_path(id: Uuid) -> Option<PathBuf> {
    bak_dir().map(|d| bak_path_in(&d, id))
}

fn bak_path_in(dir: &Path, id: Uuid) -> PathBuf {
    dir.join(format!("{id}.bak"))
}

fn undo_path(id: Uuid) -> Option<PathBuf> {
    bak_dir().map(|d| undo_path_in(&d, id))
}

fn undo_path_in(dir: &Path, id: Uuid) -> PathBuf {
    dir.join(format!("{id}.undo"))
}

// ---- Manifest read / write ------------------------------------------------

pub(crate) fn load_manifest() -> Option<SessionManifest> {
    let path = manifest_path()?;
    let dir = bak_dir()?;
    load_manifest_at(&path, &dir)
}

/// Read the manifest at `path`. A manifest that no longer matches the
/// format keeps every field and tab entry that still parses; one that
/// isn't TOML is moved aside to `session.toml.corrupt`.
///
/// Autosaves in `bak_dir` that the manifest doesn't point at come back as
/// untitled tabs. A save writes the `.bak` files before the manifest and
/// then reaps every `.bak` the manifest doesn't list, so an unlisted one
/// means the manifest was lost, damaged, emptied by a crash, or never
/// written; it may hold the only copy of what the user typed.
fn load_manifest_at(path: &Path, bak_dir: &Path) -> Option<SessionManifest> {
    let loaded = persist::load_toml(path, salvage_manifest, SessionManifest::default);
    let found = !matches!(loaded, persist::Loaded::Missing);
    let manifest = loaded.into_option().unwrap_or_default();
    let manifest = recover_unreferenced_baks(manifest, bak_dir);
    if !found && manifest.tabs.is_empty() {
        return None;
    }
    Some(manifest)
}

/// Field-by-field salvage of a manifest that parsed as TOML but not as a
/// `SessionManifest`: one bad tab entry costs that entry, one bad field in
/// an entry costs that field.
fn salvage_manifest(table: &toml::Table) -> SessionManifest {
    let mut top = table.clone();
    let tabs = top.remove("tabs");
    let closed = top.remove("closed_tabs");
    let mut manifest: SessionManifest = persist::salvage_table(&top, &[]).unwrap_or_default();
    let (tabs, dropped_tabs) = persist::salvage_array::<SessionTab>(tabs.as_ref(), &["id"]);
    let (closed, dropped_closed) =
        persist::salvage_array::<SessionClosedTab>(closed.as_ref(), &["id"]);
    if dropped_tabs + dropped_closed > 0 {
        eprintln!("[session] dropped {dropped_tabs} tab and {dropped_closed} closed-tab entries");
    }
    manifest.tabs = tabs;
    manifest.closed_tabs = closed;
    manifest
}

/// Append an untitled, dirty tab for every non-empty `<uuid>.bak` in
/// `bak_dir` that `manifest` doesn't reference, oldest first.
fn recover_unreferenced_baks(mut manifest: SessionManifest, bak_dir: &Path) -> SessionManifest {
    let referenced: BTreeSet<Uuid> = manifest
        .tabs
        .iter()
        .map(|t| t.id)
        .chain(manifest.closed_tabs.iter().map(|c| c.id))
        .collect();
    let Ok(entries) = fs::read_dir(bak_dir) else {
        return manifest;
    };
    let mut orphans: Vec<(std::time::SystemTime, Uuid)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bak") {
                return None;
            }
            let id = Uuid::parse_str(path.file_stem()?.to_str()?).ok()?;
            if referenced.contains(&id) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if meta.len() == 0 {
                return None;
            }
            Some((meta.modified().unwrap_or(UNIX_EPOCH), id))
        })
        .collect();
    orphans.sort();
    let mut labels: Vec<SharedString> = manifest
        .tabs
        .iter()
        .filter_map(|t| t.new_label.clone().map(SharedString::from))
        .collect();
    // The `.bak` holds the rope; the line ending and encoding it was going
    // to be saved with were in the lost entry, so use the new-document ones.
    let line_ending =
        LineEndingTag::from_line_ending(crate::settings::resolved_default_line_ending());
    let encoding = EncodingTag::from_encoding(crate::settings::resolved_default_encoding());
    for (_, id) in orphans {
        let label = crate::buffer::next_new_label_for(labels.iter().map(Some));
        eprintln!("[session] recovered autosave {id} as {label}");
        manifest.tabs.push(SessionTab {
            id,
            path: None,
            dirty: true,
            line_ending,
            encoding,
            language: None,
            cursor_anchor: 0,
            cursor_head: 0,
            scroll_top: 0.0,
            scroll_left: 0.0,
            bookmarks: Vec::new(),
            original_mtime: None,
            new_label: Some(label.to_string()),
            read_only: false,
            tab_color: None,
            is_changelog: false,
            markdown_preview: false,
        });
        labels.push(label);
    }
    manifest
}

fn ensure_dirs() -> io::Result<()> {
    if let Some(root) = session_root() {
        fs::create_dir_all(&root)?;
    }
    if let Some(b) = bak_dir() {
        fs::create_dir_all(&b)?;
    }
    Ok(())
}

pub(crate) fn write_manifest(manifest: &SessionManifest) -> io::Result<()> {
    ensure_dirs()?;
    let Some(path) = manifest_path() else {
        return Ok(());
    };
    write_manifest_at(manifest, &path)
}

fn write_manifest_at(manifest: &SessionManifest, path: &Path) -> io::Result<()> {
    let serialized = toml::to_string_pretty(manifest).map_err(io::Error::other)?;
    persist::atomic_write(path, serialized.as_bytes())
}

pub(crate) fn write_bak(id: Uuid, contents: &str) -> io::Result<()> {
    ensure_dirs()?;
    let Some(path) = bak_path(id) else {
        return Ok(());
    };
    persist::atomic_write(&path, contents.as_bytes())
}

/// The `.bak` is written from the rope, so it is UTF-8. If it was damaged
/// on disk, keep what's readable rather than calling the autosave missing,
/// which would let the next save reap it.
fn read_bak_in(dir: &Path, id: Uuid) -> Option<String> {
    let raw = fs::read(bak_path_in(dir, id)).ok()?;
    Some(match String::from_utf8(raw) {
        Ok(text) => text,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
    })
}

pub(crate) fn delete_bak(id: Uuid) {
    if let Some(path) = bak_path(id) {
        let _ = fs::remove_file(&path);
    }
}

/// Persist the per-tab undo stack alongside the `.bak`. Empty stacks delete
/// the file instead of writing a zero-entry payload so the session dir doesn't
/// accumulate empty undo shadows for fresh tabs.
pub(crate) fn write_undo(id: Uuid, bytes: &[u8]) -> io::Result<()> {
    ensure_dirs()?;
    let Some(path) = undo_path(id) else {
        return Ok(());
    };
    if bytes.is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    persist::atomic_write(&path, bytes)
}

fn read_undo_in(dir: &Path, id: Uuid) -> Option<Vec<u8>> {
    fs::read(undo_path_in(dir, id)).ok()
}

pub(crate) fn delete_undo(id: Uuid) {
    if let Some(path) = undo_path(id) {
        let _ = fs::remove_file(&path);
    }
}

// ---- Snapshotting ----------------------------------------------------------

fn mtime_unix(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    modified.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Whether `snapshot_tab` should also write the rope contents to a `.bak`
/// shadow file. Most paths use `Full` (write a `.bak` for every dirty or
/// untitled tab). The eager close-too-fast path uses `UntitledOnly` —
/// path-backed dirty `.bak`s stay on the 250ms debounced path because their
/// rope can be megabytes and we don't want a per-keystroke megabyte write.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BakPolicy {
    /// Default — write a `.bak` for every dirty or untitled tab.
    Full,
    /// Only write a `.bak` for untitled tabs (where the `.bak` is the one
    /// and only place the content lives). Path-backed dirty tabs get the
    /// fall-back behaviour: the manifest says `dirty = false` and restore
    /// re-reads the file from disk on next launch — losing the
    /// typed-but-unsaved keystrokes is preferable to a per-keystroke
    /// multi-megabyte write.
    UntitledOnly,
}

/// Build a `SessionTab` snapshot of `tab`, writing a `.bak` file if the tab
/// is dirty or untitled (under `BakPolicy::Full`) or only if untitled
/// (under `BakPolicy::UntitledOnly`). Reuses `tab.session_id` so per-tab
/// autosave can keep writing to the same `.bak` across edits.
pub(crate) fn snapshot_tab_with_policy(tab: &Tab, bak_policy: BakPolicy) -> SessionTab {
    let id = tab.session_id;
    let buffer = &tab.buffer;
    let needs_backup = match bak_policy {
        BakPolicy::Full => buffer.dirty || buffer.file_path.is_none(),
        BakPolicy::UntitledOnly => buffer.file_path.is_none(),
    };
    if needs_backup {
        let _ = write_bak(id, &buffer.rope.to_string());
        let _ = write_undo(id, &serialize_undo_stack(&tab.undo_stack));
    } else if matches!(bak_policy, BakPolicy::Full) {
        // Clean disk-backed tab — drop any stale `.bak` from a previous dirty
        // state. Manifest still references the path; restore reads disk.
        // Skip this delete on `UntitledOnly` so a path-backed dirty buffer's
        // older `.bak` (written by an earlier `Full` save) isn't wiped on
        // every keystroke — the next `Full` flush owns that decision.
        delete_bak(id);
        delete_undo(id);
    }
    // Manifest's `dirty` flag must mirror whether a `.bak` exists: if we
    // skipped the `.bak` write under `UntitledOnly` for a path-backed dirty
    // buffer, claim `dirty = false` so restore reads from the file path
    // instead of looking for a missing `.bak`.
    let manifest_dirty = match bak_policy {
        BakPolicy::Full => needs_backup,
        BakPolicy::UntitledOnly => buffer.file_path.is_none(),
    };
    SessionTab {
        id,
        path: buffer.file_path.clone(),
        dirty: manifest_dirty,
        line_ending: LineEndingTag::from_line_ending(buffer.line_ending),
        encoding: EncodingTag::from_encoding(buffer.encoding),
        language: buffer.language,
        cursor_anchor: tab.selection.anchor,
        cursor_head: tab.selection.head,
        scroll_top: f32::from(tab.scroll_top),
        scroll_left: f32::from(tab.scroll_left),
        bookmarks: buffer.bookmarks.iter().copied().collect(),
        original_mtime: buffer
            .file_path
            .as_ref()
            .and_then(|p| mtime_unix(p)),
        new_label: buffer.new_label.as_ref().map(|s| s.to_string()),
        // A streaming load holds the tab read-only until the file is in;
        // that is not the user's flag.
        read_only: buffer.read_only && buffer.loading.is_none(),
        tab_color: buffer.tab_color,
        is_changelog: buffer.is_changelog,
        markdown_preview: matches!(tab.view_mode, crate::tab::TabViewMode::MarkdownPreview),
    }
}

pub(crate) fn snapshot_closed_tab(closed: &ClosedTab) -> SessionClosedTab {
    let id = closed.session_id;
    let _ = write_bak(id, &closed.buffer.rope.to_string());
    SessionClosedTab {
        id,
        path: closed.buffer.file_path.clone(),
        dirty: Some(closed.buffer.dirty),
        read_only: closed.buffer.read_only,
        line_ending: LineEndingTag::from_line_ending(closed.buffer.line_ending),
        encoding: EncodingTag::from_encoding(closed.buffer.encoding),
        language: closed.buffer.language,
        new_label: closed.buffer.new_label.as_ref().map(|s| s.to_string()),
        tab_color: closed.buffer.tab_color,
    }
}

// ---- Restoration -----------------------------------------------------------

/// Outcome of restoring a single tab — either a live `Tab` or a reason the
/// entry was dropped (so the workspace can flash a single status line).
pub(crate) enum RestoreOutcome {
    Restored(Tab),
    Skipped(String),
}

/// Reconstruct a `Tab` from a manifest entry. For dirty / untitled entries
/// the content lives in the `.bak`; for clean disk-backed entries it loads
/// from the file path. Restore best-effort — corrupt or missing sources
/// produce `Skipped(reason)` instead of failing the whole session.
pub(crate) fn restore_tab(entry: &SessionTab) -> RestoreOutcome {
    let Some(dir) = bak_dir() else {
        return RestoreOutcome::Skipped("no config directory".to_string());
    };
    restore_tab_in(entry, &dir)
}

/// Contents of a clean disk-backed tab, read the way File ▸ Open reads it.
struct CleanFile {
    rope: ropey::Rope,
    line_ending: LineEnding,
    encoding: Encoding,
    /// Some bytes couldn't be decoded; the tab opens read-only, as on open.
    lossy: bool,
}

/// Read `path` for a clean tab. The encoding and line ending come from the
/// bytes, as on File ▸ Open, not from the manifest: the file can have
/// changed since, and a UTF-16 or Latin-1 file isn't valid UTF-8 at all.
/// Folders and files past the open-size guard are refused, since restore
/// runs before the window shows and can't ask.
fn read_clean_file(path: &Path) -> Result<CleanFile, String> {
    let shown = path.display();
    let meta = fs::metadata(path).map_err(|err| format!("couldn't reopen {shown}: {err}"))?;
    if meta.is_dir() {
        return Err(format!("{shown} is a folder"));
    }
    if meta.len() >= crate::io_stream::HUGE_FILE_WARNING_BYTES {
        return Err(format!(
            "{shown} is too large to reopen automatically; open it from File ▸ Open"
        ));
    }
    let raw = fs::read(path).map_err(|err| format!("couldn't reopen {shown}: {err}"))?;
    let decoded = crate::text::encoding::decode_bytes(&raw);
    drop(raw);
    let (line_ending, text) = crate::text::line_ending::split_line_endings(decoded.text);
    Ok(CleanFile {
        rope: ropey::Rope::from_str(&text),
        line_ending,
        encoding: decoded.encoding,
        lossy: decoded.lossy,
    })
}

fn restore_tab_in(entry: &SessionTab, bak_dir: &Path) -> RestoreOutcome {
    // Rich-document tabs (DOCX / RTF / ODT / DOC) re-parse from disk on
    // restore. The disk file is a binary container (zip for DOCX/ODT,
    // bespoke encoding for RTF/DOC), so the plain-text restore paths
    // below — decoding a clean tab's bytes as text, or the .bak which
    // only holds the rope's extracted XML — can't reconstruct the
    // rich_doc payload. Without that payload `Buffer::is_rich_doc()`
    // returns false, the toolbar greys out the Doc View / Raw View
    // toggles, and the tab opens stuck in source mode showing raw XML.
    //
    // The trade-off: any in-memory edits a user made since the last
    // explicit Save get discarded. That's no worse than the prior
    // restore behaviour (which dumped the same edits into a buffer
    // with no rich-doc context, leaving them unrecoverable anyway),
    // and it preserves the document's primary affordance — render.
    let rich_doc_payload = entry
        .path
        .as_ref()
        .filter(|p| crate::rich_doc::detect_format(p).is_some())
        .and_then(|p| crate::rich_doc::open(p).ok());

    // Line ending and encoding default to what the manifest recorded, which
    // is what a `.bak` (the rope as it was) is saved back with.
    let mut line_ending = entry.line_ending.into_line_ending();
    let mut encoding = entry.encoding.into_encoding();
    let mut lossy = false;
    let (rope, dirty) = if let Some(p) = rich_doc_payload.as_ref() {
        // Rope mirrors raw_view_text so the Raw View source pane stays
        // in sync with the parsed AST. Clean (no .bak) — re-parsing
        // from disk supersedes any prior dirty state.
        let le = crate::text::line_ending::LineEnding::detect(&p.raw_view_text);
        let normalized =
            crate::text::line_ending::normalize_to_lf(p.raw_view_text.clone(), le);
        (ropey::Rope::from_str(&normalized), false)
    } else if entry.dirty {
        match read_bak_in(bak_dir, entry.id) {
            Some(s) => (ropey::Rope::from_str(&s), true),
            None => {
                let label = entry
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .or_else(|| entry.new_label.clone())
                    .unwrap_or_else(|| "untitled".into());
                return RestoreOutcome::Skipped(format!("missing autosave: {label}"));
            }
        }
    } else if let Some(path) = entry.path.as_ref() {
        match read_clean_file(path) {
            Ok(file) => {
                line_ending = file.line_ending;
                encoding = file.encoding;
                lossy = file.lossy;
                (file.rope, false)
            }
            Err(reason) => return RestoreOutcome::Skipped(reason),
        }
    } else {
        // No path AND not dirty — nothing to restore.
        return RestoreOutcome::Skipped("untitled with no autosave".to_string());
    };

    let is_rich_doc_restore = rich_doc_payload.is_some();
    let total = rope.len_bytes();
    let bookmarks: BTreeSet<usize> = entry
        .bookmarks
        .iter()
        .copied()
        .filter(|&b| b <= total)
        .collect();

    let mut buffer = Buffer {
        rope,
        file_path: entry.path.clone(),
        dirty,
        line_ending,
        encoding,
        bookmarks,
        marks: Vec::new(),
        language: entry.language,
        highlights: Vec::new(),
        loading: None,
        new_label: entry.new_label.clone().map(SharedString::from),
        // A lossy decode can't be saved back without rewriting the bad
        // bytes, so it opens read-only as File ▸ Open does.
        read_only: entry.read_only || lossy,
        tab_color: entry.tab_color,
        is_changelog: entry.is_changelog,
        rich_doc: rich_doc_payload,
        flat_text_cache: None,
    };
    buffer.normalize_bookmarks();
    buffer.reparse_highlights();

    let mut tab = Tab::empty();
    tab.buffer = buffer;
    if is_rich_doc_restore {
        // Default rich-doc tabs back into the rendered view — matches
        // `Workspace::open_path`'s post-open behaviour for fresh opens.
        tab.view_mode = crate::tab::TabViewMode::RichDocView;
    } else if entry.markdown_preview
        && crate::ui::markdown_view::is_markdown_path(tab.buffer.file_path.as_deref())
    {
        // Markdown tab was in preview mode at quit time — restore there.
        // The is_markdown_path guard handles the (unlikely) case where the
        // file extension changed between sessions.
        tab.view_mode = crate::tab::TabViewMode::MarkdownPreview;
    } else if crate::ui::csv_doc::is_csv_path(tab.buffer.file_path.as_deref()) {
        // CSV/TSV tabs always land in the grid preview on restore, matching
        // `Workspace::open_path`'s default. Source view is still reachable
        // via the toolbar Source toggle.
        tab.view_mode = crate::tab::TabViewMode::CsvPreview;
    }
    // Preserve the tab's identity across launches so autosave keeps writing
    // to the same `.bak` after restore.
    tab.session_id = entry.id;

    // The file may be shorter, or different, than when the caret was saved.
    let rope = &tab.buffer.rope;
    tab.selection = Selection {
        anchor: floor_char_boundary(rope, entry.cursor_anchor),
        head: floor_char_boundary(rope, entry.cursor_head),
        is_dragging: false,
    };
    tab.scroll_top = gpui::px(restored_scroll(entry.scroll_top));
    tab.scroll_left = gpui::px(restored_scroll(entry.scroll_left));

    // Restore the per-tab undo history. Rich-doc tabs re-parse from disk so
    // there's no rope-level undo to restore; everything else gets whatever
    // was last persisted alongside the `.bak`, minus any entries that don't
    // undo against the text actually restored. Malformed payloads yield an
    // empty stack rather than failing the restore.
    if !is_rich_doc_restore {
        if let Some(bytes) = read_undo_in(bak_dir, entry.id) {
            tab.undo_stack =
                retain_replayable_undo(deserialize_undo_stack(&bytes), &tab.buffer.rope);
        }
    }

    RestoreOutcome::Restored(tab)
}

/// A saved scroll offset, or 0 for a negative, NaN or infinite one from a
/// damaged or hand-edited manifest.
fn restored_scroll(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

/// Reconstruct a `ClosedTab` from a manifest entry. Closed-tab content is
/// always in a `.bak` — the closed-tab restore stack carries unsaved drafts
/// across launches the same way the open-tab list does.
pub(crate) fn restore_closed_tab(entry: &SessionClosedTab) -> Option<ClosedTab> {
    restore_closed_tab_in(entry, &bak_dir()?)
}

/// Rebuild a native payload only when it represents the backed-up text.
/// A container's extracted XML alone cannot reconstruct its other parts.
fn restore_rich_backup(path: &Path, body: &str, backup: &Path) -> Option<(crate::rich_doc::RichDocPayload, bool)> {
    use crate::rich_doc::{self, DocFormat};
    let format = rich_doc::detect_format(path)?;
    if fs::metadata(path).is_ok_and(|meta| meta.len() < crate::io_stream::HUGE_FILE_WARNING_BYTES) {
        if let Ok(payload) = rich_doc::open(path) {
            let raw = crate::text::line_ending::split_line_endings(payload.raw_view_text.clone()).1;
            if raw == body {
                return Some((payload, false));
            }
        }
    }
    // RTF source is itself a complete document, unlike extracted DOCX/ODT XML.
    if format == DocFormat::Rtf && body.trim_start().starts_with(r"{\rtf") {
        if let Ok(payload) = rich_doc::open_as(backup, format) {
            return Some((payload, true));
        }
    }
    None
}

fn restore_closed_tab_in(entry: &SessionClosedTab, dir: &Path) -> Option<ClosedTab> {
    let body = read_bak_in(dir, entry.id)?;
    let mut file_path = entry.path.clone();
    let mut new_label = entry.new_label.clone().map(SharedString::from);
    let mut dirty = entry.dirty.unwrap_or(false) || file_path.is_none();
    let mut rich_doc = None;
    if let Some(path) = entry.path.as_ref().filter(|p| crate::rich_doc::detect_format(p).is_some()) {
        if let Some((payload, recovered_edits)) = restore_rich_backup(path, &body, &bak_path_in(dir, entry.id)) {
            dirty |= recovered_edits;
            rich_doc = Some(payload);
        } else {
            // Keep the backup reachable without pointing plain text at a
            // native document. Save now asks for a new destination.
            let name = path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default();
            new_label = Some(format!("Recovered {name}.txt").into());
            file_path = None;
            dirty = true;
        }
    } else if let Some(path) = file_path.as_ref().filter(|_| entry.dirty.is_none()) {
        // Older manifests omitted dirty. Compare their saved text with disk
        // so reopening a recovered draft cannot delete its only backup.
        dirty |= read_clean_file(path).map_or(true, |file| file.rope != body.as_str());
    }
    let read_only = entry.read_only || rich_doc.as_ref().is_some_and(|p| p.format == crate::rich_doc::DocFormat::Doc);
    let buffer = Buffer {
        rope: ropey::Rope::from_str(&body),
        file_path,
        dirty,
        line_ending: entry.line_ending.into_line_ending(),
        encoding: entry.encoding.into_encoding(),
        bookmarks: BTreeSet::new(),
        marks: Vec::new(),
        language: entry.language,
        highlights: Vec::new(),
        loading: None,
        new_label,
        read_only,
        tab_color: entry.tab_color,
        is_changelog: false,
        rich_doc,
        flat_text_cache: None,
    };
    Some(ClosedTab {
        buffer,
        session_id: entry.id,
    })
}

/// Replace any leftover `.bak` files with the freshly written set — every
/// save scrubs orphans so the dir can't grow unbounded across many sessions.
pub(crate) fn reap_orphan_baks(manifest: &SessionManifest) {
    let Some(dir) = bak_dir() else { return };
    reap_orphan_baks_at(manifest, &dir);
}

fn reap_orphan_baks_at(manifest: &SessionManifest, dir: &Path) {
    let referenced: BTreeSet<Uuid> = manifest
        .tabs
        .iter()
        .map(|t| t.id)
        .chain(manifest.closed_tabs.iter().map(|c| c.id))
        .collect();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Reap both `.bak` and `.undo` shadows whose UUID stem no longer
        // appears in the manifest. The two file types live in the same dir
        // and are bound to the same session_id, so one pass handles both.
        let ext_ok = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "bak" || e == "undo")
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str());
        let Some(stem) = stem else { continue };
        if let Ok(id) = Uuid::parse_str(stem) {
            if !referenced.contains(&id) {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

// ---- High-level entry points used by the workspace -------------------------

/// True if a tab is "worth persisting" — has either a real file path or
/// unsaved edits. Filters out fresh blank scratch tabs and the welcome
/// placeholder so `save_session` doesn't litter the manifest with empty
/// entries. The auto-opened `change.log` tab is path-backed (it lives on
/// disk next to the binary) so it persists like any other file — closing
/// the window and re-opening via the dock keeps it visible.
fn tab_is_persistable(tab: &Tab) -> bool {
    tab.buffer.file_path.is_some() || tab.buffer.dirty
}

/// Capture the workspace's full state into a `SessionManifest`. Pure read —
/// caller decides whether to write it to disk.
pub(crate) fn snapshot_workspace_with_policy(
    tabs: &[Tab],
    active: usize,
    closed_tabs: &[ClosedTab],
    theme: ThemeMode,
    show_whitespace: bool,
    word_wrap: bool,
    mru: &[Uuid],
    bak_policy: BakPolicy,
) -> SessionManifest {
    // Find which tabs we'll keep AND remap `active` so it still points at the
    // user's current tab after the filter drops untitled-and-clean ones.
    let mut tab_entries: Vec<SessionTab> = Vec::with_capacity(tabs.len());
    let mut new_active = 0usize;
    for (idx, tab) in tabs.iter().enumerate() {
        if !tab_is_persistable(tab) {
            // Even though we skip serializing, still scrub any stale `.bak`
            // left over from a previous dirty state. Skip on UntitledOnly so
            // we don't kick path-backed `.bak`s out from under the debounced
            // full-save path.
            if matches!(bak_policy, BakPolicy::Full) {
                delete_bak(tab.session_id);
                delete_undo(tab.session_id);
            }
            continue;
        }
        if idx == active {
            new_active = tab_entries.len();
        }
        tab_entries.push(snapshot_tab_with_policy(tab, bak_policy));
    }

    // Filter the MRU to only ids that survived the persistability filter,
    // preserving order. Anything we dropped from `tab_entries` (untitled
    // clean buffers) shouldn't sit in the persisted MRU either — there's
    // nothing to point at on restore.
    let persisted_ids: std::collections::HashSet<Uuid> =
        tab_entries.iter().map(|e| e.id).collect();
    let mru_filtered: Vec<Uuid> = mru
        .iter()
        .copied()
        .filter(|id| persisted_ids.contains(id))
        .collect();

    SessionManifest {
        theme: Some(theme),
        show_whitespace,
        word_wrap,
        active: new_active,
        tabs: tab_entries,
        closed_tabs: closed_tabs.iter().map(snapshot_closed_tab).collect(),
        mru: mru_filtered,
    }
}

/// Combine the window snapshots, write atomically, then prune orphaned
/// recovery files under the full-backup policy.
pub(crate) fn save_workspace(window_id: Uuid, manifest: SessionManifest, bak_policy: BakPolicy) {
    let Some(manifest) = windows::update(window_id, manifest) else {
        return;
    };
    if let Err(err) = write_manifest(&manifest) {
        eprintln!("[session] failed to write manifest: {err}");
        return;
    }
    // The `Full` save is the canonical owner of `.bak` lifecycle; only it
    // gets to reap orphans. Eager `UntitledOnly` saves leave the debounced
    // full-save path's `.bak`s alone.
    if matches!(bak_policy, BakPolicy::Full) {
        reap_orphan_baks(&manifest);
    }
}

/// NPP `Backup ▸ Backup on save`. Copies `source_path` to a `.bak` location
/// after a successful save. `mode == None` is a no-op. `Simple` writes one
/// `<source>.bak` overwriting the prior copy. `Verbose` keeps history by
/// appending a UTC timestamp suffix `<source>.YYYY-MM-DD_HHMMSS.bak`.
/// `custom_dir`, when `Some`, redirects the `.bak` into that directory
/// (preserving the source filename); `None` keeps it next to the source.
/// Errors are returned but expected to be surfaced through the status bar
/// without aborting the underlying save.
pub(crate) fn write_save_backup(
    source_path: &Path,
    mode: crate::settings::BackupOnSaveMode,
    custom_dir: Option<&Path>,
) -> Result<(), String> {
    use crate::settings::BackupOnSaveMode;
    if matches!(mode, BackupOnSaveMode::None) {
        return Ok(());
    }
    let Some(file_name) = source_path.file_name() else {
        return Err("backup: source has no file name".to_string());
    };

    // Resolve target directory and ensure it exists. Custom-dir users can
    // point at a folder that doesn't exist yet — match NPP and create it
    // on first use rather than silently dropping the backup.
    let target_dir = match custom_dir {
        Some(dir) => {
            if !dir.exists() {
                fs::create_dir_all(dir).map_err(|e| format!("backup: mkdir: {e}"))?;
            }
            dir.to_path_buf()
        }
        None => source_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    };

    let target_name = match mode {
        BackupOnSaveMode::None => return Ok(()),
        BackupOnSaveMode::Simple => {
            // `<filename>.bak` — overwrite each save.
            let mut s = file_name.to_os_string();
            s.push(".bak");
            s
        }
        BackupOnSaveMode::Verbose => {
            // `<stem>.YYYY-MM-DD_HHMMSS.<ext>.bak` — never overwrites.
            let stamp = verbose_stamp();
            let stem = source_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| file_name.to_string_lossy().into_owned());
            let ext = source_path
                .extension()
                .map(|s| format!(".{}", s.to_string_lossy()))
                .unwrap_or_default();
            std::ffi::OsString::from(format!("{stem}.{stamp}{ext}.bak"))
        }
    };

    let target = target_dir.join(target_name);
    fs::copy(source_path, &target)
        .map(|_| ())
        .map_err(|e| format!("backup: copy: {e}"))
}

/// `YYYY-MM-DD_HHMMSS` UTC timestamp for the verbose-backup filename.
/// Computes from `SystemTime::now()` directly so we don't pull in `chrono`
/// for one stamp; lossy if the system clock predates UNIX_EPOCH (we just
/// fall back to "0" so a stale clock still produces a valid path).
fn verbose_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Civil-from-days conversion (Howard Hinnant's algorithm) for a no-dep
    // UTC breakdown. Accurate for any 32-bit second timestamp.
    let z = secs.div_euclid(86400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = if m <= 2 { y + 1 } else { y };

    let rem = secs.rem_euclid(86400) as u32;
    let hh = rem / 3600;
    let mm = (rem / 60) % 60;
    let ss = rem % 60;

    format!(
        "{:04}-{:02}-{:02}_{:02}{:02}{:02}",
        year, m, d, hh, mm, ss
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{EditOp, UndoEntry};
    use crate::persist::TempDir;
    use crate::text::encoding;
    use crate::text::line_ending::rope_to_bytes;

    fn entry(id: Uuid, path: Option<PathBuf>, dirty: bool) -> SessionTab {
        SessionTab {
            id,
            path,
            dirty,
            line_ending: LineEndingTag::Lf,
            encoding: EncodingTag::Utf8,
            language: None,
            cursor_anchor: 0,
            cursor_head: 0,
            scroll_top: 0.0,
            scroll_left: 0.0,
            bookmarks: Vec::new(),
            original_mtime: None,
            new_label: None,
            read_only: false,
            tab_color: None,
            is_changelog: false,
            markdown_preview: false,
        }
    }

    fn restored(e: &SessionTab, dir: &Path) -> Tab {
        match restore_tab_in(e, dir) {
            RestoreOutcome::Restored(tab) => tab,
            RestoreOutcome::Skipped(reason) => panic!("skipped: {reason}"),
        }
    }

    /// What Save writes for `tab`, as in `Workspace::write_active_inner`.
    fn saved_bytes(tab: &Tab) -> Vec<u8> {
        let utf8 = rope_to_bytes(&tab.buffer.rope, tab.buffer.line_ending);
        let text = String::from_utf8(utf8).unwrap_or_default();
        encoding::encode_checked(&text, tab.buffer.encoding).unwrap_or_default()
    }

    fn closed_entry(id: Uuid, path: PathBuf) -> SessionClosedTab {
        toml::from_str(&format!("id = \"{id}\"\npath = {}\n", toml::Value::String(path.to_string_lossy().into_owned())))
            .expect("closed entry")
    }

    fn rich_fixture(path: &Path) {
        use std::io::Write;
        let entries = if path.extension().and_then(|e| e.to_str()) == Some("docx") {
            vec![
                ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#),
                ("_rels/.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
                ("word/document.xml", r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>café</w:t></w:r></w:p></w:body></w:document>"#),
            ]
        } else {
            vec![("content.xml", r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:p>café</text:p></office:text></office:body></office:document-content>"#)]
        };
        let mut zip = zip::ZipWriter::new(fs::File::create(path).expect("fixture"));
        for (name, xml) in entries {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).expect("entry");
            zip.write_all(xml.as_bytes()).expect("xml");
        }
        zip.finish().expect("zip");
    }

    #[test]
    fn closed_rich_documents_restore_native_payload_and_save_intact() {
        let dir = TempDir::new("closed-rich-container");
        for name in ["document.docx", "document.odt", "document.rtf"] {
            let path = dir.join(name);
            if name.ends_with("rtf") {
                fs::write(&path, b"{\\rtf1\\ansi source}\r\n").expect("rtf");
            } else {
                rich_fixture(&path);
            }
            let original = fs::read(&path).expect("original");
            let payload = crate::rich_doc::open(&path).expect("parse fixture");
            let raw = crate::text::line_ending::split_line_endings(payload.raw_view_text).1;
            let entry = closed_entry(Uuid::new_v4(), path.clone());
            fs::write(bak_path_in(dir.path(), entry.id), &raw).expect("backup");
            let tab = Tab::from_closed(restore_closed_tab_in(&entry, dir.path()).expect("restore"));
            assert_eq!(tab.buffer.rope.to_string(), raw);
            let payload = tab.buffer.rich_doc.as_ref().expect("native payload missing; Save would overwrite the container with XML");
            assert_eq!(tab.view_mode, crate::tab::TabViewMode::RichDocView);
            crate::rich_doc::save(payload, &path, None).expect("save");
            assert_eq!(fs::read(&path).expect("saved"), original);
            assert!(!tab.buffer.dirty);
        }
    }

    #[test]
    fn closed_rich_backup_that_cannot_be_rebuilt_becomes_a_scratch_document() {
        let dir = TempDir::new("closed-rich-scratch");
        for name in ["missing.docx", "damaged.odt", "changed.docx", "legacy.doc"] {
            let path = dir.join(name);
            if name.starts_with("changed") {
                rich_fixture(&path);
            } else if name.starts_with("damaged") {
                fs::write(&path, "corrupt archive").expect("fixture");
            }
            let original = fs::read(&path).ok();
            let entry = closed_entry(Uuid::new_v4(), path.clone());
            let raw = "<recovered>keep café\nexactly</recovered>";
            fs::write(bak_path_in(dir.path(), entry.id), raw).expect("backup");
            let closed = restore_closed_tab_in(&entry, dir.path()).expect("restore");
            assert!(closed.buffer.file_path.is_none(), "{name}: must not save extracted text over a native document");
            assert_eq!(closed.buffer.rope.to_string(), raw);
            assert!(closed.buffer.dirty);
            assert!(closed.buffer.rich_doc.is_none());
            assert_eq!(closed.buffer.new_label.as_ref().map(|s| s.as_ref()), Some(format!("Recovered {name}.txt").as_str()));
            assert_eq!(fs::read(&path).ok(), original);
        }
    }

    #[test]
    fn closed_rtf_raw_edits_recover_even_without_the_original() {
        let dir = TempDir::new("closed-rtf-edits");
        let raw = r"{\rtf1\ansi unsaved source}";
        for name in ["changed.rtf", "missing.rtf"] {
            let path = dir.join(name);
            if name.starts_with("changed") {
                fs::write(&path, r"{\rtf1\ansi original}").expect("fixture");
            }
            let entry = closed_entry(Uuid::new_v4(), path.clone());
            fs::write(bak_path_in(dir.path(), entry.id), raw).expect("backup");
            let tab = Tab::from_closed(restore_closed_tab_in(&entry, dir.path()).expect("restore"));
            assert_eq!(tab.buffer.file_path.as_deref(), Some(path.as_path()));
            assert!(tab.buffer.dirty);
            assert_eq!(tab.view_mode, crate::tab::TabViewMode::RichDocView);
            let payload = tab.buffer.rich_doc.as_ref().expect("RTF payload");
            assert!(payload.block_text(0).contains("unsaved source"));
            crate::rich_doc::save(payload, &path, Some(raw)).expect("save");
            assert_eq!(fs::read_to_string(&path).expect("saved"), raw);
        }
    }

    #[test]
    fn closed_tab_flags_and_legacy_drafts_remain_recoverable() {
        let dir = TempDir::new("closed-tab-flags");
        let path = dir.join("draft.txt");
        fs::write(&path, "original").expect("source");
        let mut entry = closed_entry(Uuid::new_v4(), path);
        assert!(entry.dirty.is_none() && !entry.read_only, "legacy defaults");
        fs::write(bak_path_in(dir.path(), entry.id), "unsaved café").expect("backup");
        let closed = restore_closed_tab_in(&entry, dir.path()).expect("restore");
        assert!(closed.buffer.dirty, "legacy backup differing from disk");
        assert_eq!(closed.buffer.rope.to_string(), "unsaved café");
        entry.dirty = Some(true);
        entry.read_only = true;
        let mut entry: SessionClosedTab = toml::from_str(&toml::to_string(&entry).expect("encode")).expect("decode");
        let closed = restore_closed_tab_in(&entry, dir.path()).expect("restore");
        assert!(closed.buffer.read_only && closed.buffer.dirty);
        entry.path = None;
        entry.dirty = Some(false);
        assert!(restore_closed_tab_in(&entry, dir.path()).expect("untitled").buffer.dirty);
    }

    #[test]
    fn closed_csv_reopens_in_the_grid_view() {
        let mut buffer = Buffer::empty();
        buffer.file_path = Some(PathBuf::from("sample.csv"));
        let id = Uuid::new_v4();
        let tab = Tab::from_closed(ClosedTab { buffer, session_id: id });
        assert_eq!(tab.view_mode, crate::tab::TabViewMode::CsvPreview);
        assert_eq!(tab.session_id, id);
    }

    #[test]
    fn clean_tabs_reopen_in_any_encoding_and_save_byte_identical() {
        // Regression: restore read clean files with `read_to_string` and
        // the manifest's line ending. UTF-16 and Latin-1 files failed to
        // decode and were dropped from the session, and a UTF-8 BOM file
        // kept U+FEFF in the text, so the next save wrote the BOM twice.
        let dir = TempDir::new("session-restore-enc");
        let mut utf16le = vec![0xFF, 0xFE];
        utf16le.extend("a\r\nb\u{e9}\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()));
        let mut utf16be = vec![0xFE, 0xFF];
        utf16be.extend("x\ny".encode_utf16().flat_map(|u| u.to_be_bytes()));
        let cases: Vec<(&str, Vec<u8>, EncodingTag, LineEndingTag)> = vec![
            ("bom.txt", b"\xEF\xBB\xBFone\r\ntwo\r\n".to_vec(), EncodingTag::Utf8Bom, LineEndingTag::Crlf),
            ("le.txt", utf16le, EncodingTag::Utf16Le, LineEndingTag::Crlf),
            ("be.txt", utf16be, EncodingTag::Utf16Be, LineEndingTag::Lf),
            ("latin1.txt", b"caf\xE9\nna\xEFve\n".to_vec(), EncodingTag::Latin1, LineEndingTag::Lf),
            ("mixed.txt", b"a\r\nb\nc\rd".to_vec(), EncodingTag::Utf8, LineEndingTag::Crlf),
        ];
        for (name, bytes, enc, eol) in cases {
            let path = dir.join(name);
            assert!(fs::write(&path, &bytes).is_ok());
            let mut e = entry(Uuid::new_v4(), Some(path), false);
            e.encoding = enc;
            e.line_ending = eol;
            let tab = restored(&e, dir.path());
            assert!(!tab.buffer.read_only, "{name}");
            assert_eq!(saved_bytes(&tab), bytes, "{name}: save changed the bytes");
        }
    }

    #[test]
    fn undecodable_clean_file_reopens_read_only() {
        let dir = TempDir::new("session-restore-lossy");
        let path = dir.join("bad16.txt");
        // UTF-16LE with a lone high surrogate.
        assert!(fs::write(&path, [0xFF, 0xFE, 0x00, 0xD8, 0x41, 0x00]).is_ok());
        let tab = restored(&entry(Uuid::new_v4(), Some(path), false), dir.path());
        assert!(tab.buffer.read_only);
    }

    #[test]
    fn folders_and_missing_files_are_skipped() {
        let dir = TempDir::new("session-restore-skip");
        let folder = dir.join("folder.txt");
        assert!(fs::create_dir(&folder).is_ok());
        for path in [folder, dir.join("gone.txt")] {
            let e = entry(Uuid::new_v4(), Some(path), false);
            assert!(matches!(restore_tab_in(&e, dir.path()), RestoreOutcome::Skipped(_)));
        }
    }

    #[test]
    fn damaged_autosave_is_restored_not_dropped() {
        let dir = TempDir::new("session-restore-bak");
        let id = Uuid::new_v4();
        assert!(fs::write(bak_path_in(dir.path(), id), b"draft \xFF text").is_ok());
        let tab = restored(&entry(id, None, true), dir.path());
        assert_eq!(tab.buffer.rope.to_string(), "draft \u{FFFD} text");
        assert!(tab.buffer.dirty);
    }

    #[test]
    fn saved_caret_and_scroll_are_made_safe() {
        // Regression: the caret was clamped to the length but not to a
        // char boundary, so a file that changed under a saved caret put
        // it inside a character.
        let dir = TempDir::new("session-restore-caret");
        let path = dir.join("e.txt");
        assert!(fs::write(&path, "é").is_ok());
        let mut e = entry(Uuid::new_v4(), Some(path), false);
        e.cursor_head = 1;
        e.cursor_anchor = usize::MAX;
        e.scroll_top = f32::NAN;
        e.scroll_left = f32::INFINITY;
        let tab = restored(&e, dir.path());
        assert_eq!((tab.selection.anchor, tab.selection.head), (2, 0));
        assert_eq!(f32::from(tab.scroll_top), 0.0);
        assert_eq!(f32::from(tab.scroll_left), 0.0);
    }

    #[test]
    fn undo_recorded_on_other_text_is_not_restored() {
        let dir = TempDir::new("session-restore-undo");
        let path = dir.join("u.txt");
        assert!(fs::write(&path, "éé").is_ok());
        let id = Uuid::new_v4();
        let stale = vec![UndoEntry::Single(EditOp {
            byte_start: 1,
            removed: String::new(),
            inserted: "xy".into(),
            sel_before_anchor: 1,
            sel_before_head: 1,
        })];
        assert!(fs::write(undo_path_in(dir.path(), id), serialize_undo_stack(&stale)).is_ok());
        let tab = restored(&entry(id, Some(path), false), dir.path());
        assert!(tab.undo_stack.is_empty());
    }

    fn write_manifest_text(dir: &TempDir, text: &str) -> (PathBuf, PathBuf) {
        let manifest = dir.join("session.toml");
        let baks = dir.join("session");
        assert!(fs::create_dir_all(&baks).is_ok());
        assert!(fs::write(&manifest, text).is_ok());
        (manifest, baks)
    }

    #[test]
    fn one_bad_entry_does_not_lose_the_session() {
        // Regression: one value the schema rejected made the whole manifest
        // unreadable, the window opened empty, and the first save reaped
        // every autosave the manifest pointed at.
        let dir = TempDir::new("session-salvage");
        let good = Uuid::new_v4();
        let bent = Uuid::new_v4();
        let orphan = Uuid::new_v4();
        let text = format!(
            "active = 1\nword_wrap = true\n\n\
             [[tabs]]\nid = \"{good}\"\ndirty = true\nnew_label = \"new 1\"\n\n\
             [[tabs]]\nid = \"{bent}\"\ndirty = true\ncursor_head = -4\nline_ending = \"sideways\"\n\n\
             [[tabs]]\ndirty = true\n"
        );
        let (manifest_path, baks) = write_manifest_text(&dir, &text);
        assert!(toml::from_str::<SessionManifest>(&text).is_err());
        assert!(fs::write(bak_path_in(&baks, good), "kept").is_ok());
        assert!(fs::write(bak_path_in(&baks, bent), "bent").is_ok());
        assert!(fs::write(bak_path_in(&baks, orphan), "only copy").is_ok());

        let Some(m) = load_manifest_at(&manifest_path, &baks) else {
            panic!("session lost");
        };
        assert!(m.word_wrap);
        assert_eq!(m.active, 1);
        let ids: Vec<Uuid> = m.tabs.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![good, bent, orphan]);
        assert_eq!(m.tabs.get(1).map(|t| t.cursor_head), Some(0));
        let recovered = m.tabs.get(2).map(|t| (t.dirty, t.path.clone(), t.new_label.clone()));
        assert_eq!(recovered, Some((true, None, Some("new 2".to_string()))));
        let Some(t) = m.tabs.get(2) else { panic!("no recovered tab") };
        assert_eq!(restored(t, &baks).buffer.rope.to_string(), "only copy");
        assert!(manifest_path.exists(), "a salvageable manifest isn't moved");
    }

    #[test]
    fn unreadable_manifest_is_kept_aside_and_autosaves_recovered() {
        let dir = TempDir::new("session-quarantine");
        let (manifest_path, baks) = write_manifest_text(&dir, "[[tabs]\nid = ");
        let orphan = Uuid::new_v4();
        assert!(fs::write(bak_path_in(&baks, orphan), "draft").is_ok());
        assert!(fs::write(bak_path_in(&baks, Uuid::new_v4()), "").is_ok());
        let Some(m) = load_manifest_at(&manifest_path, &baks) else {
            panic!("autosave lost");
        };
        assert_eq!(m.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), vec![orphan]);
        assert!(dir.join("session.toml.corrupt").exists());
    }

    #[test]
    fn no_manifest_and_no_autosaves_is_no_session() {
        let dir = TempDir::new("session-none");
        let baks = dir.join("session");
        assert!(load_manifest_at(&dir.join("session.toml"), &baks).is_none());
        // A manifest that reads cleanly comes back untouched.
        let (manifest_path, baks) = write_manifest_text(&dir, "active = 0\n");
        let m = load_manifest_at(&manifest_path, &baks);
        assert!(matches!(m, Some(ref m) if m.tabs.is_empty()));
    }
}
