// Preferences dialog — Notepad++-style settings window.
//
// NPP layout (PowerEditor/src/WinControls/Preference/preference.rc):
//   ┌────────────────────────────────────────────────────┐
//   │ [Editing  ]  ┌───── Tab Settings ──────┐           │
//   │ [Insert   ]  │ Tab size: [____]         │          │
//   │ [...      ]  └──────────────────────────┘           │
//   │              ┌───── Line Operations ───┐            │
//   │              │ Edge column: [____]     │            │
//   │              └─────────────────────────┘            │
//   │                                                    │
//   │                                       [ Close ]    │
//   └────────────────────────────────────────────────────┘
//
// Module split: each NPP category renders its content from a sibling
// `panel_*.rs` (e.g. `general.rs`). Shared NPP-style primitives live in
// `widgets.rs`. This file owns the dialog entity, lifecycle, IME plumbing,
// key handling, sidebar, panel router, and footer.
//
// Lifecycle mirrors `shortcut_mapper.rs`: a single window-handle cached at
// module scope, recreated on each open.
//
// State + persistence: a draft `Settings` lives on the dialog entity. The
// user types into draft fields; on focus-out (text inputs) and on click
// (buttons / radios) the draft commits to `crate::settings::replace_and_save`.
// Close just closes the window — there is NO Apply / OK / Cancel because
// NPP itself doesn't have those buttons either (every change is live).
//
// Text input: the dialog implements `EntityInputHandler` so IME / paste
// route to whichever field is focused. A small `DialogTextInput` element
// renders the field, registers the input handler against the right focus
// handle, and paints the caret.

mod auto_completion;
mod backup;
mod cloud_link;
mod color_picker;
mod dark_mode;
mod default_directory;
mod default_lang_dropdown;
mod delimiter;
mod editing1;
mod editing2;
mod file_association;
mod general;
mod highlighting;
mod indentation;
mod language;
mod margins;
mod misc;
mod multi_instance;
mod new_document;
mod performance;
mod print;
mod recent_files;
mod reset_dropdown;
mod search_engine;
mod searching;
mod user_pair_input;
mod widgets;

use std::cell::RefCell;
use std::ops::Range;

use gpui::{
    App, Bounds, Context, CursorStyle, Entity, EntityInputHandler, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render, SharedString, Styled,
    Subscription, TitlebarOptions, UTF16Selection, WeakEntity, Window, WindowAppearance,
    WindowHandle, WindowKind, WindowOptions, div, point, prelude::*, px, rems, rgb, size,
};

use crate::actions::{ViewZoomIn, ViewZoomOut, ViewZoomReset};
use crate::consts::{ZOOM_DEFAULT, ZOOM_REM_BASE};
use crate::settings::{
    self, AUTOCOMPLETION_FROM_CHARS_MAX, AUTOCOMPLETION_FROM_CHARS_MIN,
    BACKUP_PERIODIC_INTERVAL_MAX_MS, BACKUP_PERIODIC_INTERVAL_MIN_MS, CARET_BLINK_RATE_MAX_MS,
    CARET_BLINK_RATE_MIN_MS, DEFAULT_AUTOCOMPLETION_FROM_CHARS, DEFAULT_BACKUP_PERIODIC_INTERVAL_MS,
    DEFAULT_CARET_BLINK_RATE_MS, DEFAULT_DATETIME_CUSTOM_FORMAT,
    DEFAULT_LARGE_FILE_THRESHOLD_BYTES, DEFAULT_RECENT_FILES_MAX,
    DEFAULT_SPLIT_LINES_EDGE_COLUMN, RECENT_FILES_MAX_HARD_CAP, Settings,
};
use crate::text::utf16::{
    input_selection_range_utf16, replace_input_utf16, slice_str_chars_utf16,
};
use crate::theme::{Theme, ThemeMode};
use crate::workspace::Workspace;

use widgets::{SidebarResizeHandle, move_caret_or_select, next_grapheme, prev_grapheme};

thread_local! {
    static OPEN_WINDOW: RefCell<Option<WindowHandle<PreferencesDialog>>> =
        const { RefCell::new(None) };
}

// --- categories + fields ------------------------------------------------

/// All 22 NPP Preference categories, in NPP listbox order.
/// Source: `notepad-plus-plus/PowerEditor/src/WinControls/Preference/preferenceDlg.cpp:312-335`.
/// Each category renders its own panel in `render_content_panel`; categories
/// without a backing Notepad Studio feature show a single deferral message instead
/// of fake controls (no mock or placeholder state).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Category {
    General,
    Editing1,
    Editing2,
    DarkMode,
    MarginsBorderEdge,
    NewDocument,
    DefaultDirectory,
    RecentFilesHistory,
    FileAssociation,
    Language,
    Indentation,
    Highlighting,
    Print,
    Searching,
    Backup,
    AutoCompletion,
    MultiInstanceDate,
    Delimiter,
    Performance,
    CloudLink,
    SearchEngine,
    Misc,
}

