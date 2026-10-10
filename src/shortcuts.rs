// User-customisable shortcut catalog + override persistence.
//
// Notepad++ exposes its keymap through Settings ▸ Shortcut Mapper. Notepad Studio
// mirrors that surface: every keymap action is enumerated here as a static
// `ActionEntry`, the `Shortcut Mapper` UI displays the entries, and any user
// overrides are written to `<config>/Notepad Studio/shortcuts.toml`.
//
// Schema (simple, future-proof):
//
//   [[binding]]
//   action = "EditDuplicateLine"
//   keys = ["cmd-d"]
//
// `keys` is a gpui keystroke string (the same syntax accepted by
// `KeyBinding::load` — `cmd-shift-f`, `f3`, `escape`, etc.). An entry with
// `keys = []` means the user explicitly unbound the action.
//
// Defaults are per platform. Windows and Linux get Notepad++'s default chord
// (PowerEditor/src/Parameters.cpp, `winKeyDefs[]` for menu commands and
// `scintKeyDefs[]` for editor keys); macOS gets the `cmd-` mirror, or the Mac
// convention where Notepad++'s chord collides with one. `platform_default_keys`
// is the single place that picks between the two lists.
//
// `register_catalog` consumes the override map and rebinds every catalog
// entry. It is the single source of truth for keymap registration —
// `app::keymap::bind_app_keys` delegates to it on startup, and the Shortcut
// Mapper window calls `reload_keymap` after the user saves so the new chords
// take effect immediately without a relaunch.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, KeyBinding};
use serde::{Deserialize, Serialize};

use crate::actions::*;
use crate::consts::APP_NAME;

#[cfg(test)]
mod npp_defaults;

/// One row in the Shortcut Mapper. Pairs an action's stable string id (its
/// gpui Action type name) with a human-readable label, the per-platform
/// default chord sets, and a constructor for the concrete `Action`.
pub(crate) struct ActionEntry {
    /// Stable identifier — the gpui Action type name (e.g. `"EditDuplicateLine"`).
    /// Persisted in `shortcuts.toml`; never localized, never renamed.
    pub(crate) id: &'static str,
    /// User-facing label, shown in the Action column of the Shortcut Mapper.
    pub(crate) label: &'static str,
    /// The Notepad++ command this row mirrors (`IDM_*`, `IDC_*` or `SCI_*`
    /// from `menuCmdID.h` / Scintilla), or `None` for a Notepad Studio
    /// extra. Rows with a command must default to exactly Notepad++'s
    /// chord on Windows and Linux; extras must stay clear of every
    /// Notepad++ default chord. The unit tests enforce both.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) npp: Option<&'static str>,
    /// Windows and Linux defaults.
    pub(crate) keys: &'static [&'static str],
    /// macOS defaults.
    pub(crate) mac_keys: &'static [&'static str],
    /// Key context the binding lives in. `None` = global. Catalog entries are
    /// scoped to the editor window's `Workspace` context — dialog-internal
    /// chords (the FindDialog mirrors) are NOT user-editable; they're
    /// registered separately in `app::keymap::register_dialog_only`.
    pub(crate) context: Option<&'static str>,
    /// Builds a fresh boxed instance of the row's action.
    pub(crate) action: fn() -> Box<dyn gpui::Action>,
}

fn act<A: gpui::Action + Default>() -> Box<dyn gpui::Action> {
    Box::new(A::default())
}

/// The defaults that apply on the platform this binary was built for.
/// Overrides replace this list wholesale when present; resetting a row
/// drops the override entry so these re-apply on the next bind.
pub(crate) fn platform_default_keys(entry: &ActionEntry) -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        entry.mac_keys
    } else {
        entry.keys
    }
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------
//
// Order here = display order in the Shortcut Mapper. Group by menu so the
// list reads like a flattened menubar (File, Edit, Search, View, Tabs,
// Bookmarks, Editor — same shape as the menubar minus dynamic submenus).
//
// `row!(Type, label, npp, context, [windows/linux keys], [mac keys])`. The
// id is `stringify!(Type)`, so it always matches the action type name that
// `shortcuts.toml` is keyed by.

pub(crate) fn catalog() -> &'static [ActionEntry] {
    CATALOG
}

const W: Option<&'static str> = Some("Workspace");
const GLOBAL: Option<&'static str> = None;
const EXTRA: Option<&'static str> = None;

macro_rules! row {
    ($ty:ident, $label:literal, $npp:expr, $ctx:expr, [$($k:literal),* $(,)?], [$($m:literal),* $(,)?] $(,)?) => {
        ActionEntry {
            id: stringify!($ty),
            label: $label,
            npp: $npp,
            keys: &[$($k),*],
            mac_keys: &[$($m),*],
            context: $ctx,
            action: act::<$ty>,
        }
    };
}

