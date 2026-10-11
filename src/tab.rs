use gpui::{Pixels, px};
use uuid::Uuid;

use crate::buffer::{BlockSelection, Buffer, EditTransaction, Selection, UndoEntry};
use crate::workspace::WrapCache;

/// One tab — its own buffer, selection, undo history, vertical-movement
/// column memory, and scroll position. Multiple tabs live inside a single
/// `Workspace`; switching tabs preserves where each was scrolled to.
pub(crate) struct Tab {
    pub(crate) buffer: Buffer,
    pub(crate) selection: Selection,
    /// Optional Notepad++ column-mode selection. When `Some`, paint and
    /// edit funnels switch to the rectangular path; the linear `selection`
    /// is treated as collapsed at the block's head.
    pub(crate) block_selection: Option<BlockSelection>,
    pub(crate) desired_col_x: Option<Pixels>,
    pub(crate) undo_stack: Vec<UndoEntry>,
    pub(crate) redo_stack: Vec<UndoEntry>,
    /// Open edit-group transaction. While `Some`, every per-edit op is
    /// appended here instead of pushed to `undo_stack` as a fresh entry.
    /// Used by Replace All so undoing reverses every replacement at once.
    pub(crate) edit_transaction: Option<EditTransaction>,
    pub(crate) scroll_top: Pixels,
    pub(crate) scroll_left: Pixels,
    /// Stable identity used to name the autosave `.bak` file. Generated on
    /// construction so per-tab autosave can write to the same path across
    /// every keystroke and edit, without re-snapshotting the whole manifest.
    pub(crate) session_id: Uuid,
    /// Cached visual-row geometry for word wrap. `None` when wrap is off,
    /// or when the cache is stale (viewport width / font size / rope
    /// content changed) and needs rebuilding. Lazily filled in
    /// `EditorElement::prepaint` on the next frame.
    pub(crate) wrap_cache: Option<WrapCache>,
    /// NPP Begin/End Select marker — first press stashes the caret offset
    /// here, second press extends the selection from the stashed offset to
    /// the current caret and clears the marker. Strictly in-memory; not
    /// persisted to `session.toml`. Shifted on edits via
    /// `Workspace::apply_edit` to stay glued to its character.
    pub(crate) begin_select_marker: Option<usize>,
    /// Render mode for this tab. `Edit` shows the standard editor pane;
    /// `MarkdownPreview` swaps in the rendered markdown view (toolbar
    /// toggle, only enabled when the tab's file extension is `.md`).
    pub(crate) view_mode: TabViewMode,
    /// Recovered inline text awaiting the grid's lazy construction.
    pub(crate) csv_draft: Option<crate::session::CsvDraft>,
}

/// How a tab is currently being displayed. Per-tab so toggling preview on
/// one markdown file doesn't flip every other tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TabViewMode {
    Edit,
    MarkdownPreview,
    /// Rendered Doc View for rich-document buffers (Phase 1: DOCX).
    /// Raw View for rich-doc buffers reuses `Edit` (the rope already holds the raw XML).
    RichDocView,
    /// Spreadsheet-style grid view for CSV/TSV buffers. Source view reuses
    /// `Edit` (the rope already holds the delimited text). Default landing
    /// mode for `.csv` / `.tsv` files on open.
    CsvPreview,
}

impl Tab {
    pub(crate) fn welcome() -> Self {
        Self {
            buffer: Buffer::untitled(),
            selection: Selection::default(),
            block_selection: None,
            desired_col_x: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            edit_transaction: None,
            scroll_top: px(0.),
            scroll_left: px(0.),
            session_id: Uuid::new_v4(),
            wrap_cache: None,
            begin_select_marker: None,
            view_mode: TabViewMode::Edit,
            csv_draft: None,
        }
    }

    pub(crate) fn from_closed(closed: ClosedTab) -> Self {
        let mut tab = Self::empty();
        tab.buffer = closed.buffer;
        tab.session_id = closed.session_id;
        tab.view_mode = if tab.buffer.rich_doc.is_some() {
            TabViewMode::RichDocView
        } else if crate::ui::csv_doc::is_csv_path(tab.buffer.file_path.as_deref()) {
            TabViewMode::CsvPreview
        } else {
            TabViewMode::Edit
        };
        tab.buffer.reparse_highlights();
        tab
    }