impl Category {
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Editing1 => "Editing 1",
            Self::Editing2 => "Editing 2",
            Self::DarkMode => "Dark Mode",
            Self::MarginsBorderEdge => "Margins/Border/Edge",
            Self::NewDocument => "New Document",
            Self::DefaultDirectory => "Default Directory",
            Self::RecentFilesHistory => "Recent Files History",
            Self::FileAssociation => "File Association",
            Self::Language => "Language",
            Self::Indentation => "Indentation",
            Self::Highlighting => "Highlighting",
            Self::Print => "Print",
            Self::Searching => "Searching",
            Self::Backup => "Backup",
            Self::AutoCompletion => "Auto-Completion",
            Self::MultiInstanceDate => "Multi-Instance & Date",
            Self::Delimiter => "Delimiter",
            Self::Performance => "Performance",
            Self::CloudLink => "Cloud & Link",
            Self::SearchEngine => "Search Engine",
            Self::Misc => "MISC.",
        }
    }

    /// Stable id used to persist the active panel across reopens. Stays
    /// stable even if NPP reorders the listbox in a future revision —
    /// reordering `all()` won't strand the user on the wrong panel.
    fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Editing1 => "editing1",
            Self::Editing2 => "editing2",
            Self::DarkMode => "dark_mode",
            Self::MarginsBorderEdge => "margins_border_edge",
            Self::NewDocument => "new_document",
            Self::DefaultDirectory => "default_directory",
            Self::RecentFilesHistory => "recent_files_history",
            Self::FileAssociation => "file_association",
            Self::Language => "language",
            Self::Indentation => "indentation",
            Self::Highlighting => "highlighting",
            Self::Print => "print",
            Self::Searching => "searching",
            Self::Backup => "backup",
            Self::AutoCompletion => "auto_completion",
            Self::MultiInstanceDate => "multi_instance_date",
            Self::Delimiter => "delimiter",
            Self::Performance => "performance",
            Self::CloudLink => "cloud_link",
            Self::SearchEngine => "search_engine",
            Self::Misc => "misc",
        }
    }

    fn from_id(s: &str) -> Self {
        Self::all()
            .iter()
            .copied()
            .find(|c| c.id() == s)
            .unwrap_or(Self::General)
    }

    /// Display order in the left-side category list. Matches NPP listbox
    /// order from `preferenceDlg.cpp:312-335`.
    fn all() -> &'static [Category] {
        &[
            Self::General,
            Self::Editing1,
            Self::Editing2,
            Self::DarkMode,
            Self::MarginsBorderEdge,
            Self::NewDocument,
            Self::DefaultDirectory,
            Self::RecentFilesHistory,
            Self::FileAssociation,
            Self::Language,
            Self::Indentation,
            Self::Highlighting,
            Self::Print,
            Self::Searching,
            Self::Backup,
            Self::AutoCompletion,
            Self::MultiInstanceDate,
            Self::Delimiter,
            Self::Performance,
            Self::CloudLink,
            Self::SearchEngine,
            Self::Misc,
        ]
    }

    /// First (and only) editable field for a category, if any. Used by
    /// `switch_category` to focus the input when the user moves to a
    /// panel that hosts one — most panels return `None` and the dialog's
    /// default focus stays where it was.
    fn first_field(self) -> Option<FieldId> {
        match self {
            Self::Indentation => Some(FieldId::TabWidth),
            Self::MarginsBorderEdge => Some(FieldId::SplitColumn),
            Self::MultiInstanceDate => Some(FieldId::DateTimeFormat),
            Self::RecentFilesHistory => Some(FieldId::RecentFilesMax),
            Self::Editing1 => Some(FieldId::CaretBlinkRateMs),
            Self::Performance => Some(FieldId::LargeFileThresholdMb),
            Self::Backup => Some(FieldId::BackupPeriodicIntervalMs),
            Self::Print => Some(FieldId::PrintMarginTopMm),
            Self::AutoCompletion => Some(FieldId::AutocompletionFromChars),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FieldId {
    TabWidth,
    SplitColumn,
    DateTimeFormat,
    RecentFilesMax,
    CaretBlinkRateMs,
    /// NPP `Default Directory ▸ Set your default directory:`. Free-text path
    /// edited only when the user has picked the `Custom` radio.
    DefaultDirectoryCustom,
    /// NPP `Performance ▸ File size threshold (in MB)`. Persisted as bytes
    /// in `Settings::large_file_threshold_bytes`; this field edits MB.
    LargeFileThresholdMb,
    /// NPP `Search Engine ▸ Custom`. Free-text URL template containing
    /// `$(CURRENT_WORD)` for query substitution. Edited only when the
    /// `Custom` radio is chosen; the other radios use built-in templates.
    SearchEngineCustomUrl,
    /// NPP `Backup ▸ Snapshot ▸ every N seconds`. Edits in milliseconds;
    /// persisted in `Settings::backup_periodic_interval_ms`.
    BackupPeriodicIntervalMs,
    /// NPP `Backup ▸ Custom Backup Directory ▸ path edit`. Free-text path;
    /// persisted in `Settings::backup_custom_dir`.
    BackupCustomDir,
    /// NPP `Print ▸ Margins (mm)` — four numeric fields. Persisted in
    /// `Settings::print_margin_*_mm`.
    PrintMarginTopMm,
    PrintMarginLeftMm,
    PrintMarginRightMm,
    PrintMarginBottomMm,
    /// NPP `Print ▸ Header ▸ {Left/Centre/Right}` — three free-text
    /// templates. The variable token language (`$(FILE_NAME)` etc.) is
    /// resolved at print time by `crate::print::substitute_template`.
    PrintHeaderLeft,
    PrintHeaderMiddle,
    PrintHeaderRight,
    /// NPP `Print ▸ Header ▸ font` — name + size text fields. Bold/italic
    /// are checkboxes (no FieldId entries).
    PrintHeaderFontName,
    PrintHeaderFontSize,
    /// NPP `Print ▸ Footer ▸ {Left/Centre/Right}`. Same as header.
    PrintFooterLeft,
    PrintFooterMiddle,
    PrintFooterRight,
    PrintFooterFontName,
    PrintFooterFontSize,
    /// NPP `Auto-Completion ▸ From N-th character`. Numeric, range 1..=9.
    /// Persisted in `Settings::autocompletion_from_chars`. Engine consumer
    /// pending — value is only stored today.
    AutocompletionFromChars,
}

/// Numeric vs. free-text. Numeric fields filter typed input to digits only
/// in `replace_text_in_range`; free-text accepts anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FieldKind {
    Numeric,
    Text,
}

impl FieldId {
    fn kind(self) -> FieldKind {
        match self {
            Self::TabWidth
            | Self::SplitColumn
            | Self::RecentFilesMax
            | Self::CaretBlinkRateMs
            | Self::LargeFileThresholdMb
            | Self::BackupPeriodicIntervalMs
            | Self::PrintMarginTopMm
            | Self::PrintMarginLeftMm
            | Self::PrintMarginRightMm
            | Self::PrintMarginBottomMm
            | Self::PrintHeaderFontSize
            | Self::PrintFooterFontSize
            | Self::AutocompletionFromChars => FieldKind::Numeric,
            Self::DateTimeFormat
            | Self::DefaultDirectoryCustom
            | Self::SearchEngineCustomUrl
            | Self::BackupCustomDir
            | Self::PrintHeaderLeft
            | Self::PrintHeaderMiddle
            | Self::PrintHeaderRight
            | Self::PrintHeaderFontName
            | Self::PrintFooterLeft
            | Self::PrintFooterMiddle
            | Self::PrintFooterRight
            | Self::PrintFooterFontName => FieldKind::Text,
        }
    }

    /// `(min, max, step)` for every numeric field — drives the up/down
    /// spinner buttons rendered alongside the input. `None` for non-numeric
    /// fields. Bounds mirror the clamps applied by `commit_all` so the
    /// spinner can't push the field out of range; step values follow NPP
    /// defaults (millisecond fields step by their unit, MB / px / mm step
    /// by 1, etc.).
    pub(super) fn numeric_bounds(self) -> Option<(i64, i64, i64)> {
        match self {
            Self::TabWidth => Some((
                crate::settings::TAB_WIDTH_MIN as i64,
                crate::settings::TAB_WIDTH_MAX as i64,
                1,
            )),
            Self::SplitColumn => Some((1, 1000, 1)),
            Self::RecentFilesMax => Some((0, RECENT_FILES_MAX_HARD_CAP as i64, 1)),
            Self::CaretBlinkRateMs => {
                Some((CARET_BLINK_RATE_MIN_MS as i64, CARET_BLINK_RATE_MAX_MS as i64, 50))
            }
            Self::LargeFileThresholdMb => Some((
                performance::THRESHOLD_MIN_MB as i64,
                performance::THRESHOLD_MAX_MB as i64,
                1,
            )),
            Self::BackupPeriodicIntervalMs => Some((
                BACKUP_PERIODIC_INTERVAL_MIN_MS as i64,
                BACKUP_PERIODIC_INTERVAL_MAX_MS as i64,
                1000,
            )),
            Self::PrintMarginTopMm
            | Self::PrintMarginLeftMm
            | Self::PrintMarginRightMm
            | Self::PrintMarginBottomMm => Some((0, 50, 1)),
            Self::PrintHeaderFontSize | Self::PrintFooterFontSize => Some((6, 72, 1)),
            Self::AutocompletionFromChars => Some((
                AUTOCOMPLETION_FROM_CHARS_MIN as i64,
                AUTOCOMPLETION_FROM_CHARS_MAX as i64,
                1,
            )),
            _ => None,
        }
    }
}

/// One editable field's text + cursor + anchor. Mirrors the per-field
/// state pattern used by `FindState` / `FifState` so the same UTF-16
/// helpers can drive IME for free.
#[derive(Default, Clone)]
pub(super) struct FieldState {
    pub(super) text: String,
    pub(super) cursor: usize,
    pub(super) anchor: usize,
}

impl FieldState {
    fn from_text(text: String) -> Self {
        let cursor = text.len();
        Self { anchor: cursor, cursor, text }
    }

    pub(super) fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    pub(super) fn collapse_to(&mut self, byte: usize) {
        let b = byte.min(self.text.len());
        self.cursor = b;
        self.anchor = b;
    }

    fn ordered(&self) -> Range<usize> {
        let lo = self.cursor.min(self.anchor);
        let hi = self.cursor.max(self.anchor);
        lo..hi
    }
}

// --- dialog entity -------------------------------------------------------

/// Resize-drag state for the sidebar divider. Holds the mouse-down position
/// and the sidebar width at drag start so move events can compute
/// `start_width + (mouse_x - start_x)` cleanly.
#[derive(Clone, Copy)]
pub(super) struct SidebarDrag {
    pub(super) start_mouse_x: Pixels,
    pub(super) start_width: Pixels,
}

/// Min/max bounds for the sidebar drag handle. Min is wide enough for the
/// shortest sensible label; max is computed at drag time as half the
/// dialog's viewport width.
pub(super) const SIDEBAR_MIN_WIDTH_PX: f32 = 100.0;

pub(crate) struct PreferencesDialog {
    focus: FocusHandle,
    /// Lazy per-field focus handles. Created on first paint so the
    /// constructor stays narrow.
    tab_width_focus: FocusHandle,
    split_column_focus: FocusHandle,
    datetime_format_focus: FocusHandle,
    recent_files_max_focus: FocusHandle,
    caret_blink_rate_focus: FocusHandle,
    default_directory_custom_focus: FocusHandle,
    large_file_threshold_mb_focus: FocusHandle,
    search_engine_custom_url_focus: FocusHandle,
    backup_periodic_interval_ms_focus: FocusHandle,
    backup_custom_dir_focus: FocusHandle,
    print_margin_top_mm_focus: FocusHandle,
    print_margin_left_mm_focus: FocusHandle,
    print_margin_right_mm_focus: FocusHandle,
    print_margin_bottom_mm_focus: FocusHandle,
    print_header_left_focus: FocusHandle,
    print_header_middle_focus: FocusHandle,
    print_header_right_focus: FocusHandle,
    print_header_font_name_focus: FocusHandle,
    print_header_font_size_focus: FocusHandle,
    print_footer_left_focus: FocusHandle,
    print_footer_middle_focus: FocusHandle,
    print_footer_right_focus: FocusHandle,
    print_footer_font_name_focus: FocusHandle,
    print_footer_font_size_focus: FocusHandle,
    autocompletion_from_chars_focus: FocusHandle,
    pub(super) fields: PreferencesFields,
    active_category: Category,
    /// Cached for footer status. Set by `commit_field` and shown briefly.
    pub(super) last_action: Option<String>,
    theme_mode: ThemeMode,
    /// Width of the left category list. Loaded from settings on open;
    /// updated live during drag; persisted on mouse-up.
    pub(super) sidebar_width: Pixels,
    /// `Some` while the user is mid-drag on the sidebar divider.
    pub(super) sidebar_drag: Option<SidebarDrag>,
    /// Mirrors the active main-window's zoom factor. Set in `new` from the
    /// active Workspace (or `ZOOM_DEFAULT` when none exists), updated live
    /// by `_zoom_observer` whenever the Workspace re-notifies, and pushed
    /// into the dialog window's `set_rem_size` on the next render tick.
    zoom_target: f32,
    /// Last zoom factor pushed to this dialog window. Render compares this
    /// against `zoom_target` and only calls `set_rem_size` on a real change.
    zoom_applied: f32,
    /// Live subscription on the Workspace entity. Keeps the zoom observer
    /// alive for the dialog's lifetime; dropping the dialog drops this and
    /// the live-sync stops automatically.
    _zoom_observer: Option<Subscription>,
    /// Cached OS appearance for the dialog window — kept in lockstep with
    /// `Workspace::system_appearance` so the dialog's theme honours
    /// `ThemeMode::FollowSystem`. Observed live via
    /// `cx.observe_window_appearance` installed at construction.
    system_appearance: WindowAppearance,
    /// Live subscription on the dialog's own window-appearance stream. Keeps
    /// `system_appearance` fresh while the dialog is open; dropped on close.
    _appearance_observer: Option<Subscription>,
    /// `Some` while the customized-slot color picker popover is open.
    /// Dismissed by clicking outside the popover or selecting a swatch.
    color_picker: Option<color_picker::ColorPickerState>,
    /// `Some` while the Dark Mode → Tones Reset split-button dropdown is
    /// open. Dismissed by clicking outside the menu or selecting a row.
    reset_dropdown: Option<reset_dropdown::ResetDropdownState>,
    /// `true` while the New Document ▸ Default language picker dropdown is
    /// open. Anchored under its trigger button via the find-dialog popover
    /// pattern; dismissed by an outside click or by picking a row.
    pub(super) default_lang_dropdown_open: bool,
    /// Auto-Insert ▸ user-defined matched pairs — one row per editable pair.
    /// Mirrors `Settings::auto_insert_user_pairs`. Every keystroke / add /
    /// delete writes the vec back to settings via `sync_user_pairs_to_settings`.
    pub(super) user_pairs: Vec<user_pair_input::UserPairBuffer>,
    /// Dark Mode ▸ customized-slot color picker "More Colours…" free-form
    /// hex entry. Lives at dialog scope so the same input survives across
    /// picker close/reopen; reset on each `open_color_picker` call.
    pub(super) picker_hex: FieldState,
    pub(super) picker_hex_focus: FocusHandle,
    /// HSV decomposition of `picker_hex.text` for the embedded HSV
    /// surface (SV gradient + hue strip). The hex buffer stays the
    /// source of truth — these derive on every buffer change so the
    /// SV / hue thumbs paint at the right spot. Kept here (not
    /// recomputed every render) so the user's hue choice survives a
    /// drag through grey (s = 0) where hue is mathematically undefined:
    /// `hsv_surface::sync_from_buffer` only overwrites hue when s > 0.
    pub(super) picker_hue: f32,
    pub(super) picker_sat: f32,
    pub(super) picker_val: f32,
    /// Which HSV surface (SV gradient or hue strip) is being dragged.
    /// Mirrors the field on `StyleConfiguratorDialog`; consumed by the
    /// window-global mouse-move handler inside `hsv_surface`.
    pub(super) picker_hsv_drag: crate::ui::style_configurator::hsv_surface::HsvDrag,
    /// Selected row in the Indentation panel's per-language listbox. Index
    /// into `LanguageId::all()`. Drives which language the indent radios,
    /// width cycler, and Reset button operate on. Defaults to 0 (first
    /// language in the alphabetised list) on dialog open.
    pub(super) selected_indent_lang_idx: usize,
}

pub(super) struct PreferencesFields {
    tab_width: FieldState,
    split_column: FieldState,
    datetime_format: FieldState,
    recent_files_max: FieldState,
    caret_blink_rate: FieldState,
    default_directory_custom: FieldState,
    large_file_threshold_mb: FieldState,
    search_engine_custom_url: FieldState,
    backup_periodic_interval_ms: FieldState,
    backup_custom_dir: FieldState,
    print_margin_top_mm: FieldState,
    print_margin_left_mm: FieldState,
    print_margin_right_mm: FieldState,
    print_margin_bottom_mm: FieldState,
    print_header_left: FieldState,
    print_header_middle: FieldState,
    print_header_right: FieldState,
    print_header_font_name: FieldState,
    print_header_font_size: FieldState,
    print_footer_left: FieldState,
    print_footer_middle: FieldState,
    print_footer_right: FieldState,
    print_footer_font_name: FieldState,
    print_footer_font_size: FieldState,
    autocompletion_from_chars: FieldState,
}

impl PreferencesFields {
    fn from_settings(s: &Settings) -> Self {
        let threshold_mb = (s.large_file_threshold_bytes / (1024 * 1024)).to_string();
        Self {
            tab_width: FieldState::from_text(s.tab_width.to_string()),
            split_column: FieldState::from_text(s.split_lines_edge_column.to_string()),
            datetime_format: FieldState::from_text(s.datetime_custom_format.clone()),
            recent_files_max: FieldState::from_text(s.recent_files_max.to_string()),
            caret_blink_rate: FieldState::from_text(s.caret_blink_rate_ms.to_string()),
            default_directory_custom: FieldState::from_text(s.default_directory_custom.clone()),
            large_file_threshold_mb: FieldState::from_text(threshold_mb),
            search_engine_custom_url: FieldState::from_text(
                s.search_engine_custom_url.clone(),
            ),
            backup_periodic_interval_ms: FieldState::from_text(
                s.backup_periodic_interval_ms.to_string(),
            ),
            backup_custom_dir: FieldState::from_text(s.backup_custom_dir.clone()),
            print_margin_top_mm: FieldState::from_text(s.print_margin_top_mm.to_string()),
            print_margin_left_mm: FieldState::from_text(s.print_margin_left_mm.to_string()),
            print_margin_right_mm: FieldState::from_text(s.print_margin_right_mm.to_string()),
            print_margin_bottom_mm: FieldState::from_text(s.print_margin_bottom_mm.to_string()),
            print_header_left: FieldState::from_text(s.print_header_left.clone()),
            print_header_middle: FieldState::from_text(s.print_header_middle.clone()),
            print_header_right: FieldState::from_text(s.print_header_right.clone()),
            print_header_font_name: FieldState::from_text(s.print_header_font_name.clone()),
            print_header_font_size: FieldState::from_text(s.print_header_font_size.to_string()),
            print_footer_left: FieldState::from_text(s.print_footer_left.clone()),
            print_footer_middle: FieldState::from_text(s.print_footer_middle.clone()),
            print_footer_right: FieldState::from_text(s.print_footer_right.clone()),
            print_footer_font_name: FieldState::from_text(s.print_footer_font_name.clone()),
            print_footer_font_size: FieldState::from_text(s.print_footer_font_size.to_string()),
            autocompletion_from_chars: FieldState::from_text(
                s.autocompletion_from_chars.to_string(),
            ),
        }
    }

    pub(super) fn get(&self, id: FieldId) -> &FieldState {
        match id {
            FieldId::TabWidth => &self.tab_width,
            FieldId::SplitColumn => &self.split_column,
            FieldId::DateTimeFormat => &self.datetime_format,
            FieldId::RecentFilesMax => &self.recent_files_max,
            FieldId::CaretBlinkRateMs => &self.caret_blink_rate,
            FieldId::DefaultDirectoryCustom => &self.default_directory_custom,
            FieldId::LargeFileThresholdMb => &self.large_file_threshold_mb,
            FieldId::SearchEngineCustomUrl => &self.search_engine_custom_url,
            FieldId::BackupPeriodicIntervalMs => &self.backup_periodic_interval_ms,
            FieldId::BackupCustomDir => &self.backup_custom_dir,
            FieldId::PrintMarginTopMm => &self.print_margin_top_mm,
            FieldId::PrintMarginLeftMm => &self.print_margin_left_mm,
            FieldId::PrintMarginRightMm => &self.print_margin_right_mm,
            FieldId::PrintMarginBottomMm => &self.print_margin_bottom_mm,
            FieldId::PrintHeaderLeft => &self.print_header_left,
            FieldId::PrintHeaderMiddle => &self.print_header_middle,
            FieldId::PrintHeaderRight => &self.print_header_right,
            FieldId::PrintHeaderFontName => &self.print_header_font_name,
            FieldId::PrintHeaderFontSize => &self.print_header_font_size,
            FieldId::PrintFooterLeft => &self.print_footer_left,
            FieldId::PrintFooterMiddle => &self.print_footer_middle,
            FieldId::PrintFooterRight => &self.print_footer_right,
            FieldId::PrintFooterFontName => &self.print_footer_font_name,
            FieldId::PrintFooterFontSize => &self.print_footer_font_size,
            FieldId::AutocompletionFromChars => &self.autocompletion_from_chars,
        }
    }

    pub(super) fn get_mut(&mut self, id: FieldId) -> &mut FieldState {
        match id {
            FieldId::TabWidth => &mut self.tab_width,
            FieldId::SplitColumn => &mut self.split_column,
            FieldId::DateTimeFormat => &mut self.datetime_format,
            FieldId::RecentFilesMax => &mut self.recent_files_max,
            FieldId::CaretBlinkRateMs => &mut self.caret_blink_rate,
            FieldId::DefaultDirectoryCustom => &mut self.default_directory_custom,
            FieldId::LargeFileThresholdMb => &mut self.large_file_threshold_mb,
            FieldId::SearchEngineCustomUrl => &mut self.search_engine_custom_url,
            FieldId::BackupPeriodicIntervalMs => &mut self.backup_periodic_interval_ms,
            FieldId::BackupCustomDir => &mut self.backup_custom_dir,
            FieldId::PrintMarginTopMm => &mut self.print_margin_top_mm,
            FieldId::PrintMarginLeftMm => &mut self.print_margin_left_mm,
            FieldId::PrintMarginRightMm => &mut self.print_margin_right_mm,
            FieldId::PrintMarginBottomMm => &mut self.print_margin_bottom_mm,
            FieldId::PrintHeaderLeft => &mut self.print_header_left,
            FieldId::PrintHeaderMiddle => &mut self.print_header_middle,
            FieldId::PrintHeaderRight => &mut self.print_header_right,
            FieldId::PrintHeaderFontName => &mut self.print_header_font_name,
            FieldId::PrintHeaderFontSize => &mut self.print_header_font_size,
            FieldId::PrintFooterLeft => &mut self.print_footer_left,
            FieldId::PrintFooterMiddle => &mut self.print_footer_middle,
            FieldId::PrintFooterRight => &mut self.print_footer_right,
            FieldId::PrintFooterFontName => &mut self.print_footer_font_name,
            FieldId::PrintFooterFontSize => &mut self.print_footer_font_size,
            FieldId::AutocompletionFromChars => &mut self.autocompletion_from_chars,
        }
    }
}

impl PreferencesDialog {
    fn new(
        theme_mode: ThemeMode,
        workspace: Option<Entity<Workspace>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = settings::snapshot();
        let fields = PreferencesFields::from_settings(&snapshot);
        let saved_sidebar_width = snapshot.preferences_sidebar_width;
        let sidebar_width = px(if saved_sidebar_width.is_finite() {
            saved_sidebar_width.max(SIDEBAR_MIN_WIDTH_PX)
        } else {
            settings::DEFAULT_PREFERENCES_SIDEBAR_WIDTH
        });
        let active_category = Category::from_id(&snapshot.preferences_active_panel);
        let initial_zoom = workspace
            .as_ref()
            .map(|w| w.read(cx).zoom_level)
            .unwrap_or(ZOOM_DEFAULT);
        let observer = workspace.as_ref().map(|ws| {
            cx.observe(ws, |this, ws, cx| {
                let new_zoom = ws.read(cx).zoom_level;
                if (this.zoom_target - new_zoom).abs() > f32::EPSILON {
                    this.zoom_target = new_zoom;
                    cx.notify();
                }
            })
        });
        Self {
            focus: cx.focus_handle(),
            tab_width_focus: cx.focus_handle(),
            split_column_focus: cx.focus_handle(),
            datetime_format_focus: cx.focus_handle(),
            recent_files_max_focus: cx.focus_handle(),
            caret_blink_rate_focus: cx.focus_handle(),
            default_directory_custom_focus: cx.focus_handle(),
            large_file_threshold_mb_focus: cx.focus_handle(),
            search_engine_custom_url_focus: cx.focus_handle(),
            backup_periodic_interval_ms_focus: cx.focus_handle(),
            backup_custom_dir_focus: cx.focus_handle(),
            print_margin_top_mm_focus: cx.focus_handle(),
            print_margin_left_mm_focus: cx.focus_handle(),
            print_margin_right_mm_focus: cx.focus_handle(),
            print_margin_bottom_mm_focus: cx.focus_handle(),
            print_header_left_focus: cx.focus_handle(),
            print_header_middle_focus: cx.focus_handle(),
            print_header_right_focus: cx.focus_handle(),
            print_header_font_name_focus: cx.focus_handle(),
            print_header_font_size_focus: cx.focus_handle(),
            print_footer_left_focus: cx.focus_handle(),
            print_footer_middle_focus: cx.focus_handle(),
            print_footer_right_focus: cx.focus_handle(),
            print_footer_font_name_focus: cx.focus_handle(),
            print_footer_font_size_focus: cx.focus_handle(),
            autocompletion_from_chars_focus: cx.focus_handle(),
            fields,
            active_category,
            last_action: None,
            theme_mode,
            sidebar_width,
            sidebar_drag: None,
            zoom_target: initial_zoom,
            zoom_applied: initial_zoom,
            _zoom_observer: observer,
            // Defaulted; the real appearance + observer are installed later
            // by `install_appearance_observer` from `open_or_focus`'s deferred
            // path (which is when the dialog window is fully on-stack).
            system_appearance: WindowAppearance::Dark,
            _appearance_observer: None,
            color_picker: None,
            reset_dropdown: None,
            default_lang_dropdown_open: false,
            user_pairs: snapshot
                .auto_insert_user_pairs
                .iter()
                .map(|(o, c)| user_pair_input::UserPairBuffer::new(o.clone(), c.clone(), cx))
                .collect(),
            picker_hex: FieldState::from_text(String::new()),
            picker_hex_focus: cx.focus_handle(),
            picker_hue: 0.0,
            picker_sat: 0.0,
            picker_val: 0.0,
            picker_hsv_drag: crate::ui::style_configurator::hsv_surface::HsvDrag::None,
            selected_indent_lang_idx: 0,
        }
    }

    /// Append an empty pair row. The user types into it; the empty row stays
    /// persisted until populated (matched-pair lookup ignores empty halves so
    /// it has no runtime effect on typing).
    pub(super) fn add_user_pair(&mut self, cx: &mut Context<Self>) {
        self.user_pairs
            .push(user_pair_input::UserPairBuffer::new(String::new(), String::new(), cx));
        self.sync_user_pairs_to_settings(cx);
        self.last_action = Some("User pair row added".to_string());
        cx.notify();
    }

    /// Drop pair row `idx`. Out-of-range indices are silently ignored — can
    /// happen on a stale click during fast double-click delete.
    pub(super) fn remove_user_pair(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.user_pairs.len() {
            return;
        }
        self.user_pairs.remove(idx);
        self.sync_user_pairs_to_settings(cx);
        self.last_action = Some("User pair row removed".to_string());
        cx.notify();
    }

    /// Mirror the current `user_pairs` Vec into `Settings::auto_insert_user_pairs`
    /// and save. Called after every add / delete / keystroke. Empty halves are
    /// preserved verbatim — `workspace/input_handler.rs::pair_close_for` skips
    /// rows whose either half is empty, so the user never sees a runtime
    /// effect from half-typed rows.
    pub(super) fn sync_user_pairs_to_settings(&mut self, cx: &mut Context<Self>) {
        let mut next = settings::snapshot();
        next.auto_insert_user_pairs = self
            .user_pairs
            .iter()
            .map(|p| (p.open.text.clone(), p.close.text.clone()))
            .collect();
        settings::replace_and_save(next);
        cx.notify();
    }

    /// Which user-pair half (if any) currently owns keyboard focus. Mirrors
    /// `focused_field` but iterates the dynamic `user_pairs` vec.
    pub(super) fn focused_user_pair(
        &self,
        window: &Window,
    ) -> Option<(usize, user_pair_input::PairHalf)> {
        for (i, p) in self.user_pairs.iter().enumerate() {
            if p.open_focus.is_focused(window) {
                return Some((i, user_pair_input::PairHalf::Open));
            }
            if p.close_focus.is_focused(window) {
                return Some((i, user_pair_input::PairHalf::Close));
            }
        }
        None
    }

    /// Open the customized-slot color picker for `slot_idx`, anchored at
    /// the given dialog-local pixel offset. Stale state on the previous
    /// pick (if any) is overwritten.
    pub(super) fn open_color_picker(
        &mut self,
        slot_idx: usize,
        anchor_x: Pixels,
        anchor_y: Pixels,
        cx: &mut Context<Self>,
    ) {
        self.color_picker = Some(color_picker::ColorPickerState {
            slot_idx,
            anchor_x,
            anchor_y,
        });
        // Seed hex buffer from the slot's current color so the HSV
        // thumbs land on it (instead of black) when the picker opens.
        // No prior pick → empty buffer + zeroed HSV, mirroring the old
        // behaviour for first-open slots.
        let snap = settings::snapshot();
        let seed = snap
            .customized_slots
            .get(slot_idx)
            .copied()
            .flatten();
        let buf = seed
            .map(crate::style_model::style::format_hex_rgb)
            .unwrap_or_default();
        self.picker_hex.cursor = buf.len();
        self.picker_hex.anchor = buf.len();
        self.picker_hex.text = buf;
        self.picker_hue = 0.0;
        self.picker_sat = 0.0;
        self.picker_val = 0.0;
        self.picker_hsv_drag =
            crate::ui::style_configurator::hsv_surface::HsvDrag::None;
        // sync_from_buffer derives HSV from the just-seeded hex so the
        // SV / hue thumbs paint on the current colour right away.
        crate::ui::style_configurator::hsv_surface::sync_from_buffer(self);
        cx.notify();
    }

    /// True iff the picker hex input owns keyboard focus right now.
    pub(super) fn picker_hex_is_focused(&self, window: &Window) -> bool {
        self.picker_hex_focus.is_focused(window)
    }

    /// Parse the picker hex buffer as `RRGGBB` (with or without leading `#`)
    /// and commit it as the chosen color for the open slot. No-op if the
    /// picker is closed, the buffer isn't exactly 6 hex chars, or parsing
    /// fails. Last-action footer reflects success/failure.
    pub(super) fn commit_picker_hex(&mut self, cx: &mut Context<Self>) {
        let text = self.picker_hex.text.trim().trim_start_matches('#');
        if text.len() != 6 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
            self.last_action = Some("Hex must be 6 chars (RRGGBB)".to_string());
            cx.notify();
            return;
        }
        let Ok(color) = u32::from_str_radix(text, 16) else {
            self.last_action = Some("Hex parse failed".to_string());
            cx.notify();
            return;
        };
        self.commit_color_pick(color, cx);
    }

    /// Dismiss the color picker without committing a color.
    pub(super) fn close_color_picker(&mut self, cx: &mut Context<Self>) {
        if self.color_picker.take().is_some() {
            cx.notify();
        }
    }

    /// Commit the user's color choice for the open slot, persist it, and
    /// dismiss the picker. Notifies all open Workspace windows so the new
    /// color paints on the next frame.
    pub(super) fn commit_color_pick(&mut self, color: u32, cx: &mut Context<Self>) {
        let Some(state) = self.color_picker.take() else {
            return;
        };
        let mut next = settings::snapshot();
        if next.customized_slots.len() < crate::theme::SLOT_COUNT {
            next.customized_slots
                .resize(crate::theme::SLOT_COUNT, None);
        }
        if state.slot_idx < crate::theme::SLOT_COUNT {
            next.customized_slots[state.slot_idx] = Some(color);
            settings::replace_and_save(next);
        }
        for handle in cx.windows() {
            if let Some(ws) = handle.downcast::<crate::workspace::Workspace>() {
                let _ = ws.update(cx, |_w, _, cx| cx.notify());
            }
        }
        self.last_action = Some(format!(
            "Slot color set: #{:06X}",
            color & 0x00FF_FFFF
        ));
        cx.notify();
    }

    /// Open the Dark Mode → Tones Reset dropdown menu, anchored at the
    /// given dialog-local pixel offset (typically just below the caret).
    /// Toggles: opening when one is already open is a no-op caller-side;
    /// the caret click handler calls `close_reset_dropdown` first.
    pub(super) fn open_reset_dropdown(
        &mut self,
        anchor_x: Pixels,
        anchor_y: Pixels,
        cx: &mut Context<Self>,
    ) {
        self.reset_dropdown = Some(reset_dropdown::ResetDropdownState {
            anchor_x,
            anchor_y,
        });
        cx.notify();
    }

    /// Dismiss the Reset dropdown without selecting an item.
    pub(super) fn close_reset_dropdown(&mut self, cx: &mut Context<Self>) {
        if self.reset_dropdown.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn reset_dropdown_open(&self) -> bool {
        self.reset_dropdown.is_some()
    }

    /// Toggle the New Document ▸ Default language picker dropdown. State lives
    /// directly on the dialog because the popover is anchored relative to its
    /// trigger button — no pixel offsets to stash.
    pub(super) fn open_default_lang_dropdown(&mut self, cx: &mut Context<Self>) {
        self.default_lang_dropdown_open = true;
        cx.notify();
    }

    pub(super) fn close_default_lang_dropdown(&mut self, cx: &mut Context<Self>) {
        if self.default_lang_dropdown_open {
            self.default_lang_dropdown_open = false;
            cx.notify();
        }
    }

    pub(super) fn default_lang_dropdown_is_open(&self) -> bool {
        self.default_lang_dropdown_open
    }

    /// Cache the dialog window's appearance + subscribe to live changes.
    /// Called from `open_or_focus`'s deferred path because the appearance
    /// observer needs `&mut Window`, which isn't available inside the
    /// dialog-construction closure.
    fn install_appearance_observer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.system_appearance = window.appearance();
        self._appearance_observer = Some(cx.observe_window_appearance(
            window,
            |this, win, cx| {
                let next = win.appearance();
                if this.system_appearance != next {
                    this.system_appearance = next;
                    cx.notify();
                }
            },
        ));
        cx.notify();
    }

    /// Bind the dialog to a Workspace entity for live zoom mirroring after
    /// the dialog window is open. Called from `open_or_focus`'s deferred
    /// path, since reading the workspace entity during menu dispatch
    /// would panic ("window already on the stack").
    fn install_zoom_observer(
        &mut self,
        ws: Entity<Workspace>,
        cx: &mut Context<Self>,
    ) {
        let new_zoom = ws.read(cx).zoom_level;
        if (self.zoom_target - new_zoom).abs() > f32::EPSILON {
            self.zoom_target = new_zoom;
        }
        self._zoom_observer = Some(cx.observe(&ws, |this, ws, cx| {
            let new_zoom = ws.read(cx).zoom_level;
            if (this.zoom_target - new_zoom).abs() > f32::EPSILON {
                this.zoom_target = new_zoom;
                cx.notify();
            }
        }));
        cx.notify();
    }

    /// Cmd/Ctrl + `=` handler. Delegates to the active Workspace so main
    /// + dialog stay in sync — the observer set up in `new` re-applies
    /// the bumped value to this dialog window. No-op when no Workspace
    /// exists (rare — Preferences launches from the main-window menu).
    fn zoom_in(&mut self, _: &ViewZoomIn, _window: &mut Window, cx: &mut Context<Self>) {
        delegate_zoom_to_workspace(&mut *cx, |ws, win, cx| ws.zoom_in(&ViewZoomIn, win, cx));
    }

    fn zoom_out(&mut self, _: &ViewZoomOut, _window: &mut Window, cx: &mut Context<Self>) {
        delegate_zoom_to_workspace(&mut *cx, |ws, win, cx| ws.zoom_out(&ViewZoomOut, win, cx));
    }

    fn zoom_reset(&mut self, _: &ViewZoomReset, _window: &mut Window, cx: &mut Context<Self>) {
        delegate_zoom_to_workspace(&mut *cx, |ws, win, cx| ws.zoom_reset(&ViewZoomReset, win, cx));
    }

    pub(super) fn theme(&self) -> Theme {
        let snap = settings::snapshot();
        Theme::resolve(
            self.theme_mode,
            self.system_appearance,
            snap.tone_palette,
            &snap.customized_slots_array(),
        )
    }

    pub(super) fn set_theme_mode(&mut self, mode: ThemeMode) {
        self.theme_mode = mode;
    }

    pub(super) fn focus_for(&self, id: FieldId) -> FocusHandle {
        match id {
            FieldId::TabWidth => self.tab_width_focus.clone(),
            FieldId::SplitColumn => self.split_column_focus.clone(),
            FieldId::DateTimeFormat => self.datetime_format_focus.clone(),
            FieldId::RecentFilesMax => self.recent_files_max_focus.clone(),
            FieldId::CaretBlinkRateMs => self.caret_blink_rate_focus.clone(),
            FieldId::DefaultDirectoryCustom => self.default_directory_custom_focus.clone(),
            FieldId::LargeFileThresholdMb => self.large_file_threshold_mb_focus.clone(),
            FieldId::SearchEngineCustomUrl => self.search_engine_custom_url_focus.clone(),
            FieldId::BackupPeriodicIntervalMs => self.backup_periodic_interval_ms_focus.clone(),
            FieldId::BackupCustomDir => self.backup_custom_dir_focus.clone(),
            FieldId::PrintMarginTopMm => self.print_margin_top_mm_focus.clone(),
            FieldId::PrintMarginLeftMm => self.print_margin_left_mm_focus.clone(),
            FieldId::PrintMarginRightMm => self.print_margin_right_mm_focus.clone(),
            FieldId::PrintMarginBottomMm => self.print_margin_bottom_mm_focus.clone(),
            FieldId::PrintHeaderLeft => self.print_header_left_focus.clone(),
            FieldId::PrintHeaderMiddle => self.print_header_middle_focus.clone(),
            FieldId::PrintHeaderRight => self.print_header_right_focus.clone(),
            FieldId::PrintHeaderFontName => self.print_header_font_name_focus.clone(),
            FieldId::PrintHeaderFontSize => self.print_header_font_size_focus.clone(),
            FieldId::PrintFooterLeft => self.print_footer_left_focus.clone(),
            FieldId::PrintFooterMiddle => self.print_footer_middle_focus.clone(),
            FieldId::PrintFooterRight => self.print_footer_right_focus.clone(),
            FieldId::PrintFooterFontName => self.print_footer_font_name_focus.clone(),
            FieldId::PrintFooterFontSize => self.print_footer_font_size_focus.clone(),
            FieldId::AutocompletionFromChars => self.autocompletion_from_chars_focus.clone(),
        }
    }

    fn focused_field(&self, window: &Window) -> Option<FieldId> {
        if self.tab_width_focus.is_focused(window) {
            Some(FieldId::TabWidth)
        } else if self.split_column_focus.is_focused(window) {
            Some(FieldId::SplitColumn)
        } else if self.datetime_format_focus.is_focused(window) {
            Some(FieldId::DateTimeFormat)
        } else if self.recent_files_max_focus.is_focused(window) {
            Some(FieldId::RecentFilesMax)
        } else if self.caret_blink_rate_focus.is_focused(window) {
            Some(FieldId::CaretBlinkRateMs)
        } else if self.default_directory_custom_focus.is_focused(window) {
            Some(FieldId::DefaultDirectoryCustom)
        } else if self.large_file_threshold_mb_focus.is_focused(window) {
            Some(FieldId::LargeFileThresholdMb)
        } else if self.search_engine_custom_url_focus.is_focused(window) {
            Some(FieldId::SearchEngineCustomUrl)
        } else if self.backup_periodic_interval_ms_focus.is_focused(window) {
            Some(FieldId::BackupPeriodicIntervalMs)
        } else if self.backup_custom_dir_focus.is_focused(window) {
            Some(FieldId::BackupCustomDir)
        } else if self.print_margin_top_mm_focus.is_focused(window) {
            Some(FieldId::PrintMarginTopMm)
        } else if self.print_margin_left_mm_focus.is_focused(window) {
            Some(FieldId::PrintMarginLeftMm)
        } else if self.print_margin_right_mm_focus.is_focused(window) {
            Some(FieldId::PrintMarginRightMm)
        } else if self.print_margin_bottom_mm_focus.is_focused(window) {
            Some(FieldId::PrintMarginBottomMm)
        } else if self.print_header_left_focus.is_focused(window) {
            Some(FieldId::PrintHeaderLeft)
        } else if self.print_header_middle_focus.is_focused(window) {
            Some(FieldId::PrintHeaderMiddle)
        } else if self.print_header_right_focus.is_focused(window) {
            Some(FieldId::PrintHeaderRight)
        } else if self.print_header_font_name_focus.is_focused(window) {
            Some(FieldId::PrintHeaderFontName)
        } else if self.print_header_font_size_focus.is_focused(window) {
            Some(FieldId::PrintHeaderFontSize)
        } else if self.print_footer_left_focus.is_focused(window) {
            Some(FieldId::PrintFooterLeft)
        } else if self.print_footer_middle_focus.is_focused(window) {
            Some(FieldId::PrintFooterMiddle)
        } else if self.print_footer_right_focus.is_focused(window) {
            Some(FieldId::PrintFooterRight)
        } else if self.print_footer_font_name_focus.is_focused(window) {
            Some(FieldId::PrintFooterFontName)
        } else if self.print_footer_font_size_focus.is_focused(window) {
            Some(FieldId::PrintFooterFontSize)
        } else if self.autocompletion_from_chars_focus.is_focused(window) {
            Some(FieldId::AutocompletionFromChars)
        } else {
            None
        }
    }

    /// Pull the current draft into a Settings, fixing up numeric fields to
    /// their minimum bound and leaving the strftime string alone (chrono
    /// validates lazily at format time). Non-Preferences fields (status bar
    /// hidden, etc.) come straight off the on-disk snapshot — they're
    /// written by their own toggle handlers, never via this method.
    fn commit_all(&self) -> Settings {
        let tab_width = parse_bounded(FieldId::TabWidth, &self.fields.tab_width.text, 1);
        let split_column =
            parse_clamp(&self.fields.split_column.text, DEFAULT_SPLIT_LINES_EDGE_COLUMN).max(1);
        let datetime_custom_format = if self.fields.datetime_format.text.trim().is_empty() {
            DEFAULT_DATETIME_CUSTOM_FORMAT.to_string()
        } else {
            self.fields.datetime_format.text.clone()
        };
        // NPP caps Recent Files History entries at 30; 0 disables the
        // list. Parse permissively then clamp to the valid range.
        let recent_files_max = parse_clamp(
            &self.fields.recent_files_max.text,
            DEFAULT_RECENT_FILES_MAX as usize,
        )
        .min(RECENT_FILES_MAX_HARD_CAP as usize) as u32;
        let caret_blink_rate_ms = parse_bounded_u32(
            FieldId::CaretBlinkRateMs,
            &self.fields.caret_blink_rate.text,
            DEFAULT_CARET_BLINK_RATE_MS,
        );
        let default_directory_custom = self.fields.default_directory_custom.text.clone();
        let backup_periodic_interval_ms = parse_bounded_u32(
            FieldId::BackupPeriodicIntervalMs,
            &self.fields.backup_periodic_interval_ms.text,
            DEFAULT_BACKUP_PERIODIC_INTERVAL_MS,
        );
        let backup_custom_dir = self.fields.backup_custom_dir.text.clone();
        let large_file_threshold_bytes = {
            let mb = parse_clamp(
                &self.fields.large_file_threshold_mb.text,
                (DEFAULT_LARGE_FILE_THRESHOLD_BYTES / (1024 * 1024)) as usize,
            ) as u64;
            mb.clamp(performance::THRESHOLD_MIN_MB, performance::THRESHOLD_MAX_MB) * 1024 * 1024
        };
        let prior = settings::snapshot();
        Settings {
            tab_width,
            split_lines_edge_column: split_column,
            datetime_custom_format,
            preferences_sidebar_width: f32::from(self.sidebar_width)
                .max(SIDEBAR_MIN_WIDTH_PX),
            preferences_active_panel: self.active_category.id().to_string(),
            status_bar_hidden: prior.status_bar_hidden,
            recent_files_max,
            title_bar_filename_only: prior.title_bar_filename_only,
            tab_bar_hidden: prior.tab_bar_hidden,
            toolbar_hidden: prior.toolbar_hidden,
            tab_close_button_visible: prior.tab_close_button_visible,
            tab_double_click_to_close: prior.tab_double_click_to_close,
            tab_reduce: prior.tab_reduce,
            tab_active_bar: prior.tab_active_bar,
            tab_darken_inactive: prior.tab_darken_inactive,
            tab_exit_on_last_close: prior.tab_exit_on_last_close,
            tab_lock_drag: prior.tab_lock_drag,
            tab_multi_line: prior.tab_multi_line,
            tab_vertical: prior.tab_vertical,
            caret_blink_rate_ms,
            theme_mode: prior.theme_mode,
            tone_palette: prior.tone_palette,
            customized_slots: prior.customized_slots.clone(),
            show_bookmark_glyph: prior.show_bookmark_glyph,
            show_line_numbers: prior.show_line_numbers,
            line_number_width_dynamic: prior.line_number_width_dynamic,
            indent_uses_tabs: prior.indent_uses_tabs,
            save_all_confirm: prior.save_all_confirm,
            replace_dont_move_to_next: prior.replace_dont_move_to_next,
            confirm_replace_all_in_all_docs: prior.confirm_replace_all_in_all_docs,
            default_line_ending: prior.default_line_ending,
            default_encoding: prior.default_encoding,
            default_language: prior.default_language,
            startup_open_blank: prior.startup_open_blank,
            untitled_uses_first_line: prior.untitled_uses_first_line,
            default_directory_mode: prior.default_directory_mode,
            default_directory_custom,
            last_used_directory: prior.last_used_directory,
            multi_instance_mode: prior.multi_instance_mode,
            scroll_beyond_last_line: prior.scroll_beyond_last_line,
            copy_cut_line_without_selection: prior.copy_cut_line_without_selection,
            highlight_current_line: prior.highlight_current_line,
            document_switcher_enabled: prior.document_switcher_enabled,
            document_switcher_mru: prior.document_switcher_mru,
            backup_periodic_enabled: prior.backup_periodic_enabled,
            backup_periodic_interval_ms,
            backup_on_save_mode: prior.backup_on_save_mode,
            backup_use_custom_dir: prior.backup_use_custom_dir,
            backup_custom_dir,
            search_engine: prior.search_engine,
            search_engine_custom_url: self.fields.search_engine_custom_url.text.clone(),
            delimiter_left: prior.delimiter_left.clone(),
            delimiter_right: prior.delimiter_right.clone(),
            delimiter_match_entire_document: prior.delimiter_match_entire_document,
            large_file_restriction_enabled: prior.large_file_restriction_enabled,
            large_file_threshold_bytes,
            large_file_disable_syntax: prior.large_file_disable_syntax,
            excluded_languages: prior.excluded_languages.clone(),
            recent_files_check_at_launch: prior.recent_files_check_at_launch,
            recent_files_show_full_path: prior.recent_files_show_full_path,
            recent_files_in_submenu: prior.recent_files_in_submenu,
            clickable_urls_enabled: prior.clickable_urls_enabled,
            clickable_urls_no_underline: prior.clickable_urls_no_underline,
            multi_editing_enabled: prior.multi_editing_enabled,
            whitespace_visible: prior.whitespace_visible,
            eol_visible: prior.eol_visible,
            eol_display_mode: prior.eol_display_mode,
            npc_visible: prior.npc_visible,
            npc_display_mode: prior.npc_display_mode,
            npc_include_cc_uni_eol: prior.npc_include_cc_uni_eol,
            prevent_c0_input: prior.prevent_c0_input,
            smart_highlight_enabled: prior.smart_highlight_enabled,
            smart_highlight_case_sensitive: prior.smart_highlight_case_sensitive,
            smart_highlight_whole_word: prior.smart_highlight_whole_word,
            smart_highlight_use_find_settings: prior.smart_highlight_use_find_settings,
            mark_all_case_sensitive: prior.mark_all_case_sensitive,
            mark_all_whole_word: prior.mark_all_whole_word,
            match_tags_enabled: prior.match_tags_enabled,
            match_tag_attrs_enabled: prior.match_tag_attrs_enabled,
            match_tags_in_comment_zone: prior.match_tags_in_comment_zone,
            auto_indent_mode: prior.auto_indent_mode,
            language_indent_overrides: prior.language_indent_overrides.clone(),
            print_line_numbers: prior.print_line_numbers,
            print_color_mode: prior.print_color_mode,
            print_margin_top_mm: parse_clamp(
                &self.fields.print_margin_top_mm.text,
                prior.print_margin_top_mm as usize,
            ) as u32,
            print_margin_left_mm: parse_clamp(
                &self.fields.print_margin_left_mm.text,
                prior.print_margin_left_mm as usize,
            ) as u32,
            print_margin_right_mm: parse_clamp(
                &self.fields.print_margin_right_mm.text,
                prior.print_margin_right_mm as usize,
            ) as u32,
            print_margin_bottom_mm: parse_clamp(
                &self.fields.print_margin_bottom_mm.text,
                prior.print_margin_bottom_mm as usize,
            ) as u32,
            print_header_left: self.fields.print_header_left.text.clone(),
            print_header_middle: self.fields.print_header_middle.text.clone(),
            print_header_right: self.fields.print_header_right.text.clone(),
            print_header_font_name: self.fields.print_header_font_name.text.clone(),
            print_header_font_size: parse_clamp(
                &self.fields.print_header_font_size.text,
                prior.print_header_font_size as usize,
            ) as u32,
            print_header_font_bold: prior.print_header_font_bold,
            print_header_font_italic: prior.print_header_font_italic,
            print_footer_left: self.fields.print_footer_left.text.clone(),
            print_footer_middle: self.fields.print_footer_middle.text.clone(),
            print_footer_right: self.fields.print_footer_right.text.clone(),
            print_footer_font_name: self.fields.print_footer_font_name.text.clone(),
            print_footer_font_size: parse_clamp(
                &self.fields.print_footer_font_size.text,
                prior.print_footer_font_size as usize,
            ) as u32,
            print_footer_font_bold: prior.print_footer_font_bold,
            print_footer_font_italic: prior.print_footer_font_italic,
            proofread_spelling: prior.proofread_spelling,
            proofread_grammar: prior.proofread_grammar,
            autocompletion_mode: prior.autocompletion_mode,
            autocompletion_from_chars: parse_bounded_u32(
                FieldId::AutocompletionFromChars,
                &self.fields.autocompletion_from_chars.text,
                DEFAULT_AUTOCOMPLETION_FROM_CHARS,
            ),
            autocompletion_brief: prior.autocompletion_brief,
            autocompletion_func_params_hint: prior.autocompletion_func_params_hint,
            autocompletion_ignore_numbers: prior.autocompletion_ignore_numbers,
            autocompletion_insert_with_tab: prior.autocompletion_insert_with_tab,
            autocompletion_insert_with_enter: prior.autocompletion_insert_with_enter,
            auto_insert_parens: prior.auto_insert_parens,
            auto_insert_brackets: prior.auto_insert_brackets,
            auto_insert_braces: prior.auto_insert_braces,
            auto_insert_quotes: prior.auto_insert_quotes,
            auto_insert_double_quotes: prior.auto_insert_double_quotes,
            auto_insert_html_xml_tag: prior.auto_insert_html_xml_tag,
            auto_insert_user_pairs: prior.auto_insert_user_pairs.clone(),
            folder_panel_visible: prior.folder_panel_visible,
            folder_panel_width: prior.folder_panel_width,
            folder_panel_roots: prior.folder_panel_roots.clone(),
            folder_panel_expanded: prior.folder_panel_expanded.clone(),
            folder_panel_show_hidden: prior.folder_panel_show_hidden,
            theme_path: prior.theme_path.clone(),
            global_override: prior.global_override,
            find_history: prior.find_history,
            disable_selected_text_drag_drop: prior.disable_selected_text_drag_drop,
        }
    }

    /// Persist the current draft + flash a status message. Called on every
    /// edit (live) so changes survive a window close even without an
    /// explicit Apply button.
    pub(super) fn save_now(&mut self, cx: &mut Context<Self>) {
        let next = self.commit_all();
        settings::replace_and_save(next);
        self.last_action = Some("Saved".to_string());
        cx.notify();
    }
}

