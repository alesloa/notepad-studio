// User-facing preferences. Persisted alongside `window.toml` /
// `recent.toml` / `shortcuts.toml` in `<config>/Notepad Studio/settings.toml`.
//
// Loaded once on demand into a process-global `Mutex<Settings>` (same
// pattern as `recent.rs` and `macros.rs`). The Preferences dialog reads
// the snapshot on open, mutates fields as the user edits, and saves on
// `Apply` / `OK`. Other code (Split Lines, DateTimeCustomized, the Tab
// key inserter) reads via `snapshot()` so callers stay off the lock.
//
// Schema is stable + extension-friendly: every field is `#[serde(default)]`
// so adding a new field doesn't break a settings file written by an older
// build.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::consts::{APP_NAME, TAB_SPACES};
use crate::syntax::LanguageId;
use crate::text::encoding::Encoding;
use crate::text::line_ending::LineEnding;
use crate::theme::{SLOT_COUNT, ThemeMode, TonePalette};

/// NPP `Default Directory ▸ Default Open/Save file Directory ▸ {radio}`. Picks
/// which folder the OS file dialog lands in. `FollowDocument` mirrors the
/// active buffer's directory when the buffer has a path; `RememberLast`
/// remembers the last folder the user actually picked from a dialog;
/// `Custom` always lands in `default_directory_custom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DirectoryMode {
    FollowDocument,
    RememberLast,
    Custom,
}

impl Default for DirectoryMode {
    fn default() -> Self {
        Self::RememberLast
    }
}

/// NPP `Multi-Instance & Date ▸ Multi-instance settings`. Drives whether
/// `single_instance::acquire_or_raise` skips the lock at startup. The third
/// variant is documented for parity but its full session-on-new-instance
/// behaviour is partially deferred — see UNTESTED.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum MultiInstanceMode {
    /// Default — second launch raises the existing window and exits.
    MonoInstance,
    /// Always start a new process; no lock is acquired.
    AlwaysMulti,
    /// Open the saved session in a brand-new process. Treated as
    /// `AlwaysMulti` for now (session-handoff plumbing is deferred).
    NewInstanceForSession,
}

impl Default for MultiInstanceMode {
    fn default() -> Self {
        Self::MonoInstance
    }
}

/// NPP `Backup ▸ Backup on save ▸ {radio}` (`BackupFeature` in
/// `MISC/Common/NppConstants.h:197`). `None` keeps Notepad Studio's current "no
/// `.bak` written next to the file on save" behaviour. `Simple` writes
/// `<file>.bak` next to the saved file (or under `backup_custom_dir` when
/// `backup_use_custom_dir` is set). `Verbose` adds a date-stamped copy
/// instead of overwriting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum BackupOnSaveMode {
    None,
    Simple,
    Verbose,
}

impl Default for BackupOnSaveMode {
    fn default() -> Self {
        Self::None
    }
}

/// NPP `Search Engine ▸ {radio}` (`NppGUI::SearchEngineChoice` in
/// `Parameters.h:861`). Selects the URL template `Edit ▸ Search on Internet`
/// uses to open the browser with the current selection. `Custom` uses
/// `search_engine_custom_url` (with `$(CURRENT_WORD)` substituted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SearchEngineChoice {
    Custom,
    DuckDuckGo,
    Google,
    Bing,
    Yahoo,
    StackOverflow,
}

impl Default for SearchEngineChoice {
    fn default() -> Self {
        // NPP `_searchEngineChoice = se_google;` (Parameters.h:862)
        Self::Google
    }
}

/// NPP `Editing 2 ▸ EOL ▸ {Plain Text / Rounded Rectangle}`
/// (`ScintillaViewParams::crlfMode` in `Parameters.h:905`). Controls how
/// CRLF/LF/CR markers paint when EOL display is enabled. NPP also has
/// custom-color variants; Notepad Studio collapses to the two visible-style
/// variants and exposes the custom-color toggle separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EolDisplayMode {
    PlainText,
    RoundedRect,
}

impl Default for EolDisplayMode {
    fn default() -> Self {
        // NPP `_eolMode = roundedRectangleText;` (Parameters.h:906)
        Self::RoundedRect
    }
}

/// NPP `Editing 2 ▸ Non-Printing Characters ▸ {Abbreviation / Codepoint}`
/// (`ScintillaViewParams::npcMode` in `Parameters.h:908`). NPP also has an
/// `identity` variant (= "show as-is") that's gated off by another path;
/// Notepad Studio only exposes the two user-visible radios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum NpcDisplayMode {
    Abbreviation,
    Codepoint,
}

impl Default for NpcDisplayMode {
    fn default() -> Self {
        // NPP `_npcMode = abbreviation;` (Parameters.h:909)
        Self::Abbreviation
    }
}

/// NPP `Indentation ▸ Auto-indent ▸ {None / Basic / Advanced}`
/// (`AutoIndentMode` in `MISC/Common/NppConstants.h:240`,
/// `NppGUI::_maintainIndent` in `Parameters.h:749`). Drives what fires when
/// the user hits Enter at end-of-line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AutoIndentMode {
    None,
    Basic,
    Advanced,
}

impl Default for AutoIndentMode {
    fn default() -> Self {
        // NPP `_maintainIndent = autoIndent_advanced;` (Parameters.h:749)
        Self::Advanced
    }
}

/// Per-language override for the global `indent_uses_tabs` + `tab_width`
/// pair. Stored on `Settings::language_indent_overrides`; the resolver
/// `indent_for_lang` returns the override if present, otherwise the
/// globals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IndentOverride {
    pub(crate) uses_tabs: bool,
    pub(crate) width: usize,
}

/// NPP `Print ▸ Color Options ▸ {radio}` (`PrintSettings::_printOption` =
/// `SC_PRINT_*` in `Parameters.h:519`). Mirrors the four Scintilla
/// `SC_PRINT_*` modes: WYSIWYG, inverted brightness, force black-on-white,
/// and "colour glyphs but white background" (= "no background colour" in
/// NPP's panel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PrintColorMode {
    Wysiwyg,
    InvertLight,
    BlackOnWhite,
    ColourOnWhite,
}

impl Default for PrintColorMode {
    fn default() -> Self {
        // NPP `_printOption = SC_PRINT_COLOURONWHITE;` (Parameters.h:519)
        Self::ColourOnWhite
    }
}

/// NPP `Auto-Completion ▸ {radio}` (`NppGUI::AutocStatus` enum in
/// `Parameters.h:802`). `FunctionAndWord` matches NPP's `autoc_both`
/// default; `None` means "no auto-completion list shown on input".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AutocompletionMode {
    None,
    Function,
    Word,
    FunctionAndWord,
}

impl Default for AutocompletionMode {
    fn default() -> Self {
        // NPP `_autocStatus = autoc_both;` (Parameters.h:803)
        Self::FunctionAndWord
    }
}

// Defaults. Surfaced as constants so the Preferences dialog can show them
// in placeholder text and the `Reset` button has a fixed source of truth.
pub(crate) const DEFAULT_SPLIT_LINES_EDGE_COLUMN: usize = 80;
pub(crate) const DEFAULT_DATETIME_CUSTOM_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// NPP `Backup ▸ Snapshot ▸ Enable session snapshot and periodic backup`.
/// `_isSnapshotMode = true` in `NppGUI` (Parameters.h:855).
pub(crate) const DEFAULT_BACKUP_PERIODIC_ENABLED: bool = true;

/// NPP `Backup ▸ Snapshot ▸ every N seconds`. `_snapshotBackupTiming = 7000`
/// in `NppGUI` (Parameters.h:856) — milliseconds. The MIN/MAX bounds are
/// surfaced for the panel's numeric input clamp + the
/// `backup_periodic_interval_ms()` runtime helper; they're consumed by the
/// follow-up Backup-panel UI fork (Wave 1 plan in fork report).
pub(crate) const DEFAULT_BACKUP_PERIODIC_INTERVAL_MS: u32 = 7000;
pub(crate) const BACKUP_PERIODIC_INTERVAL_MIN_MS: u32 = 1000;
pub(crate) const BACKUP_PERIODIC_INTERVAL_MAX_MS: u32 = 600_000;

/// NPP `Performance ▸ Enable large file restriction`. `_largeFileSizeDefInByte`
/// = `NPP_STYLING_FILESIZE_LIMIT_DEFAULT` = `200 * 1024 * 1024` bytes
/// (NppConstants.h:290).
pub(crate) const DEFAULT_LARGE_FILE_THRESHOLD_BYTES: u64 = 200 * 1024 * 1024;

/// NPP `Delimiter ▸ Open / Close delimiter` characters. `_leftmostDelimiter`
/// = `'('`, `_rightmostDelimiter` = `')'` in `NppGUI` (Parameters.h:769–770).
pub(crate) const DEFAULT_DELIMITER_LEFT: &str = "(";
pub(crate) const DEFAULT_DELIMITER_RIGHT: &str = ")";

// NPP `IDD_PREFERENCE_BOX ▸ IDC_LIST_DLGTITLE` listbox is 100 DLU wide
// (preference.rc:32). 100 DLU at NPP's MS Shell Dlg 8pt baseline maps to
// ~150 px. Notepad Studio renders Menlo monospace which is ~30% wider than MS
// Shell Dlg variable-width at the same point size, so the equivalent
// visual width is ~195–200 px. 200 also fits the longest NPP category
// labels we'll add later (e.g. "Auto-Completion", "Multi-Instance & Date").
pub(crate) const DEFAULT_PREFERENCES_SIDEBAR_WIDTH: f32 = 200.0;

/// Default category id selected on first dialog open. Matches NPP order
/// (General is the topmost listbox row).
pub(crate) const DEFAULT_PREFERENCES_ACTIVE_PANEL: &str = "general";

/// NPP `Recent Files History ▸ Max. number of entries:` default. NPP caps
/// the input at 30; we surface the same range. 10 was the previous
/// hardcoded `MAX_RECENT` const in `recent.rs`.
pub(crate) const DEFAULT_RECENT_FILES_MAX: u32 = 10;
pub(crate) const RECENT_FILES_MAX_HARD_CAP: u32 = 30;