#[rustfmt::skip]
static CATALOG: &[ActionEntry] = &[
    // --- File ---
    row!(Quit,                 "Quit",                      Some("IDM_FILE_EXIT"),                  GLOBAL, ["alt-f4"],       ["cmd-q"]),
    row!(FileNewWindow,        "New Window",                EXTRA,                                  GLOBAL, ["ctrl-shift-n"], ["cmd-shift-n"]),
    row!(FileNew,              "New",                       Some("IDM_FILE_NEW"),                   W,      ["ctrl-n"],       ["cmd-n"]),
    row!(FileOpen,             "Open",                      Some("IDM_FILE_OPEN"),                  W,      ["ctrl-o"],       ["cmd-o"]),
    row!(FileSave,             "Save",                      Some("IDM_FILE_SAVE"),                  W,      ["ctrl-s"],       ["cmd-s"]),
    row!(FileSaveAs,           "Save As",                   Some("IDM_FILE_SAVEAS"),                W,      ["ctrl-alt-s"],   ["cmd-shift-s"]),
    row!(FileSaveAll,          "Save All",                  Some("IDM_FILE_SAVEALL"),               W,      ["ctrl-shift-s"], ["cmd-alt-s"]),
    row!(FileCloseTab,         "Close Tab",                 Some("IDM_FILE_CLOSE"),                 W,      ["ctrl-w"],       ["cmd-w"]),
    row!(FileCloseAllTabs,     "Close All Tabs",            Some("IDM_FILE_CLOSEALL"),              W,      ["ctrl-shift-w"], ["cmd-shift-w"]),
    row!(FileRestoreClosedTab, "Restore Recent Closed Tab", Some("IDM_FILE_RESTORELASTCLOSEDFILE"), W,      ["ctrl-shift-t"], ["cmd-shift-t"]),

    // --- Edit ---
    row!(EditUndo,          "Undo",               Some("SCI_UNDO"),               W, ["ctrl-z", "alt-backspace"],         ["cmd-z"]),
    row!(EditRedo,          "Redo",               Some("SCI_REDO"),               W, ["ctrl-y", "ctrl-shift-z"],          ["cmd-shift-z"]),
    row!(EditCut,           "Cut",                Some("IDM_EDIT_CUT"),           W, ["ctrl-x", "shift-delete"],          ["cmd-x"]),
    row!(EditCopy,          "Copy",               Some("IDM_EDIT_COPY"),          W, ["ctrl-c", "ctrl-insert"],           ["cmd-c"]),
    row!(EditPaste,         "Paste",              Some("IDM_EDIT_PASTE"),         W, ["ctrl-v", "shift-insert"],          ["cmd-v"]),
    row!(EditSelectAll,     "Select All",         Some("SCI_SELECTALL"),          W, ["ctrl-a"],                          ["cmd-a"]),
    row!(EditBackspace,     "Backspace",          Some("SCI_DELETEBACK"),         W, ["backspace", "shift-backspace"],    ["backspace", "shift-backspace"]),
    row!(EditDelete,        "Delete",             Some("SCI_CLEAR"),              W, ["delete"],                          ["delete"]),
    // Scintilla editor keys (Parameters.cpp scintKeyDefs[]). macOS takes the
    // Cocoa text conventions: Option+Delete for words, Cmd+Delete for the
    // line. Ctrl+Up/Down are Mission Control there and Mac keyboards have no
    // Insert key, so those three have no Mac default.
    row!(EditDeleteWordLeft,  "Delete Word Left",     Some("SCI_DELWORDLEFT"),        W, ["ctrl-backspace"],                  ["alt-backspace"]),
    row!(EditDeleteWordRight, "Delete Word Right",    Some("SCI_DELWORDRIGHT"),       W, ["ctrl-delete"],                     ["alt-delete"]),
    row!(EditDeleteLineLeft,  "Delete to Line Start", Some("SCI_DELLINELEFT"),        W, ["ctrl-shift-backspace"],            ["cmd-backspace"]),
    row!(EditDeleteLineRight, "Delete to Line End",   Some("SCI_DELLINERIGHT"),       W, ["ctrl-shift-delete"],               ["cmd-delete"]),
    row!(EditScrollLineUp,    "Scroll Line Up",       Some("SCI_LINESCROLLUP"),       W, ["ctrl-up"],                         []),
    row!(EditScrollLineDown,  "Scroll Line Down",     Some("SCI_LINESCROLLDOWN"),     W, ["ctrl-down"],                       []),
    row!(EditToggleOvertype,  "Toggle Overtype",      Some("SCI_EDITTOGGLEOVERTYPE"), W, ["insert"],                          []),
    // Shift+Enter is a plain newline in the editor (SCI_NEWLINE); in the
    // find bar and the CSV grid `insert_newline` routes it backwards.
    row!(EditInsertNewline, "Insert Newline",     Some("SCI_NEWLINE"),            W, ["enter", "shift-enter"],            ["enter", "shift-enter"]),
    row!(EditInsertTab,     "Insert Tab",         Some("SCI_TAB"),                W, ["tab"],                             ["tab"]),
    row!(EditOutdent,       "Decrease Line Indent", Some("SCI_BACKTAB"),          W, ["shift-tab"],                       ["shift-tab"]),
    row!(EditDuplicateLine, "Duplicate Line",     Some("SCI_SELECTIONDUPLICATE"), W, ["ctrl-d"],                          ["cmd-d"]),
    // CSV extras. Ctrl+R is Notepad++'s Reload, so Fill Right has no
    // Windows/Linux default.
    row!(EditFillRight,       "CSV: Fill Right",          EXTRA, W, [],               ["cmd-r"]),
    row!(EditInsertTodayDate, "CSV: Insert Today's Date", EXTRA, W, ["ctrl-;"],       ["cmd-;"]),
    row!(EditInsertNowTime,   "CSV: Insert Now Time",     EXTRA, W, ["ctrl-shift-;"], ["cmd-shift-;"]),
    // macOS keeps the Mac convention (Option+Shift+Arrow moves lines); on
    // Windows/Linux Alt+Shift+Arrow is Notepad++'s column select.
    row!(EditMoveLineUp,      "Move Line Up",             Some("IDM_EDIT_LINE_UP"),            W, ["ctrl-shift-up"],        ["alt-shift-up"]),
    row!(EditMoveLineDown,    "Move Line Down",           Some("IDM_EDIT_LINE_DOWN"),          W, ["ctrl-shift-down"],      ["alt-shift-down"]),
    row!(EditDeleteLine,      "Delete Line",              Some("SCI_LINEDELETE"),              W, ["ctrl-shift-l"],         ["cmd-shift-l"]),
    row!(EditUppercase,       "UPPERCASE",                Some("IDM_EDIT_UPPERCASE"),          W, ["ctrl-shift-u"],         ["cmd-shift-u"]),
    row!(EditLowercase,       "lowercase",                Some("IDM_EDIT_LOWERCASE"),          W, ["ctrl-u"],               ["cmd-u"]),
    // Option+letter types accented characters on macOS, so the two
    // Alt-only case chords have no Mac default.
    row!(EditProperCaseForce,   "Proper Case",            Some("IDM_EDIT_PROPERCASE_FORCE"),   W, ["alt-u"],                []),
    row!(EditProperCaseBlend,   "Proper Case (blend)",    Some("IDM_EDIT_PROPERCASE_BLEND"),   W, ["alt-shift-u"],          []),
    row!(EditSentenceCaseForce, "Sentence case",          Some("IDM_EDIT_SENTENCECASE_FORCE"), W, ["ctrl-alt-u"],           ["cmd-alt-u"]),
    row!(EditSentenceCaseBlend, "Sentence case (blend)",  Some("IDM_EDIT_SENTENCECASE_BLEND"), W, ["ctrl-alt-shift-u"],     ["cmd-alt-shift-u"]),
    // Cmd+I / Cmd+K belong to the Markdown extras on macOS, so these two
    // keep Notepad++'s Control chord there.
    row!(EditSplitLines,      "Split Lines",              Some("IDM_EDIT_SPLIT_LINES"),        W, ["ctrl-i"],               ["ctrl-i"]),
    row!(EditJoinLines,       "Join Lines",               Some("IDM_EDIT_JOIN_LINES"),         W, ["ctrl-j"],               ["cmd-j"]),
    row!(EditBlockComment,    "Toggle Block Comment",     Some("IDM_EDIT_BLOCK_COMMENT"),      W, ["ctrl-q"],               ["cmd-/"]),
    row!(EditBlockCommentSet, "Block Comment",            Some("IDM_EDIT_BLOCK_COMMENT_SET"),  W, ["ctrl-k"],               ["ctrl-k"]),
    row!(EditBlockUncomment,  "Block Uncomment",          Some("IDM_EDIT_BLOCK_UNCOMMENT"),    W, ["ctrl-shift-k"],         ["cmd-shift-k"]),
    row!(EditStreamComment,   "Toggle Stream Comment",    Some("IDM_EDIT_STREAM_COMMENT"),     W, ["ctrl-shift-q"],         ["cmd-shift-/"]),
    row!(EditInsertBlankLineAbove, "Insert Blank Line Above", Some("IDM_EDIT_BLANKLINEABOVECURRENT"), W, ["ctrl-alt-enter"],       ["cmd-alt-enter"]),
    row!(EditInsertBlankLineBelow, "Insert Blank Line Below", Some("IDM_EDIT_BLANKLINEBELOWCURRENT"), W, ["ctrl-alt-shift-enter"], ["cmd-shift-enter"]),
    row!(EditBeginEndSelect,           "Begin/End Select",                Some("IDM_EDIT_BEGINENDSELECT"),            W, ["ctrl-shift-b"], ["cmd-shift-b"]),
    row!(EditBeginEndSelectColumnMode, "Begin/End Select in Column Mode", Some("IDM_EDIT_BEGINENDSELECT_COLUMNMODE"), W, ["alt-shift-b"],  []),

    // --- Auto-Completion ---
    // Ctrl-family on every platform, as in Notepad++. Ctrl+Alt+Space is
    // Notepad++'s path completion, so the combined Function and Word
    // completion (a Notepad Studio extra) only keeps it on macOS.
    row!(EditAutocompleteFunc,        "Function Completion",          Some("IDM_EDIT_AUTOCOMPLETE"),             W, ["ctrl-space"],       ["ctrl-space"]),
    row!(EditAutocompleteWord,        "Word Completion",              Some("IDM_EDIT_AUTOCOMPLETE_CURRENTFILE"), W, ["ctrl-enter"],       ["ctrl-enter"]),
    row!(EditAutocompleteFuncAndWord, "Function and Word Completion", EXTRA,                                     W, [],                   ["ctrl-alt-space"]),
    row!(EditAutocompleteFuncParams,  "Function Parameters Hint",     Some("IDM_EDIT_FUNCCALLTIP"),              W, ["ctrl-shift-space"], ["ctrl-shift-space"]),

    // --- Caret movement ---
    row!(MoveLeft,        "Move Left",                Some("SCI_CHARLEFT"),            W, ["left"],             ["left"]),
    row!(MoveRight,       "Move Right",               Some("SCI_CHARRIGHT"),           W, ["right"],            ["right"]),
    row!(MoveUp,          "Move Up",                  Some("SCI_LINEUP"),              W, ["up"],               ["up"]),
    row!(MoveDown,        "Move Down",                Some("SCI_LINEDOWN"),            W, ["down"],             ["down"]),
    row!(SelectLeft,      "Select Left",              Some("SCI_CHARLEFTEXTEND"),      W, ["shift-left"],       ["shift-left"]),
    row!(SelectRight,     "Select Right",             Some("SCI_CHARRIGHTEXTEND"),     W, ["shift-right"],      ["shift-right"]),
    row!(SelectUp,        "Select Up",                Some("SCI_LINEUPEXTEND"),        W, ["shift-up"],         ["shift-up"]),
    row!(SelectDown,      "Select Down",              Some("SCI_LINEDOWNEXTEND"),      W, ["shift-down"],       ["shift-down"]),
    row!(EditPageUp,      "Page Up",                  Some("SCI_PAGEUP"),              W, ["pageup"],           ["pageup"]),
    row!(EditPageDown,    "Page Down",                Some("SCI_PAGEDOWN"),            W, ["pagedown"],         ["pagedown"]),
    row!(SelectPageUp,    "Select Page Up",           Some("SCI_PAGEUPEXTEND"),        W, ["shift-pageup"],     ["shift-pageup"]),
    row!(SelectPageDown,  "Select Page Down",         Some("SCI_PAGEDOWNEXTEND"),      W, ["shift-pagedown"],   ["shift-pagedown"]),
    row!(MoveLineStart,   "Move to Line Start",       Some("SCI_VCHOMEWRAP"),          W, ["home"],             ["home", "cmd-left"]),
    row!(MoveLineEnd,     "Move to Line End",         Some("SCI_LINEENDWRAP"),         W, ["end"],              ["end", "cmd-right"]),
    row!(SelectLineStart, "Select to Line Start",     Some("SCI_VCHOMEWRAPEXTEND"),    W, ["shift-home"],       ["shift-home", "cmd-shift-left"]),
    row!(SelectLineEnd,   "Select to Line End",       Some("SCI_LINEENDWRAPEXTEND"),   W, ["shift-end"],        ["shift-end", "cmd-shift-right"]),
    row!(MoveWordLeft,    "Move Word Left",           Some("SCI_WORDLEFT"),            W, ["ctrl-left"],        ["alt-left"]),
    row!(MoveWordRight,   "Move Word Right",          Some("SCI_WORDRIGHT"),           W, ["ctrl-right"],       ["alt-right"]),
    row!(SelectWordLeft,  "Select Word Left",         Some("SCI_WORDLEFTEXTEND"),      W, ["ctrl-shift-left"],  ["alt-shift-left"]),
    row!(SelectWordRight, "Select Word Right",        Some("SCI_WORDRIGHTENDEXTEND"),  W, ["ctrl-shift-right"], ["alt-shift-right"]),
    row!(MoveDocStart,    "Move to Document Start",   Some("SCI_DOCUMENTSTART"),       W, ["ctrl-home"],        ["cmd-up", "cmd-home"]),
    row!(MoveDocEnd,      "Move to Document End",     Some("SCI_DOCUMENTEND"),         W, ["ctrl-end"],         ["cmd-down", "cmd-end"]),
    row!(SelectDocStart,  "Select to Document Start", Some("SCI_DOCUMENTSTARTEXTEND"), W, ["ctrl-shift-home"],  ["cmd-shift-up", "cmd-shift-home"]),
    row!(SelectDocEnd,    "Select to Document End",   Some("SCI_DOCUMENTENDEXTEND"),   W, ["ctrl-shift-end"],   ["cmd-shift-down", "cmd-shift-end"]),

    // --- Column-mode (Notepad++ block selection) keyboard extension. Mouse
    //     alt-drag is the other entry point. Notepad++ uses Alt+Shift+Arrow;
    //     on macOS Option+Shift+Arrow is word select / move line, so the Mac
    //     chord adds Cmd.
    row!(BlockExtendUp,    "Extend Block Up",    Some("SCI_LINEUPRECTEXTEND"),    W, ["alt-shift-up"],    ["cmd-alt-shift-up"]),
    row!(BlockExtendDown,  "Extend Block Down",  Some("SCI_LINEDOWNRECTEXTEND"),  W, ["alt-shift-down"],  ["cmd-alt-shift-down"]),
    row!(BlockExtendLeft,  "Extend Block Left",  Some("SCI_CHARLEFTRECTEXTEND"),  W, ["alt-shift-left"],  ["cmd-alt-shift-left"]),
    row!(BlockExtendRight, "Extend Block Right", Some("SCI_CHARRIGHTRECTEXTEND"), W, ["alt-shift-right"], ["cmd-alt-shift-right"]),

    // --- Search ---
    // Go to Line is Cmd+L on macOS because Cmd+G is Find Next there.
    row!(EditFind,          "Find",          Some("IDM_SEARCH_FIND"),        W, ["ctrl-f"],       ["cmd-f"]),
    row!(EditFindClose,     "Close Find",    Some("SCI_CANCEL"),             W, ["escape"],       ["escape"]),
    row!(EditFindNext,      "Find Next",     Some("IDM_SEARCH_FINDNEXT"),    W, ["f3"],           ["f3", "cmd-g"]),
    row!(EditFindPrev,      "Find Previous", Some("IDM_SEARCH_FINDPREV"),    W, ["shift-f3"],     ["shift-f3", "cmd-shift-g"]),
    row!(EditReplace,       "Replace",       Some("IDM_SEARCH_REPLACE"),     W, ["ctrl-h"],       ["cmd-alt-f"]),
    row!(EditGotoLine,      "Go to Line",    Some("IDM_SEARCH_GOTOLINE"),    W, ["ctrl-g"],       ["cmd-l"]),
    row!(SearchFindInFiles, "Find in Files", Some("IDM_SEARCH_FINDINFILES"), W, ["ctrl-shift-f"], ["cmd-shift-f"]),
    row!(SearchMark,        "Mark",          Some("IDM_SEARCH_MARK"),        W, ["ctrl-m"],       ["cmd-m"]),

    // --- Tabs ---
    // Notepad++ binds Ctrl+Numpad1..9 to the tab slots. gpui reports a
    // numpad digit as the plain digit, so the slot chord is Ctrl+1..9.
    row!(TabNext,           "Next Tab",              Some("IDC_NEXT_DOC"),      W, ["ctrl-tab"],       ["ctrl-tab"]),
    row!(TabPrev,           "Previous Tab",          Some("IDC_PREV_DOC"),      W, ["ctrl-shift-tab"], ["ctrl-shift-tab"]),
    row!(TabNextSequential, "Next Tab (Visual)",     Some("IDM_VIEW_TAB_NEXT"), W, ["ctrl-pagedown"],  ["ctrl-pagedown"]),
    row!(TabPrevSequential, "Previous Tab (Visual)", Some("IDM_VIEW_TAB_PREV"), W, ["ctrl-pageup"],    ["ctrl-pageup"]),
    row!(TabGoto1,          "Go to Tab 1",           Some("IDM_VIEW_TAB1"),     W, ["ctrl-1"],         ["cmd-1"]),
    row!(TabGoto2,          "Go to Tab 2",           Some("IDM_VIEW_TAB2"),     W, ["ctrl-2"],         ["cmd-2"]),
    row!(TabGoto3,          "Go to Tab 3",           Some("IDM_VIEW_TAB3"),     W, ["ctrl-3"],         ["cmd-3"]),
    row!(TabGoto4,          "Go to Tab 4",           Some("IDM_VIEW_TAB4"),     W, ["ctrl-4"],         ["cmd-4"]),
    row!(TabGoto5,          "Go to Tab 5",           Some("IDM_VIEW_TAB5"),     W, ["ctrl-5"],         ["cmd-5"]),
    row!(TabGoto6,          "Go to Tab 6",           Some("IDM_VIEW_TAB6"),     W, ["ctrl-6"],         ["cmd-6"]),
    row!(TabGoto7,          "Go to Tab 7",           Some("IDM_VIEW_TAB7"),     W, ["ctrl-7"],         ["cmd-7"]),
    row!(TabGoto8,          "Go to Tab 8",           Some("IDM_VIEW_TAB8"),     W, ["ctrl-8"],         ["cmd-8"]),
    row!(TabGoto9,          "Go to Tab 9",           Some("IDM_VIEW_TAB9"),     W, ["ctrl-9"],         ["cmd-9"]),

    // --- View ---
    // Notepad++ zooms with Ctrl+Numpad+/-//. Windows reports those keys as
    // `+`, `-`, `/`; Linux as `add`, `subtract`, `divide`. Both spellings
    // are listed so the same row works on either.
    row!(ViewZoomIn,             "Zoom In",                Some("SCI_ZOOMIN"),                W, ["ctrl-+", "ctrl-add"],      ["cmd-=", "cmd-+"]),
    row!(ViewZoomOut,            "Zoom Out",               Some("SCI_ZOOMOUT"),               W, ["ctrl--", "ctrl-subtract"], ["cmd--"]),
    row!(ViewZoomReset,          "Reset Zoom",             Some("SCI_SETZOOM"),               W, ["ctrl-/", "ctrl-divide"],   ["cmd-0"]),
    row!(ViewToggleFullScreen,   "Toggle Full Screen",     Some("IDM_VIEW_FULLSCREENTOGGLE"), W, ["f11"],                     ["f11"]),
    // Ctrl/Cmd+Shift+L is Delete Line (Notepad++ SCI_LINEDELETE).
    row!(ViewCsvToggleFilterRow, "CSV: Toggle Filter Row", EXTRA,                             W, [],                          []),

    // --- Macros ---
    row!(MacroStartStopRecording, "Start/Stop Recording",          Some("IDC_EDIT_TOGGLEMACRORECORDING"),  W, ["ctrl-shift-r"], ["cmd-shift-r"]),
    row!(MacroPlayback,           "Playback",                      Some("IDM_MACRO_PLAYBACKRECORDEDMACRO"), W, ["ctrl-shift-p"], ["cmd-shift-p"]),
    row!(MacroRunMultipleTimes,   "Run a Macro Multiple Times",    Some("IDM_MACRO_RUNMULTIMACRODLG"),     W, [],               ["cmd-shift-m"]),
    row!(MacroSaveCurrent,        "Save Currently Recorded Macro", Some("IDM_MACRO_SAVECURRENTMACRO"),     W, [],               []),

    // --- Bookmarks ---
    // Notepad++: Ctrl+F2 toggles, F2 / Shift+F2 jump next / previous.
    row!(BookmarkToggle,   "Toggle Bookmark",     Some("IDM_SEARCH_TOGGLE_BOOKMARK"), W, ["ctrl-f2"],  ["cmd-f2"]),
    row!(BookmarkNext,     "Next Bookmark",       Some("IDM_SEARCH_NEXT_BOOKMARK"),   W, ["f2"],       ["f2"]),
    row!(BookmarkPrev,     "Previous Bookmark",   Some("IDM_SEARCH_PREV_BOOKMARK"),   W, ["shift-f2"], ["shift-f2"]),
    row!(BookmarkClearAll, "Clear All Bookmarks", Some("IDM_SEARCH_CLEAR_BOOKMARKS"), W, [],           ["cmd-shift-f2"]),

    // --- Settings ---
    // Global (no Workspace context) so the chord works with no editor
    // focused. Notepad++ ships Preferences and the Style Configurator
    // without a chord; macOS gets the platform's Cmd+, convention.
    row!(OpenPreferences,       "Open Preferences",        Some("IDM_SETTING_PREFERENCE"),   GLOBAL, [], ["cmd-,"]),
    row!(OpenStyleConfigurator, "Open Style Configurator", Some("IDM_LANGSTYLE_CONFIG_DLG"), GLOBAL, [], ["cmd-shift-,"]),

    // --- Help ---
    row!(HelpAbout, "About", Some("IDM_ABOUT"), GLOBAL, ["f1"], []),

    // --- Markdown formatting ---
    // Notepad Studio extras. The handlers are guarded by `md_guard`
    // (extension-based) so a stray dispatch on a non-markdown tab no-ops.
    // On Windows/Linux Ctrl+B / I / K and Ctrl+Shift+X / 1 / 2 / 3 are
    // Notepad++ chords, so those rows have no default there.
    row!(MdBold,           "Markdown: Bold",            EXTRA, W, [],                   ["cmd-b"]),
    row!(MdItalic,         "Markdown: Italic",          EXTRA, W, [],                   ["cmd-i"]),
    row!(MdStrikethrough,  "Markdown: Strikethrough",   EXTRA, W, [],                   ["cmd-shift-x"]),
    row!(MdInlineCode,     "Markdown: Inline Code",     EXTRA, W, ["ctrl-e"],           ["cmd-e"]),
    row!(MdLink,           "Markdown: Link",            EXTRA, W, [],                   ["cmd-k"]),
    // Cmd+Shift+3 is the macOS screenshot chord; Cmd+Option+digit is the
    // Google Docs / Notion heading convention.
    row!(MdHeading1,       "Markdown: Heading 1",       EXTRA, W, [],                   ["cmd-alt-1"]),
    row!(MdHeading2,       "Markdown: Heading 2",       EXTRA, W, [],                   ["cmd-alt-2"]),
    row!(MdHeading3,       "Markdown: Heading 3",       EXTRA, W, [],                   ["cmd-alt-3"]),
    row!(MdBulletList,     "Markdown: Bullet List",     EXTRA, W, ["ctrl-shift-8"],     ["cmd-shift-8"]),
    row!(MdOrderedList,    "Markdown: Ordered List",    EXTRA, W, ["ctrl-shift-7"],     ["cmd-shift-7"]),
    row!(MdTaskList,       "Markdown: Task List",       EXTRA, W, ["ctrl-shift-9"],     ["cmd-shift-9"]),
    row!(MdBlockQuote,     "Markdown: Block Quote",     EXTRA, W, ["ctrl-shift-."],     ["cmd-shift-."]),
    row!(MdCodeBlock,      "Markdown: Code Block",      EXTRA, W, ["ctrl-alt-shift-c"], ["cmd-alt-shift-c"]),
    row!(MdImage,          "Markdown: Image",           EXTRA, W, ["ctrl-shift-i"],     ["cmd-shift-i"]),
    row!(MdHorizontalRule, "Markdown: Horizontal Rule", EXTRA, W, ["ctrl-shift--"],     ["cmd-shift--"]),

    // --- Rich-doc formatting ---
    // Promote / Demote follow the Notion convention on macOS (Cmd+Shift+]
    // / [). Ctrl+Shift+] / [ are Notepad++'s paragraph select, so no
    // Windows/Linux default.
    row!(RichDocPromoteHeading, "Rich-doc: Promote Heading", EXTRA, W, [], ["cmd-shift-]"]),
    row!(RichDocDemoteHeading,  "Rich-doc: Demote Heading",  EXTRA, W, [], ["cmd-shift-["]),
];