fn parse_clamp(s: &str, fallback: usize) -> usize {
    s.trim().parse::<usize>().unwrap_or(fallback)
}

/// Parse a numeric field and clamp it to the range its spinner allows
/// (`FieldId::numeric_bounds`). Empty or non-numeric text falls back to
/// `fallback`, clamped the same way.
fn parse_bounded(id: FieldId, text: &str, fallback: usize) -> usize {
    let value = parse_clamp(text, fallback);
    let Some((min, max, _)) = id.numeric_bounds() else {
        return value;
    };
    let min = usize::try_from(min).unwrap_or(0);
    let max = usize::try_from(max).unwrap_or(usize::MAX).max(min);
    value.clamp(min, max)
}

/// `parse_bounded` for a field stored as `u32`. Clamps before narrowing,
/// so a value past `u32::MAX` lands on the maximum instead of wrapping.
fn parse_bounded_u32(id: FieldId, text: &str, fallback: u32) -> u32 {
    let value = parse_bounded(id, text, fallback as usize);
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The value one spinner click (`delta` = +1 or -1) moves `text` to,
/// clamped to `(min, max, step)`. Non-numeric text starts from `min`.
fn stepped_value(text: &str, delta: i64, (min, max, step): (i64, i64, i64)) -> i64 {
    let current = text.trim().parse::<i64>().unwrap_or(min);
    current
        .saturating_add(delta.saturating_mul(step))
        .clamp(min, max.max(min))
}

/// Find any Workspace window and run `f` against its entity + window.
/// Used by the dialog's zoom action handlers so a Cmd+/-/0 chord pressed
/// inside Preferences zooms the main window the same way the chord on
/// the editor would; the dialog's observer then picks up the new zoom
/// and applies it to its own window. Silently no-ops when no Workspace
/// exists (rare — Preferences is launched from the menu which lives on
/// the main window).
///
/// `active_window()` here returns the dialog's own handle while the
/// dialog has focus, so we always fall through to the windows-list scan.
fn delegate_zoom_to_workspace<F>(cx: &mut App, f: F)
where
    F: FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
{
    let handle = cx
        .windows()
        .into_iter()
        .find_map(|h| h.downcast::<Workspace>());
    let Some(handle) = handle else { return };
    handle.update(cx, |ws, window, cx| f(ws, window, cx)).ok();
}

impl Focusable for PreferencesDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl crate::ui::style_configurator::hsv_surface::HsvHost for PreferencesDialog {
    fn picker_hue(&self) -> f32 { self.picker_hue }
    fn picker_sat(&self) -> f32 { self.picker_sat }
    fn picker_val(&self) -> f32 { self.picker_val }
    fn picker_hsv_drag(&self) -> crate::ui::style_configurator::hsv_surface::HsvDrag {
        self.picker_hsv_drag
    }
    fn set_picker_hue(&mut self, h: f32) { self.picker_hue = h; }
    fn set_picker_sat(&mut self, s: f32) { self.picker_sat = s; }
    fn set_picker_val(&mut self, v: f32) { self.picker_val = v; }
    fn set_picker_hsv_drag(
        &mut self,
        drag: crate::ui::style_configurator::hsv_surface::HsvDrag,
    ) {
        self.picker_hsv_drag = drag;
    }
    fn picker_hex_text(&self) -> &str { &self.picker_hex.text }
    fn set_picker_hex_text(&mut self, text: String) {
        self.picker_hex.cursor = text.len();
        self.picker_hex.anchor = text.len();
        self.picker_hex.text = text;
    }
    fn on_hsv_picker_buffer_changed(&mut self, cx: &mut Context<Self>) {
        // Swatch popover has no live editor-preview surface — just
        // request a repaint so the hex input + HSV thumbs reflect the
        // new buffer. The user commits via the Apply button (or by
        // clicking a swatch); HSV drag does not auto-close the picker.
        cx.notify();
    }
}

// --- IME / OS keyboard input --------------------------------------------

impl EntityInputHandler for PreferencesDialog {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        if self.picker_hex_is_focused(window) {
            return Some(slice_str_chars_utf16(
                &self.picker_hex.text,
                range_utf16,
                actual_range,
            ));
        }
        if let Some((idx, half)) = self.focused_user_pair(window) {
            let s = &self.user_pairs[idx].buffer(half).text;
            return Some(slice_str_chars_utf16(s, range_utf16, actual_range));
        }
        let id = self.focused_field(window)?;
        let s = &self.fields.get(id).text;
        Some(slice_str_chars_utf16(s, range_utf16, actual_range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.picker_hex_is_focused(window) {
            return Some(input_selection_range_utf16(
                &self.picker_hex.text,
                self.picker_hex.cursor,
                self.picker_hex.anchor,
            ));
        }
        if let Some((idx, half)) = self.focused_user_pair(window) {
            let f = self.user_pairs[idx].buffer(half);
            return Some(input_selection_range_utf16(&f.text, f.cursor, f.anchor));
        }
        let id = self.focused_field(window)?;
        let f = self.fields.get(id);
        Some(input_selection_range_utf16(&f.text, f.cursor, f.anchor))
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.picker_hex_is_focused(window) {
            // Filter to hex digits only and uppercase — pasted `#FF00aa`
            // arrives as `FF00AA`, `not-hex` is dropped entirely.
            let filtered: String = new_text
                .chars()
                .filter(|c| c.is_ascii_hexdigit())
                .map(|c| c.to_ascii_uppercase())
                .collect();
            if filtered.is_empty() && !new_text.is_empty() {
                return;
            }
            let f = &mut self.picker_hex;
            let cursor = f.cursor;
            let anchor = f.anchor;
            replace_input_utf16(
                &mut f.text,
                &mut f.cursor,
                &mut f.anchor,
                cursor,
                anchor,
                range_utf16,
                &filtered,
            );
            // Hard cap at 6 chars (RRGGBB). Truncate from the end, clamp caret.
            if self.picker_hex.text.len() > 6 {
                self.picker_hex.text.truncate(6);
                self.picker_hex.cursor = self.picker_hex.cursor.min(6);
                self.picker_hex.anchor = self.picker_hex.anchor.min(6);
            }
            // Keep HSV thumbs in step with typed hex. No-op when the
            // text isn't 6 valid hex chars; preserves hue on grey via
            // the s > 0 guard inside sync_from_buffer.
            crate::ui::style_configurator::hsv_surface::sync_from_buffer(self);
            cx.notify();
            return;
        }
        if let Some((idx, half)) = self.focused_user_pair(window) {
            let f = self.user_pairs[idx].buffer_mut(half);
            let cursor = f.cursor;
            let anchor = f.anchor;
            replace_input_utf16(
                &mut f.text,
                &mut f.cursor,
                &mut f.anchor,
                cursor,
                anchor,
                range_utf16,
                new_text,
            );
            self.sync_user_pairs_to_settings(cx);
            return;
        }
        let Some(id) = self.focused_field(window) else {
            return;
        };
        // Numeric filter — drop any non-digit codepoints from the incoming
        // chunk so the user can't paste alpha into Tab Size.
        let filtered: String = match id.kind() {
            FieldKind::Numeric => new_text.chars().filter(|c| c.is_ascii_digit()).collect(),
            FieldKind::Text => new_text.to_string(),
        };
        if filtered.is_empty() && !new_text.is_empty() {
            return;
        }
        let f = self.fields.get_mut(id);
        let cursor = f.cursor;
        let anchor = f.anchor;
        replace_input_utf16(
            &mut f.text,
            &mut f.cursor,
            &mut f.anchor,
            cursor,
            anchor,
            range_utf16,
            &filtered,
        );
        self.save_now(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

// --- key handling (navigation / select / clipboard) ---------------------

impl PreferencesDialog {
    fn handle_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker_hex_is_focused(window) {
            self.handle_picker_hex_key(ev, window, cx);
            return;
        }
        if let Some((idx, half)) = self.focused_user_pair(window) {
            self.handle_user_pair_key(idx, half, ev, window, cx);
            return;
        }
        let Some(id) = self.focused_field(window) else {
            if ev.keystroke.key == "escape" {
                close_current_window(window);
            }
            return;
        };

        let mods = &ev.keystroke.modifiers;
        let key = ev.keystroke.key.as_str();
        let cmd_or_ctrl = mods.platform || mods.control;
        let shift = mods.shift;

        if key == "escape" && !cmd_or_ctrl && !shift {
            close_current_window(window);
            return;
        }

        if key == "enter" && !cmd_or_ctrl && !shift {
            self.save_now(cx);
            close_current_window(window);
            return;
        }

        if key == "tab" && !cmd_or_ctrl {
            self.cycle_focus(id, !shift, window, cx);
            return;
        }

        let f = self.fields.get_mut(id);

        match key {
            "left" => move_caret_or_select(f, |t, c| prev_grapheme(t, c), shift),
            "right" => move_caret_or_select(f, |t, c| next_grapheme(t, c), shift),
            "home" => move_caret_or_select(f, |_, _| 0, shift),
            "end" => move_caret_or_select(f, |t, _| t.len(), shift),
            "backspace" => {
                if f.ordered().is_empty() {
                    let cur = f.cursor;
                    let prev = prev_grapheme(&f.text, cur);
                    if prev < cur {
                        f.text.replace_range(prev..cur, "");
                        f.cursor = prev;
                        f.anchor = prev;
                    }
                } else {
                    let r = f.ordered();
                    f.text.replace_range(r.clone(), "");
                    f.cursor = r.start;
                    f.anchor = r.start;
                }
                self.save_now(cx);
                return;
            }
            "delete" => {
                if f.ordered().is_empty() {
                    let cur = f.cursor;
                    let nxt = next_grapheme(&f.text, cur);
                    if nxt > cur {
                        f.text.replace_range(cur..nxt, "");
                    }
                } else {
                    let r = f.ordered();
                    f.text.replace_range(r.clone(), "");
                    f.cursor = r.start;
                    f.anchor = r.start;
                }
                self.save_now(cx);
                return;
            }
            "a" if cmd_or_ctrl => {
                f.select_all();
            }
            "c" if cmd_or_ctrl => {
                let sel = f.text[f.ordered()].to_string();
                if !sel.is_empty() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                }
                return;
            }
            "x" if cmd_or_ctrl => {
                let r = f.ordered();
                if !r.is_empty() {
                    let sel = f.text[r.clone()].to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                    f.text.replace_range(r.clone(), "");
                    f.cursor = r.start;
                    f.anchor = r.start;
                    self.save_now(cx);
                }
                return;
            }
            _ => return,
        }

        cx.notify();
    }

    /// Keyboard handler dedicated to the user-defined pair inputs. Mirrors
    /// `handle_key_down` for plain FieldId-bound inputs but reads/writes
    /// `self.user_pairs[idx]` and persists via `sync_user_pairs_to_settings`
    /// instead of `save_now`.
    fn handle_user_pair_key(
        &mut self,
        idx: usize,
        half: user_pair_input::PairHalf,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mods = &ev.keystroke.modifiers;
        let key = ev.keystroke.key.as_str();
        let cmd_or_ctrl = mods.platform || mods.control;
        let shift = mods.shift;

        if key == "escape" && !cmd_or_ctrl && !shift {
            close_current_window(window);
            return;
        }
        if key == "enter" && !cmd_or_ctrl && !shift {
            self.save_now(cx);
            close_current_window(window);
            return;
        }
        if key == "tab" && !cmd_or_ctrl {
            self.cycle_user_pair_focus(idx, half, !shift, window, cx);
            return;
        }

        let Some(buf) = self.user_pairs.get_mut(idx) else {
            return;
        };
        let f = buf.buffer_mut(half);
        let mut text_changed = false;

        match key {
            "left" => move_caret_or_select(f, |t, c| prev_grapheme(t, c), shift),
            "right" => move_caret_or_select(f, |t, c| next_grapheme(t, c), shift),
            "home" => move_caret_or_select(f, |_, _| 0, shift),
            "end" => move_caret_or_select(f, |t, _| t.len(), shift),
            "backspace" => {
                if f.ordered().is_empty() {
                    let cur = f.cursor;
                    let prev = prev_grapheme(&f.text, cur);
                    if prev < cur {
                        f.text.replace_range(prev..cur, "");
                        f.cursor = prev;
                        f.anchor = prev;
                        text_changed = true;
                    }
                } else {
                    let r = f.ordered();
                    f.text.replace_range(r.clone(), "");
                    f.cursor = r.start;
                    f.anchor = r.start;
                    text_changed = true;
                }
            }
            "delete" => {
                if f.ordered().is_empty() {
                    let cur = f.cursor;
                    let nxt = next_grapheme(&f.text, cur);
                    if nxt > cur {
                        f.text.replace_range(cur..nxt, "");
                        text_changed = true;
                    }
                } else {
                    let r = f.ordered();
                    f.text.replace_range(r.clone(), "");
                    f.cursor = r.start;
                    f.anchor = r.start;
                    text_changed = true;
                }
            }
            "a" if cmd_or_ctrl => {
                f.select_all();
            }
            "c" if cmd_or_ctrl => {
                let sel = f.text[f.ordered()].to_string();
                if !sel.is_empty() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                }
                return;
            }
            "x" if cmd_or_ctrl => {
                let r = f.ordered();
                if !r.is_empty() {
                    let sel = f.text[r.clone()].to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                    f.text.replace_range(r.clone(), "");
                    f.cursor = r.start;
                    f.anchor = r.start;
                    text_changed = true;
                }
            }
            _ => return,
        }

        if text_changed {
            self.sync_user_pairs_to_settings(cx);
        } else {
            cx.notify();
        }
    }

    /// Keyboard handler for the picker hex input. Enter commits the parsed
    /// color via `commit_picker_hex` (which closes the picker on success);
    /// Escape closes the picker without committing. Navigation / Backspace /
    /// Clipboard mirror the regular field handler but write to
    /// `self.picker_hex` instead of `self.fields`.
    fn handle_picker_hex_key(
        &mut self,
        ev: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mods = &ev.keystroke.modifiers;
        let key = ev.keystroke.key.as_str();
        let cmd_or_ctrl = mods.platform || mods.control;
        let shift = mods.shift;

        if key == "escape" && !cmd_or_ctrl && !shift {
            self.close_color_picker(cx);
            return;
        }
        if key == "enter" && !cmd_or_ctrl && !shift {
            self.commit_picker_hex(cx);
            return;
        }

        let f = &mut self.picker_hex;

        match key {
            "left" => move_caret_or_select(f, |t, c| prev_grapheme(t, c), shift),
            "right" => move_caret_or_select(f, |t, c| next_grapheme(t, c), shift),
            "home" => move_caret_or_select(f, |_, _| 0, shift),
            "end" => move_caret_or_select(f, |t, _| t.len(), shift),
            "backspace" => {
                if f.ordered().is_empty() {
                    let cur = f.cursor;
                    let prev = prev_grapheme(&f.text, cur);
                    if prev < cur {
                        f.text.replace_range(prev..cur, "");
                        f.cursor = prev;
                        f.anchor = prev;
                    }
                } else {
                    let r = f.ordered();
                    f.text.replace_range(r.clone(), "");
                    f.cursor = r.start;
                    f.anchor = r.start;
                }
            }
            "delete" => {
                if f.ordered().is_empty() {
                    let cur = f.cursor;
                    let nxt = next_grapheme(&f.text, cur);
                    if nxt > cur {
                        f.text.replace_range(cur..nxt, "");
                    }
                } else {
                    let r = f.ordered();
                    f.text.replace_range(r.clone(), "");
                    f.cursor = r.start;
                    f.anchor = r.start;
                }
            }
            "a" if cmd_or_ctrl => {
                f.select_all();
            }
            "c" if cmd_or_ctrl => {
                let sel = f.text[f.ordered()].to_string();
                if !sel.is_empty() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                }
                return;
            }
            "x" if cmd_or_ctrl => {
                let r = f.ordered();
                if !r.is_empty() {
                    let sel = f.text[r.clone()].to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                    f.text.replace_range(r.clone(), "");
                    f.cursor = r.start;
                    f.anchor = r.start;
                }
            }
            _ => return,
        }

        cx.notify();
    }

    /// Tab order across user pairs: each row contributes open then close.
    /// Wraps around at the ends. Cycling beyond the pair list (e.g. into
    /// `AutocompletionFromChars`) is out of scope — clicking the field
    /// remains the way to move there.
    fn cycle_user_pair_focus(
        &mut self,
        from_idx: usize,
        from_half: user_pair_input::PairHalf,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.user_pairs.is_empty() {
            return;
        }
        let total = self.user_pairs.len() * 2;
        let current = from_idx * 2
            + match from_half {
                user_pair_input::PairHalf::Open => 0,
                user_pair_input::PairHalf::Close => 1,
            };
        let next = if forward {
            (current + 1) % total
        } else {
            (current + total - 1) % total
        };
        let next_idx = next / 2;
        let next_half = if next % 2 == 0 {
            user_pair_input::PairHalf::Open
        } else {
            user_pair_input::PairHalf::Close
        };
        let buf = &mut self.user_pairs[next_idx];
        let f = buf.buffer_mut(next_half);
        f.select_all();
        let handle = buf.focus(next_half);
        window.focus(&handle, cx);
        cx.notify();
    }

    fn cycle_focus(
        &mut self,
        from: FieldId,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let order: &[FieldId] = match self.active_category {
            Category::Indentation => &[FieldId::TabWidth],
            Category::MarginsBorderEdge => &[FieldId::SplitColumn],
            Category::MultiInstanceDate => &[FieldId::DateTimeFormat],
            Category::RecentFilesHistory => &[FieldId::RecentFilesMax],
            Category::Performance => &[FieldId::LargeFileThresholdMb],
            Category::AutoCompletion => &[FieldId::AutocompletionFromChars],
            Category::Backup => &[
                FieldId::BackupPeriodicIntervalMs,
                FieldId::BackupCustomDir,
            ],
            Category::Print => &[
                FieldId::PrintMarginTopMm,
                FieldId::PrintMarginLeftMm,
                FieldId::PrintMarginRightMm,
                FieldId::PrintMarginBottomMm,
                FieldId::PrintHeaderLeft,
                FieldId::PrintHeaderMiddle,
                FieldId::PrintHeaderRight,
                FieldId::PrintHeaderFontName,
                FieldId::PrintHeaderFontSize,
                FieldId::PrintFooterLeft,
                FieldId::PrintFooterMiddle,
                FieldId::PrintFooterRight,
                FieldId::PrintFooterFontName,
                FieldId::PrintFooterFontSize,
            ],
            _ => &[],
        };
        if order.len() < 2 {
            return;
        }
        let idx = order.iter().position(|f| *f == from).unwrap_or(0);
        let next = if forward {
            (idx + 1) % order.len()
        } else {
            (idx + order.len() - 1) % order.len()
        };
        let target = order[next];
        let handle = self.focus_for(target);
        let f = self.fields.get_mut(target);
        f.select_all();
        window.focus(&handle, cx);
        cx.notify();
    }

    fn switch_category(&mut self, cat: Category, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_category == cat {
            return;
        }
        self.active_category = cat;
        // Close any panel-local popovers that don't belong to the new
        // category — otherwise the New Document picker's anchor div would
        // unmount under the dropdown and the panel would paint orphaned
        // over a different category's body.
        self.default_lang_dropdown_open = false;
        if let Some(first) = cat.first_field() {
            let handle = self.focus_for(first);
            window.focus(&handle, cx);
        }
        self.save_now(cx);
        cx.notify();
    }

    /// Step a numeric field by `delta * step` (where `step` comes from
    /// `FieldId::numeric_bounds`). No-op for non-numeric fields. Clamps to
    /// the field's `(min, max)` and persists via `save_now`.
    pub(super) fn step_field(&mut self, id: FieldId, delta: i64, cx: &mut Context<Self>) {
        let Some(bounds) = id.numeric_bounds() else {
            return;
        };
        let f = self.fields.get_mut(id);
        let current = f.text.trim().parse::<i64>().unwrap_or(bounds.0);
        let next = stepped_value(&f.text, delta, bounds);
        if next == current {
            return;
        }
        f.text = next.to_string();
        f.cursor = f.text.len();
        f.anchor = f.cursor;
        self.save_now(cx);
    }

    pub(super) fn reset_field(&mut self, id: FieldId, cx: &mut Context<Self>) {
        let next = match id {
            FieldId::TabWidth => crate::consts::TAB_SPACES.to_string(),
            FieldId::SplitColumn => DEFAULT_SPLIT_LINES_EDGE_COLUMN.to_string(),
            FieldId::DateTimeFormat => DEFAULT_DATETIME_CUSTOM_FORMAT.to_string(),
            FieldId::RecentFilesMax => DEFAULT_RECENT_FILES_MAX.to_string(),
            FieldId::CaretBlinkRateMs => DEFAULT_CARET_BLINK_RATE_MS.to_string(),
            FieldId::DefaultDirectoryCustom => String::new(),
            FieldId::LargeFileThresholdMb => {
                (DEFAULT_LARGE_FILE_THRESHOLD_BYTES / (1024 * 1024)).to_string()
            }
            FieldId::SearchEngineCustomUrl => String::new(),
            FieldId::BackupPeriodicIntervalMs => DEFAULT_BACKUP_PERIODIC_INTERVAL_MS.to_string(),
            FieldId::BackupCustomDir => String::new(),
            // NPP `Print ▸ Margin (mm)` defaults to 0 on every side
            // (`Parameters.h:519`). Reset matches.
            FieldId::PrintMarginTopMm
            | FieldId::PrintMarginLeftMm
            | FieldId::PrintMarginRightMm
            | FieldId::PrintMarginBottomMm => "0".to_string(),
            // Header/footer template strings + font name default to empty
            // (NPP populates them only on user input).
            FieldId::PrintHeaderLeft
            | FieldId::PrintHeaderMiddle
            | FieldId::PrintHeaderRight
            | FieldId::PrintHeaderFontName
            | FieldId::PrintFooterLeft
            | FieldId::PrintFooterMiddle
            | FieldId::PrintFooterRight
            | FieldId::PrintFooterFontName => String::new(),
            // NPP `Parameters.h:520` — header/footer font sizes default to 0.
            FieldId::PrintHeaderFontSize | FieldId::PrintFooterFontSize => "0".to_string(),
            FieldId::AutocompletionFromChars => DEFAULT_AUTOCOMPLETION_FROM_CHARS.to_string(),
        };
        let f = self.fields.get_mut(id);
        f.text = next;
        f.cursor = f.text.len();
        f.anchor = f.cursor;
        self.save_now(cx);
    }
}

fn close_current_window(window: &mut Window) {
    OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
    window.remove_window();
}

// --- public entry point --------------------------------------------------

pub(crate) fn open_or_focus(cx: &mut App) {
    let cached = OPEN_WINDOW.with(|cell| cell.borrow().clone());
    if let Some(handle) = cached {
        let result = handle.update(cx, |_, window, cx| {
            window.activate_window();
            cx.notify();
        });
        if result.is_ok() {
            return;
        }
        OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
    }

    let theme_mode = current_theme_mode(cx);
    // Inherit the active main-window's zoom factor. Reads via `handle.read`
    // (Result-returning, safe while the source window is on the update
    // stack); calling `handle.entity` here would panic with "attempted to
    // read a window that is already on the stack" since this runs inside
    // the menu-item dispatch on the Workspace window.
    let initial_zoom = active_workspace_zoom(cx).unwrap_or(ZOOM_DEFAULT);
    // NPP Preferences resource is 580x235 dialog units (~870x350 px) but
    // its listbox fits ~22 categories in that height because Win32 listbox
    // rows are tighter than gpui's flex rows. The Dark Mode panel's Tones
    // group is the widest content: sidebar (~200) + dialog padding (~40) +
    // Tones group inner padding (~28) + tone radio column (~120) + inner
    // gap (~20) + 4-column slot grid with "Edge highlight" and "Disabled
    // text" labels (~560) + scrollbar gutter (~12) ≈ 980 px. Bumped to
    // 1100 px default / 1000 px min so every panel renders without
    // horizontal clipping.
    let bounds = crate::app::window::modal_bounds_centered_on_workspace(
        cx,
        size(px(1100.), px(620.)),
    );
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
            kind: WindowKind::Floating,
            is_resizable: true,
            // Min width fits the Dark Mode Tones group; height stays modest
            // — the sidebar list scrolls when 22 categories overflow.
            window_min_size: Some(size(px(1000.), px(420.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Preferences".into()),
                appears_transparent: false,
                traffic_light_position: Some(point(px(10.), px(10.))),
            }),
            ..Default::default()
        },
        |window, cx| {
            window.set_rem_size(px(ZOOM_REM_BASE * initial_zoom));
            cx.new(|cx| PreferencesDialog::new(theme_mode, None, cx))
        },
    );

    let handle = match result {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to open Preferences: {err}");
            return;
        }
    };

    OPEN_WINDOW.with(|cell| *cell.borrow_mut() = Some(handle));
    // Defer the workspace-entity lookup + zoom observer install until after
    // the current menu dispatch unwinds — `handle.entity(cx)` on the
    // workspace window panics while that window is still on the update
    // stack. `cx.defer` runs after the outer update completes.
    cx.defer(move |cx| {
        let Some(workspace_entity) = active_workspace_entity(cx) else {
            return;
        };
        // observe_release for OPEN_WINDOW cleanup is also deferred so the
        // entity read is safe.
        if let Ok(dialog_entity) = handle.entity(cx) {
            cx.observe_release(&dialog_entity, move |_, _| {
                OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
            })
            .detach();
        }
        let _ = handle.update(cx, |dialog, window, cx| {
            dialog.install_zoom_observer(workspace_entity, cx);
            dialog.install_appearance_observer(window, cx);
        });
    });
    let _ = handle.update(cx, |this, window, cx| {
        if let Some(field) = this.active_category.first_field() {
            let handle = this.focus_for(field);
            window.focus(&handle, cx);
            this.fields.get_mut(field).select_all();
        } else {
            let root = this.focus.clone();
            window.focus(&root, cx);
        }
        cx.notify();
    });
}