/// NPP `Editing 1 ▸ Caret Settings ▸ Blink rate:`. Hardcoded as 500 ms in
/// `app/window.rs::spawn_caret_blink` before this knob existed; we keep
/// the same default and bound the user input to a sane range.
pub(crate) const DEFAULT_CARET_BLINK_RATE_MS: u32 = 500;
pub(crate) const CARET_BLINK_RATE_MIN_MS: u32 = 100;
pub(crate) const CARET_BLINK_RATE_MAX_MS: u32 = 2000;

/// NPP `Auto-Completion ▸ From N-th character` (`NppGUI::_autocFromLen` in
/// `Parameters.h:804`). Range 1..=9 in the NPP slider; default = 1. The
/// MIN/MAX bounds are consumed by the follow-up Auto-Completion-panel UI
/// fork (see fork report) — surfaced now so the field validator and the
/// slider widget share one source of truth.
pub(crate) const DEFAULT_AUTOCOMPLETION_FROM_CHARS: u32 = 1;
#[allow(dead_code)]
pub(crate) const AUTOCOMPLETION_FROM_CHARS_MIN: u32 = 1;
#[allow(dead_code)]
pub(crate) const AUTOCOMPLETION_FROM_CHARS_MAX: u32 = 9;

/// NPP `<FindHistory>` block in `config.xml`. Per-input ring buffer of recent
/// values, surfaced via the chevron dropdown on each Find dialog text input.
/// Latest value at index 0; cap [`FIND_HISTORY_MAX`] per slot. Persisted so
/// the dropdown survives relaunch (matches NPP).
pub(crate) const FIND_HISTORY_MAX: usize = 16;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct FindHistory {
    #[serde(default)]
    pub(crate) find_query: Vec<String>,
    #[serde(default)]
    pub(crate) replacement: Vec<String>,
    #[serde(default)]
    pub(crate) fif_query: Vec<String>,
    #[serde(default)]
    pub(crate) fif_replacement: Vec<String>,
    #[serde(default)]
    pub(crate) fif_filter: Vec<String>,
    #[serde(default)]
    pub(crate) fif_directory: Vec<String>,
    #[serde(default)]
    pub(crate) mark_query: Vec<String>,
}

/// Which Find dialog input a history operation targets. Mirrors the subset of
/// `InputFocus` that has a chevron dropdown — Goto, MacroPrompt, etc. don't
/// participate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindHistorySlot {
    FindQuery,
    Replacement,
    FifQuery,
    FifReplacement,
    FifFilter,
    FifDirectory,
    MarkQuery,
}

impl FindHistory {
    pub(crate) fn slot(&self, slot: FindHistorySlot) -> &Vec<String> {
        match slot {
            FindHistorySlot::FindQuery => &self.find_query,
            FindHistorySlot::Replacement => &self.replacement,
            FindHistorySlot::FifQuery => &self.fif_query,
            FindHistorySlot::FifReplacement => &self.fif_replacement,
            FindHistorySlot::FifFilter => &self.fif_filter,
            FindHistorySlot::FifDirectory => &self.fif_directory,
            FindHistorySlot::MarkQuery => &self.mark_query,
        }
    }

    fn slot_mut(&mut self, slot: FindHistorySlot) -> &mut Vec<String> {
        match slot {
            FindHistorySlot::FindQuery => &mut self.find_query,
            FindHistorySlot::Replacement => &mut self.replacement,
            FindHistorySlot::FifQuery => &mut self.fif_query,
            FindHistorySlot::FifReplacement => &mut self.fif_replacement,
            FindHistorySlot::FifFilter => &mut self.fif_filter,
            FindHistorySlot::FifDirectory => &mut self.fif_directory,
            FindHistorySlot::MarkQuery => &mut self.mark_query,
        }
    }

    /// MRU push: if `value` already exists, move it to the front; otherwise
    /// prepend. Empty strings are ignored so an unintentional Enter on a
    /// blank input doesn't pollute the dropdown. Cap at [`FIND_HISTORY_MAX`].
    fn push(&mut self, slot: FindHistorySlot, value: &str) -> bool {
        if value.is_empty() {
            return false;
        }
        let bucket = self.slot_mut(slot);
        if let Some(pos) = bucket.iter().position(|v| v == value) {
            if pos == 0 {
                return false;
            }
            bucket.remove(pos);
        }
        bucket.insert(0, value.to_string());
        if bucket.len() > FIND_HISTORY_MAX {
            bucket.truncate(FIND_HISTORY_MAX);
        }
        true
    }
}