// ---------------------------------------------------------------------------
// Chord strings
// ---------------------------------------------------------------------------

/// What Shift does to a US-layout digit or punctuation key. Every gpui
/// platform reports Shift+digit / Shift+punctuation as the shifted
/// character with Shift cleared (Ctrl+Shift+1 arrives as `ctrl-!`), so a
/// binding written as `ctrl-shift-1` has to be folded to match.
fn us_shifted(key: &str) -> Option<&'static str> {
    Some(match key {
        "1" => "!",
        "2" => "@",
        "3" => "#",
        "4" => "$",
        "5" => "%",
        "6" => "^",
        "7" => "&",
        "8" => "*",
        "9" => "(",
        "0" => ")",
        "-" => "_",
        "=" => "+",
        "[" => "{",
        "]" => "}",
        "\\" => "|",
        ";" => ":",
        "'" => "\"",
        "," => "<",
        "." => ">",
        "/" => "?",
        "`" => "~",
        _ => return None,
    })
}

fn canonical_keystroke(source: &str) -> String {
    let Ok(ks) = gpui::Keystroke::parse(source) else {
        return source.to_string();
    };
    let mut m = ks.modifiers;
    let mut key = ks.key;
    if m.shift {
        if let Some(shifted) = us_shifted(&key) {
            key = shifted.to_string();
            m.shift = false;
        }
    }
    let mut out = String::new();
    for (on, name) in [
        (m.function, "fn-"),
        (m.control, "ctrl-"),
        (m.alt, "alt-"),
        (m.shift, "shift-"),
        (m.platform, "cmd-"),
    ] {
        if on {
            out.push_str(name);
        }
    }
    out.push_str(&key);
    out
}

