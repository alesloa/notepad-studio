use std::ops::Range;

/// Which top-level tab the Find dialog is currently showing. Persisted across
/// dialog close/reopen so the user lands back on the tab they last used.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogTab {
    Find,
    Replace,
    FindInFiles,
    /// NPP's "Find in Projects" — same engine as FIF but the scope is
    /// the folder-panel root(s) instead of an explicit Directory. Sharing
    /// state with FIF mirrors NPP's dialog (same query/replacement/
    /// filter/options carry across the two tabs).
    FindInProject,
    Mark,
}

/// State shared by Find and Replace tabs of the dialog. Borrowed across the
/// active editor tab — query and toggles persist when switching editor tabs,
/// matches are recomputed against the current buffer.
pub(crate) struct FindState {
    /// Mirror of "the Find or Replace tab of the dialog is currently visible".
    /// Drives the editor's match-highlight painting code path so the existing
    /// `editor_element` highlight branch (which gates on this flag) keeps
    /// behaving the same way it did when the inline find bar was the only
    /// surface. Owned by the dialog's open/close path.
    pub(crate) open: bool,
    pub(crate) query: String,
    pub(crate) query_cursor: usize,
    /// Selection anchor for the query input. When `query_anchor != query_cursor`,
    /// the bytes between them are the active selection (used by Cmd+C / Cut /
    /// type-to-replace and rendered with a highlight rect under the text).
    pub(crate) query_anchor: usize,
    pub(crate) replacement: String,
    pub(crate) replacement_cursor: usize,
    pub(crate) replacement_anchor: usize,
    pub(crate) match_case: bool,
    pub(crate) whole_word: bool,
    pub(crate) wrap_around: bool,
    pub(crate) regex: bool,
    /// Notepad++'s "Extended" mode — `\n`, `\r`, `\t`, `\0`, `\\` literal
    /// escapes are interpreted before the search runs. Mutually-exclusive
    /// with `regex` in Notepad++; we mirror that here.
    pub(crate) extended: bool,
    /// Notepad++'s "Backward direction" toggle. When true, Find Next walks
    /// backward through `matches` (and Find Prev walks forward) — both
    /// navigation handlers invert direction so the persistent radio behaves
    /// like the NPP Up/Down direction radio.
    pub(crate) backward: bool,
    /// Notepad++'s "In selection" toggle — when set, refresh_matches scopes
    /// the regex scan to `in_selection_scope` (captured at toggle time).
    pub(crate) in_selection: bool,
    /// Captured selection range when `in_selection` toggles ON. Persists
    /// across query keystrokes and Replace All so the scope doesn't drift
    /// after the cursor jumps to a match (which would otherwise collapse
    /// the live selection to a single match's width). `None` means either
    /// `in_selection` is off, the selection was empty/block at toggle time,
    /// or the user toggled off — in which case refresh_matches falls back
    /// to a full-buffer scan even with the flag on.
    pub(crate) in_selection_scope: Option<Range<usize>>,
    /// Notepad++'s "`. matches newline`" checkbox under the Regular expression
    /// radio. Maps directly onto `regex::RegexBuilder::dot_matches_new_line`
    /// when `regex` is on. UI-only when `regex` is off (we still record the
    /// flag so flipping the radio doesn't lose the user's choice).
    pub(crate) dot_matches_newline: bool,
    /// Byte ranges of every match in the active buffer. For plain-text tabs
    /// these are byte offsets into the rope; for rich-doc tabs they are byte
    /// offsets into the per-block joined flat text returned by
    /// `RichDocPayload::flat_search_text` — see `rich_doc_block_offsets`.
    pub(crate) matches: Vec<Range<usize>>,
    /// When the active tab is a rich-doc, this carries the block→flat-offset
    /// map captured at `refresh_matches` time. `Some(_)` means `matches`
    /// indexes the joined flat text; navigation / replace / highlight all
    /// branch on this. `None` for plain-text tabs.
    pub(crate) rich_doc_block_offsets: Option<Vec<(usize, usize, usize)>>,
    /// Index into `matches` for the one the cursor is currently on.
    /// `None` when there are no matches.
    pub(crate) current: Option<usize>,
    /// Set when the regex (or whole-word regex) failed to compile, or any
    /// other status string the dialog footer needs to surface.
    pub(crate) error: Option<String>,
    /// One-line success status — "Replaced 12 occurrences", "8 matches" — for
    /// the dialog footer. Cleared on the next query mutation so it doesn't
    /// linger past relevance.
    pub(crate) status: Option<String>,
}

impl FindState {
    pub(crate) fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            query_cursor: 0,
            query_anchor: 0,
            replacement: String::new(),
            replacement_cursor: 0,
            replacement_anchor: 0,
            match_case: false,
            whole_word: false,
            wrap_around: true,
            regex: false,
            extended: false,
            backward: false,
            in_selection: false,
            in_selection_scope: None,
            dot_matches_newline: false,
            matches: Vec::new(),
            rich_doc_block_offsets: None,
            current: None,
            error: None,
            status: None,
        }
    }
}