/// Push `value` to the Find history slot's MRU ring and persist the settings
/// snapshot in one step. Skips disk writes when nothing actually changed
/// (value already at the front, or empty).
pub(crate) fn push_find_history(slot: FindHistorySlot, value: &str) {
    let trimmed = value.trim_end_matches('\n');
    if trimmed.is_empty() {
        return;
    }
    let mut next = snapshot();
    if !next.find_history.push(slot, trimmed) {
        return;
    }
    replace_and_save(next);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Settings {
    /// Spaces inserted by the Tab key. NPP `Indentation ▸ Indent Settings ▸
    /// Indent size:`. Migrated from the `TAB_SPACES` const default.
    #[serde(default = "default_tab_width")]
    pub(crate) tab_width: usize,

    /// Column at which `Edit ▸ Line Operations ▸ Split Lines` wraps. NPP
    /// `Margins/Border/Edge ▸ Vertical Edge Settings ▸ IDC_COLUMNPOS_EDIT`
    /// — NPP accepts a comma-separated list; Notepad Studio keeps a single column
    /// for now. Users who don't enable an edge-marker still have a wrap
    /// column.
    #[serde(default = "default_split_lines_edge_column")]
    pub(crate) split_lines_edge_column: usize,

    /// `strftime`-style template used by `Edit ▸ Insert ▸ Date/Time Customized`.
    /// NPP `Multi-Instance & Date ▸ Customize insert Date Time ▸ Custom format:`.
    /// `chrono::format` accepts the same `%Y %m %d %H %M %S` codes as libc's
    /// strftime; defaults to ISO-8601 with seconds for portability.
    #[serde(default = "default_datetime_custom_format")]
    pub(crate) datetime_custom_format: String,

    /// Sidebar width (px) of the Preferences dialog's category list. Saved
    /// when the user drags the divider; restored on dialog reopen and
    /// across app restart.
    #[serde(default = "default_preferences_sidebar_width")]
    pub(crate) preferences_sidebar_width: f32,

    /// Last-selected category in the Preferences sidebar (NPP listbox).
    /// Stored as an id string instead of an index so reordering categories
    /// in code doesn't strand the selection on the wrong panel after upgrade.
    #[serde(default = "default_preferences_active_panel")]
    pub(crate) preferences_active_panel: String,

    /// NPP `General ▸ Status Bar ▸ Hide`. When true, the workspace omits
    /// the bottom status bar from the render tree.
    #[serde(default)]
    pub(crate) status_bar_hidden: bool,

    /// NPP `Recent Files History ▸ Max. number of entries:`. Range 0..=30
    /// (NPP's spinner cap). 0 disables the recents list entirely; 1..=30
    /// trims the MRU queue to that many entries on push and on load.
    #[serde(default = "default_recent_files_max")]
    pub(crate) recent_files_max: u32,

    /// NPP `MISC. ▸ Show only filename in title bar`. When true, the OS
    /// title bar shows the basename of the active tab's file (or its "new
    /// N" label) instead of the full absolute path.
    #[serde(default)]
    pub(crate) title_bar_filename_only: bool,

    /// NPP `General ▸ Tab Bar ▸ Hide`. When true, the tab strip is omitted.
    #[serde(default)]
    pub(crate) tab_bar_hidden: bool,

    /// NPP `General ▸ Toolbar ▸ Hide`. When true, the icon toolbar above the
    /// tab strip is omitted from the render tree.
    #[serde(default)]
    pub(crate) toolbar_hidden: bool,

    /// NPP `General ▸ Tab Bar ▸ Show close button on each tab`. Gates the
    /// per-tab `×` glyph render.
    #[serde(default = "default_true")]
    pub(crate) tab_close_button_visible: bool,

    /// NPP `General ▸ Tab Bar ▸ Double click to close document`. When true,
    /// a left-button double-click on a tab closes it (with the usual dirty
    /// prompt).
    #[serde(default)]
    pub(crate) tab_double_click_to_close: bool,

    /// NPP `General ▸ Tab Bar ▸ Reduce`. Renders the tab strip at a smaller
    /// row height for users who want more vertical space for the editor.
    #[serde(default)]
    pub(crate) tab_reduce: bool,

    /// NPP `General ▸ Tab Bar ▸ Draw a coloured bar on active tab`. When
    /// true, paints a thin theme-accent line at the bottom edge of the
    /// active tab.
    #[serde(default = "default_true")]
    pub(crate) tab_active_bar: bool,

    /// NPP `General ▸ Tab Bar ▸ Darken inactive tabs`. When true (default,
    /// matches current Notepad Studio behaviour) inactive tabs render against
    /// `theme.chrome`; when false they share the active-tab background so
    /// only the coloured bar / dirty dot distinguishes the active tab.
    #[serde(default = "default_true")]
    pub(crate) tab_darken_inactive: bool,

    /// NPP `General ▸ Tab Bar ▸ Exit on close the last tab`. When true
    /// (default, matches current Notepad Studio behaviour), removing the last tab
    /// closes the window. When false, the window stays open with a fresh
    /// blank tab in place of the just-closed one.
    #[serde(default = "default_true")]
    pub(crate) tab_exit_on_last_close: bool,

    /// NPP `General ▸ Tab Bar ▸ Lock (no drag and drop)`. When true, tab
    /// drag-reorder is disabled — dragging a tab does nothing.
    #[serde(default)]
    pub(crate) tab_lock_drag: bool,

    /// NPP `General ▸ Tab Bar ▸ Multi-line`. When true, tabs wrap to
    /// additional rows once they overflow the strip width instead of
    /// clipping at the right edge. Each row is one strip-height tall;
    /// the `+` button trails the last tab and wraps with the others.
    #[serde(default)]
    pub(crate) tab_multi_line: bool,

    /// NPP `General ▸ Tab Bar ▸ Vertical`. When true, the tab strip
    /// renders as a fixed-width column on the LEFT side of the editor
    /// body (between the folder panel and the editor pane) instead of
    /// a horizontal band above the editor. Vertical mode supersedes
    /// `tab_multi_line` — a column doesn't wrap. Width is fixed.
    #[serde(default)]
    pub(crate) tab_vertical: bool,

    /// NPP `Editing 1 ▸ Caret Settings ▸ Blink rate:` (ms). Drives the
    /// `gpui::Timer` cadence in `app/window.rs::spawn_caret_blink`. Bounded
    /// to `CARET_BLINK_RATE_MIN_MS..=CARET_BLINK_RATE_MAX_MS` on read so a
    /// hand-edited TOML never starves the timer.
    #[serde(default = "default_caret_blink_rate_ms")]
    pub(crate) caret_blink_rate_ms: u32,

    /// NPP `Dark Mode ▸ Light/Dark/Follow Windows`. `FollowSystem` resolves
    /// against `gpui::Window::appearance()` and is observed live via
    /// `cx.observe_window_appearance`. On first launch, new windows pick
    /// this mode; the existing session-manifest theme on disk wins on
    /// restore so users who toggled with `Cmd/Ctrl+Shift+T` before this
    /// knob shipped don't see their preference reset.
    #[serde(default = "default_theme_mode")]
    pub(crate) theme_mode: ThemeMode,

    /// Active Style Configurator theme. `None` resolves to the synthetic
    /// "Default" entry (`<userdata>/stylers.xml`). When `Some(path)`, the
    /// dialog reopens onto that theme; if the path no longer exists we
    /// fall back to "Default".
    #[serde(default)]
    pub(crate) theme_path: Option<PathBuf>,

    /// NPP `Dark Mode ▸ Tones`. Selects one of seven preset tinted dark
    /// palettes (Black/Red/Green/Blue/Purple/Cyan/Olive) or `Customized`,
    /// which uses `customized_slots` for per-slot overrides. Light mode
    /// ignores this field — the palette only layers onto the Dark theme.
    #[serde(default)]
    pub(crate) tone_palette: TonePalette,

    /// NPP `Dark Mode ▸ Tones ▸ {12 customized slot checkboxes}`. Each
    /// `Some(value)` is a 0xRRGGBB override for the corresponding slot in
    /// `theme::PaletteColors`; `None` falls through to the Black tone's
    /// value. Only consulted when `tone_palette == TonePalette::Customized`.
    /// Stored as `Vec<Option<u32>>` because `serde` doesn't derive
    /// `Deserialize` for fixed-size arrays larger than 32 by default;
    /// converted to/from `[Option<u32>; SLOT_COUNT]` via `customized_slots()`.
    ///
    /// Custom serde adapter: TOML rejects `null` inside arrays, so the
    /// vanilla `Vec<Option<u32>>` would make the WHOLE settings.toml write
    /// silently fail (`toml::to_string_pretty` returns `Err` and
    /// `save_to_disk` swallows it). We round-trip None as `0xFF00_0000`
    /// — same sentinel NPP uses in stylers.xml — so the on-disk array is
    /// always a flat `[u32; 12]` that TOML can serialize.
    #[serde(default = "default_customized_slots_vec", with = "customized_slots_serde")]
    pub(crate) customized_slots: Vec<Option<u32>>,

    /// NPP `Margins/Border/Edge ▸ Display bookmark`. Gates the filled-circle
    /// glyph painted in the gutter for `F2`-bookmarked lines (`gutter.rs`).
    #[serde(default = "default_true")]
    pub(crate) show_bookmark_glyph: bool,

    /// NPP `Margins/Border/Edge ▸ Line Number ▸ Display`. When false, the
    /// gutter still reserves a small left padding for bookmark glyphs but
    /// no line-number text is painted.
    #[serde(default = "default_true")]
    pub(crate) show_line_numbers: bool,

    /// NPP `Margins/Border/Edge ▸ Line Number ▸ Dynamic width`. When true
    /// (default — NPP's default too), the gutter widens to fit the digit
    /// count of `total_lines` so the rightmost line number always fits.
    /// When false, the gutter holds a constant width matching what 5 digits
    /// would need — keeps the editor pane from reflowing every time the
    /// digit count crosses a power of 10.
    #[serde(default = "default_true")]
    pub(crate) line_number_width_dynamic: bool,

    /// NPP `Indentation ▸ Indent Settings ▸ Tab character / Space character(s)`.
    /// When true, the Tab key inserts a literal `\t`; when false (default,
    /// matches current behaviour), it inserts `tab_width` spaces. Only the
    /// Tab key insert path is wired — the older TAB→Space converter / new-line
    /// auto-indent paths still use the spaces-only model from `consts::TAB_SPACES`.
    #[serde(default)]
    pub(crate) indent_uses_tabs: bool,

    /// NPP `MISC. ▸ Enable Save All confirm dialog`. When true, `Save All`
    /// (`Cmd-Opt-S` / `Ctrl-Shift-S`) shows a Yes/No prompt before iterating
    /// over every dirty path-backed buffer. Default: false (no prompt) to
    /// match Notepad Studio's existing zero-friction Save All behaviour.
    #[serde(default)]
    pub(crate) save_all_confirm: bool,

    /// NPP `Searching ▸ Replace: Don't move to the following occurrence`.
    /// When true, Replace leaves the current-match highlight on the same
    /// logical position instead of advancing to the next match (the cursor
    /// itself still lands at the end of the just-replaced range — that's
    /// the gpui caret model). Default: false (existing advance behaviour).
    #[serde(default)]
    pub(crate) replace_dont_move_to_next: bool,

    /// NPP `Searching ▸ Confirm Replace All in All Opened Documents`. When
    /// true, `Replace All in All Opened` shows a Yes/No prompt before
    /// rewriting every match across every open buffer. Default: true
    /// (the destructive default — easier to keep than to discover later).
    #[serde(default = "default_true")]
    pub(crate) confirm_replace_all_in_all_docs: bool,

    /// NPP `New Document ▸ Format (Line ending) ▸ Windows / Unix / Macintosh`.
    /// When `Some`, every fresh `Buffer::empty()` / `Buffer::untitled()` adopts
    /// this EOL regardless of platform. `None` means follow the platform
    /// default (LF on mac/Linux, CRLF on Windows) — matches pre-setting
    /// behaviour, so existing users see no change until they pick a value.
    #[serde(default)]
    pub(crate) default_line_ending: Option<LineEnding>,

    /// NPP `New Document ▸ Encoding`. When set, fresh buffers adopt this
    /// encoding for the next save. Decoding on load is unaffected (the
    /// loader still autodetects from BOM/heuristics — see `text/encoding.rs`).
    /// `None` keeps the prior `Encoding::Utf8` default. All five Notepad Studio
    /// encodings (UTF-8, UTF-8 BOM, UTF-16 LE/BE, Latin-1) are valid here —
    /// the EOL/encoding cycle in the status bar already supports the full
    /// set so this radio just lets users pin the default.
    #[serde(default)]
    pub(crate) default_encoding: Option<Encoding>,

    /// NPP `New Document ▸ Default language:`. When set, fresh buffers open
    /// with this tree-sitter grammar pre-attached so the user gets syntax
    /// colour from keystroke 1. `None` means "no syntax until you save with
    /// a recognised extension" — current behaviour.
    #[serde(default)]
    pub(crate) default_language: Option<LanguageId>,

    /// NPP `New Document ▸ Always open a new document in addition at startup`.
    /// When true, boot-time session restore appends a fresh blank tab to the
    /// restored set so users always land on a scratch buffer. Default false:
    /// boot exactly mirrors what was open when the app last quit (current
    /// behaviour).
    #[serde(default)]
    pub(crate) startup_open_blank: bool,

    /// NPP `New Document ▸ Use the first line of document as untitled tab name`.
    /// When true, untitled tabs (no `file_path`) display the first non-empty
    /// line of the buffer (truncated NPP-style) instead of the auto-allocated
    /// `new N` label. Default false: keeps the `new 1`, `new 2` convention.
    #[serde(default)]
    pub(crate) untitled_uses_first_line: bool,

    /// NPP `Default Directory ▸ Default Open/Save file Directory` radio. Drives
    /// which directory `rfd::AsyncFileDialog::set_directory` is seeded with at
    /// every Open / Save / Save As / Find-in-Folder call site.
    #[serde(default)]
    pub(crate) default_directory_mode: DirectoryMode,

    /// NPP `Default Directory ▸ Set your default directory:`. Used only when
    /// `default_directory_mode == DirectoryMode::Custom`. Stored as a string
    /// so a missing/invalid path round-trips cleanly through TOML.
    #[serde(default)]
    pub(crate) default_directory_custom: String,

    /// NPP `Default Directory ▸ Remember last used directory`. Updated by
    /// every dialog wrapper after the user picks a file/folder, then read on
    /// the next dialog open when `default_directory_mode == RememberLast`.
    #[serde(default)]
    pub(crate) last_used_directory: String,

    /// NPP `Multi-Instance & Date ▸ Multi-instance settings *`. Read by
    /// `single_instance::acquire_or_raise` at startup; changes take effect on
    /// next app launch.
    #[serde(default)]
    pub(crate) multi_instance_mode: MultiInstanceMode,

    /// NPP `Editing 1 ▸ Enable scrolling beyond last line`. When true,
    /// `Workspace::max_scroll_top` adds a half-viewport pad past the end of
    /// the buffer so the last line can be scrolled near the top of the
    /// viewport. Default: false (current single-line breathing-room).
    #[serde(default)]
    pub(crate) scroll_beyond_last_line: bool,

    /// NPP `Editing 1 ▸ Enable Copy/Cut Line without selection`. When true
    /// (default — matches NPP), Cmd+C / Cmd+X with an empty selection grabs
    /// the current line (Copy includes the line terminator; Cut also removes
    /// it). When false, the keystroke is a no-op for empty selections.
    /// Routed through `Workspace::copy` / `Workspace::cut`.
    #[serde(default = "default_true")]
    pub(crate) copy_cut_line_without_selection: bool,

    /// NPP `Editing 1 ▸ Highlight current line`. When true (default — matches
    /// NPP), the editor paints a band of `theme.current_line_bg` across the
    /// entire viewport width on the line containing the caret, plus the same
    /// band across the gutter row for the same line. Painted under find /
    /// mark / smart / selection quads so the existing overlays still read
    /// through.
    #[serde(default = "default_true")]
    pub(crate) highlight_current_line: bool,

    /// NPP `Editing 1 ▸ Disable selected text drag-and-drop`. When false
    /// (default — matches NPP), clicking inside an active selection and
    /// dragging moves the text to the drop location; Cmd+drag copies
    /// instead of moves. When true, click-inside-selection collapses the
    /// selection to the click point as it would for an unselected click.
    /// Read at `Workspace::on_mouse_down` to gate the drag-pending bookkeeping.
    #[serde(default)]
    pub(crate) disable_selected_text_drag_drop: bool,

    /// NPP `MISC. ▸ Document Switcher (Ctrl+TAB) ▸ Enable`. When false,
    /// Ctrl+Tab / Ctrl+Shift+Tab + the visual-order PageDown/PageUp variants
    /// no-op. Default: true (current cycle behaviour). Hard-binding stays
    /// registered — the action handler short-circuits.
    #[serde(default = "default_true")]
    pub(crate) document_switcher_enabled: bool,

    /// NPP `MISC. ▸ Document Switcher (Ctrl+TAB) ▸ Enable MRU behaviour`.
    /// When true (default), Ctrl+Tab / Ctrl+Shift+Tab walk tabs in most-
    /// recently-used order via `tab_mru`. When false, Ctrl+Tab falls back
    /// to visual (positional) cycling — same handler as
    /// Ctrl+PageDown/PageUp.
    #[serde(default = "default_true")]
    pub(crate) document_switcher_mru: bool,

    // --- Backup panel ----------------------------------------------------

    /// NPP `Backup ▸ Snapshot ▸ Enable session snapshot and periodic backup`.
    /// Gates whether `workspace::save_session` runs on the periodic timer
    /// (`session.rs::BakPolicy::Full`). When false, sessions still snapshot
    /// on quit, but no per-keystroke `.bak` is written under
    /// `<config>/Notepad Studio/sessions/`. Mirrors `NppGUI::_isSnapshotMode` (true).
    #[serde(default = "default_true")]
    pub(crate) backup_periodic_enabled: bool,

    /// NPP `Backup ▸ Snapshot ▸ every N seconds`. Drives the periodic
    /// auto-snapshot timer cadence. Bounded to
    /// `BACKUP_PERIODIC_INTERVAL_MIN_MS..=BACKUP_PERIODIC_INTERVAL_MAX_MS` on
    /// read so a hand-edited TOML never starves the timer. Mirrors
    /// `NppGUI::_snapshotBackupTiming` (7000 ms).
    #[serde(default = "default_backup_periodic_interval_ms")]
    pub(crate) backup_periodic_interval_ms: u32,

    /// NPP `Backup ▸ Backup on save ▸ {None / Simple / Verbose}`. When the
    /// active mode is `Simple` or `Verbose`, every successful `Buffer::save`
    /// writes a copy alongside (or under `backup_custom_dir` if
    /// `backup_use_custom_dir` is true). `None` keeps Notepad Studio's current
    /// behaviour. Mirrors `NppGUI::_backup` (`bak_none`).
    #[serde(default)]
    pub(crate) backup_on_save_mode: BackupOnSaveMode,

    /// NPP `Backup ▸ Backup on save ▸ Use custom backup directory`. When
    /// false, on-save backups land next to the file. When true, they go to
    /// `backup_custom_dir`. Mirrors `NppGUI::_useDir` (false).
    #[serde(default)]
    pub(crate) backup_use_custom_dir: bool,

    /// NPP `Backup ▸ Backup on save ▸ Custom backup path`. Used only when
    /// `backup_use_custom_dir` is true. Mirrors `NppGUI::_backupDir` (empty).
    #[serde(default)]
    pub(crate) backup_custom_dir: String,

    // --- Search Engine panel --------------------------------------------

    /// NPP `Search Engine ▸ {radio}`. Selects which URL template the new
    /// `Edit ▸ Search on Internet` action uses to open the browser with the
    /// currently-selected text. Mirrors `NppGUI::_searchEngineChoice`
    /// (`se_google`).
    #[serde(default)]
    pub(crate) search_engine: SearchEngineChoice,

    /// NPP `Search Engine ▸ Custom search URL:`. Required when
    /// `search_engine == SearchEngineChoice::Custom`. NPP token
    /// `$(CURRENT_WORD)` is the substitution point. Mirrors
    /// `NppGUI::_searchEngineCustom` (empty).
    #[serde(default)]
    pub(crate) search_engine_custom_url: String,

    // --- Delimiter panel ------------------------------------------------

    /// NPP `Delimiter ▸ Open delimiter`. The single character that bounds the
    /// left side of a delimited selection (Ctrl+double-click in NPP). Stored
    /// as `String` because TOML has no `char` type and char defaults can't
    /// round-trip through serde without a wrapper. Only the first grapheme
    /// is read at use time. Mirrors `NppGUI::_leftmostDelimiter` (`'('`).
    #[serde(default = "default_delimiter_left")]
    pub(crate) delimiter_left: String,

    /// NPP `Delimiter ▸ Close delimiter`. Mirrors `NppGUI::_rightmostDelimiter`
    /// (`')'`).
    #[serde(default = "default_delimiter_right")]
    pub(crate) delimiter_right: String,

    /// NPP `Delimiter ▸ Allow on entire document` (when on, the matcher
    /// scans across line boundaries; when off, it stays within the cursor's
    /// current line). Mirrors `NppGUI::_delimiterSelectionOnEntireDocument`
    /// (false).
    #[serde(default)]
    pub(crate) delimiter_match_entire_document: bool,

    // --- Performance panel ----------------------------------------------

    /// NPP `Performance ▸ Enable large file restriction`. When true, files
    /// larger than `large_file_threshold_bytes` skip the heavy editor
    /// pipelines (initially: tree-sitter syntax). Mirrors
    /// `LargeFileRestriction::_isEnabled` (true).
    #[serde(default = "default_true")]
    pub(crate) large_file_restriction_enabled: bool,

    /// NPP `Performance ▸ File size threshold (in MB)` — stored as bytes for
    /// precision. Mirrors `LargeFileRestriction::_largeFileSizeDefInByte`
    /// (`NPP_STYLING_FILESIZE_LIMIT_DEFAULT` = 200 MiB).
    #[serde(default = "default_large_file_threshold_bytes")]
    pub(crate) large_file_threshold_bytes: u64,

    /// NPP `Performance ▸ Disable syntax highlighting`. When true, files over
    /// the threshold skip tree-sitter parse/highlight (`syntax::*`). Mirrors
    /// the Notepad Studio-side equivalent of NPP's `_allowSmartHilite=false` /
    /// `_allowAutoCompletion=false` flags collapsed into a single
    /// "skip the expensive stuff" knob — Notepad Studio has no separate brace
    /// match / smart-highlight / autocomplete pipelines yet, so a single
    /// gate covers what's expensive today (parsing).
    #[serde(default = "default_true")]
    pub(crate) large_file_disable_syntax: bool,

    // --- Language panel -------------------------------------------------

    /// NPP `Language ▸ Disable Language Menu Entries`. Languages listed here
    /// are hidden from the `View ▸ Language` picker and the New Document
    /// default-language cycler. Mirrors `NppGUI::_excludedLangList` (empty
    /// vec).
    #[serde(default)]
    pub(crate) excluded_languages: Vec<LanguageId>,

    // --- Recent Files History panel -------------------------------------

    /// NPP `Recent Files History ▸ Don't check at launch`. When true, the
    /// recents list is shown as-is at startup; when false, missing files are
    /// filtered out before the menu paints. Mirrors `NppGUI::_checkHistoryFiles`
    /// (false — i.e. NPP defaults to "don't check").
    #[serde(default)]
    pub(crate) recent_files_check_at_launch: bool,

    /// NPP `Recent Files History ▸ Display ▸ Full file name path / Only
    /// file name`. When true the menu shows full absolute paths; when false
    /// (default) it shows basenames. NPP default: filename only.
    #[serde(default)]
    pub(crate) recent_files_show_full_path: bool,

    /// NPP `Recent Files History ▸ Location ▸ In submenu / Right after
    /// recent files separator`. When true, recents are nested under a
    /// `Recent Files ▸` submenu; when false (default), they sit flat at the
    /// bottom of the File menu. NPP default: flat.
    #[serde(default)]
    pub(crate) recent_files_in_submenu: bool,

    // --- Cloud & Link panel ---------------------------------------------

    /// NPP `Cloud & Link ▸ URL setting ▸ {Show/Underline/None}` collapsed to
    /// a single bool: when true, http/https URLs in the buffer paint as
    /// underlined and Cmd-click opens them in the OS browser. When false,
    /// URLs render as plain text. Mirrors `NppGUI::_styleURL == urlUnderLineFg`
    /// (the NPP default is on — the panel just lets the user turn it off).
    /// Cloud sync (the rest of NPP's Cloud panel) is intentionally not
    /// surfaced — Notepad Studio has no cloud-folder backend.
    #[serde(default = "default_true")]
    pub(crate) clickable_urls_enabled: bool,

    /// NPP `Cloud & Link ▸ No underline` — when the parent `clickable_urls_enabled`
    /// is on, this sub-toggle drops the underline overlay while keeping the
    /// link-colour swap. Defaults to false (underline on) to match NPP's
    /// default and `_styleURL == urlUnderLineFg`.
    #[serde(default)]
    pub(crate) clickable_urls_no_underline: bool,

    // --- Editing 2 panel ------------------------------------------------

    /// NPP `Editing 2 ▸ Multi-Editing ▸ Enable Multi-Editing` (Ctrl+click /
    /// Ctrl+drag adds a secondary caret). Mirrors `_multiSelection` in
    /// `Parameters.h:943` (NPP default = true). Notepad Studio currently has a
    /// single-selection model in `buffer::Selection`; the field persists so
    /// the next fork can gate a multi-cursor engine on it without changing
    /// the on-disk schema again.
    #[serde(default = "default_true")]
    pub(crate) multi_editing_enabled: bool,

    /// NPP `View ▸ Show Symbol ▸ Show White Space` (also surfaced under the
    /// Editing 2 panel for the radio default). Mirrors `_whiteSpaceShow` in
    /// `Parameters.h:902` (NPP default = false).
    #[serde(default)]
    pub(crate) whitespace_visible: bool,

    /// NPP `Editing 2 ▸ EOL ▸ Show EOL`. Mirrors `_eolShow` in
    /// `Parameters.h:903` (NPP default = false).
    #[serde(default)]
    pub(crate) eol_visible: bool,

    /// NPP `Editing 2 ▸ EOL ▸ {Plain Text / Rounded Rectangle}`. Mirrors
    /// `_eolMode` in `Parameters.h:906` (NPP default = `roundedRectangleText`).
    #[serde(default)]
    pub(crate) eol_display_mode: EolDisplayMode,

    /// NPP `Editing 2 ▸ Non-Printing Characters ▸ Show NPC`. Mirrors
    /// `_npcShow` in `Parameters.h:907` (NPP default = false).
    #[serde(default)]
    pub(crate) npc_visible: bool,

    /// NPP `Editing 2 ▸ Non-Printing Characters ▸ {Abbreviation / Codepoint}`.
    /// Mirrors `_npcMode` in `Parameters.h:909` (NPP default = `abbreviation`).
    #[serde(default)]
    pub(crate) npc_display_mode: NpcDisplayMode,

    /// NPP `Editing 2 ▸ Non-Printing Characters ▸ Apply Appearance settings to
    /// C0, C1 & Unicode EOL`. Mirrors `_npcIncludeCcUniEol` in
    /// `Parameters.h:911` (NPP default = false).
    #[serde(default)]
    pub(crate) npc_include_cc_uni_eol: bool,

    /// NPP `Editing 2 ▸ Non-Printing Characters ▸ Prevent control character
    /// (C0 code) typing into document`. Mirrors `_npcNoInputC0` in
    /// `Parameters.h:913` (NPP default = true).
    #[serde(default = "default_true")]
    pub(crate) prevent_c0_input: bool,

    // --- Highlighting panel ---------------------------------------------

    /// NPP `Highlighting ▸ Smart Highlighting ▸ Enable`. Mirrors
    /// `_enableSmartHilite` in `Parameters.h:750` (NPP default = true).
    #[serde(default = "default_true")]
    pub(crate) smart_highlight_enabled: bool,

    /// NPP `Highlighting ▸ Smart Highlighting ▸ Match case`. Mirrors
    /// `_smartHiliteCaseSensitive` in `Parameters.h:752` (NPP default = false).
    #[serde(default)]
    pub(crate) smart_highlight_case_sensitive: bool,

    /// NPP `Highlighting ▸ Smart Highlighting ▸ Match whole word only`.
    /// Mirrors `_smartHiliteWordOnly` in `Parameters.h:753` (NPP default =
    /// true).
    #[serde(default = "default_true")]
    pub(crate) smart_highlight_whole_word: bool,

    /// NPP `Highlighting ▸ Smart Highlighting ▸ Use Find dialog settings`.
    /// Mirrors `_smartHiliteUseFindSettings` in `Parameters.h:754` (NPP
    /// default = false).
    #[serde(default)]
    pub(crate) smart_highlight_use_find_settings: bool,

    /// NPP `Highlighting ▸ Style All Occurrences of Token ▸ Match case`.
    /// Mirrors `_markAllCaseSensitive` in `Parameters.h:757` (NPP default =
    /// false). Drives `Search ▸ Mark All` (Notepad Studio equivalent: TBD).
    #[serde(default)]
    pub(crate) mark_all_case_sensitive: bool,

    /// NPP `Highlighting ▸ Style All Occurrences of Token ▸ Match whole word
    /// only`. Mirrors `_markAllWordOnly` in `Parameters.h:758` (NPP default =
    /// true).
    #[serde(default = "default_true")]
    pub(crate) mark_all_whole_word: bool,

    /// NPP `Highlighting ▸ Highlight Matching Tags ▸ Enable`. Mirrors
    /// `_enableTagsMatchHilite` in `Parameters.h:761` (NPP default = true).
    /// Drives HTML/XML matching-tag highlight when the cursor is on a tag.
    #[serde(default = "default_true")]
    pub(crate) match_tags_enabled: bool,

    /// NPP `Highlighting ▸ Highlight Matching Tags ▸ Highlight tag attributes`.
    /// Mirrors `_enableTagAttrsHilite` in `Parameters.h:762` (NPP default =
    /// true).
    #[serde(default = "default_true")]
    pub(crate) match_tag_attrs_enabled: bool,

    /// NPP `Highlighting ▸ Highlight Matching Tags ▸ Highlight comment/php/asp
    /// zone`. Mirrors `_enableHiliteNonHTMLZone` in `Parameters.h:763` (NPP
    /// default = false).
    #[serde(default)]
    pub(crate) match_tags_in_comment_zone: bool,

    // --- Indentation panel ----------------------------------------------

    /// NPP `Indentation ▸ Auto-indent ▸ {None / Basic / Advanced}`. Mirrors
    /// `_maintainIndent` in `Parameters.h:749` (NPP default = `advanced`).
    /// `Advanced` reproduces the previous-line indent + token-based open/close
    /// awareness. `Basic` only reproduces the previous-line indent. `None`
    /// inserts a bare newline.
    #[serde(default)]
    pub(crate) auto_indent_mode: AutoIndentMode,

    /// NPP `Indentation ▸ language listbox` — per-language indent overrides.
    /// Keys are the snake_case serde representation of `LanguageId`
    /// (`"rust"`, `"python"`, `"c_sharp"`...). Empty by default: the
    /// resolver `indent_for_lang` falls back to the global
    /// `indent_uses_tabs` + `tab_width` when no override exists. NPP
    /// materialises an entry only on first interaction with a per-language
    /// radio / width; a "Reset to default" button removes it.
    #[serde(default)]
    pub(crate) language_indent_overrides: BTreeMap<String, IndentOverride>,

    // --- Print panel ----------------------------------------------------

    /// NPP `Print ▸ Print line number`. Mirrors `_printLineNumber` in
    /// `Parameters.h:518` (NPP default = true).
    #[serde(default = "default_true")]
    pub(crate) print_line_numbers: bool,

    /// NPP `Print ▸ Color Options ▸ {radio}`. Mirrors `_printOption` in
    /// `Parameters.h:519` (NPP default = `SC_PRINT_COLOURONWHITE`).
    #[serde(default)]
    pub(crate) print_color_mode: PrintColorMode,

    /// NPP `Print ▸ Margin Setting (mm) ▸ Top`. Mirrors `_marge.top` in
    /// `Parameters.h:535` (NPP default = 0).
    #[serde(default)]
    pub(crate) print_margin_top_mm: u32,

    /// NPP `Print ▸ Margin Setting (mm) ▸ Left`. Mirrors `_marge.left` in
    /// `Parameters.h:535` (NPP default = 0).
    #[serde(default)]
    pub(crate) print_margin_left_mm: u32,

    /// NPP `Print ▸ Margin Setting (mm) ▸ Right`. Mirrors `_marge.right` in
    /// `Parameters.h:535` (NPP default = 0).
    #[serde(default)]
    pub(crate) print_margin_right_mm: u32,

    /// NPP `Print ▸ Margin Setting (mm) ▸ Bottom`. Mirrors `_marge.bottom` in
    /// `Parameters.h:535` (NPP default = 0).
    #[serde(default)]
    pub(crate) print_margin_bottom_mm: u32,

    /// NPP `Print ▸ Header ▸ Left part`. Mirrors `_headerLeft` in
    /// `Parameters.h:521` (NPP default = empty).
    #[serde(default)]
    pub(crate) print_header_left: String,

    /// NPP `Print ▸ Header ▸ Middle part`. Mirrors `_headerMiddle` in
    /// `Parameters.h:522` (NPP default = empty).
    #[serde(default)]
    pub(crate) print_header_middle: String,

    /// NPP `Print ▸ Header ▸ Right part`. Mirrors `_headerRight` in
    /// `Parameters.h:523` (NPP default = empty).
    #[serde(default)]
    pub(crate) print_header_right: String,

    /// NPP `Print ▸ Header ▸ Font name`. Mirrors `_headerFontName` in
    /// `Parameters.h:524` (NPP default = empty — "use system default").
    #[serde(default)]
    pub(crate) print_header_font_name: String,

    /// NPP `Print ▸ Header ▸ Font size` (points). Mirrors `_headerFontSize`
    /// in `Parameters.h:526` (NPP default = 0 = "system default").
    #[serde(default)]
    pub(crate) print_header_font_size: u32,

    /// NPP `Print ▸ Header ▸ Bold`. Bit 0 of `_headerFontStyle`
    /// (`Parameters.h:525`). NPP default = false.
    #[serde(default)]
    pub(crate) print_header_font_bold: bool,

    /// NPP `Print ▸ Header ▸ Italic`. Bit 1 of `_headerFontStyle`. NPP default
    /// = false.
    #[serde(default)]
    pub(crate) print_header_font_italic: bool,

    /// NPP `Print ▸ Footer ▸ Left part`. Mirrors `_footerLeft` in
    /// `Parameters.h:528` (NPP default = empty).
    #[serde(default)]
    pub(crate) print_footer_left: String,

    /// NPP `Print ▸ Footer ▸ Middle part`. Mirrors `_footerMiddle` in
    /// `Parameters.h:529` (NPP default = empty).
    #[serde(default)]
    pub(crate) print_footer_middle: String,

    /// NPP `Print ▸ Footer ▸ Right part`. Mirrors `_footerRight` in
    /// `Parameters.h:530` (NPP default = empty).
    #[serde(default)]
    pub(crate) print_footer_right: String,

    /// NPP `Print ▸ Footer ▸ Font name`. Mirrors `_footerFontName` in
    /// `Parameters.h:531` (NPP default = empty).
    #[serde(default)]
    pub(crate) print_footer_font_name: String,

    /// NPP `Print ▸ Footer ▸ Font size`. Mirrors `_footerFontSize` in
    /// `Parameters.h:533` (NPP default = 0).
    #[serde(default)]
    pub(crate) print_footer_font_size: u32,

    /// NPP `Print ▸ Footer ▸ Bold`. Bit 0 of `_footerFontStyle`. NPP default
    /// = false.
    #[serde(default)]
    pub(crate) print_footer_font_bold: bool,

    /// NPP `Print ▸ Footer ▸ Italic`. Bit 1 of `_footerFontStyle`. NPP default
    /// = false.
    #[serde(default)]
    pub(crate) print_footer_font_italic: bool,

    // --- Auto-Completion panel ------------------------------------------

    /// NPP `Auto-Completion ▸ {radio}`. Mirrors `_autocStatus` in
    /// `Parameters.h:803` (NPP default = `autoc_both` =
    /// `AutocompletionMode::FunctionAndWord`).
    #[serde(default)]
    pub(crate) autocompletion_mode: AutocompletionMode,

    /// NPP `Auto-Completion ▸ From N-th character`. Mirrors `_autocFromLen`
    /// in `Parameters.h:804` (NPP default = 1; range 1..=9 enforced by the
    /// slider).
    #[serde(default = "default_autocompletion_from_chars")]
    pub(crate) autocompletion_from_chars: u32,

    /// NPP `Auto-Completion ▸ Make auto-completion list brief`. Mirrors
    /// `_autocBrief` in `Parameters.h:808` (NPP default = false).
    #[serde(default)]
    pub(crate) autocompletion_brief: bool,

    /// NPP `Auto-Completion ▸ Function parameters hint on input`. Mirrors
    /// `_funcParams` in `Parameters.h:809` (NPP default = true).
    #[serde(default = "default_true")]
    pub(crate) autocompletion_func_params_hint: bool,

    /// NPP `Auto-Completion ▸ Insert Selection ▸ Ignore numbers`. Mirrors
    /// `_autocIgnoreNumbers` in `Parameters.h:805` (NPP default = true).
    #[serde(default = "default_true")]
    pub(crate) autocompletion_ignore_numbers: bool,

    /// NPP `Auto-Completion ▸ Insert Selection ▸ TAB`. Mirrors
    /// `_autocInsertSelectedUseTAB` in `Parameters.h:807` (NPP default =
    /// true).
    #[serde(default = "default_true")]
    pub(crate) autocompletion_insert_with_tab: bool,

    /// NPP `Auto-Completion ▸ Insert Selection ▸ ENTER`. Mirrors
    /// `_autocInsertSelectedUseENTER` in `Parameters.h:806` (NPP default =
    /// true).
    #[serde(default = "default_true")]
    pub(crate) autocompletion_insert_with_enter: bool,

    /// NPP `Auto-Completion ▸ Auto-Insert ▸ ` `(`. `MatchedPairConf::_doParentheses`
    /// in `Parameters.h:646` (NPP default = false).
    #[serde(default)]
    pub(crate) auto_insert_parens: bool,

    /// NPP `Auto-Completion ▸ Auto-Insert ▸ ` `[`. `MatchedPairConf::_doBrackets`
    /// in `Parameters.h:647` (NPP default = false).
    #[serde(default)]
    pub(crate) auto_insert_brackets: bool,

    /// NPP `Auto-Completion ▸ Auto-Insert ▸ ` `{`. `MatchedPairConf::_doCurlyBrackets`
    /// in `Parameters.h:648` (NPP default = false).
    #[serde(default)]
    pub(crate) auto_insert_braces: bool,

    /// NPP `Auto-Completion ▸ Auto-Insert ▸ ` `'`. `MatchedPairConf::_doQuotes`
    /// in `Parameters.h:649` (NPP default = false).
    #[serde(default)]
    pub(crate) auto_insert_quotes: bool,

    /// NPP `Auto-Completion ▸ Auto-Insert ▸ ` `"`. `MatchedPairConf::_doDoubleQuotes`
    /// in `Parameters.h:650` (NPP default = false).
    #[serde(default)]
    pub(crate) auto_insert_double_quotes: bool,

    /// NPP `Auto-Completion ▸ Auto-Insert ▸ HTML/XML close tag`.
    /// `MatchedPairConf::_doHtmlXmlTag` in `Parameters.h:644` (NPP default =
    /// false).
    #[serde(default)]
    pub(crate) auto_insert_html_xml_tag: bool,

    /// NPP `Auto-Completion ▸ Matched pair 1/2/3` (three user-defined open/close
    /// pairs). `MatchedPairConf::_matchedPairs` in `Parameters.h:642` (NPP
    /// default = empty vec). Stored as variable-length (vs. NPP's hard-coded
    /// 3 slots) to leave room for future expansion without a schema bump.
    /// Each entry is `(open, close)`; only the first grapheme of each string
    /// is read at use time.
    #[serde(default)]
    pub(crate) auto_insert_user_pairs: Vec<(String, String)>,

    /// `View ▸ Folder as Workspace` toggle — NPP-style left dock. When false,
    /// the panel is omitted from the render tree even if `folder_panel_roots`
    /// is non-empty (so a hidden panel can keep its roots ready for the next
    /// re-open).
    #[serde(default)]
    pub(crate) folder_panel_visible: bool,

    /// Pixel width of the folder-as-workspace panel. Resized live by dragging
    /// the splitter on the panel's right edge; clamped to
    /// `[FOLDER_PANEL_MIN_WIDTH, FOLDER_PANEL_MAX_WIDTH]` on every set.
    #[serde(default = "default_folder_panel_width")]
    pub(crate) folder_panel_width: f32,

    /// Top-level folder roots shown in the panel, in display order. Each path
    /// is normalised via `dunce::canonicalize` (where available) on add.
    /// Removed entries via "Remove from Workspace" are dropped from this vec
    /// only — never touch disk.
    #[serde(default)]
    pub(crate) folder_panel_roots: Vec<PathBuf>,

    /// Folder paths whose children are currently expanded in the tree.
    /// Restored on launch so the user lands back on the same view they left.
    /// Pruned passively when a path no longer exists on disk.
    #[serde(default)]
    pub(crate) folder_panel_expanded: Vec<PathBuf>,

    /// `Show Hidden Files` toggle — when false, dotfiles + dot-folders are
    /// filtered out of every directory listing. When true, they appear (NPP
    /// matches the OS file explorer's hidden-file convention).
    #[serde(default)]
    pub(crate) folder_panel_show_hidden: bool,

    /// Style Configurator "Global Override" checkboxes. User-level prefs
    /// (NOT theme XML) — when an `enable_*` flag is true, that attribute on
    /// the active theme's "Global override" style overrides the matching
    /// per-style attribute across every style. Defaults all-false.
    #[serde(default)]
    pub(crate) global_override: crate::style_model::GlobalOverride,

    /// Per-input MRU history for the Find dialog text inputs. Mirrors NPP's
    /// `<FindHistory>` block in `config.xml`. Surfaced via the chevron
    /// dropdown on each input; populated on every commit (Find Next /
    /// Replace / Replace All / Mark All / FIF Run).
    #[serde(default)]
    pub(crate) find_history: FindHistory,
}

/// Lower bound for the folder-as-workspace panel width (px). Below this the
/// row labels start truncating beyond usefulness.
pub(crate) const FOLDER_PANEL_MIN_WIDTH: f32 = 160.0;
/// Upper bound for the folder-as-workspace panel width (px). NPP caps near
/// half a 1080p screen; 600 keeps headroom for narrower windows.
pub(crate) const FOLDER_PANEL_MAX_WIDTH: f32 = 600.0;
/// Default width when the panel is first opened (px).
pub(crate) const DEFAULT_FOLDER_PANEL_WIDTH: f32 = 240.0;

fn default_folder_panel_width() -> f32 {
    DEFAULT_FOLDER_PANEL_WIDTH
}

fn default_true() -> bool {
    true
}

fn default_tab_width() -> usize {
    TAB_SPACES
}

/// Indent width range: the bounds of the Preferences ▸ Indentation
/// spinner (`FieldId::numeric_bounds`). A width outside it only comes from
/// a hand-edited settings.toml.
pub(crate) const TAB_WIDTH_MIN: usize = 1;
pub(crate) const TAB_WIDTH_MAX: usize = 16;

fn default_split_lines_edge_column() -> usize {
    DEFAULT_SPLIT_LINES_EDGE_COLUMN
}

fn default_datetime_custom_format() -> String {
    DEFAULT_DATETIME_CUSTOM_FORMAT.to_string()
}

fn default_preferences_sidebar_width() -> f32 {
    DEFAULT_PREFERENCES_SIDEBAR_WIDTH
}

fn default_preferences_active_panel() -> String {
    DEFAULT_PREFERENCES_ACTIVE_PANEL.to_string()
}

fn default_recent_files_max() -> u32 {
    DEFAULT_RECENT_FILES_MAX
}

fn default_caret_blink_rate_ms() -> u32 {
    DEFAULT_CARET_BLINK_RATE_MS
}

fn default_theme_mode() -> ThemeMode {
    ThemeMode::Dark
}

fn default_customized_slots_vec() -> Vec<Option<u32>> {
    vec![None; SLOT_COUNT]
}

/// Custom serde adapter for `Settings.customized_slots`. See the field's
/// doc-comment for why this exists — TOML can't represent `null` inside an
/// array, so a vanilla `Vec<Option<u32>>` makes `to_string_pretty` fail
/// and the entire settings file fails to save. We map `None` ↔ the NPP
/// "colour unset" sentinel `0xFF00_0000` on the wire.
mod customized_slots_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    /// `0xFF000000` — alpha-byte FF, the same "unset" marker NPP uses in
    /// stylers.xml. Any value with the high byte cleared (i.e. a real
    /// 0xRRGGBB colour) deserializes as `Some`.
    const UNSET: u32 = 0xFF00_0000;

    pub(super) fn serialize<S>(
        slots: &Vec<Option<u32>>,
        ser: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let raw: Vec<u32> = slots.iter().map(|o| o.unwrap_or(UNSET)).collect();
        ser.collect_seq(raw.iter())
    }

    pub(super) fn deserialize<'de, D>(de: D) -> Result<Vec<Option<u32>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw: Vec<u32> = Vec::deserialize(de)?;
        Ok(raw
            .into_iter()
            .map(|v| if v == UNSET { None } else { Some(v) })
            .collect())
    }
}