fn current_theme_mode(cx: &mut App) -> ThemeMode {
    let handle = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| cx.windows().into_iter().find_map(|h| h.downcast::<Workspace>()));
    let Some(handle) = handle else {
        return ThemeMode::Dark;
    };
    handle
        .read(cx)
        .map(|w| w.theme_mode)
        .unwrap_or(ThemeMode::Dark)
}

/// Active (or any) Workspace entity. Returned `Entity<Workspace>` is held
/// by the dialog so it can `cx.observe` on it for live zoom mirroring.
/// Must be called outside the workspace window's update stack — see the
/// deferred call in `open_or_focus`. Otherwise `handle.entity` panics with
/// "attempted to read a window that is already on the stack".
fn active_workspace_entity(cx: &mut App) -> Option<Entity<Workspace>> {
    let handle = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| cx.windows().into_iter().find_map(|h| h.downcast::<Workspace>()))?;
    handle.entity(cx).ok()
}

/// Read the active Workspace's `zoom_level` without holding the entity.
/// Uses `WindowHandle::read` which returns `Result` and is safe to call
/// while the source window is on the update stack — unlike
/// `WindowHandle::entity`, which panics in that case.
fn active_workspace_zoom(cx: &mut App) -> Option<f32> {
    let handle = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| cx.windows().into_iter().find_map(|h| h.downcast::<Workspace>()))?;
    handle.read(cx).map(|w| w.zoom_level).ok()
}