/// Mode of the inline macro prompt bar. `RunCount` accepts a number from
/// the user (defaulting to a sensible value) and runs the current macro
/// that many times. `SaveName` accepts a name string and persists the
/// current macro under that name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacroPromptMode {
    RunCount,
    SaveName,
}

/// State for the inline macro prompt bar — shown above the editor pane when
/// the user invokes "Run a Macro Multiple Times..." or "Save Currently
/// Recorded Macro...". Mirrors the goto-line bar plumbing.
pub(crate) struct MacroPromptState {
    pub(crate) mode: MacroPromptMode,
    pub(crate) text: String,
    pub(crate) text_cursor: usize,
    pub(crate) text_anchor: usize,
}

impl MacroPromptState {
    pub(crate) fn new(mode: MacroPromptMode, initial: &str) -> Self {
        let len = initial.len();
        Self {
            mode,
            text: initial.to_string(),
            text_cursor: len,
            text_anchor: 0,
        }
    }
}

/// State for the inline tab-rename bar. Shown above the editor when the
/// user picks Rename from the tab right-click menu — the tab whose file is
/// being renamed is `target_tab_idx`. On Enter the workspace runs
/// `rename_tab_file` and clears this state; Esc just clears.
pub(crate) struct TabRenameState {
    pub(crate) target_tab_idx: usize,
    pub(crate) text: String,
    pub(crate) text_cursor: usize,
    pub(crate) text_anchor: usize,
}

impl TabRenameState {
    pub(crate) fn new(target: usize, initial: &str) -> Self {
        let len = initial.len();
        Self {
            target_tab_idx: target,
            text: initial.to_string(),
            text_cursor: len,
            text_anchor: 0,
        }
    }
}

/// Whether the inline-create draft is making a file or a folder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FolderInlineCreateKind {
    File,
    Folder,
}

/// State for the folder-panel inline-create draft. Rendered as an extra row
/// inside `parent_dir` (the row appears at the top of that directory's
/// children, indented one level deeper than the parent). On Enter the
/// workspace creates the file/folder on disk and opens it in a tab (file
/// only); on Esc the draft is dropped without touching the filesystem.
pub(crate) struct FolderInlineCreateState {
    pub(crate) parent_dir: std::path::PathBuf,
    pub(crate) kind: FolderInlineCreateKind,
    pub(crate) text: String,
    pub(crate) text_cursor: usize,
    pub(crate) text_anchor: usize,
    /// Last validation error from a failed Enter — rendered inline so the
    /// user sees why their input was rejected without the draft closing.
    pub(crate) error: Option<String>,
}

impl FolderInlineCreateState {
    pub(crate) fn new(parent_dir: std::path::PathBuf, kind: FolderInlineCreateKind) -> Self {
        Self {
            parent_dir,
            kind,
            text: String::new(),
            text_cursor: 0,
            text_anchor: 0,
            error: None,
        }
    }
}

/// State for the inline rich-doc link-prompt bar. Shown above the editor
/// when the user clicks the toolbar Link button while a rich-doc block is
/// active. On Apply, the typed URL replaces the link on every run of the
/// target block; on Unlink, the link is cleared.
pub(crate) struct RichDocLinkBarState {
    /// Index into `RichDocPayload::ast.blocks` — the block this bar will
    /// commit to. Captured at open time so a stray click on a different
    /// block while the bar is up doesn't redirect the apply.
    pub(crate) target_block: usize,
    pub(crate) text: String,
    pub(crate) text_cursor: usize,
    pub(crate) text_anchor: usize,
    /// Captured at open time: true when the active block already had a
    /// link. Drives whether the "Unlink" button renders.
    pub(crate) had_link: bool,
}

impl RichDocLinkBarState {
    pub(crate) fn new(target_block: usize, initial: &str, had_link: bool) -> Self {
        let len = initial.len();
        Self {
            target_block,
            text: initial.to_string(),
            text_cursor: len,
            text_anchor: 0,
            had_link,
        }
    }
}