fn default_backup_periodic_interval_ms() -> u32 {
    DEFAULT_BACKUP_PERIODIC_INTERVAL_MS
}

fn default_large_file_threshold_bytes() -> u64 {
    DEFAULT_LARGE_FILE_THRESHOLD_BYTES
}

fn default_delimiter_left() -> String {
    DEFAULT_DELIMITER_LEFT.to_string()
}

fn default_delimiter_right() -> String {
    DEFAULT_DELIMITER_RIGHT.to_string()
}

fn default_autocompletion_from_chars() -> u32 {
    DEFAULT_AUTOCOMPLETION_FROM_CHARS
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tab_width: TAB_SPACES,
            split_lines_edge_column: DEFAULT_SPLIT_LINES_EDGE_COLUMN,
            datetime_custom_format: DEFAULT_DATETIME_CUSTOM_FORMAT.to_string(),
            preferences_sidebar_width: DEFAULT_PREFERENCES_SIDEBAR_WIDTH,
            preferences_active_panel: DEFAULT_PREFERENCES_ACTIVE_PANEL.to_string(),
            status_bar_hidden: false,
            recent_files_max: DEFAULT_RECENT_FILES_MAX,
            title_bar_filename_only: false,
            tab_bar_hidden: false,
            toolbar_hidden: false,
            tab_close_button_visible: true,
            tab_double_click_to_close: false,
            tab_reduce: false,
            tab_active_bar: true,
            tab_darken_inactive: true,
            tab_exit_on_last_close: true,
            tab_lock_drag: false,
            tab_multi_line: false,
            tab_vertical: false,
            caret_blink_rate_ms: DEFAULT_CARET_BLINK_RATE_MS,
            theme_mode: ThemeMode::Dark,
            theme_path: None,
            tone_palette: TonePalette::Black,
            customized_slots: default_customized_slots_vec(),
            show_bookmark_glyph: true,
            show_line_numbers: true,
            line_number_width_dynamic: true,
            indent_uses_tabs: false,
            save_all_confirm: false,
            replace_dont_move_to_next: false,
            confirm_replace_all_in_all_docs: true,
            default_line_ending: None,
            default_encoding: None,
            default_language: None,
            startup_open_blank: false,
            untitled_uses_first_line: false,
            default_directory_mode: DirectoryMode::default(),
            default_directory_custom: String::new(),
            last_used_directory: String::new(),
            multi_instance_mode: MultiInstanceMode::default(),
            scroll_beyond_last_line: false,
            copy_cut_line_without_selection: true,
            highlight_current_line: true,
            disable_selected_text_drag_drop: false,
            document_switcher_enabled: true,
            document_switcher_mru: true,
            backup_periodic_enabled: DEFAULT_BACKUP_PERIODIC_ENABLED,
            backup_periodic_interval_ms: DEFAULT_BACKUP_PERIODIC_INTERVAL_MS,
            backup_on_save_mode: BackupOnSaveMode::default(),
            backup_use_custom_dir: false,
            backup_custom_dir: String::new(),
            search_engine: SearchEngineChoice::default(),
            search_engine_custom_url: String::new(),
            delimiter_left: DEFAULT_DELIMITER_LEFT.to_string(),
            delimiter_right: DEFAULT_DELIMITER_RIGHT.to_string(),
            delimiter_match_entire_document: false,
            large_file_restriction_enabled: true,
            large_file_threshold_bytes: DEFAULT_LARGE_FILE_THRESHOLD_BYTES,
            large_file_disable_syntax: true,
            excluded_languages: Vec::new(),
            recent_files_check_at_launch: false,
            recent_files_show_full_path: false,
            recent_files_in_submenu: false,
            clickable_urls_enabled: true,
            clickable_urls_no_underline: false,
            multi_editing_enabled: true,
            whitespace_visible: false,
            eol_visible: false,
            eol_display_mode: EolDisplayMode::default(),
            npc_visible: false,
            npc_display_mode: NpcDisplayMode::default(),
            npc_include_cc_uni_eol: false,
            prevent_c0_input: true,
            smart_highlight_enabled: true,
            smart_highlight_case_sensitive: false,
            smart_highlight_whole_word: true,
            smart_highlight_use_find_settings: false,
            mark_all_case_sensitive: false,
            mark_all_whole_word: true,
            match_tags_enabled: true,
            match_tag_attrs_enabled: true,
            match_tags_in_comment_zone: false,
            auto_indent_mode: AutoIndentMode::default(),
            language_indent_overrides: BTreeMap::new(),
            print_line_numbers: true,
            print_color_mode: PrintColorMode::default(),
            print_margin_top_mm: 0,
            print_margin_left_mm: 0,
            print_margin_right_mm: 0,
            print_margin_bottom_mm: 0,
            print_header_left: String::new(),
            print_header_middle: String::new(),
            print_header_right: String::new(),
            print_header_font_name: String::new(),
            print_header_font_size: 0,
            print_header_font_bold: false,
            print_header_font_italic: false,
            print_footer_left: String::new(),
            print_footer_middle: String::new(),
            print_footer_right: String::new(),
            print_footer_font_name: String::new(),
            print_footer_font_size: 0,
            print_footer_font_bold: false,
            print_footer_font_italic: false,
            autocompletion_mode: AutocompletionMode::default(),
            autocompletion_from_chars: DEFAULT_AUTOCOMPLETION_FROM_CHARS,
            autocompletion_brief: false,
            autocompletion_func_params_hint: true,
            autocompletion_ignore_numbers: true,
            autocompletion_insert_with_tab: true,
            autocompletion_insert_with_enter: true,
            auto_insert_parens: false,
            auto_insert_brackets: false,
            auto_insert_braces: false,
            auto_insert_quotes: false,
            auto_insert_double_quotes: false,
            auto_insert_html_xml_tag: false,
            auto_insert_user_pairs: Vec::new(),
            folder_panel_visible: false,
            folder_panel_width: DEFAULT_FOLDER_PANEL_WIDTH,
            folder_panel_roots: Vec::new(),
            folder_panel_expanded: Vec::new(),
            folder_panel_show_hidden: false,
            global_override: crate::style_model::GlobalOverride::new(),
            find_history: FindHistory::default(),
        }
    }
}