// --- render --------------------------------------------------------------

impl Render for PreferencesDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let entity = cx.entity().clone();
        let weak = entity.downgrade();

        // Push the latest observed Workspace zoom into this dialog's
        // window. Cheap when nothing changed (epsilon-compare); the
        // observer + the open-time seed are what actually drive value
        // updates here.
        if (self.zoom_target - self.zoom_applied).abs() > f32::EPSILON {
            window.set_rem_size(px(ZOOM_REM_BASE * self.zoom_target));
            self.zoom_applied = self.zoom_target;
        }

        let root = div()
            .key_context("PreferencesDialog")
            .track_focus(&self.focus)
            .cursor(CursorStyle::Arrow)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.text))
            .font_family(crate::fonts::mono())
            .text_size(rems(13. / 16.))
            .line_height(rems(20. / 16.))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_key_down(ev, window, cx);
            }));

        let body = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.))
            .child(self.render_category_list(&theme, &entity))
            .child(self.render_content_panel(&theme, &weak, cx));

        let footer = self.render_footer(&theme, &entity);

        let picker_overlay = self
            .color_picker
            .map(|state| color_picker::render(&theme, &weak, state));
        let reset_overlay = self
            .reset_dropdown
            .map(|state| reset_dropdown::render(&theme, &weak, state));

        root.relative()
            .child(body)
            .child(footer)
            .children(picker_overlay)
            .children(reset_overlay)
    }
}

