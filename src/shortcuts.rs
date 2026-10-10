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
// `keys` is the canonical gpui keystroke string (the same syntax accepted by
// `KeyBinding::new` — `cmd-shift-f`, `f3`, `escape`, etc.). An entry with
// `keys = []` means the user explicitly unbound the action.
//
// `register_all_with_overrides` consumes the override map and rebinds every
// catalog entry. It is the single source of truth for keymap registration —
// `app::keymap::bind_app_keys` delegates to it on startup, and the Shortcut
// Mapper window calls it again after the user saves so the new chords take
// effect immediately without a relaunch.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gpui::{App, KeyBinding};
use serde::{Deserialize, Serialize};

use crate::actions::*;
use crate::consts::APP_NAME;

/// One row in the Shortcut Mapper. Pairs an action's stable string id (its
/// gpui Action type name) with a human-readable label, the cross-platform
/// default chord set, and a closure that knows how to bind a runtime
/// keystroke string to the right concrete `Action` type.
pub(crate) struct ActionEntry {
    /// Stable identifier — the gpui Action type name (e.g. `"EditDuplicateLine"`).
    /// Persisted in `shortcuts.toml`; never localized.
    pub(crate) id: &'static str,
    /// User-facing label, shown in the Action column of the Shortcut Mapper.
    pub(crate) label: &'static str,
    /// Defaults baked into the binary. Overrides replace this list wholesale
    /// when present; resetting a row drops the override entry so the defaults
    /// re-apply on the next bind.
    pub(crate) default_keys: &'static [&'static str],
    /// Key context the binding lives in. `None` = global. Catalog entries are
    /// scoped to the editor window's `Workspace` context — dialog-internal
    /// chords (the FindDialog mirrors in `keymap/search.rs`) are NOT user-
    /// editable in v1; they're registered separately in `apply_dialog_keymap`.
    pub(crate) context: Option<&'static str>,
    /// Builds the typed `KeyBinding` for a given chord string. Stored as a
    /// closure so the catalog stays a flat data table — no per-action
    /// `match` dispatch lives outside this file.
    pub(crate) bind: fn(keys: &str, ctx: Option<&str>) -> KeyBinding,
}

/// Helper: condense `KeyBinding::new(keys, A, ctx)` into a function pointer
/// so each `ActionEntry` can list it in one place. Generic over the action
/// type — every action type that lives in the catalog has its own monomorph.
fn bind<A: gpui::Action + Clone + Default>(keys: &str, ctx: Option<&str>) -> KeyBinding {
    KeyBinding::new(keys, A::default(), ctx)
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------
//
// Order here = display order in the Shortcut Mapper. Group by menu so the
// list reads like a flattened menubar (File, Edit, Search, View, Tabs,
// Bookmarks, Editor — same shape as the menubar minus dynamic submenus).
//
// `default_keys` lists the user-facing chords ONLY. Cross-platform mirrors
// (cmd-/ on macOS vs ctrl-/ everywhere else) are kept on the same row so the
// Shortcut Mapper shows a single cell per action, just like Notepad++. When
// the user records a new chord, it replaces the entire mirror set — same
// behaviour Notepad++ has on its single-platform binary.

pub(crate) fn catalog() -> &'static [ActionEntry] {
    &CATALOG
}

const W: Option<&'static str> = Some("Workspace");