impl Settings {
    /// Materialise `customized_slots` (stored as a `Vec<Option<u32>>` for
    /// serde compatibility) into the fixed-size `[Option<u32>; SLOT_COUNT]`
    /// expected by `theme::Theme::resolve`. Pads with `None` if the on-disk
    /// vec is shorter than the slot count (older settings file) and trims
    /// extras (defensive).
    pub(crate) fn customized_slots_array(&self) -> [Option<u32>; SLOT_COUNT] {
        let mut out: [Option<u32>; SLOT_COUNT] = [None; SLOT_COUNT];
        for (i, v) in self.customized_slots.iter().take(SLOT_COUNT).enumerate() {
            out[i] = *v;
        }
        out
    }
}

/// Resolve the directory a file dialog should start in given the active
/// buffer's path (if any). Returns `None` when the resolved candidate
/// doesn't exist — caller can chain into `dirs::document_dir` /
/// `dirs::home_dir` per-callsite preference.
pub(crate) fn resolve_starting_dir(active_buffer_dir: Option<&Path>) -> Option<PathBuf> {
    let snap = snapshot();
    let candidate = match snap.default_directory_mode {
        DirectoryMode::FollowDocument => active_buffer_dir.map(Path::to_path_buf),
        DirectoryMode::RememberLast => {
            let s = snap.last_used_directory.trim();
            (!s.is_empty()).then(|| PathBuf::from(s))
        }
        DirectoryMode::Custom => {
            let s = snap.default_directory_custom.trim();
            (!s.is_empty()).then(|| PathBuf::from(s))
        }
    };
    candidate.filter(|p| p.is_dir())
}