/// State for the inline rich-doc color-picker bar. Shown above the editor
/// when the user clicks the toolbar Color button while a rich-doc block is
/// active. The hex buffer drives the live preview — every keystroke or
/// swatch click writes into `text` and applies `Edit::SetBlockColor` so
/// the user sees the result without a separate Apply step. Clear button
/// strips the color; Close button leaves whatever the last value was.
pub(crate) struct RichDocColorBarState {
    pub(crate) target_block: usize,
    pub(crate) text: String,
    pub(crate) text_cursor: usize,
    pub(crate) text_anchor: usize,
    /// Original color of the block at open time. Used by Cancel-equivalent
    /// behavior if we ever add one — currently kept so the renderer can
    /// show "was" vs "new" swatches.
    pub(crate) original_color: Option<u32>,
    /// "More colors…" popover open state. Drops a wider hue × lightness
    /// grid + greyscale strip below the bar so users can pick something
    /// outside the 12-swatch quick row without leaving for the Style
    /// Configurator. State lives on the bar so the popover closes when the
    /// bar closes (Esc / Clear / × all dismiss both surfaces in one shot).
    pub(crate) more_colors_open: bool,
    /// HSV decomposition of `text` for the shared HSV picker surface
    /// (`style_configurator::hsv_surface`) embedded in the More-colours
    /// popover. Lives on the bar so it survives popover open/close and
    /// stays in sync with the source-of-truth hex via `sync_from_buffer`.
    pub(crate) picker_hue: f32,
    pub(crate) picker_sat: f32,
    pub(crate) picker_val: f32,
    pub(crate) picker_hsv_drag: crate::ui::style_configurator::hsv_surface::HsvDrag,
}

impl RichDocColorBarState {
    pub(crate) fn new(target_block: usize, initial_hex: &str, original_color: Option<u32>) -> Self {
        let len = initial_hex.len();
        Self {
            target_block,
            text: initial_hex.to_string(),
            text_cursor: len,
            text_anchor: 0,
            original_color,
            more_colors_open: false,
            picker_hue: 0.0,
            picker_sat: 0.0,
            picker_val: 0.0,
            picker_hsv_drag: crate::ui::style_configurator::hsv_surface::HsvDrag::None,
        }
    }
}

/// State for the Go to Line bar. Lives in its own struct so the find bar
/// keeps its query when goto opens and closes. The Go-to-line input remains
/// inline above the editor; only Find/Replace/FIF/Mark moved to the dialog.
pub(crate) struct GotoState {
    pub(crate) open: bool,
    pub(crate) text: String,
    pub(crate) text_cursor: usize,
    pub(crate) text_anchor: usize,
}

impl GotoState {
    pub(crate) fn new() -> Self {
        Self {
            open: false,
            text: String::new(),
            text_cursor: 0,
            text_anchor: 0,
        }
    }
}

/// State for the Mark tab of the dialog. "Mark" in Notepad++ paints a
/// persistent highlight over every occurrence of a query; here it also drives
/// the optional "Bookmark Line" companion that toggles a bookmark on every
/// matched line.
pub(crate) struct MarkState {
    pub(crate) query: String,
    pub(crate) query_cursor: usize,
    pub(crate) query_anchor: usize,
    pub(crate) match_case: bool,
    pub(crate) whole_word: bool,
    pub(crate) regex: bool,
    /// Notepad++'s "Extended" mode for the Mark search box. `\n` `\r` `\t`
    /// `\0` `\\` literal escapes are interpreted before the regex builds.
    /// Mutually-exclusive with `regex` to mirror Notepad++.
    pub(crate) extended: bool,
    /// `. matches newline` companion checkbox under the Regular expression
    /// radio. Threaded into `mark_all`'s `RegexBuilder::dot_matches_new_line`
    /// when `regex` is on.
    pub(crate) dot_matches_newline: bool,
    /// Notepad++'s "Wrap around" toggle. When on, Mark All re-orders matches
    /// to start from the current caret line (so the first highlight a user
    /// sees is the next-after-caret hit) and wraps to the buffer start.
    /// When off, matches are returned in document order.
    pub(crate) wrap_around: bool,
    /// Notepad++'s "Purge for each search". When on, Mark All clears the
    /// existing mark layer before adding the new run's hits. When off, new
    /// hits append to the existing marks (deduped by exact range).
    pub(crate) purge_for_each_search: bool,
    /// When set, `mark_all` also bookmarks every line containing a match.
    pub(crate) bookmark_line: bool,
    /// Mirror of the active buffer's mark layer at the time of the last
    /// "Mark All" run. Source of truth lives on `Buffer::marks` — this stays
    /// in sync after `mark_all`/`mark_clear_all` so the Copy Marked Text
    /// button can pull the same ranges without re-borrowing the active tab.
    pub(crate) marks: Vec<Range<usize>>,
    /// Status string shown in the dialog footer — "12 marked".
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
}

impl MarkState {
    pub(crate) fn new() -> Self {
        // NPP `Highlighting ▸ Style All Occurrences of Token` settings drive
        // the *defaults* of the Mark dialog's match-case + whole-word toggles.
        // The user can still flip them inside the dialog without affecting
        // the persisted preference.
        let snap = crate::settings::snapshot();
        Self {
            query: String::new(),
            query_cursor: 0,
            query_anchor: 0,
            match_case: snap.mark_all_case_sensitive,
            whole_word: snap.mark_all_whole_word,
            regex: false,
            extended: false,
            dot_matches_newline: false,
            wrap_around: true,
            purge_for_each_search: true,
            bookmark_line: false,
            marks: Vec::new(),
            status: None,
            error: None,
        }
    }
}