/// The form a chord takes when gpui matches it: modifiers in a fixed order,
/// `cmd` for the platform key, and Shift folded into digits and
/// punctuation the way a US layout types them (`ctrl-shift-1` → `ctrl-!`,
/// `cmd-shift-[` → `cmd-{`). Two chords conflict exactly when their
/// canonical forms are equal. An unparseable chord comes back unchanged.
pub(crate) fn canonical_chord(chord: &str) -> String {
    chord
        .split_whitespace()
        .map(canonical_keystroke)
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when two chord strings name the same keystroke.
pub(crate) fn same_chord(a: &str, b: &str) -> bool {
    canonical_chord(a) == canonical_chord(b)
}

/// The string handed to `KeyBinding::load`. Windows has a layout-aware
/// keyboard mapper that turns `ctrl-shift-7` into whatever Shift+7 types
/// on the active layout, so it gets the chord as written. The macOS and
/// Linux mappers leave chords alone, so they get the US-folded form.
fn binding_source(chord: &str) -> String {
    if cfg!(target_os = "windows") {
        chord.to_string()
    } else {
        canonical_chord(chord)
    }
}

/// Build a `KeyBinding` without panicking. `KeyBinding::new` unwraps both
/// the keystroke and the context parse; chords here can come from a
/// hand-edited `shortcuts.toml`, so errors come back to the caller.
pub(crate) fn load_binding(
    chord: &str,
    action: Box<dyn gpui::Action>,
    context: Option<&str>,
    mapper: &dyn gpui::PlatformKeyboardMapper,
) -> anyhow::Result<KeyBinding> {
    let predicate = match context {
        Some(ctx) => Some(Rc::new(gpui::KeyBindingContextPredicate::parse(ctx)?)),
        None => None,
    };
    Ok(KeyBinding::load(
        &binding_source(chord),
        action,
        predicate,
        false,
        None,
        mapper,
    )?)
}

/// Split a chord into its modifier names and its key. Handles a `-` key
/// (`ctrl--` → `(["ctrl"], "-")`), which a plain `split('-')` gets wrong.
pub(crate) fn split_chord(chord: &str) -> (Vec<&str>, &str) {
    if chord == "-" {
        return (Vec::new(), "-");
    }
    if let Some(prefix) = chord.strip_suffix("--") {
        let mods = if prefix.is_empty() {
            Vec::new()
        } else {
            prefix.split('-').collect()
        };
        return (mods, "-");
    }
    match chord.rsplit_once('-') {
        Some((mods, key)) => (mods.split('-').collect(), key),
        None => (Vec::new(), chord),
    }
}

// ---------------------------------------------------------------------------
// Override file: load / save
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct OverridesFile {
    #[serde(default)]
    binding: Vec<BindingEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct BindingEntry {
    action: String,
    keys: Vec<String>,
}

/// In-memory map of action id → user-supplied keystroke list. An action
/// missing from this map keeps its default chord set; an action present with
/// `[]` is explicitly unbound.
pub(crate) type Overrides = BTreeMap<String, Vec<String>>;

fn shortcuts_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join("shortcuts.toml"))
}