/// Persist the parent directory of the just-picked path back to
/// `last_used_directory` — only when the user is on `DirectoryMode::RememberLast`,
/// otherwise no-op so a custom-path user doesn't have their pin clobbered.
pub(crate) fn record_last_used_directory(picked: &Path) {
    let mut next = snapshot();
    if !matches!(next.default_directory_mode, DirectoryMode::RememberLast) {
        return;
    }
    let dir = if picked.is_dir() {
        Some(picked.to_path_buf())
    } else {
        picked.parent().map(Path::to_path_buf)
    };
    let Some(dir) = dir else { return };
    let dir_str = dir.to_string_lossy().into_owned();
    if dir_str == next.last_used_directory {
        return;
    }
    next.last_used_directory = dir_str;
    replace_and_save(next);
}

/// Resolves the EOL a fresh buffer should adopt, with the explicit user
/// choice (when set) taking precedence over the platform default.
pub(crate) fn resolved_default_line_ending() -> LineEnding {
    snapshot()
        .default_line_ending
        .unwrap_or_else(LineEnding::platform_default)
}

/// Resolves the encoding a fresh buffer should adopt. Defaults to
/// `Encoding::Utf8` to match prior behaviour when the user hasn't picked
/// otherwise.
pub(crate) fn resolved_default_encoding() -> Encoding {
    snapshot().default_encoding.unwrap_or(Encoding::Utf8)
}