#[rustfmt::skip]
static CATALOG: &[ActionEntry] = &[
    // --- File ---
    ActionEntry { id: "Quit",                 label: "Quit",                       default_keys: &["cmd-q"],                                  context: None, bind: bind::<Quit> },
    ActionEntry { id: "FileNewWindow",        label: "New Window",                 default_keys: &["cmd-shift-n", "ctrl-shift-n"],            context: None, bind: bind::<FileNewWindow> },
    ActionEntry { id: "FileNew",              label: "New",                        default_keys: &["cmd-n", "ctrl-n"],                        context: W,    bind: bind::<FileNew> },
    ActionEntry { id: "FileOpen",             label: "Open",                       default_keys: &["cmd-o", "ctrl-o"],                        context: W,    bind: bind::<FileOpen> },
    ActionEntry { id: "FileSave",             label: "Save",                       default_keys: &["cmd-s", "ctrl-s"],                        context: W,    bind: bind::<FileSave> },
    ActionEntry { id: "FileSaveAs",           label: "Save As",                    default_keys: &["cmd-shift-s", "ctrl-alt-s"],              context: W,    bind: bind::<FileSaveAs> },
    ActionEntry { id: "FileSaveAll",          label: "Save All",                   default_keys: &["cmd-alt-s", "ctrl-shift-s"],              context: W,    bind: bind::<FileSaveAll> },
    ActionEntry { id: "FileCloseTab",         label: "Close Tab",                  default_keys: &["cmd-w", "ctrl-w"],                        context: W,    bind: bind::<FileCloseTab> },
    ActionEntry { id: "FileCloseAllTabs",     label: "Close All Tabs",             default_keys: &["cmd-shift-w", "ctrl-shift-w"],            context: W,    bind: bind::<FileCloseAllTabs> },
    ActionEntry { id: "FileRestoreClosedTab", label: "Restore Recent Closed Tab",  default_keys: &["cmd-shift-t", "ctrl-shift-t"],            context: W,    bind: bind::<FileRestoreClosedTab> },

    // --- Edit ---
    ActionEntry { id: "EditUndo",                  label: "Undo",                          default_keys: &["cmd-z", "ctrl-z"],                                       context: W, bind: bind::<EditUndo> },
    ActionEntry { id: "EditRedo",                  label: "Redo",                          default_keys: &["cmd-shift-z", "ctrl-shift-z", "ctrl-y"],                 context: W, bind: bind::<EditRedo> },
    ActionEntry { id: "EditCut",                   label: "Cut",                           default_keys: &["cmd-x", "ctrl-x"],                                       context: W, bind: bind::<EditCut> },
    ActionEntry { id: "EditCopy",                  label: "Copy",                          default_keys: &["cmd-c", "ctrl-c"],                                       context: W, bind: bind::<EditCopy> },
    ActionEntry { id: "EditPaste",                 label: "Paste",                         default_keys: &["cmd-v", "ctrl-v"],                                       context: W, bind: bind::<EditPaste> },
    ActionEntry { id: "EditSelectAll",             label: "Select All",                    default_keys: &["cmd-a", "ctrl-a"],                                       context: W, bind: bind::<EditSelectAll> },
    ActionEntry { id: "EditBackspace",             label: "Backspace",                     default_keys: &["backspace"],                                             context: W, bind: bind::<EditBackspace> },
    ActionEntry { id: "EditDelete",                label: "Delete",                        default_keys: &["delete"],                                                context: W, bind: bind::<EditDelete> },
    ActionEntry { id: "EditInsertNewline",         label: "Insert Newline",                default_keys: &["enter"],                                                 context: W, bind: bind::<EditInsertNewline> },
    ActionEntry { id: "EditInsertTab",             label: "Insert Tab",                    default_keys: &["tab"],                                                   context: W, bind: bind::<EditInsertTab> },
    ActionEntry { id: "EditDuplicateLine",         label: "Duplicate Line",                default_keys: &["cmd-d", "ctrl-d"],                                       context: W, bind: bind::<EditDuplicateLine> },
    ActionEntry { id: "EditFillRight",             label: "CSV: Fill Right",               default_keys: &["cmd-r", "ctrl-r"],                                       context: W, bind: bind::<EditFillRight> },
    ActionEntry { id: "EditInsertTodayDate",       label: "CSV: Insert Today's Date",      default_keys: &["cmd-;", "ctrl-;"],                                       context: W, bind: bind::<EditInsertTodayDate> },
    ActionEntry { id: "EditInsertNowTime",         label: "CSV: Insert Now Time",          default_keys: &["cmd-shift-;", "ctrl-shift-;"],                           context: W, bind: bind::<EditInsertNowTime> },
    ActionEntry { id: "EditMoveLineUp",            label: "Move Line Up",                  default_keys: &["alt-shift-up", "ctrl-shift-up"],                         context: W, bind: bind::<EditMoveLineUp> },
    ActionEntry { id: "EditMoveLineDown",          label: "Move Line Down",                default_keys: &["alt-shift-down", "ctrl-shift-down"],                     context: W, bind: bind::<EditMoveLineDown> },
    ActionEntry { id: "EditDeleteLine",            label: "Delete Line",                   default_keys: &["cmd-shift-l", "ctrl-shift-l"],                           context: W, bind: bind::<EditDeleteLine> },
    ActionEntry { id: "EditUppercase",             label: "UPPERCASE",                     default_keys: &["cmd-shift-u", "ctrl-shift-u"],                           context: W, bind: bind::<EditUppercase> },
    ActionEntry { id: "EditLowercase",             label: "lowercase",                     default_keys: &["cmd-u", "ctrl-u"],                                       context: W, bind: bind::<EditLowercase> },
    ActionEntry { id: "EditBlockComment",          label: "Toggle Block Comment",          default_keys: &["cmd-/", "ctrl-q"],                                       context: W, bind: bind::<EditBlockComment> },
    ActionEntry { id: "EditStreamComment",         label: "Toggle Stream Comment",         default_keys: &["cmd-shift-/", "ctrl-shift-q"],                           context: W, bind: bind::<EditStreamComment> },
    ActionEntry { id: "EditInsertBlankLineAbove",  label: "Insert Blank Line Above",       default_keys: &["cmd-alt-enter", "ctrl-alt-enter"],                       context: W, bind: bind::<EditInsertBlankLineAbove> },
    ActionEntry { id: "EditInsertBlankLineBelow",  label: "Insert Blank Line Below",       default_keys: &["cmd-shift-enter", "ctrl-shift-enter"],                   context: W, bind: bind::<EditInsertBlankLineBelow> },

    // --- Auto-Completion ---
    // NPP defaults: Function = Ctrl+Space, Word = Ctrl+Enter, Function and
    // Word = Ctrl+Alt+Space, Function Parameters Hint = Ctrl+Shift+Space.
    // Same chords cross-platform — Ctrl-family is the NPP convention.
    ActionEntry { id: "EditAutocompleteFunc",        label: "Function Completion",        default_keys: &["ctrl-space"],            context: W, bind: bind::<EditAutocompleteFunc> },
    ActionEntry { id: "EditAutocompleteWord",        label: "Word Completion",            default_keys: &["ctrl-enter"],            context: W, bind: bind::<EditAutocompleteWord> },
    ActionEntry { id: "EditAutocompleteFuncAndWord", label: "Function and Word Completion", default_keys: &["ctrl-alt-space"],      context: W, bind: bind::<EditAutocompleteFuncAndWord> },
    ActionEntry { id: "EditAutocompleteFuncParams",  label: "Function Parameters Hint",   default_keys: &["ctrl-shift-space"],      context: W, bind: bind::<EditAutocompleteFuncParams> },

    // --- Caret movement ---
    ActionEntry { id: "MoveLeft",        label: "Move Left",                  default_keys: &["left"],                                                  context: W, bind: bind::<MoveLeft> },
    ActionEntry { id: "MoveRight",       label: "Move Right",                 default_keys: &["right"],                                                 context: W, bind: bind::<MoveRight> },
    ActionEntry { id: "MoveUp",          label: "Move Up",                    default_keys: &["up"],                                                    context: W, bind: bind::<MoveUp> },
    ActionEntry { id: "MoveDown",        label: "Move Down",                  default_keys: &["down"],                                                  context: W, bind: bind::<MoveDown> },
    ActionEntry { id: "SelectLeft",      label: "Select Left",                default_keys: &["shift-left"],                                            context: W, bind: bind::<SelectLeft> },
    ActionEntry { id: "SelectRight",     label: "Select Right",               default_keys: &["shift-right"],                                           context: W, bind: bind::<SelectRight> },
    ActionEntry { id: "SelectUp",        label: "Select Up",                  default_keys: &["shift-up"],                                              context: W, bind: bind::<SelectUp> },
    ActionEntry { id: "SelectDown",      label: "Select Down",                default_keys: &["shift-down"],                                            context: W, bind: bind::<SelectDown> },
    ActionEntry { id: "EditPageUp",      label: "Page Up",                    default_keys: &["pageup"],                                                context: W, bind: bind::<EditPageUp> },
    ActionEntry { id: "EditPageDown",    label: "Page Down",                  default_keys: &["pagedown"],                                              context: W, bind: bind::<EditPageDown> },
    ActionEntry { id: "SelectPageUp",    label: "Select Page Up",             default_keys: &["shift-pageup"],                                          context: W, bind: bind::<SelectPageUp> },
    ActionEntry { id: "SelectPageDown",  label: "Select Page Down",           default_keys: &["shift-pagedown"],                                        context: W, bind: bind::<SelectPageDown> },
    ActionEntry { id: "MoveLineStart",   label: "Move to Line Start",         default_keys: &["home", "cmd-left"],                                      context: W, bind: bind::<MoveLineStart> },
    ActionEntry { id: "MoveLineEnd",     label: "Move to Line End",           default_keys: &["end", "cmd-right"],                                      context: W, bind: bind::<MoveLineEnd> },
    ActionEntry { id: "SelectLineStart", label: "Select to Line Start",       default_keys: &["shift-home", "cmd-shift-left"],                          context: W, bind: bind::<SelectLineStart> },
    ActionEntry { id: "SelectLineEnd",   label: "Select to Line End",         default_keys: &["shift-end", "cmd-shift-right"],                          context: W, bind: bind::<SelectLineEnd> },
    ActionEntry { id: "MoveWordLeft",    label: "Move Word Left",             default_keys: &["alt-left", "ctrl-left"],                                 context: W, bind: bind::<MoveWordLeft> },
    ActionEntry { id: "MoveWordRight",   label: "Move Word Right",            default_keys: &["alt-right", "ctrl-right"],                               context: W, bind: bind::<MoveWordRight> },
    ActionEntry { id: "SelectWordLeft",  label: "Select Word Left",           default_keys: &["alt-shift-left", "ctrl-shift-left"],                     context: W, bind: bind::<SelectWordLeft> },
    ActionEntry { id: "SelectWordRight", label: "Select Word Right",          default_keys: &["alt-shift-right", "ctrl-shift-right"],                   context: W, bind: bind::<SelectWordRight> },
    ActionEntry { id: "MoveDocStart",    label: "Move to Document Start",     default_keys: &["cmd-up", "cmd-home", "ctrl-home"],                       context: W, bind: bind::<MoveDocStart> },
    ActionEntry { id: "MoveDocEnd",      label: "Move to Document End",       default_keys: &["cmd-down", "cmd-end", "ctrl-end"],                       context: W, bind: bind::<MoveDocEnd> },
    ActionEntry { id: "SelectDocStart",  label: "Select to Document Start",   default_keys: &["cmd-shift-up", "cmd-shift-home", "ctrl-shift-home"],     context: W, bind: bind::<SelectDocStart> },
    ActionEntry { id: "SelectDocEnd",    label: "Select to Document End",     default_keys: &["cmd-shift-down", "cmd-shift-end", "ctrl-shift-end"],     context: W, bind: bind::<SelectDocEnd> },

    // --- Column-mode (Notepad++ block selection) keyboard extension. Mouse
    //     alt-drag is the primary entry point; these chords let the user
    //     extend the rectangle with the keyboard. Default chord is
    //     cmd/ctrl-alt-shift-arrow because plain alt-shift-arrow is taken
    //     (Move-Line-Up/Down on macOS, Select-Word-Left/Right elsewhere).
    ActionEntry { id: "BlockExtendUp",    label: "Extend Block Up",    default_keys: &["cmd-alt-shift-up", "ctrl-alt-shift-up"],       context: W, bind: bind::<BlockExtendUp> },
    ActionEntry { id: "BlockExtendDown",  label: "Extend Block Down",  default_keys: &["cmd-alt-shift-down", "ctrl-alt-shift-down"],   context: W, bind: bind::<BlockExtendDown> },
    ActionEntry { id: "BlockExtendLeft",  label: "Extend Block Left",  default_keys: &["cmd-alt-shift-left", "ctrl-alt-shift-left"],   context: W, bind: bind::<BlockExtendLeft> },
    ActionEntry { id: "BlockExtendRight", label: "Extend Block Right", default_keys: &["cmd-alt-shift-right", "ctrl-alt-shift-right"], context: W, bind: bind::<BlockExtendRight> },

    // --- Search ---
    ActionEntry { id: "EditFind",            label: "Find",                  default_keys: &["cmd-f", "ctrl-f"],         context: W, bind: bind::<EditFind> },
    ActionEntry { id: "EditFindClose",       label: "Close Find",            default_keys: &["escape"],                  context: W, bind: bind::<EditFindClose> },
    ActionEntry { id: "EditFindNext",        label: "Find Next",             default_keys: &["f3"],                      context: W, bind: bind::<EditFindNext> },
    ActionEntry { id: "EditFindPrev",        label: "Find Previous",         default_keys: &["shift-f3", "shift-enter"], context: W, bind: bind::<EditFindPrev> },
    ActionEntry { id: "EditReplace",         label: "Replace",               default_keys: &["cmd-alt-f", "ctrl-h"],     context: W, bind: bind::<EditReplace> },
    ActionEntry { id: "EditGotoLine",        label: "Go to Line",            default_keys: &["cmd-l", "ctrl-g"],         context: W, bind: bind::<EditGotoLine> },
    ActionEntry { id: "SearchFindInFiles",   label: "Find in Files",         default_keys: &["cmd-shift-f", "ctrl-shift-f"], context: W, bind: bind::<SearchFindInFiles> },
    ActionEntry { id: "SearchMark",          label: "Mark",                  default_keys: &["cmd-m", "ctrl-m"],         context: W, bind: bind::<SearchMark> },

    // --- Tabs ---
    ActionEntry { id: "TabNext",            label: "Next Tab",            default_keys: &["ctrl-tab", "cmd-tab"],             context: W, bind: bind::<TabNext> },
    ActionEntry { id: "TabPrev",            label: "Previous Tab",        default_keys: &["ctrl-shift-tab", "cmd-shift-tab"], context: W, bind: bind::<TabPrev> },
    ActionEntry { id: "TabNextSequential", label: "Next Tab (Visual)",   default_keys: &["ctrl-pagedown"],                   context: W, bind: bind::<TabNextSequential> },
    ActionEntry { id: "TabPrevSequential", label: "Previous Tab (Visual)", default_keys: &["ctrl-pageup"],                   context: W, bind: bind::<TabPrevSequential> },
    ActionEntry { id: "TabGoto1",          label: "Go to Tab 1",         default_keys: &["cmd-1", "ctrl-1"],                 context: W, bind: bind::<TabGoto1> },
    ActionEntry { id: "TabGoto2",          label: "Go to Tab 2",         default_keys: &["cmd-2", "ctrl-2"],                 context: W, bind: bind::<TabGoto2> },
    ActionEntry { id: "TabGoto3",          label: "Go to Tab 3",         default_keys: &["cmd-3", "ctrl-3"],                 context: W, bind: bind::<TabGoto3> },
    ActionEntry { id: "TabGoto4",          label: "Go to Tab 4",         default_keys: &["cmd-4", "ctrl-4"],                 context: W, bind: bind::<TabGoto4> },
    ActionEntry { id: "TabGoto5",          label: "Go to Tab 5",         default_keys: &["cmd-5", "ctrl-5"],                 context: W, bind: bind::<TabGoto5> },
    ActionEntry { id: "TabGoto6",          label: "Go to Tab 6",         default_keys: &["cmd-6", "ctrl-6"],                 context: W, bind: bind::<TabGoto6> },
    ActionEntry { id: "TabGoto7",          label: "Go to Tab 7",         default_keys: &["cmd-7", "ctrl-7"],                 context: W, bind: bind::<TabGoto7> },
    ActionEntry { id: "TabGoto8",          label: "Go to Tab 8",         default_keys: &["cmd-8", "ctrl-8"],                 context: W, bind: bind::<TabGoto8> },
    ActionEntry { id: "TabGoto9",          label: "Go to Tab 9",         default_keys: &["cmd-9", "ctrl-9"],                 context: W, bind: bind::<TabGoto9> },

    // --- View ---
    ActionEntry { id: "ViewZoomIn",           label: "Zoom In",              default_keys: &["cmd-=", "ctrl-=", "cmd-shift-=", "ctrl-shift-="], context: W, bind: bind::<ViewZoomIn> },
    ActionEntry { id: "ViewZoomOut",          label: "Zoom Out",             default_keys: &["cmd--", "ctrl--"],                                context: W, bind: bind::<ViewZoomOut> },
    ActionEntry { id: "ViewZoomReset",        label: "Reset Zoom",           default_keys: &["cmd-0", "ctrl-0"],                                context: W, bind: bind::<ViewZoomReset> },
    ActionEntry { id: "ViewToggleFullScreen", label: "Toggle Full Screen",   default_keys: &["f11"],                                            context: W, bind: bind::<ViewToggleFullScreen> },
    ActionEntry { id: "ViewCsvToggleFilterRow", label: "CSV: Toggle Filter Row", default_keys: &["cmd-shift-l", "ctrl-shift-l"],                context: W, bind: bind::<ViewCsvToggleFilterRow> },

    // --- Macros ---
    ActionEntry { id: "MacroStartStopRecording", label: "Start/Stop Recording",   default_keys: &["cmd-shift-r", "ctrl-shift-r"], context: W, bind: bind::<MacroStartStopRecording> },
    ActionEntry { id: "MacroPlayback",            label: "Playback",               default_keys: &["cmd-shift-p", "ctrl-shift-p"], context: W, bind: bind::<MacroPlayback> },
    ActionEntry { id: "MacroRunMultipleTimes",    label: "Run a Macro Multiple Times", default_keys: &["cmd-shift-m", "ctrl-shift-m"], context: W, bind: bind::<MacroRunMultipleTimes> },
    ActionEntry { id: "MacroSaveCurrent",         label: "Save Currently Recorded Macro", default_keys: &[],                          context: W, bind: bind::<MacroSaveCurrent> },

    // --- Bookmarks ---
    ActionEntry { id: "BookmarkToggle",   label: "Toggle Bookmark",      default_keys: &["f2"],                              context: W, bind: bind::<BookmarkToggle> },
    ActionEntry { id: "BookmarkNext",     label: "Next Bookmark",        default_keys: &["ctrl-f2"],                         context: W, bind: bind::<BookmarkNext> },
    ActionEntry { id: "BookmarkPrev",     label: "Previous Bookmark",    default_keys: &["shift-f2"],                        context: W, bind: bind::<BookmarkPrev> },
    ActionEntry { id: "BookmarkClearAll", label: "Clear All Bookmarks",  default_keys: &["cmd-shift-f2", "ctrl-shift-f2"],   context: W, bind: bind::<BookmarkClearAll> },

    // --- Settings ---
    // OpenPreferences is a global action (no Workspace context) so the
    // chord works even when no editor window is focused — Cmd+, on macOS
    // is the platform convention for Preferences; Ctrl+Alt+P keeps the
    // Linux/Windows side mnemonic-friendly without colliding with cut/
    // copy/paste/undo.
    ActionEntry { id: "OpenPreferences", label: "Open Preferences",  default_keys: &["cmd-,", "ctrl-alt-p"], context: None, bind: bind::<OpenPreferences> },

    // Style Configurator — `Cmd+Shift+,` (mac) / `Ctrl+Shift+,` (linux/win)
    // sits adjacent to Preferences (`Cmd+,`) so the muscle memory is "comma
    // family". Global like Preferences — works without a focused editor.
    ActionEntry { id: "OpenStyleConfigurator", label: "Open Style Configurator", default_keys: &["cmd-shift-,", "ctrl-shift-,"], context: None, bind: bind::<OpenStyleConfigurator> },

    // --- Markdown formatting ---
    // Chords mirror the dotcom / pandoc / VS Code conventions. The handlers
    // are guarded by `md_guard` (extension-based) so a stray dispatch on a
    // non-markdown tab silently no-ops — none of these chords have a
    // non-markdown action in Notepad Studio, so binding them workspace-wide
    // doesn't shadow anything.
    ActionEntry { id: "MdBold",           label: "Markdown: Bold",            default_keys: &["cmd-b", "ctrl-b"],                       context: W, bind: bind::<MdBold> },
    ActionEntry { id: "MdItalic",         label: "Markdown: Italic",          default_keys: &["cmd-i", "ctrl-i"],                       context: W, bind: bind::<MdItalic> },
    ActionEntry { id: "MdStrikethrough",  label: "Markdown: Strikethrough",   default_keys: &["cmd-shift-x", "ctrl-shift-x"],           context: W, bind: bind::<MdStrikethrough> },
    ActionEntry { id: "MdInlineCode",     label: "Markdown: Inline Code",     default_keys: &["cmd-e", "ctrl-e"],                       context: W, bind: bind::<MdInlineCode> },
    ActionEntry { id: "MdLink",           label: "Markdown: Link",            default_keys: &["cmd-k", "ctrl-k"],                       context: W, bind: bind::<MdLink> },
    ActionEntry { id: "MdHeading1",       label: "Markdown: Heading 1",       default_keys: &["cmd-shift-1", "ctrl-shift-1"],           context: W, bind: bind::<MdHeading1> },
    ActionEntry { id: "MdHeading2",       label: "Markdown: Heading 2",       default_keys: &["cmd-shift-2", "ctrl-shift-2"],           context: W, bind: bind::<MdHeading2> },
    ActionEntry { id: "MdHeading3",       label: "Markdown: Heading 3",       default_keys: &["cmd-shift-3", "ctrl-shift-3"],           context: W, bind: bind::<MdHeading3> },
    ActionEntry { id: "MdBulletList",     label: "Markdown: Bullet List",     default_keys: &["cmd-shift-8", "ctrl-shift-8"],           context: W, bind: bind::<MdBulletList> },
    ActionEntry { id: "MdOrderedList",    label: "Markdown: Ordered List",    default_keys: &["cmd-shift-7", "ctrl-shift-7"],           context: W, bind: bind::<MdOrderedList> },
    ActionEntry { id: "MdTaskList",       label: "Markdown: Task List",       default_keys: &["cmd-shift-9", "ctrl-shift-9"],           context: W, bind: bind::<MdTaskList> },
    ActionEntry { id: "MdBlockQuote",     label: "Markdown: Block Quote",     default_keys: &["cmd-shift-.", "ctrl-shift-."],           context: W, bind: bind::<MdBlockQuote> },
    ActionEntry { id: "MdCodeBlock",      label: "Markdown: Code Block",      default_keys: &["cmd-shift-alt-c", "ctrl-shift-alt-c"],   context: W, bind: bind::<MdCodeBlock> },
    ActionEntry { id: "MdImage",          label: "Markdown: Image",           default_keys: &["cmd-shift-i", "ctrl-shift-i"],           context: W, bind: bind::<MdImage> },
    ActionEntry { id: "MdHorizontalRule", label: "Markdown: Horizontal Rule", default_keys: &["cmd-shift--", "ctrl-shift--"],           context: W, bind: bind::<MdHorizontalRule> },

    // --- Rich-doc formatting ---
    // Promote / Demote follow the Notion convention (cmd-shift-] / cmd-shift-[)
    // — both chords are free in this build. ctrl-shift-up/down are taken by
    // EditMoveLineUp/Down, so those weren't usable for headings.
    ActionEntry { id: "RichDocPromoteHeading", label: "Rich-doc: Promote Heading", default_keys: &["cmd-shift-]", "ctrl-shift-]"], context: W, bind: bind::<RichDocPromoteHeading> },
    ActionEntry { id: "RichDocDemoteHeading",  label: "Rich-doc: Demote Heading",  default_keys: &["cmd-shift-[", "ctrl-shift-["], context: W, bind: bind::<RichDocDemoteHeading> },
];

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