impl PreferencesDialog {
    fn render_category_list(
        &self,
        theme: &Theme,
        entity: &Entity<Self>,
    ) -> impl IntoElement {
        let active = self.active_category;
        // `flex_shrink_0` is the bug fix — without it, gpui's flex row
        // shrank the sidebar whenever the right pane's content (e.g. the
        // 220-rem-wide DateTime input on the Insert tab) couldn't fit at
        // the row's width. Pinning `flex-shrink: 0` keeps the sidebar at
        // exactly `self.sidebar_width` regardless of right-pane content.
        let mut list = div()
            .id("prefs-sidebar")
            .flex_shrink_0()
            .w(self.sidebar_width)
            .h_full()
            .relative()
            .border_r_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.chrome))
            .py(rems(8. / 16.))
            .flex()
            .flex_col()
            // 22 NPP categories overflow the dialog at small window sizes;
            // fall back to vertical scroll when the row stack is taller
            // than the dialog.
            .overflow_y_scroll();

        for cat in Category::all() {
            let is_active = *cat == active;
            let entity_for_click = entity.clone();
            let cat_for_click = *cat;
            // `.id()` is mandatory for `.hover()` to take effect in GPUI —
            // without a stable element id the row stays a static div and
            // hover styling never applies.
            let row = div()
                .id(SharedString::from(format!("prefs-cat-{}", cat.id())))
                .px(rems(12. / 16.))
                .py(rems(6. / 16.))
                .text_color(rgb(theme.text))
                .when(is_active, |s| {
                    s.bg(rgb(theme.bg))
                        .border_l_2()
                        .border_color(rgb(theme.toggle_active))
                })
                .when(!is_active, |s| {
                    s.cursor(CursorStyle::PointingHand)
                        .hover(|hs| hs.bg(rgb(theme.button_hover_bg)))
                })
                .child(SharedString::from(cat.label()))
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    entity_for_click.update(cx, |this, cx| {
                        this.switch_category(cat_for_click, window, cx);
                    });
                });
            list = list.child(row);
        }

        // Resize handle — 4 px wide hit area centered on the right border.
        list.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(-px(2.))
                .w(px(4.))
                .cursor(CursorStyle::ResizeLeftRight)
                .child(SidebarResizeHandle {
                    dialog: entity.downgrade(),
                }),
        )
    }

    fn render_content_panel(
        &self,
        theme: &Theme,
        weak: &WeakEntity<Self>,
        _cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let body = div()
            .flex_1()
            .h_full()
            .px(rems(20. / 16.))
            .py(rems(18. / 16.))
            .flex()
            .flex_col()
            .gap(rems(14. / 16.));

        match self.active_category {
            Category::General => general::render(theme, weak, body),
            Category::Editing1 => editing1::render(theme, weak, body),
            Category::Editing2 => editing2::render(theme, weak, body),
            Category::DarkMode => dark_mode::render(theme, weak, body),
            Category::MarginsBorderEdge => margins::render(theme, weak, body),
            Category::NewDocument => new_document::render(
                theme,
                weak,
                body,
                self.default_lang_dropdown_open,
            ),
            Category::DefaultDirectory => default_directory::render(theme, weak, body),
            Category::RecentFilesHistory => recent_files::render(theme, weak, body),
            Category::FileAssociation => file_association::render(theme, body),
            Category::Language => language::render(theme, weak, body),
            Category::Indentation => {
                indentation::render(theme, weak, body, self.selected_indent_lang_idx)
            }
            Category::Highlighting => highlighting::render(theme, weak, body),
            Category::Print => print::render(theme, weak, body),
            Category::Searching => searching::render(theme, weak, body),
            Category::Backup => backup::render(theme, weak, body),
            Category::AutoCompletion => {
                auto_completion::render(theme, weak, body, self.user_pairs.len())
            }
            Category::MultiInstanceDate => multi_instance::render(theme, weak, body),
            Category::Delimiter => delimiter::render(theme, weak, body),
            Category::Performance => performance::render(theme, weak, body),
            Category::CloudLink => cloud_link::render(theme, weak, body),
            Category::SearchEngine => search_engine::render(theme, weak, body),
            Category::Misc => misc::render(theme, weak, body),
        }
    }

    fn render_footer(
        &self,
        theme: &Theme,
        _entity: &Entity<Self>,
    ) -> impl IntoElement {
        let status = self
            .last_action
            .clone()
            .unwrap_or_else(|| "Changes save automatically".to_string());

        let close = div()
            .id("prefs-close-btn")
            .px(rems(20. / 16.))
            .py(rems(4. / 16.))
            .bg(rgb(theme.button_bg))
            .border_1()
            .border_color(rgb(theme.border))
            .text_color(rgb(theme.text))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme.button_hover_bg)))
            .active(|s| s.bg(rgb(theme.button_pressed_bg)))
            .child("Close")
            .on_mouse_down(MouseButton::Left, move |_e, window, _cx| {
                close_current_window(window);
            });

        div()
            .h(rems(40. / 16.))
            .border_t_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.chrome))
            .flex()
            .flex_row()
            .items_center()
            .px(rems(1.))
            .child(
                div()
                    .flex_1()
                    .text_size(rems(11. / 16.))
                    .text_color(rgb(theme.chrome_text))
                    .child(SharedString::from(status)),
            )
            .child(close)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The editor indents with `" ".repeat(tab_width)`, so an unbounded
    /// width typed here aborted the next Tab key with a capacity overflow.
    #[test]
    fn tab_width_is_capped_to_the_spinner_range() {
        let parse = |text: &str| parse_bounded(FieldId::TabWidth, text, 1);
        assert_eq!(parse("18446744073709551615"), 16);
        assert_eq!(parse("99999999999"), 16);
        assert_eq!(parse("17"), 16);
        assert_eq!(parse(" 8 "), 8);
        assert_eq!(parse("0"), 1);
    }

    #[test]
    fn unparsable_tab_width_falls_back_to_one() {
        // Empty, negative, overflowing usize and non-ASCII digits.
        for text in ["", "-1", "184467440737095516150", "٣", "８"] {
            assert_eq!(parse_bounded(FieldId::TabWidth, text, 1), 1, "{text:?}");
        }
    }

    /// Past `u32::MAX` the value used to wrap before clamping, so 2^32 + 1
    /// landed on the minimum instead of the maximum.
    #[test]
    fn u32_fields_clamp_before_narrowing() {
        let huge = "4294967297";
        assert_eq!(
            parse_bounded_u32(FieldId::CaretBlinkRateMs, huge, DEFAULT_CARET_BLINK_RATE_MS),
            CARET_BLINK_RATE_MAX_MS
        );
        assert_eq!(
            parse_bounded_u32(
                FieldId::BackupPeriodicIntervalMs,
                huge,
                DEFAULT_BACKUP_PERIODIC_INTERVAL_MS
            ),
            BACKUP_PERIODIC_INTERVAL_MAX_MS
        );
        assert_eq!(
            parse_bounded_u32(
                FieldId::AutocompletionFromChars,
                huge,
                DEFAULT_AUTOCOMPLETION_FROM_CHARS
            ),
            AUTOCOMPLETION_FROM_CHARS_MAX
        );
        assert_eq!(
            parse_bounded_u32(FieldId::CaretBlinkRateMs, "", DEFAULT_CARET_BLINK_RATE_MS),
            DEFAULT_CARET_BLINK_RATE_MS
        );
    }

    /// Stepping from a value at the edge of `i64` overflowed the addition:
    /// a panic in debug builds, a wrap to the wrong end of the range in
    /// release.
    #[test]
    fn stepping_from_an_extreme_value_saturates() {
        let bounds = (1, 16, 1);
        assert_eq!(stepped_value("9223372036854775807", 1, bounds), 16);
        assert_eq!(stepped_value("-9223372036854775808", -1, bounds), 1);
        assert_eq!(stepped_value("4", 1, bounds), 5);
        assert_eq!(stepped_value("", 1, bounds), 2);
        assert_eq!(stepped_value("abc", -1, bounds), 1);
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod layout_tests;