/// Caret blink interval clamped to a sane window. `app/window.rs` calls this
/// each tick so a Preferences edit takes effect on the next blink without
/// needing to restart the spawned task.
pub(crate) fn caret_blink_interval_ms() -> u32 {
    snapshot()
        .caret_blink_rate_ms
        .clamp(CARET_BLINK_RATE_MIN_MS, CARET_BLINK_RATE_MAX_MS)
}

/// Periodic-backup interval clamped to a sane window. Same pattern as
/// `caret_blink_interval_ms` — the backup task reads this each tick so a
/// Preferences edit takes effect on the next snapshot without restart.
pub(crate) fn backup_periodic_interval_ms() -> u32 {
    snapshot()
        .backup_periodic_interval_ms
        .clamp(BACKUP_PERIODIC_INTERVAL_MIN_MS, BACKUP_PERIODIC_INTERVAL_MAX_MS)
}

/// Push a new theme mode to every open Workspace + redraw. Call after writing
/// to `Settings.theme_mode` so the Preferences radio, Cmd+Shift+T toggle, and
/// fresh windows all stay in lockstep. The session manifest is rewritten on
/// the active workspace's next save_session — that's the only place the
/// per-window theme gets persisted, so a restart still picks up the choice.
pub(crate) fn propagate_theme_to_all_workspaces(mode: crate::theme::ThemeMode, cx: &mut gpui::App) {
    for handle in cx.windows() {
        if let Some(ws_handle) = handle.downcast::<crate::workspace::Workspace>() {
            let _ = ws_handle.update(cx, |ws, _, cx| {
                if ws.theme_mode != mode {
                    ws.theme_mode = mode;
                    ws.save_session(cx);
                }
                cx.notify();
            });
        }
    }
}

// --- on-disk i/o ---------------------------------------------------------

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join("settings.toml"))
}