    pub(crate) fn empty() -> Self {
        Self {
            buffer: Buffer::empty(),
            selection: Selection::default(),
            block_selection: None,
            desired_col_x: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            edit_transaction: None,
            scroll_top: px(0.),
            scroll_left: px(0.),
            session_id: Uuid::new_v4(),
            wrap_cache: None,
            begin_select_marker: None,
            view_mode: TabViewMode::Edit,
            csv_draft: None,
        }
    }
}

/// A tab that was just closed, kept around so `Cmd/Ctrl+Shift+T` can bring
/// it back. Selection and undo history are intentionally not preserved —
/// matches browser-style "restore tab" semantics.
pub(crate) struct ClosedTab {
    pub(crate) buffer: Buffer,
    pub(crate) session_id: Uuid,
}

/// User's choice from the dirty-tab close prompt.
pub(crate) enum CloseDecision {
    Save,
    DontSave,
    Cancel,
}

/// Hard cap on remembered closed tabs. Notepad++'s exact value isn't in the
/// shortcut map; 20 keeps a session's worth of recents without unbounded
/// growth from a long-running window.
pub(crate) const CLOSED_TAB_LIMIT: usize = 20;

/// Which input field typed text should land in. Resolved per-call from focus
/// state so we never have a stale cached value to keep in sync.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputFocus {
    Editor,
    FindQuery,
    Replacement,
    Goto,
    /// Search field of the Find-in-Files dialog tab.
    FifQuery,
    /// Replacement field of the Find-in-Files dialog tab.
    FifReplacement,
    /// File-filter field of the Find-in-Files dialog tab.
    FifFilter,
    /// Directory text field of the Find-in-Files dialog tab.
    FifDirectory,
    /// Search field of the Mark dialog tab.
    MarkQuery,
    /// Inline macro prompt bar — accepts a count or a name depending on
    /// `Workspace::macro_prompt.mode`.
    MacroPrompt,
    /// Inline tab-rename bar — text input that holds the new basename for
    /// the file being renamed via the tab right-click menu.
    TabRename,
    /// Inline rich-doc link-prompt bar — URL field shown above the editor
    /// when the user clicks the toolbar Link button in Doc View. On Apply
    /// the typed URL is committed to the active block via `Edit::SetBlockLink`.
    RichDocLinkUrl,
    /// Inline rich-doc color-picker bar — hex field shown above the editor
    /// when the user clicks the toolbar Color button in Doc View. Swatch
    /// clicks write into the same field; on Apply / hex-edit, the buffer
    /// commits to the active block via `Edit::SetBlockColor`.
    RichDocColorHex,
    /// Active editable paragraph inside the rich-doc Doc View. Typing /
    /// paste / IME flow into the paragraph's runs via
    /// `Edit::InsertText` / `Edit::DeleteRange`. Phase 2g.A — single-block
    /// scope; cross-block selection lands in Phase 2g.B.
    RichDocBody,
    /// Inline folder-panel new-file / new-folder draft. Renders as an extra
    /// tree row inside the parent directory; Enter creates the entry on disk,
    /// Esc cancels.
    FolderInlineCreate,
    /// Active CSV grid cell editor. Typing / paste / IME route into the
    /// per-cell edit buffer on the active tab's `CsvModel`; commit on Enter
    /// or Tab serializes the model back to the rope.
    CsvCell,
    /// CSV grid container has focus but no cell is being edited. Arrow
    /// keys move the active cell, F2 / typed chars open the editor, Esc
    /// clears in-flight drag / selection, Cmd+A selects all rows. Used
    /// to redirect workspace key actions away from the underlying rope's
    /// caret movements while the user is browsing the grid.
    CsvGrid,
    /// CSV find-bar query input. Typed chars append to `find_query`,
    /// backspace pops the last grapheme, Esc closes the bar. Enter +
    /// Shift-Enter step through matches.
    CsvFindBar,
    /// CSV find-bar replace input. Only present when `find_show_replace`
    /// is true. Enter replaces current match + advances; Shift+Enter
    /// (or Cmd+Enter) replaces all matches in one undo grain.
    CsvReplaceBar,
}
