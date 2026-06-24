// Session persistence — saves every tab, every dirty buffer, every cursor /
// scroll / bookmark, plus the active theme and closed-tab restore stack.
// Lives at `<config>/Notepad Studio/session.toml` (manifest) and
// `<config>/Notepad Studio/session/<uuid>.bak` (one per dirty / untitled tab).
//
// Atomic write: every save goes to `<path>.tmp` first then renames. A crash
// mid-write leaves the prior session intact.
//
// Crash-recovery falls out for free: clean exits with no remaining dirty
// tabs clear the session dir; otherwise the manifest survives and the next
// launch restores from it.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use gpui::SharedString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::buffer::{Buffer, Selection, deserialize_undo_stack, serialize_undo_stack};
use crate::consts::APP_NAME;
use crate::syntax::LanguageId;
use crate::tab::{ClosedTab, Tab};
use crate::text::encoding::Encoding;
use crate::text::line_ending::LineEnding;
use crate::theme::ThemeMode;

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
    bak_dir().map(|d| d.join(format!("{id}.bak")))
}

fn undo_path(id: Uuid) -> Option<PathBuf> {
    bak_dir().map(|d| d.join(format!("{id}.undo")))
}

// ---- Manifest read / write ------------------------------------------------

pub(crate) fn load_manifest() -> Option<SessionManifest> {
    let path = manifest_path()?;
    let raw = fs::read_to_string(&path).ok()?;
    toml::from_str(&raw).ok()
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

fn atomic_write_string(path: &Path, contents: &str) -> io::Result<()> {
    let mut tmp = path.to_path_buf();
    tmp.set_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.tmp"),
        None => "tmp".into(),
    });
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn atomic_write_bytes(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut tmp = path.to_path_buf();
    tmp.set_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.tmp"),
        None => "tmp".into(),
    });
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn write_manifest(manifest: &SessionManifest) -> io::Result<()> {
    ensure_dirs()?;
    let Some(path) = manifest_path() else {
        return Ok(());
    };
    let serialized = toml::to_string_pretty(manifest)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    atomic_write_string(&path, &serialized)
}

pub(crate) fn write_bak(id: Uuid, contents: &str) -> io::Result<()> {
    ensure_dirs()?;
    let Some(path) = bak_path(id) else {
        return Ok(());
    };
    atomic_write_bytes(&path, contents.as_bytes())
}

pub(crate) fn read_bak(id: Uuid) -> Option<String> {
    let path = bak_path(id)?;
    fs::read_to_string(&path).ok()
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
    atomic_write_bytes(&path, bytes)
}