fn load_from_disk() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let s = load_from_path(&path);
    record_seen_mtime(&path);
    s
}

/// Settings stored in `path`. A value that no longer fits its field (a
/// typo, a renamed enum variant) resets that one field instead of every
/// preference; the original file is kept as `settings.toml.corrupt`.
fn load_from_path(path: &Path) -> Settings {
    let mut s = crate::persist::load_toml(
        path,
        |t| crate::persist::salvage_table(t, &[]).unwrap_or_default(),
        Settings::default,
    )
    .into_option()
    .unwrap_or_default();
    clamp_indent_widths(&mut s);
    s
}

/// Pull indent widths into `TAB_WIDTH_MIN..=TAB_WIDTH_MAX`. A width of
/// billions reaches `" ".repeat(width)` on the next Tab press and aborts.
fn clamp_indent_widths(s: &mut Settings) {
    s.tab_width = s.tab_width.clamp(TAB_WIDTH_MIN, TAB_WIDTH_MAX);
    for o in s.language_indent_overrides.values_mut() {
        o.width = o.width.clamp(TAB_WIDTH_MIN, TAB_WIDTH_MAX);
    }
}

fn save_to_disk(s: &Settings) {
    let Some(path) = settings_path() else {
        return;
    };
    match save_to_path(&path, s) {
        // Capture the post-write mtime so the cross-process watcher
        // doesn't treat our own save as an external change.
        Ok(()) => record_seen_mtime(&path),
        Err(err) => eprintln!("[settings] couldn't save {}: {err}", path.display()),
    }
}

fn save_to_path(path: &Path, s: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let serialized = toml::to_string_pretty(s).map_err(std::io::Error::other)?;
    crate::persist::atomic_write(path, serialized.as_bytes())
}

fn current_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn record_seen_mtime(path: &Path) {
    if let Some(mtime) = current_mtime(path) {
        if let Ok(mut g) = last_seen_mtime().lock() {
            *g = Some(mtime);
        }
    }
}

static LAST_SEEN_MTIME: OnceLock<Mutex<Option<SystemTime>>> = OnceLock::new();

fn last_seen_mtime() -> &'static Mutex<Option<SystemTime>> {
    LAST_SEEN_MTIME.get_or_init(|| Mutex::new(None))
}

// --- process-global cache ------------------------------------------------

static GLOBAL: OnceLock<Mutex<Settings>> = OnceLock::new();

fn global() -> &'static Mutex<Settings> {
    GLOBAL.get_or_init(|| Mutex::new(load_from_disk()))
}

/// Snapshot a clone of the current settings. Cheap on the read side —
/// callers stay off the lock for the duration of any non-trivial work.
pub(crate) fn snapshot() -> Settings {
    #[cfg(test)]
    if let Some(settings) = TEST_SETTINGS.with(|settings| settings.borrow().clone()) {
        return settings;
    }
    global()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

#[cfg(test)]
thread_local! {
    static TEST_SETTINGS: std::cell::RefCell<Option<Settings>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct TestSettingsGuard(Option<Settings>);

#[cfg(test)]
impl Drop for TestSettingsGuard {
    fn drop(&mut self) {
        TEST_SETTINGS.with(|settings| *settings.borrow_mut() = self.0.take());
    }
}

/// UI-thread tests can change preferences without reading or writing user state.
#[cfg(test)]
pub(crate) fn override_for_test(mut settings: Settings) -> TestSettingsGuard {
    clamp_indent_widths(&mut settings);
    TestSettingsGuard(TEST_SETTINGS.with(|current| current.replace(Some(settings))))
}

/// Resolve the active indent character + width for a given language. Returns
/// the per-language override from `language_indent_overrides` if one exists;
/// otherwise the global `indent_uses_tabs` + `tab_width`. Pass `None` for
/// unlabelled buffers (plain text) — they always fall back to globals.
///
/// `lang_key` is the snake_case `LanguageId::key()` string (e.g. `"rust"`,
/// `"python"`, `"c_sharp"`). Width is clamped to `>= 1` so a corrupted
/// settings file can't produce a zero-space indent.
pub(crate) fn indent_for_lang(lang_key: Option<&str>) -> (bool, usize) {
    let snap = snapshot();
    if let Some(key) = lang_key {
        if let Some(o) = snap.language_indent_overrides.get(key) {
            return (o.uses_tabs, o.width.max(1));
        }
    }
    (snap.indent_uses_tabs, snap.tab_width.max(1))
}

/// Replace the global cache and persist to disk in one step. Called by
/// the Preferences dialog's `Apply` / `OK` paths.
///
/// Also marks the in-process `SETTINGS_DIRTY` atomic so the App-level
/// refresh poller (installed at startup via
/// `start_settings_refresh_poller`) can pick the change up and call
/// `cx.refresh_windows()` — that's what makes settings tweaked in one
/// window propagate to every other open window (Preferences, Style
/// Configurator, sibling editor windows) without each call site having
/// to thread `cx` through manually.
pub(crate) fn replace_and_save(mut next: Settings) {
    clamp_indent_widths(&mut next);
    if let Ok(mut g) = global().lock() {
        *g = next.clone();
    }
    save_to_disk(&next);
    SETTINGS_DIRTY.store(true, std::sync::atomic::Ordering::Release);
}

/// Set on every `replace_and_save`; cleared by the refresh poller after
/// it schedules a window redraw. Atomic is enough: latency tolerance is
/// one poll tick.
static SETTINGS_DIRTY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Spawn an App-scoped task that polls `SETTINGS_DIRTY` at 100 ms cadence
/// and calls `cx.refresh_windows()` whenever a settings mutation lands.
/// Detached — runs for the lifetime of the app.
///
/// Doubles as the cross-process settings file-watcher: each tick stats
/// `settings.toml`, and if its mtime has advanced past the last value we
/// recorded (via our own `load_from_disk` / `save_to_disk`), the file was
/// edited by a sibling notepad-studio process or by hand. Reload the
/// global cache + flag dirty so the same `refresh_windows` path fires.
pub(crate) fn start_settings_refresh_poller(cx: &mut gpui::App) {
    cx.spawn(async move |cx| {
        loop {
            gpui::Timer::after(std::time::Duration::from_millis(100)).await;
            check_external_settings_change();
            if SETTINGS_DIRTY
                .swap(false, std::sync::atomic::Ordering::Acquire)
            {
                let _ = cx.update(|cx| cx.refresh_windows());
            }
        }
    })
    .detach();
}

/// One file-watcher tick. Stat `settings.toml`; if the mtime is newer than
/// the value we last recorded, treat it as an external edit: reload from
/// disk, replace the global cache, and raise `SETTINGS_DIRTY` so the
/// existing poll path fires `refresh_windows`. Our own writes update
/// `LAST_SEEN_MTIME` inside `save_to_disk`, so they never trip this check.
fn check_external_settings_change() {
    let Some(path) = settings_path() else {
        return;
    };
    let Some(current) = current_mtime(&path) else {
        return;
    };
    let last = last_seen_mtime().lock().ok().and_then(|g| *g);
    match last {
        Some(prev) if current <= prev => return,
        _ => {}
    }
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    // Strict on purpose: a file caught mid-edit by hand shouldn't reset
    // the fields that don't parse yet. Wait for the next save instead.
    let mut next: Settings = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return,
    };
    clamp_indent_widths(&mut next);
    if let Ok(mut g) = global().lock() {
        *g = next;
    }
    if let Ok(mut g) = last_seen_mtime().lock() {
        *g = Some(current);
    }
    SETTINGS_DIRTY.store(true, std::sync::atomic::Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::TempDir;

    #[test]
    fn hand_edited_indent_widths_are_pulled_into_range() {
        let dir = TempDir::new("settings-width");
        let p = dir.join("settings.toml");
        let huge = i64::MAX;
        let src = format!(
            "tab_width = {huge}\n[language_indent_overrides.rust]\nuses_tabs = false\nwidth = 0\n\
             [language_indent_overrides.python]\nuses_tabs = true\nwidth = {huge}\n"
        );
        assert!(std::fs::write(&p, src).is_ok());
        // The old loader handed these straight to `" ".repeat(width)`, which
        // aborts the process on the failed allocation (no unwinding).
        let s = load_from_path(&p);
        assert_eq!(s.tab_width, TAB_WIDTH_MAX);
        assert_eq!(s.language_indent_overrides.get("rust").map(|o| o.width), Some(TAB_WIDTH_MIN));
        assert_eq!(s.language_indent_overrides.get("python").map(|o| o.width), Some(TAB_WIDTH_MAX));
    }

    #[test]
    fn one_bad_value_resets_only_that_setting() {
        let dir = TempDir::new("settings-salvage");
        let p = dir.join("settings.toml");
        assert!(std::fs::write(&p, "tab_width = \"wide\"\nstatus_bar_hidden = true\n").is_ok());
        let s = load_from_path(&p);
        assert!(s.status_bar_hidden, "the good field survives");
        assert_eq!(s.tab_width, TAB_SPACES);
        assert!(dir.join("settings.toml.corrupt").exists());
    }

    #[test]
    fn unreadable_settings_file_is_kept_aside() {
        let dir = TempDir::new("settings-junk");
        let p = dir.join("settings.toml");
        assert_eq!(load_from_path(&p).tab_width, TAB_SPACES);
        assert!(std::fs::write(&p, b"\x00\x01 = = [").is_ok());
        assert_eq!(load_from_path(&p).tab_width, TAB_SPACES);
        assert!(!p.exists());
        assert!(dir.join("settings.toml.corrupt").exists());
    }

    #[test]
    fn settings_round_trip() {
        let dir = TempDir::new("settings-save");
        let p = dir.join("settings.toml");
        let mut s = Settings {
            tab_width: 8,
            status_bar_hidden: true,
            ..Settings::default()
        };
        s.language_indent_overrides
            .insert("rust".into(), IndentOverride { uses_tabs: true, width: 2 });
        assert!(save_to_path(&p, &s).is_ok());
        let got = load_from_path(&p);
        assert_eq!(got.tab_width, 8);
        assert!(got.status_bar_hidden);
        assert_eq!(
            got.language_indent_overrides.get("rust"),
            Some(&IndentOverride { uses_tabs: true, width: 2 })
        );
        assert!(!dir.join("settings.toml.tmp").exists());
    }
}