pub(crate) fn load_overrides() -> Overrides {
    let Some(path) = shortcuts_path() else {
        return Overrides::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Overrides::new();
    };
    let Ok(file) = toml::from_str::<OverridesFile>(&raw) else {
        return Overrides::new();
    };
    file.binding
        .into_iter()
        .map(|b| (b.action, b.keys))
        .collect()
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
    entry.default_keys.iter().map(|s| (*s).to_string()).collect()
}

// ---------------------------------------------------------------------------
// Apply: rebuild the live keymap from defaults + overrides
// ---------------------------------------------------------------------------

/// Bind every catalog entry, honouring `overrides`. Caller is responsible for
/// clearing the existing keymap first when reloading at runtime; on startup
/// the keymap is empty so a plain bind suffices.
pub(crate) fn register_catalog(cx: &mut App, overrides: &Overrides) {
    for entry in CATALOG {
        for keystr in effective_keys(entry, overrides) {
            // Failed-to-parse strings come from corrupt user files; we log
            // and continue so one busted row doesn't lock the user out of
            // the rest of the keymap. `KeyBinding::new` panics on parse
            // error, so we route through `Keystroke::parse` first to reject
            // garbage with a printed warning.
            if gpui::Keystroke::parse(&keystr).is_err() {
                eprintln!(
                    "shortcuts.toml: dropping unparseable chord {:?} for {}",
                    keystr, entry.id
                );
                continue;
            }
            let binding = (entry.bind)(&keystr, entry.context);
            crate::app::keymap::bind_mapped(cx, [binding]);
        }
    }
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
/// the user can re-bind a chord they already had.
pub(crate) fn conflict_for(
    chord: &str,
    proposed: &Overrides,
    skip: &str,
) -> Option<&'static str> {
    for entry in CATALOG {
        if entry.id == skip {
            continue;
        }
        for k in effective_keys(entry, proposed) {
            if k == chord {
                return Some(entry.id);
            }
        }
    }
    None
}