pub(crate) fn read_undo(id: Uuid) -> Option<Vec<u8>> {
    let path = undo_path(id)?;
    fs::read(&path).ok()
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
        read_only: buffer.read_only,
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
    // Rich-document tabs (DOCX / RTF / ODT / DOC) re-parse from disk on
    // restore. The disk file is a binary container (zip for DOCX/ODT,
    // bespoke encoding for RTF/DOC), so the plain-text restore paths
    // below — `fs::read_to_string` on a clean tab, or the .bak which
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

    let (rope, dirty) = if let Some(p) = rich_doc_payload.as_ref() {
        // Rope mirrors raw_view_text so the Raw View source pane stays
        // in sync with the parsed AST. Clean (no .bak) — re-parsing
        // from disk supersedes any prior dirty state.
        let le = crate::text::line_ending::LineEnding::detect(&p.raw_view_text);
        let normalized =
            crate::text::line_ending::normalize_to_lf(p.raw_view_text.clone(), le);
        (ropey::Rope::from_str(&normalized), false)
    } else if entry.dirty {
        match read_bak(entry.id) {
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
        match fs::read_to_string(path) {
            Ok(s) => {
                let le = entry.line_ending.into_line_ending();
                let normalized = crate::text::line_ending::normalize_to_lf(s, le);
                (ropey::Rope::from_str(&normalized), false)
            }
            Err(err) => {
                return RestoreOutcome::Skipped(format!(
                    "couldn't reopen {}: {err}",
                    path.display()
                ));
            }
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
        line_ending: entry.line_ending.into_line_ending(),
        encoding: entry.encoding.into_encoding(),
        bookmarks,
        marks: Vec::new(),
        language: entry.language,
        highlights: Vec::new(),
        loading: None,
        new_label: entry.new_label.clone().map(SharedString::from),
        read_only: entry.read_only,
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

    let head = entry.cursor_head.min(total);
    let anchor = entry.cursor_anchor.min(total);
    tab.selection = Selection {
        anchor,
        head,
        is_dragging: false,
    };
    tab.scroll_top = gpui::px(entry.scroll_top.max(0.0));
    tab.scroll_left = gpui::px(entry.scroll_left.max(0.0));

    // Restore the per-tab undo history. Rich-doc tabs re-parse from disk so
    // there's no rope-level undo to restore; everything else gets whatever
    // was last persisted alongside the `.bak`. Malformed payloads yield an
    // empty stack rather than failing the restore.
    if !is_rich_doc_restore {
        if let Some(bytes) = read_undo(entry.id) {
            tab.undo_stack = deserialize_undo_stack(&bytes);
        }
    }

    RestoreOutcome::Restored(tab)
}

/// Reconstruct a `ClosedTab` from a manifest entry. Closed-tab content is
/// always in a `.bak` — the closed-tab restore stack carries unsaved drafts
/// across launches the same way the open-tab list does.
pub(crate) fn restore_closed_tab(entry: &SessionClosedTab) -> Option<ClosedTab> {
    let body = read_bak(entry.id)?;
    let buffer = Buffer {
        rope: ropey::Rope::from_str(&body),
        file_path: entry.path.clone(),
        dirty: false,
        line_ending: entry.line_ending.into_line_ending(),
        encoding: entry.encoding.into_encoding(),
        bookmarks: BTreeSet::new(),
        marks: Vec::new(),
        language: entry.language,
        highlights: Vec::new(),
        loading: None,
        new_label: entry.new_label.clone().map(SharedString::from),
        read_only: false,
        tab_color: entry.tab_color,
        is_changelog: false,
        rich_doc: None,
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
    let referenced: BTreeSet<Uuid> = manifest
        .tabs
        .iter()
        .map(|t| t.id)
        .chain(manifest.closed_tabs.iter().map(|c| c.id))
        .collect();
    let Ok(entries) = fs::read_dir(&dir) else {
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

/// Snapshot + write atomically, then prune orphaned `.bak` files so the
/// session dir stays minimal.
pub(crate) fn save_workspace(
    tabs: &[Tab],
    active: usize,
    closed_tabs: &[ClosedTab],
    theme: ThemeMode,
    show_whitespace: bool,
    word_wrap: bool,
    mru: &[Uuid],
) {
    save_workspace_with_policy(
        tabs,
        active,
        closed_tabs,
        theme,
        show_whitespace,
        word_wrap,
        mru,
        BakPolicy::Full,
    );
}

/// Variant of `save_workspace` that lets the caller pick a `BakPolicy`. The
/// eager per-keystroke save in `replace_byte_range` uses
/// `BakPolicy::UntitledOnly` so the manifest is always current without
/// per-keystroke megabyte writes for path-backed dirty buffers.
pub(crate) fn save_workspace_with_policy(
    tabs: &[Tab],
    active: usize,
    closed_tabs: &[ClosedTab],
    theme: ThemeMode,
    show_whitespace: bool,
    word_wrap: bool,
    mru: &[Uuid],
    bak_policy: BakPolicy,
) {
    let manifest = snapshot_workspace_with_policy(
        tabs,
        active,
        closed_tabs,
        theme,
        show_whitespace,
        word_wrap,
        mru,
        bak_policy,
    );
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
        BackupOnSaveMode::None => unreachable!(),
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