/// Parse the contents of `shortcuts.toml`. A file that fails to parse
/// loads as no overrides, so startup continues on defaults.
fn parse_overrides(raw: &str) -> Overrides {
    let Ok(file) = toml::from_str::<OverridesFile>(raw) else {
        return Overrides::new();
    };
    file.binding
        .into_iter()
        .map(|b| (b.action, b.keys))
        .collect()
}

pub(crate) fn load_overrides() -> Overrides {
    let Some(path) = shortcuts_path() else {
        return Overrides::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Overrides::new();
    };
    parse_overrides(&raw)
}

pub(crate) fn save_overrides(overrides: &Overrides) -> std::io::Result<()> {
    let Some(path) = shortcuts_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config dir",
        ));
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OverridesFile {
        binding: overrides
            .iter()
            .map(|(action, keys)| BindingEntry {
                action: action.clone(),
                keys: keys.clone(),
            })
            .collect(),
    };
    let serialized = toml::to_string_pretty(&file).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
    })?;
    std::fs::write(&path, serialized)
}

/// Resolve the currently-effective key list for a single catalog row. The
/// override (if any) wins outright — no merging — so the user can drop a
/// chord entirely by saving an empty list.
pub(crate) fn effective_keys(entry: &ActionEntry, overrides: &Overrides) -> Vec<String> {
    if let Some(custom) = overrides.get(entry.id) {
        return custom.clone();
    }
    platform_default_keys(entry)
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Apply: rebuild the live keymap from defaults + overrides
// ---------------------------------------------------------------------------

/// Bind every catalog entry, honouring `overrides`. Caller is responsible for
/// clearing the existing keymap first when reloading at runtime; on startup
/// the keymap is empty so a plain bind suffices.
pub(crate) fn register_catalog(cx: &mut App, overrides: &Overrides) {
    let mapper = cx.keyboard_mapper().clone();
    let mut bindings = Vec::new();
    for entry in CATALOG {
        for chord in effective_keys(entry, overrides) {
            // A chord that fails to load came from a corrupt user file. Log
            // it and keep going so one bad row doesn't take out the rest
            // of the keymap.
            match load_binding(&chord, (entry.action)(), entry.context, mapper.as_ref()) {
                Ok(binding) => bindings.push(binding),
                Err(err) => eprintln!(
                    "shortcuts.toml: dropping chord {chord:?} for {}: {err}",
                    entry.id
                ),
            }
        }
    }
    cx.bind_keys(bindings);
}

/// Wipe every existing binding (catalog + dialog mirrors) and rebuild from
/// scratch using `overrides`. Called by the Shortcut Mapper after a save so
/// the new chord set takes effect without restarting.
pub(crate) fn reload_keymap(cx: &mut App, overrides: &Overrides) {
    cx.clear_key_bindings();
    crate::app::keymap::register_menu_bar_keys(cx);
    register_catalog(cx, overrides);
    crate::app::keymap::register_dialog_only(cx);
}

// ---------------------------------------------------------------------------
// Conflict detection
// ---------------------------------------------------------------------------

/// Return the action id that owns `chord` in the proposed override state, or
/// `None` if the chord is free. `skip` is the row currently being edited so
/// the user can re-bind a chord they already had. Chords compare by their
/// canonical form, so `ctrl-shift-8` and `ctrl-*` count as the same key.
pub(crate) fn conflict_for(
    chord: &str,
    proposed: &Overrides,
    skip: &str,
) -> Option<&'static str> {
    let wanted = canonical_chord(chord);
    CATALOG
        .iter()
        .filter(|entry| entry.id != skip)
        .find(|entry| {
            effective_keys(entry, proposed)
                .iter()
                .any(|k| canonical_chord(k) == wanted)
        })
        .map(|entry| entry.id)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::npp_defaults::{NPP_DEFAULTS, NPP_NO_DEFAULT};
    use super::*;

    #[derive(Clone, Copy, Debug)]
    enum Os {
        Windows,
        Linux,
    }

    /// Turn a Notepad++ table chord into what gpui reports on `os`. The
    /// numpad pseudo-keys are the only part that differs between Windows
    /// (`MapVirtualKeyW` gives the character) and Linux (xkb keysym names).
    fn npp_chord_on(chord: &str, os: Os) -> String {
        let (mods, key) = split_chord(chord);
        let key = match (key, os) {
            ("numpadadd", Os::Windows) => "+",
            ("numpadadd", Os::Linux) => "add",
            ("numpadsubtract", Os::Windows) => "-",
            ("numpadsubtract", Os::Linux) => "subtract",
            ("numpaddivide", Os::Windows) => "/",
            ("numpaddivide", Os::Linux) => "divide",
            (k, _) => k.strip_prefix("numpad").unwrap_or(k),
        };
        let mut out = mods.join("-");
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(key);
        canonical_chord(&out)
    }

    /// Command id → canonical chords Notepad++ binds it to, on Windows and
    /// Linux combined.
    fn npp_chords() -> BTreeMap<&'static str, BTreeSet<String>> {
        let mut map: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
        for &(cmd, chord, _line) in NPP_DEFAULTS {
            for os in [Os::Windows, Os::Linux] {
                map.entry(cmd).or_default().insert(npp_chord_on(chord, os));
            }
        }
        map
    }

    fn canonical_set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|k| canonical_chord(k)).collect()
    }

    fn contexts_overlap(a: Option<&str>, b: Option<&str>) -> bool {
        a.is_none() || b.is_none() || a == b
    }

    fn assert_no_duplicates(list: fn(&ActionEntry) -> &'static [&'static str], platform: &str) {
        let mut problems = Vec::new();
        for (i, a) in CATALOG.iter().enumerate() {
            let a_keys: Vec<String> = list(a).iter().map(|k| canonical_chord(k)).collect();
            let unique: BTreeSet<&String> = a_keys.iter().collect();
            if unique.len() != a_keys.len() {
                problems.push(format!("{}: same chord listed twice {:?}", a.id, list(a)));
            }
            for b in &CATALOG[i + 1..] {
                if !contexts_overlap(a.context, b.context) {
                    continue;
                }
                for k in list(b) {
                    let canon = canonical_chord(k);
                    if a_keys.contains(&canon) {
                        problems.push(format!("{canon} bound to both {} and {}", a.id, b.id));
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{platform} chord collisions:\n{}", problems.join("\n"));
    }

    #[test]
    fn no_chord_bound_twice_on_windows_and_linux() {
        assert_no_duplicates(|e| e.keys, "Windows/Linux");
    }

    #[test]
    fn no_chord_bound_twice_on_macos() {
        assert_no_duplicates(|e| e.mac_keys, "macOS");
    }

    #[test]
    fn every_default_chord_parses_and_loads() {
        for entry in CATALOG {
            for chord in entry.keys.iter().chain(entry.mac_keys) {
                assert!(
                    gpui::Keystroke::parse(chord).is_ok(),
                    "{}: {chord:?} does not parse",
                    entry.id
                );
                let canon = canonical_chord(chord);
                assert!(
                    gpui::Keystroke::parse(&canon).is_ok(),
                    "{}: canonical {canon:?} does not parse",
                    entry.id
                );
                assert!(
                    load_binding(chord, (entry.action)(), entry.context, &gpui::DummyKeyboardMapper)
                        .is_ok(),
                    "{}: {chord:?} does not load",
                    entry.id
                );
            }
        }
    }

    #[test]
    fn windows_linux_defaults_match_notepad_plus_plus() {
        let npp = npp_chords();
        let no_default: BTreeSet<&str> = NPP_NO_DEFAULT.iter().map(|(cmd, _)| *cmd).collect();
        let mut problems = Vec::new();
        for entry in CATALOG {
            let Some(cmd) = entry.npp else { continue };
            let expected = npp.get(cmd).cloned().unwrap_or_default();
            if expected.is_empty() && !no_default.contains(cmd) {
                problems.push(format!("{}: unknown Notepad++ command {cmd}", entry.id));
                continue;
            }
            let actual = canonical_set(entry.keys);
            if actual != expected {
                problems.push(format!(
                    "{} ({cmd}): catalog {actual:?}, Notepad++ {expected:?}",
                    entry.id
                ));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn extras_leave_every_notepad_plus_plus_chord_free() {
        let reserved: BTreeSet<String> = npp_chords().into_values().flatten().collect();
        let mut problems = Vec::new();
        for entry in CATALOG.iter().filter(|e| e.npp.is_none()) {
            for chord in entry.keys {
                let canon = canonical_chord(chord);
                if reserved.contains(&canon) {
                    problems.push(format!("{}: {chord} is a Notepad++ default", entry.id));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn ids_are_unique_and_match_action_names() {
        let mut seen = BTreeSet::new();
        for entry in CATALOG {
            assert!(seen.insert(entry.id), "duplicate catalog id {}", entry.id);
            let name = (entry.action)().name();
            assert!(
                name.ends_with(&format!("::{}", entry.id)),
                "{} builds action {name}",
                entry.id
            );
        }
    }

    #[test]
    fn canonical_chord_folds_shift_and_orders_modifiers() {
        assert_eq!(canonical_chord("ctrl-shift-1"), "ctrl-!");
        assert_eq!(canonical_chord("cmd-shift-["), "cmd-{");
        assert_eq!(canonical_chord("ctrl-shift-;"), "ctrl-:");
        assert_eq!(canonical_chord("ctrl--"), "ctrl--");
        assert_eq!(canonical_chord("ctrl-shift--"), "ctrl-_");
        assert_eq!(canonical_chord("ctrl-shift-="), "ctrl-+");
        assert_eq!(canonical_chord("A"), "shift-a");
        assert_eq!(canonical_chord("shift-alt-ctrl-up"), "ctrl-alt-shift-up");
        assert_eq!(canonical_chord("cmd-alt-shift-up"), "alt-shift-cmd-up");
        assert_eq!(canonical_chord("ctrl-shift-u"), "ctrl-shift-u");
        assert_eq!(canonical_chord("super-x"), canonical_chord("cmd-x"));
        // Garbage passes through untouched rather than panicking.
        assert_eq!(canonical_chord("ctrl-a-b"), "ctrl-a-b");
        for entry in CATALOG {
            for chord in entry.keys.iter().chain(entry.mac_keys) {
                let once = canonical_chord(chord);
                assert_eq!(canonical_chord(&once), once, "{chord} is not stable");
            }
        }
    }

    #[test]
    fn split_chord_handles_dash_key() {
        assert_eq!(split_chord("ctrl--"), (vec!["ctrl"], "-"));
        assert_eq!(split_chord("ctrl-shift--"), (vec!["ctrl", "shift"], "-"));
        assert_eq!(split_chord("-"), (vec![], "-"));
        assert_eq!(split_chord("ctrl-shift-f"), (vec!["ctrl", "shift"], "f"));
        assert_eq!(split_chord("f3"), (vec![], "f3"));
        assert_eq!(split_chord(""), (vec![], ""));
    }

    #[test]
    fn overrides_file_from_older_builds_still_loads() {
        let raw = r#"
[[binding]]
action = "EditDuplicateLine"
keys = ["cmd-d"]

[[binding]]
action = "MdBold"
keys = []
"#;
        let parsed = parse_overrides(raw);
        assert_eq!(parsed.get("EditDuplicateLine"), Some(&vec!["cmd-d".to_string()]));
        assert_eq!(parsed.get("MdBold"), Some(&Vec::new()));
        assert!(parse_overrides("not = [valid").is_empty());
        assert!(parse_overrides("").is_empty());
    }

    #[test]
    fn override_replaces_platform_defaults() {
        let Some(entry) = CATALOG.iter().find(|e| e.id == "FileNew") else {
            panic!("FileNew missing from catalog");
        };
        let mut overrides = Overrides::new();
        assert_eq!(
            effective_keys(entry, &overrides),
            platform_default_keys(entry).iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        overrides.insert("FileNew".into(), vec![]);
        assert!(effective_keys(entry, &overrides).is_empty());
        let expected: &[&str] = if cfg!(target_os = "macos") { &["cmd-n"] } else { &["ctrl-n"] };
        assert_eq!(platform_default_keys(entry), expected);
    }

    #[test]
    fn conflict_detection_compares_canonical_forms() {
        let mut overrides = Overrides::new();
        overrides.insert("MdBulletList".into(), vec!["ctrl-shift-8".into()]);
        assert_eq!(conflict_for("ctrl-*", &overrides, "FileNew"), Some("MdBulletList"));
        assert_eq!(conflict_for("ctrl-*", &overrides, "MdBulletList"), None);
        assert!(same_chord("ctrl-shift-8", "ctrl-*"));
        assert!(!same_chord("ctrl-8", "ctrl-*"));
    }
}
