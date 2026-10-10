// Workspace shell — the root entity, fields, constructor, accessors, scroll
// helpers, status, viewport-related geometry, and `impl Focusable`.
//
// The bulk of `impl Workspace` (edit/movement/tab/find/render/input-handler
// methods) still lives in `main.rs`. Subsequent refactor forks (E-K) cut
// those method groups out into sibling modules under `workspace/`.

pub(crate) mod autocomplete;
mod block;
mod close_tag;
mod cursor;
mod tag_match;
pub(crate) mod edits;
mod find;
mod find_in_files;
mod folder_panel;
mod format;
mod input_handler;
mod macros;
mod markdown_format;
mod menu_bar;
mod mouse;
mod movement;
mod plugins;
mod render;
mod status;
mod tabs;

use gpui::{
    App, Bounds, Context, FocusHandle, Focusable, Pixels, Point, ScrollHandle, Subscription, Task,
    Window, WindowAppearance, WindowHandle, point, px, size,
};

use std::collections::VecDeque;
use std::time::Duration;
use uuid::Uuid;

use crate::buffer::{Buffer, LineGeo, Selection};
use crate::config::record_zoom_level;
use crate::consts::{ZOOM_DEFAULT, ZOOM_MAX, ZOOM_MIN, ZOOM_REM_BASE, ZOOM_STEP};
use crate::find_in_files::FindInFilesState;
use crate::find_state::{
    DialogTab, FindState, GotoState, MacroPromptState, MarkState, TabRenameState,
};
use crate::session::{self, RestoreOutcome, SessionManifest};
use crate::tab::{ClosedTab, InputFocus, Tab};
use crate::theme::{Theme, ThemeMode};
use crate::ui::find_dialog::FindDialog;
use crate::ui::folder_panel::FolderPanelState;

pub(crate) struct Workspace {
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    pub(crate) closed_tabs: Vec<ClosedTab>,
    /// Most-recently-used tab order, keyed by `Tab::session_id`. Index 0 is
    /// the most-recently-activated tab. Notepad++ `MISC. ▸ Document Switcher`
    /// uses this for Ctrl+Tab / Ctrl+Shift+Tab cycling (separate from the
    /// visual-order Ctrl+PageUp / Ctrl+PageDown cycle).
    ///
    /// External tab activations (mouse click, goto-N, restore-closed) push the
    /// new active to the front; `cycle_tab_mru` walks the list *without*
    /// pushing so a held-Ctrl Tab-tap sequence can step through several tabs
    /// without scrambling the order mid-cycle. The next external activation
    /// commits the chosen tab by re-fronting it.
    pub(crate) tab_mru: VecDeque<Uuid>,
    pub(crate) theme_mode: ThemeMode,
    pub(crate) focus_handle: FocusHandle,
    /// Focus held by the find-query input. When focused, typed text routes
    /// into `find_state.query` instead of the active buffer.
    pub(crate) find_focus: FocusHandle,
    /// Focus held by the replace input.
    pub(crate) replace_focus: FocusHandle,
    /// Focus held by the goto-line input.
    pub(crate) goto_focus: FocusHandle,
    /// Focus held by the Find-in-Files search input.
    pub(crate) fif_query_focus: FocusHandle,
    /// Focus held by the Find-in-Files filter input.
    pub(crate) fif_filter_focus: FocusHandle,
    /// Focus held by the Find-in-Files replacement input. Lazily created on
    /// the first dialog open so the existing `Workspace::new` signature
    /// (which is shared with the off-limits `app/window.rs`) does not need
    /// to grow new parameters every time the dialog gains a field.
    pub(crate) fif_replacement_focus: Option<FocusHandle>,
    /// Focus held by the Find-in-Files directory text input. Lazily created.
    pub(crate) fif_directory_focus: Option<FocusHandle>,
    /// Focus held by the Mark tab's search input. Lazily created.
    pub(crate) mark_focus: Option<FocusHandle>,
    /// Visible-line geometry from the last paint, in window-space pixels.
    /// Populated by `EditorElement::paint`; consumed by mouse hit-testing
    /// and vertical caret movement.
    pub(crate) last_lines: Vec<LineGeo>,
    pub(crate) caret_visible: bool,
    /// Overtype (Scintilla "overstrike") mode, toggled by Insert. Typed
    /// characters replace the ones after the caret instead of pushing them
    /// right. Per window, like Scintilla's per-view flag; not persisted.
    pub(crate) overtype: bool,
    /// When true, `apply_edit` suppresses its per-edit flush (reparse, scroll,
    /// repaint, session + autosave disk writes) so a batched run does that
    /// work once at the end. Set during undo/redo replay of a composite group;
    /// the `edit_transaction` path (Replace All, multi-cursor, block ops) is
    /// the other trigger. Keeps batched edits O(n) instead of O(n²).
    pub(crate) bulk_edit: bool,
    /// One-line ephemeral message shown on the right side of the status bar
    /// — overrides the encoding/EOL/syntax/mode chips while present.
    /// Set by file ops; cleared on the next text edit.
    pub(crate) status_message: Option<String>,
    pub(crate) find_state: FindState,
    pub(crate) goto_state: GotoState,
    pub(crate) find_in_files: FindInFilesState,
    pub(crate) mark_state: MarkState,
    /// Currently-active tab in the Find dialog. Persists across close/reopen
    /// so the dialog comes back showing the user's last working surface.
    pub(crate) dialog_tab: DialogTab,
    /// Live handle to the Find dialog window. `Some` when the dialog is on
    /// screen; cleared when the window closes (either via close button, Esc,
    /// or OS-level close). The dialog is recreated on every open — state
    /// lives here on `Workspace`, the window itself is disposable.
    pub(crate) find_dialog_window: Option<WindowHandle<FindDialog>>,
    /// Which Find dialog input currently has its history-dropdown open.
    /// `None` = no dropdown showing. Driven by the chevron click on each
    /// input; closed by selecting an entry, clicking the scrim, pressing
    /// Esc, or opening a different input's dropdown.
    pub(crate) find_history_open: Option<crate::settings::FindHistorySlot>,
    /// Word-wrap toggle. When true, lines that overflow the viewport break
    /// at the last whitespace before the right edge.
    pub(crate) word_wrap: bool,
    /// Show-whitespace overlay toggle (NPP "View ▸ Show Symbol ▸ Show White
    /// Space and TAB"). When true, `EditorElement` paints a faint dot for
    /// each space and a faint arrow across each tab. Persisted in the
    /// session manifest so the choice survives relaunch.
    pub(crate) show_whitespace: bool,
    /// Cached viewport metrics from the last editor paint. Used by
    /// `ensure_cursor_visible`, page-nav, and the scrollbar to compute scroll
    /// extents without needing access to a `Window`.
    pub(crate) viewport: Viewport,
    /// Scrollbar drag state. `Some(offset_within_thumb)` means the mouse
    /// captured the thumb at that local y, and every subsequent mouse-move
    /// translates into a scroll_top update until release.
    pub(crate) v_scroll_drag: Option<Pixels>,
    /// Horizontal scrollbar drag state — `Some(offset_within_thumb)` while
    /// the user is dragging the thumb, `None` otherwise. Mirrors
    /// `v_scroll_drag`.
    pub(crate) h_scroll_drag: Option<Pixels>,
    /// Persistent scroll handle for the markdown preview pane. Lives on the
    /// workspace (not recreated each render) so the scroll offset survives
    /// across paints and the scrollbar element can read/write it.
    pub(crate) md_preview_scroll_handle: ScrollHandle,
    /// Same idea for the rich-doc (Doc View) pane — without this, every
    /// paint would build a fresh `ScrollHandle::new()` and the offset would
    /// snap back to 0 on the next notify (typing, mouse-move, anything).
    pub(crate) rich_doc_scroll_handle: ScrollHandle,
    /// Markdown preview scrollbar drag state. Same shape as `v_scroll_drag`
    /// but owned by the preview's own scrollbar; the editor scrollbar uses
    /// `v_scroll_drag` and they shouldn't share state.
    pub(crate) md_preview_scroll_drag: Option<Pixels>,
    /// Rich-doc (Doc View) pane scrollbar drag state. Same shape as
    /// `md_preview_scroll_drag`. Kept separate so the two preview-style
    /// scrollbars never stomp each other (they're mutually exclusive at
    /// render time but the drag-state field has to be reachable from the
    /// scrollbar element's mouse handlers).
    pub(crate) rich_doc_scroll_drag: Option<Pixels>,
    /// Markdown preview hit-target registry — one entry per rendered
    /// paragraph / heading line. Refilled every render frame (cleared in
    /// `workspace/render.rs` before delegating to `markdown_preview_pane`,
    /// then each block's `CaretOverlay::paint` pushes its
    /// `(layout, hits)`). The window-level mouse-move listener installed
    /// by `MarkdownInputBridge::paint` reads this to extend drag selection
    /// across block gaps; the outer-pane mouse-down handler reads it to
    /// start drags from dead space (margins, between blocks, the trailing
    /// scroll-past-end spacer).
    pub(crate) preview_block_targets: Vec<crate::ui::markdown_view::PreviewBlockTarget>,
    /// Set by `ensure_cursor_visible` when in markdown-preview mode to ask
    /// the next `CaretOverlay::paint` to scroll the caret into the visible
    /// viewport. Mouse-click selections explicitly clear this so a click
    /// never causes the view to jerk. Cleared by `CaretOverlay::paint` once
    /// the scroll adjustment has been applied (or once it determined no
    /// scroll was needed).
    pub(crate) preview_caret_scroll_pending: bool,
    /// Last cursor position seen during an in-flight preview drag. Captured
    /// every MouseMoveEvent so the continuous-autoscroll timer can re-fire
    /// edge scrolling against a stationary cursor (native text controls keep
    /// scrolling when the cursor is held past the edge, even with no further
    /// mouse movement). `None` between drags.
    pub(crate) preview_last_drag_pos: Option<Point<Pixels>>,
    /// Handle to the continuous-autoscroll task. `Some` while the cursor sits
    /// past the viewport's top/bottom edge during a drag; dropped (auto-
    /// cancels the spawned future) when the drag ends, the cursor moves back
    /// inside the viewport, or the drag is no longer active.
    pub(crate) preview_autoscroll_task: Option<Task<()>>,
    /// Same shape as `preview_last_drag_pos` but for the rich-doc (Doc View)
    /// pane. Captured every per-block MouseMoveEvent during an in-flight
    /// rich-doc drag so the continuous-autoscroll timer can re-fire edge
    /// scrolling against a stationary cursor (NSTextView keeps scrolling
    /// when the cursor is held past the edge with no further movement).
    /// `None` between drags.
    pub(crate) rich_doc_last_drag_pos: Option<Point<Pixels>>,
    /// Same shape as `preview_autoscroll_task` but for the rich-doc pane.
    /// `Some` while the cursor sits past the rich-doc viewport's top/bottom
    /// edge during a drag; dropped (auto-cancels the spawned future) when
    /// the drag ends, the cursor moves back inside the viewport, or
    /// `payload.drag_active` flips to false.
    pub(crate) rich_doc_autoscroll_task: Option<Task<()>>,
    /// VS Code-style window zoom multiplier. 1.0 = 100%. Persisted across
    /// launches in `<config>/Notepad Studio/window.toml`. Drives `Window::set_rem_size`
    /// so every rem-based UI dimension scales together.
    pub(crate) zoom_level: f32,
    /// Right-click context menu state. `Some(point)` = visible at the given
    /// window-space coordinate; `None` = hidden.
    pub(crate) context_menu: Option<Point<Pixels>>,
    /// Monotonic counter bumped on every text edit. The autosave debounce
    /// captures the generation when scheduled and only writes if it still
    /// matches when the timer fires — drops noop writes when the user
    /// keeps typing.
    pub(crate) autosave_generation: u64,
    /// Generation guard for the debounced post-edit reparse + eager-persist
    /// task (`schedule_reparse`). Mirrors `autosave_generation`: each edit
    /// bumps it, and only the latest scheduled task does the heavyweight
    /// reparse so a burst of edits (Replace All, fast typing) reparses once
    /// after things settle instead of once per edit on the UI thread.
    pub(crate) post_edit_generation: u64,
    /// Which dialog input is currently being drag-selected with the mouse,
    /// if any. `Some(focus)` while the user holds left-button after clicking
    /// the input; on every mouse move the cursor side of the selection on
    /// `focus` follows the pointer. Cleared on mouse-up. Mirrors the
    /// editor's `selection.is_dragging` flag; lives on the workspace so the
    /// window-level mouse listeners can read/mutate it across frames.
    pub(crate) input_drag: Option<InputDrag>,
    /// Active mouse-drag granularity. Set on mouse-down based on click count
    /// (1 = Char, 2 = Word, 3 = Line, 4+ = select-all → reverts to Char on
    /// drag start). Read by the editor's `on_mouse_event(MouseMoveEvent…)`
    /// drag handler so a double-click-drag extends by whole words and a
    /// triple-click-drag extends by whole lines, NPP-style.
    pub(crate) drag_mode: DragMode,
    /// Byte range of the word/line under the initial click when `drag_mode`
    /// is Word or Line. Used so drag-extend snaps the selection to the
    /// outside edges of either the anchor range or the range under the
    /// pointer, whichever is farther from the anchor.
    pub(crate) drag_anchor_range: Option<std::ops::Range<usize>>,
    /// Selected-text drag-and-drop state. `Some` while the user has clicked
    /// inside an active selection but hasn't moved past the start threshold
    /// yet (or while a drag is committed and in flight). See `TextDragPhase`
    /// for the two-step lifecycle. Cleared on mouse-up or Esc.
    pub(crate) text_drag: Option<TextDragState>,
    /// Inline prompt bar state for macro operations (Run N times / Save name).
    /// `Some(state)` shows the bar between the tab strip and the editor pane,
    /// borrowing the same focus / input-handler plumbing the goto bar uses.
    pub(crate) macro_prompt: Option<MacroPromptState>,
    /// Focus held by the macro prompt's text input. Lazily created on first
    /// open so the existing `Workspace::new` signature isn't widened.
    pub(crate) macro_focus: Option<FocusHandle>,
    /// Cached OS window title from the last frame — set via `set_window_title`
    /// only when this changes so we don't hammer the platform call every paint.
    pub(crate) last_window_title: Option<String>,
    /// Tab right-click context menu state. `Some(state)` while the menu is
    /// visible; `None` when hidden. Stores the target tab's index, the menu
    /// origin (window-space), and which submenu (if any) is currently
    /// expanded.
    pub(crate) tab_context_menu: Option<TabContextMenuState>,
    /// Inline rename bar state. `Some(state)` mounts the bar above the
    /// editor pane; `None` hides it.
    pub(crate) tab_rename: Option<TabRenameState>,
    /// Focus held by the tab-rename input. Lazily created on first open.
    pub(crate) tab_rename_focus: Option<FocusHandle>,
    /// Inline rich-doc link-prompt bar state. `Some` while the user is
    /// typing a URL after clicking the toolbar Link button; `None` hides
    /// the bar.
    pub(crate) rich_doc_link_bar: Option<crate::find_state::RichDocLinkBarState>,
    /// Focus held by the rich-doc link URL input.
    pub(crate) rich_doc_link_focus: Option<FocusHandle>,
    /// Inline rich-doc color-picker bar state. `Some` while the bar is
    /// mounted; `None` hides it.
    pub(crate) rich_doc_color_bar: Option<crate::find_state::RichDocColorBarState>,
    /// Focus held by the rich-doc color hex input.
    pub(crate) rich_doc_color_focus: Option<FocusHandle>,
    /// Focus held by the active rich-doc editable paragraph (Phase 2g.A).
    /// Created lazily on first paragraph click; shared across blocks since
    /// only one paragraph edits at a time — the focused-handle stays the
    /// same as the user clicks between paragraphs, only the payload's
    /// `active_block` / `selection` change.
    pub(crate) rich_doc_body_focus: Option<FocusHandle>,
    /// Help ▸ About modal — true while the centered "About Notepad Studio" overlay
    /// is mounted. Dismissed via Esc (close_find chain), click-outside the
    /// inner panel (scrim handler), or the OK button.
    pub(crate) about_dialog_open: bool,
    /// Paste-with-formatting chooser modal — `Some` while the centered
    /// "Keep formatting / Paste as plain text" overlay is mounted. Carries
    /// the pre-decoded payload (plain text + rich-doc blocks) so the user's
    /// button choice routes through the right paste path without re-reading
    /// the clipboard (which could change between the modal opening and the
    /// user clicking a button). `None` = modal closed.
    pub(crate) paste_format_pending: Option<PasteFormatPending>,
    /// Cached OS appearance for the window. Seeded at construction and kept
    /// in sync by an `observe_window_appearance` subscription installed in
    /// `app/window.rs`. Read by `theme()` to resolve `ThemeMode::FollowSystem`.
    pub(crate) system_appearance: WindowAppearance,
    /// Byte ranges of every other occurrence of the word at the cursor.
    /// Refreshed by `refresh_smart_highlight()` whenever the caret moves
    /// (via `commit_caret`) or the buffer changes. Empty when the cursor
    /// is not on a word, when the word is shorter than 2 chars, when the
    /// active buffer exceeds the smart-hilite size cutoff, or when
    /// `Settings::smart_highlight_enabled` is off.
    pub(crate) smart_match_ranges: Vec<std::ops::Range<usize>>,
    /// `Highlighting ▸ Highlight Matching Tags` overlay state. `Some` when
    /// the caret is inside a `<tag>` or `</tag>` in an HTML/XML buffer AND
    /// the matching pair has been resolved; `None` otherwise. Read by
    /// `editor_element::prepaint` to emit a highlight quad on each half.
    pub(crate) tag_match_pair: Option<tag_match::TagPair>,
    /// Attribute ranges inside the open tag containing the caret. Populated
    /// only when `Highlighting ▸ Match Tags ▸ Highlight tag attributes` is
    /// on; otherwise stays empty. Each range covers `name[=value]`. Painted
    /// alongside [`Self::tag_match_pair`] in the same smart-highlight colour.
    pub(crate) tag_match_attrs: Vec<std::ops::Range<usize>>,
    /// Comment / CDATA / PI / ASP zone containing the caret. Populated only
    /// when `Highlighting ▸ Match Tags ▸ Highlight comment/PHP/ASP zone` is
    /// on. `open` is the start delimiter range (`<!--`, `<![CDATA[`, `<?php`
    /// etc.); `close` is the matching terminator (`-->`, `]]>`, `?>`, `%>`).
    pub(crate) tag_match_zone: Option<tag_match::TagPair>,
    /// The word that produced `smart_match_ranges`. Used to skip the
    /// re-scan when the caret moves but stays inside the same word.
    pub(crate) smart_match_word: Option<String>,
    /// Bottom-docked Text Tools preview pane state. `Some` while a Text
    /// Tools tool is active in the modal — its transformed output streams
    /// into `state.content` and the workspace render adds a panel below
    /// the editor showing it. `None` hides the panel entirely.
    pub(crate) preview_pane: Option<PreviewPaneState>,
    /// Tab indices that belong to the active Text Tools session as
    /// auxiliary inputs (List B for Lists Comparison, columns 2..N for
    /// Join/Merge). They appear in the regular tab strip prefixed with
    /// `[Aux]` and are torn down when the modal closes or the tool view
    /// changes. Indices reference `self.tabs` and may shift when the
    /// user closes other tabs — refresh before use via `aux_tab_text`.
    #[allow(dead_code)]
    pub(crate) text_tools_aux_tabs: Vec<usize>,
    /// Pixel height of the Text Tools preview pane. Persisted across
    /// modal open/close so the user's resize preference survives picking
    /// a different tool. Clamped to [PREVIEW_PANE_MIN_HEIGHT, viewport_h
    /// * PREVIEW_PANE_MAX_FRACTION] inside `set_preview_pane_height`.
    pub(crate) preview_pane_height: Pixels,
    /// In-flight drag state for the splitter handle between the editor
    /// and the preview pane. `Some` while the user is holding the mouse
    /// down on the handle.
    pub(crate) preview_pane_drag: Option<PreviewPaneDrag>,
    /// Pixel height of the Find-in-Files results panel. User-draggable via
    /// the splitter handle at the top of the panel; persisted across the
    /// panel's open/close cycle so the user's resize preference survives a
    /// search re-run. Clamped to [FIF_PANEL_MIN_HEIGHT, viewport_h *
    /// FIF_PANEL_MAX_FRACTION] inside `set_fif_panel_height`.
    pub(crate) fif_panel_height: Pixels,
    /// In-flight drag state for the splitter handle between the editor and
    /// the Find-in-Files results panel. `Some` while the user is holding
    /// the mouse button down on the handle.
    pub(crate) fif_panel_drag: Option<FifPanelDrag>,
    /// `View ▸ Folder as Workspace` panel runtime state — directory cache,
    /// last-clicked selection, right-click menu, splitter drag. Persistent
    /// settings (visible flag, width, roots, expanded set, show-hidden) live
    /// in `Settings`; this struct just holds the volatile bits.
    pub(crate) folder_panel: FolderPanelState,
    /// Folder-panel inline new-file / new-folder draft. `Some` while the
    /// user is typing a name into the extra tree row; `None` otherwise.
    pub(crate) folder_inline_create: Option<crate::find_state::FolderInlineCreateState>,
    /// Focus held by the folder-panel inline-create input. Lazily created on
    /// first open.
    pub(crate) folder_inline_create_focus: Option<FocusHandle>,
    /// Auto-Completion popup state — `Some` while the candidate list is on
    /// screen. NPP `Edit ▸ Auto-Completion ▸ {Word, Function, ...}`. Cleared
    /// on Esc, outside-click, accept, or any text-buffer mutation that
    /// invalidates the originating prefix.
    pub(crate) autocomplete: Option<autocomplete::AutocompleteState>,
    /// Per-tab CSV preview model. Built lazily the first time a `.csv` /
    /// `.tsv` tab enters preview mode and re-parsed only when the
    /// buffer's edit generation has changed. See
    /// `ui::csv_doc::actions::ensure_model`.
    pub(crate) csv_models: std::collections::HashMap<Uuid, crate::ui::csv_doc::CsvModel>,
    /// Per-tab subscription that fires when the grid focus handle blurs.
    /// Drives "clicking outside the grid (toolbar, tab bar, another app)
    /// clears selection" behavior — the listener routes through
    /// `csv_doc::actions::clear_grid_selection_and_edit` after guarding
    /// against the grid → own cell editor transition.
    pub(crate) csv_grid_blur_subscriptions: std::collections::HashMap<Uuid, Subscription>,
    /// Active right-click context menu for the CSV grid. `Some` mounts a
    /// deferred anchored panel at `state.position`; `None` hides it.
    /// Opens via `show_csv_context_menu` and is dismissed by any outside
    /// click or by invoking a menu item.
    pub(crate) csv_context_menu: Option<crate::ui::csv_doc::CsvContextMenuState>,
    /// Anchor position for the CSV delimiter dropdown rendered by the
    /// format bar. `Some` mounts the deferred dropdown; `None` hides it.
    pub(crate) csv_delimiter_menu: Option<Point<Pixels>>,
    /// In-window menu bar, created on first render. Stays `None` on macOS,
    /// where the menus are native.
    pub(crate) menu_bar: Option<gpui::Entity<crate::ui::menu_bar::MenuBar>>,
}

/// Snapshot of the clipboard's plain + formatted flavors captured at the
/// moment Paste fires. Held on the workspace while the chooser modal is up
/// so the user's button click routes through the right paste path without a
/// second NSPasteboard read (which would race anything that re-wrote the
/// clipboard between the modal opening and the click).
pub(crate) struct PasteFormatPending {
    /// `public.utf8-plain-text` payload — used by the "Paste as Plain Text"
    /// button.
    pub(crate) plain: String,
    /// Pre-decoded rich-doc blocks (HTML → AST conversion already done) —
    /// used by the "Keep Formatting" button.
    pub(crate) blocks: Vec<crate::rich_doc::ast::Block>,
}

/// Resize-handle drag state for the preview pane splitter. Captured on
/// mouse-down and consumed by the window-global `MouseMoveEvent` handler
/// to compute the new pane height as `start_height + (start_mouse_y -
/// event.position.y)`.
#[derive(Clone, Copy)]
pub(crate) struct PreviewPaneDrag {
    pub(crate) start_mouse_y: Pixels,
    pub(crate) start_height: Pixels,
}

/// Default preview-pane height in pixels (matches the v1 fixed value).
pub(crate) const PREVIEW_PANE_DEFAULT_HEIGHT: f32 = 240.0;
/// Lower bound for the preview pane drag.
pub(crate) const PREVIEW_PANE_MIN_HEIGHT: f32 = 80.0;
/// Upper bound for the preview pane, expressed as a fraction of viewport
/// height. 70% leaves enough editor area to remain useful.
pub(crate) const PREVIEW_PANE_MAX_FRACTION: f32 = 0.70;
/// Pixel height of the splitter handle between editor and preview.
pub(crate) const PREVIEW_PANE_HANDLE_HEIGHT: f32 = 6.0;

/// Resize-handle drag state for the Find-in-Files results panel splitter.
/// Same mechanics as `PreviewPaneDrag` — captured on mouse-down, consumed
/// by the window-global `MouseMoveEvent` handler.
#[derive(Clone, Copy)]
pub(crate) struct FifPanelDrag {
    pub(crate) start_mouse_y: Pixels,
    pub(crate) start_height: Pixels,
}

pub(crate) const FIF_PANEL_DEFAULT_HEIGHT: f32 = 280.0;
pub(crate) const FIF_PANEL_MIN_HEIGHT: f32 = 100.0;
pub(crate) const FIF_PANEL_MAX_FRACTION: f32 = 0.75;
/// Taller than the preview-pane handle on purpose — the Find-in-Files
/// results panel docks at the very bottom of the editor where the user's
/// mouse tends to hover off the splitter quickly. A 10-px hitbox gives a
/// forgiving target without visually pushing the editor up by much.
pub(crate) const FIF_PANEL_HANDLE_HEIGHT: f32 = 10.0;

/// Bottom-pane state for the Text Tools live preview. Plain text — no
/// caret, no selection, no syntax. Transformed output is regenerated by
/// `TextToolsDialog::recompute_preview` whenever the active tab content,
/// any aux tab, or any modal field changes.
pub(crate) struct PreviewPaneState {
    /// Header label, e.g. `"Add Prefix/Suffix preview"`.
    pub(crate) title: String,
    /// Current transformed text. Empty after `clear_preview` (which
    /// keeps the pane mounted but blanks the body).
    pub(crate) content: String,
    /// Vertical scroll offset in lines. Adjusted by mouse-wheel events
    /// inside the panel.
    pub(crate) scroll_top_lines: usize,
}

/// State backing the tab right-click context menu. The menu is built fresh
/// each frame from this state so the workspace just toggles fields here
/// instead of holding entity handles.
#[derive(Clone, Copy)]
pub(crate) struct TabContextMenuState {
    pub(crate) target_tab_idx: usize,
    pub(crate) position: Point<Pixels>,
    /// Currently-expanded submenu, if any.
    pub(crate) open_submenu: Option<TabSubmenu>,
    /// Anchor point for the active submenu — top-right of the parent row,
    /// used so the cascade lands flush against its parent. Window-space
    /// pixels.
    pub(crate) submenu_anchor: Point<Pixels>,
}

/// Identifies which submenu in the tab right-click menu is open. Each value
/// drives both the cascading panel that draws to the right of the parent
/// row AND the row's hover-highlight (so the parent stays lit while the
/// child is open).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TabSubmenu {
    CloseMultiple,
    OpenInto,
    CopyToClipboard,
    MoveDocument,
    ApplyColor,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum DragMode {
    #[default]
    Char,
    Word,
    Line,
}

/// Snapshot of an in-flight input drag. We hang on to the field's hit-test
/// origin (the input's left-edge x-coordinate at click time) plus its
/// `InputFocus` so the window-level mouse listener can re-shape the line and
/// translate the new pointer x into a byte offset on subsequent moves
/// without needing the original element bounds again.
#[derive(Clone, Copy)]
pub(crate) struct InputDrag {
    pub(crate) focus: InputFocus,
    pub(crate) bounds_left: Pixels,
}

/// Two-phase lifecycle for selected-text drag-and-drop. `Pending` is the
/// pre-threshold state: the user mouse-down'd inside an existing selection
/// but hasn't moved far enough to commit to a drag. `Active` means the
/// pointer has crossed [`TEXT_DRAG_THRESHOLD_PX`] and the drag is in flight
/// — `drop_target` updates on every move and the drop commits on mouse-up.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextDragPhase {
    Pending,
    Active,
}

/// Selected-text drag-and-drop state. Snapshots the source range + text
/// payload at mouse-down so concurrent buffer edits don't shift the source
/// underneath the drag; the drop target is sampled live from each
/// mouse-move. `copy_mode` is read off the Cmd modifier at drop time, not
/// at start — matches NPP / macOS Finder where the modifier latch is
/// evaluated when the mouse releases.
#[derive(Clone)]
pub(crate) struct TextDragState {
    pub(crate) phase: TextDragPhase,
    pub(crate) source_range: std::ops::Range<usize>,
    pub(crate) source_text: String,
    pub(crate) start_position: gpui::Point<Pixels>,
    pub(crate) drop_target: usize,
}

/// Pixel-distance threshold a press-and-hold has to clear before promoting
/// a pending text-drag to an active one. Below this, the mouse-up is treated
/// as a plain click (collapses selection to caret). Matches the standard OS
/// click-vs-drag heuristic.
pub(crate) const TEXT_DRAG_THRESHOLD_PX: f32 = 4.0;

/// One row of the per-tab word-wrap layout. A logical buffer line that
/// wraps into N visual rows produces N entries, all sharing the same
/// `logical_line`. Byte offsets are relative to the buffer (not the line).
#[derive(Clone, Copy, Debug)]
pub(crate) struct WrapRow {
    pub(crate) logical_line: u32,
    /// Byte offset of the first char on this visual row (buffer-relative).
    pub(crate) byte_start: u32,
    /// Byte offset of the first char on the *next* visual row, or the
    /// printable end of the logical line for the last row of a line.
    /// Half-open `[byte_start, byte_end)` matches what the existing
    /// per-line shaped layout expects.
    pub(crate) byte_end: u32,
}

/// Cached visual-row layout for the whole active buffer at the current
/// viewport width and font metrics. Rebuilt when any input changes —
/// invalidation is conservative (drop the whole cache and re-fill on the
/// next paint) because the buffer-wide vec is cheap relative to per-edit
/// reshaping.
pub(crate) struct WrapCache {
    /// Width the cache was built for. If the editor pane resizes, the
    /// cache is dropped and rebuilt on the next paint.
    pub(crate) viewport_width: Pixels,
    /// rem size at build time — used as a proxy for "font size changed"
    /// (zoom in/out triggers `Window::set_rem_size`, so this catches it).
    pub(crate) rem_size: Pixels,
    /// Font family at build time — a theme switch can change the editor
    /// face, and with it every row's width.
    pub(crate) font_family: gpui::SharedString,
    /// Total visual-row count. Equals `rows.len()` but stashed for
    /// convenience so `max_scroll_top` doesn't have to .len() each call.
    pub(crate) total_rows: u32,
    /// Buffer-byte length the cache was built against. Cheap edit-detection
    /// signal — any apply_edit invalidates anyway, but this is a backup
    /// that `EditorElement::prepaint` checks.
    pub(crate) byte_len: u32,
    pub(crate) rows: Vec<WrapRow>,
}

/// Cached metrics from the last editor paint. All values are in window-space
/// pixels. Zero-initialized until the first paint completes.
#[derive(Clone, Copy)]
pub(crate) struct Viewport {
    /// Editor-pane content bounds (excludes gutter). Zero-sized until first paint.
    pub(crate) bounds: Bounds<Pixels>,
    /// One-logical-line height — same value `Window::line_height` returned
    /// during the last paint.
    pub(crate) line_height: Pixels,
    /// Total number of logical lines in the active buffer at last paint.
    pub(crate) total_lines: usize,
    /// Pixel width of the longest rendered line (for h-scrollbar extent).
    pub(crate) max_line_width: Pixels,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            bounds: Bounds::new(point(px(0.), px(0.)), size(px(0.), px(0.))),
            line_height: px(0.),
            total_lines: 0,
            max_line_width: px(0.),
        }
    }
}

impl Workspace {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        focus_handle: FocusHandle,
        find_focus: FocusHandle,
        replace_focus: FocusHandle,
        goto_focus: FocusHandle,
        fif_query_focus: FocusHandle,
        fif_filter_focus: FocusHandle,
        zoom_level: f32,
        system_appearance: WindowAppearance,
    ) -> Self {
        let mut welcome = Tab::welcome();
        // First tab in a fresh window is always "new 1".
        welcome.buffer.new_label = Some(crate::buffer::next_new_label_for(std::iter::empty()));
        let welcome_id = welcome.session_id;
        Self {
            tabs: vec![welcome],
            active: 0,
            closed_tabs: Vec::new(),
            tab_mru: VecDeque::from([welcome_id]),
            theme_mode: ThemeMode::Dark,
            focus_handle,
            find_focus,
            replace_focus,
            goto_focus,
            fif_query_focus,
            fif_filter_focus,
            fif_replacement_focus: None,
            fif_directory_focus: None,
            mark_focus: None,
            last_lines: Vec::new(),
            caret_visible: true,
            overtype: false,
            bulk_edit: false,
            status_message: None,
            find_state: FindState::new(),
            goto_state: GotoState::new(),
            find_in_files: FindInFilesState::new(),
            mark_state: MarkState::new(),
            dialog_tab: DialogTab::Find,
            find_dialog_window: None,
            find_history_open: None,
            word_wrap: false,
            show_whitespace: false,
            viewport: Viewport::default(),
            v_scroll_drag: None,
            h_scroll_drag: None,
            md_preview_scroll_handle: ScrollHandle::new(),
            md_preview_scroll_drag: None,
            rich_doc_scroll_handle: ScrollHandle::new(),
            rich_doc_scroll_drag: None,
            preview_block_targets: Vec::new(),
            preview_caret_scroll_pending: false,
            preview_last_drag_pos: None,
            preview_autoscroll_task: None,
            rich_doc_last_drag_pos: None,
            rich_doc_autoscroll_task: None,
            zoom_level: clamp_zoom(zoom_level),
            context_menu: None,
            autosave_generation: 0,
            post_edit_generation: 0,
            input_drag: None,
            text_drag: None,
            drag_mode: DragMode::Char,
            drag_anchor_range: None,
            macro_prompt: None,
            macro_focus: None,
            last_window_title: None,
            tab_context_menu: None,
            tab_rename: None,
            tab_rename_focus: None,
            rich_doc_link_bar: None,
            rich_doc_link_focus: None,
            rich_doc_color_bar: None,
            rich_doc_color_focus: None,
            rich_doc_body_focus: None,
            about_dialog_open: false,
            paste_format_pending: None,
            system_appearance,
            smart_match_ranges: Vec::new(),
            tag_match_pair: None,
            tag_match_attrs: Vec::new(),
            tag_match_zone: None,
            smart_match_word: None,
            preview_pane: None,
            text_tools_aux_tabs: Vec::new(),
            preview_pane_height: px(PREVIEW_PANE_DEFAULT_HEIGHT),
            preview_pane_drag: None,
            fif_panel_height: px(FIF_PANEL_DEFAULT_HEIGHT),
            fif_panel_drag: None,
            folder_panel: FolderPanelState::new(),
            folder_inline_create: None,
            folder_inline_create_focus: None,
            autocomplete: None,
            csv_models: std::collections::HashMap::new(),
            csv_grid_blur_subscriptions: std::collections::HashMap::new(),
            csv_context_menu: None,
            csv_delimiter_menu: None,
            menu_bar: None,
        }
    }

    pub(crate) fn show_csv_delimiter_menu(
        &mut self,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.csv_delimiter_menu = Some(pos);
        cx.notify();
    }

    pub(crate) fn hide_csv_delimiter_menu(&mut self, cx: &mut Context<Self>) {
        if self.csv_delimiter_menu.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn show_csv_context_menu(
        &mut self,
        state: crate::ui::csv_doc::CsvContextMenuState,
        cx: &mut Context<Self>,
    ) {
        self.csv_context_menu = Some(state);
        cx.notify();
    }

    pub(crate) fn hide_csv_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.csv_context_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Apply a new preview-pane height with clamping. `viewport_h` is the
    /// current window viewport height; the upper bound is
    /// `viewport_h * PREVIEW_PANE_MAX_FRACTION` (subject to a small
    /// floor so the pane stays usable on tiny windows).
    pub(crate) fn set_preview_pane_height(
        &mut self,
        height: Pixels,
        viewport_h: Pixels,
        cx: &mut Context<Self>,
    ) {
        let max_floor = px(PREVIEW_PANE_MIN_HEIGHT + 60.);
        let max = (viewport_h * PREVIEW_PANE_MAX_FRACTION).max(max_floor);
        let clamped = height.clamp(px(PREVIEW_PANE_MIN_HEIGHT), max);
        if clamped != self.preview_pane_height {
            self.preview_pane_height = clamped;
            cx.notify();
        }
    }

    pub(crate) fn set_fif_panel_height(
        &mut self,
        height: Pixels,
        viewport_h: Pixels,
        cx: &mut Context<Self>,
    ) {
        let max_floor = px(FIF_PANEL_MIN_HEIGHT + 60.);
        let max = (viewport_h * FIF_PANEL_MAX_FRACTION).max(max_floor);
        let clamped = height.clamp(px(FIF_PANEL_MIN_HEIGHT), max);
        if clamped != self.fif_panel_height {
            self.fif_panel_height = clamped;
            cx.notify();
        }
    }

    #[allow(dead_code)]
    pub(crate) fn open_preview_pane(&mut self, title: impl Into<String>, cx: &mut Context<Self>) {
        self.preview_pane = Some(PreviewPaneState {
            title: title.into(),
            content: String::new(),
            scroll_top_lines: 0,
        });
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn set_preview_content(&mut self, text: String, cx: &mut Context<Self>) {
        if let Some(state) = self.preview_pane.as_mut() {
            if state.content == text {
                return;
            }
            state.content = text;
            state.scroll_top_lines = 0;
            cx.notify();
        }
    }

    #[allow(dead_code)]
    pub(crate) fn clear_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.preview_pane.as_mut() {
            state.content.clear();
            state.scroll_top_lines = 0;
            cx.notify();
        }
    }

    pub(crate) fn close_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview_pane.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn preview_scroll_by_lines(&mut self, delta: i32, cx: &mut Context<Self>) {
        if let Some(state) = self.preview_pane.as_mut() {
            let total_lines = state.content.lines().count();
            let new_top = (state.scroll_top_lines as i32 + delta).max(0) as usize;
            state.scroll_top_lines = new_top.min(total_lines.saturating_sub(1));
            cx.notify();
        }
    }

    /// Read-only fetch of the rope content of an arbitrary tab as a
    /// `String`. Used by Text Tools to grab aux tab content for
    /// multi-source transforms (Lists Comparison list B, Join/Merge
    /// columns 2..N). Returns `None` if `idx` is out of bounds.
    #[allow(dead_code)]
    pub(crate) fn tab_text(&self, idx: usize) -> Option<String> {
        self.tabs.get(idx).map(|t| t.buffer.rope.to_string())
    }

    /// Open `count` new untitled tabs labeled `[Aux N]`, append their
    /// indices to `text_tools_aux_tabs`, and leave the previously-active
    /// tab focused (the dialog manages the active tab manually). Returns
    /// the new indices in creation order.
    ///
    /// Falls back to regular tab-strip tabs because Notepad Studio does not yet
    /// have a side-by-side split-pane layout. Documented limitation in
    /// `UNTESTED.md`.
    #[allow(dead_code)]
    pub(crate) fn open_text_tools_aux_tabs(
        &mut self,
        count: usize,
        cx: &mut Context<Self>,
    ) -> Vec<usize> {
        use crate::tab::Tab;
        let mut indices = Vec::with_capacity(count);
        let primary = self.active;
        for i in 0..count {
            let mut tab = Tab::welcome();
            tab.buffer.new_label = Some(gpui::SharedString::from(format!("[Aux {}]", i + 1)));
            tab.buffer.dirty = false;
            let new_idx = self.tabs.len();
            self.tabs.push(tab);
            self.text_tools_aux_tabs.push(new_idx);
            indices.push(new_idx);
        }
        // Keep the user's primary tab active.
        self.active = primary;
        cx.notify();
        indices
    }

    /// Close every aux tab opened by the Text Tools modal. Indices are
    /// processed high-to-low so the lower indices stay valid as we
    /// remove higher ones. Active index is fixed up so it points at the
    /// surviving primary tab.
    #[allow(dead_code)]
    pub(crate) fn close_text_tools_aux_tabs(&mut self, cx: &mut Context<Self>) {
        if self.text_tools_aux_tabs.is_empty() {
            return;
        }
        let mut idxs = std::mem::take(&mut self.text_tools_aux_tabs);
        idxs.sort_unstable();
        idxs.dedup();
        for idx in idxs.into_iter().rev() {
            if idx < self.tabs.len() {
                self.tabs.remove(idx);
                if self.active == idx {
                    self.active = idx.saturating_sub(1).min(self.tabs.len().saturating_sub(1));
                } else if self.active > idx {
                    self.active -= 1;
                }
            }
        }
        if self.tabs.is_empty() {
            // Keep the workspace usable — open a fresh blank tab.
            self.tabs.push(crate::tab::Tab::welcome());
            self.active = 0;
        }
        cx.notify();
    }

    pub(crate) fn show_about_dialog(&mut self, cx: &mut Context<Self>) {
        if !self.about_dialog_open {
            self.about_dialog_open = true;
            cx.notify();
        }
    }

    pub(crate) fn hide_about_dialog(&mut self, cx: &mut Context<Self>) {
        if self.about_dialog_open {
            self.about_dialog_open = false;
            cx.notify();
        }
    }

    /// Pop the paste-format chooser modal pre-loaded with the decoded
    /// clipboard payload. Called from the Edit ▸ Paste path when the
    /// clipboard advertises HTML/RTF and the cursor is in the rich-doc body
    /// (the only surface where "keep formatting" is meaningful).
    pub(crate) fn show_paste_format_dialog(
        &mut self,
        pending: PasteFormatPending,
        cx: &mut Context<Self>,
    ) {
        self.paste_format_pending = Some(pending);
        cx.notify();
    }

    /// Dismiss the paste-format modal without performing a paste. Used by
    /// the Cancel button, Esc, and click-outside.
    pub(crate) fn hide_paste_format_dialog(&mut self, cx: &mut Context<Self>) {
        if self.paste_format_pending.is_some() {
            self.paste_format_pending = None;
            cx.notify();
        }
    }

    /// User picked "Keep Formatting" — splice the pre-decoded blocks at the
    /// caret via `rich_doc_paste_formatted`, then close the modal.
    pub(crate) fn confirm_paste_format_formatted(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.paste_format_pending.take() else {
            return;
        };
        self.rich_doc_paste_formatted(pending.blocks, cx);
        cx.notify();
    }

    /// User picked "Paste as Plain Text" — drop the formatted blocks and use
    /// the plain text path. Same routing as a paste from a plain clipboard.
    pub(crate) fn confirm_paste_format_plain(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.paste_format_pending.take() else {
            return;
        };
        self.rich_doc_paste(&pending.plain, cx);
        cx.notify();
    }

    pub(crate) fn show_tab_context_menu(
        &mut self,
        tab_idx: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.tab_context_menu = Some(TabContextMenuState {
            target_tab_idx: tab_idx,
            position,
            open_submenu: None,
            submenu_anchor: position,
        });
        cx.notify();
    }

    pub(crate) fn hide_tab_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.tab_context_menu.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn open_tab_submenu(
        &mut self,
        which: TabSubmenu,
        anchor: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = self.tab_context_menu.as_mut() {
            state.open_submenu = Some(which);
            state.submenu_anchor = anchor;
            cx.notify();
        }
    }

    pub(crate) fn close_tab_submenu(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.tab_context_menu.as_mut() {
            if state.open_submenu.take().is_some() {
                cx.notify();
            }
        }
    }

    /// Open the inline rename bar for tab `idx`. Initial text is the
    /// basename of the file (or the new label for untitled — caller should
    /// disable the menu entry for those).
    pub(crate) fn open_tab_rename(
        &mut self,
        idx: usize,
        cx: &mut Context<Self>,
    ) {
        if idx >= self.tabs.len() {
            return;
        }
        let initial = self.tabs[idx]
            .buffer
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if self.tab_rename_focus.is_none() {
            self.tab_rename_focus = Some(cx.focus_handle());
        }
        self.tab_rename = Some(TabRenameState::new(idx, &initial));
        cx.notify();
    }

    pub(crate) fn close_tab_rename(&mut self, cx: &mut Context<Self>) {
        if self.tab_rename.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn commit_tab_rename(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.tab_rename.take() else {
            return;
        };
        let idx = state.target_tab_idx;
        let new_name = state.text;
        self.rename_tab_file(idx, new_name, cx);
    }

    pub(crate) fn show_context_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.context_menu = Some(position);
        cx.notify();
    }

    pub(crate) fn hide_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Snapshot the entire workspace into the session manifest and write it
    /// atomically to `<config>/Notepad Studio/session.toml`. Cheap enough to call
    /// after every tab open / close / save / theme toggle — the dirty-tab
    /// `.bak` files only get rewritten when their generation actually changed.
    pub(crate) fn save_session(&self) {
        let mru: Vec<uuid::Uuid> = self.tab_mru.iter().copied().collect();
        session::save_workspace(
            &self.tabs,
            self.active,
            &self.closed_tabs,
            self.theme_mode,
            self.show_whitespace,
            self.word_wrap,
            &mru,
        );
    }

    /// Eager save for the per-keystroke path. Writes the manifest immediately
    /// but only `.bak`s untitled buffers — path-backed dirty `.bak`s stay on
    /// the 250ms debounced full save in `schedule_autosave`. Manifest entries
    /// for path-backed dirty buffers carry `dirty = false` so restore re-reads
    /// from disk if the user closes before the debounced full save fires.
    /// Trade-off: at most 250ms of typed-but-unsaved keystrokes are lost on a
    /// close-too-fast for path-backed buffers; untitled scratch pads round-
    /// trip in full because their `.bak` is written every keystroke.
    pub(crate) fn save_session_eager(&self) {
        let mru: Vec<uuid::Uuid> = self.tab_mru.iter().copied().collect();
        session::save_workspace_with_policy(
            &self.tabs,
            self.active,
            &self.closed_tabs,
            self.theme_mode,
            self.show_whitespace,
            self.word_wrap,
            &mru,
            session::BakPolicy::UntitledOnly,
        );
    }

    /// Schedule a debounced autosave of every dirty / untitled buffer AND a
    /// fresh manifest snapshot. Each call bumps `autosave_generation`; the
    /// spawned task waits 250ms and only writes if its captured generation
    /// is still current — keeps a fast typist from triggering more disk
    /// writes than they need. Writing the manifest here (not just `.bak`
    /// files) is what lets a quit-without-save still produce a restorable
    /// session: without it, only tabs that have been through
    /// `File ▸ Save / Open` since launch survive the next boot.
    ///
    /// Debounce is short (250ms) because the `on_window_should_close` save
    /// sometimes fails to deliver on macOS red-⏺ close — likely a
    /// re-entrancy quirk in gpui's AsyncWindowContext during the platform
    /// `windowShouldClose:` callback. A short debounce shrinks the
    /// close-too-fast window where typed-but-unsaved content can be lost.
    pub(crate) fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        self.autosave_generation = self.autosave_generation.wrapping_add(1);
        let gen = self.autosave_generation;
        cx.spawn(async move |this, cx| {
            gpui::Timer::after(Duration::from_millis(250)).await;
            let _ = this.update(cx, |w, _cx| {
                if w.autosave_generation == gen {
                    w.save_session();
                }
            });
        })
        .detach();
    }

    /// Debounced post-edit settle: runs the heavyweight syntax reparse and the
    /// eager untitled-buffer persistence ~100ms after the last edit, OFF the
    /// synchronous edit-commit path. Mirrors `schedule_autosave`'s generation
    /// guard so a burst of edits (Replace All, fast typing, undo of a big
    /// group) reparses + persists once after things settle instead of once
    /// per edit. Keeping the full reparse + `serialize_undo_stack` + disk
    /// writes out of the synchronous path is what stops a large Replace All /
    /// undo from blocking the UI thread (the loading spinner). Highlights lag
    /// ~100ms after a batch — the same async-highlight tradeoff Zed makes.
    pub(crate) fn schedule_reparse(&mut self, cx: &mut Context<Self>) {
        self.post_edit_generation = self.post_edit_generation.wrapping_add(1);
        let gen = self.post_edit_generation;
        cx.spawn(async move |this, cx| {
            gpui::Timer::after(Duration::from_millis(100)).await;
            let _ = this.update(cx, |w, cx| {
                if w.post_edit_generation != gen {
                    return;
                }
                w.active_tab_mut().buffer.reparse_highlights();
                w.save_session_eager();
                cx.notify();
            });
        })
        .detach();
    }

    /// Replace the workspace's tabs / closed-tab stack / theme / active
    /// index with the contents of `manifest`. Skipped entries (missing path,
    /// corrupt `.bak`) are reported through the status bar as a single
    /// summary message. Called once during window construction.
    pub(crate) fn apply_session(&mut self, manifest: SessionManifest) {
        let mut restored: Vec<Tab> = Vec::with_capacity(manifest.tabs.len());
        let mut skipped = 0usize;
        for entry in &manifest.tabs {
            match session::restore_tab(entry) {
                RestoreOutcome::Restored(tab) => restored.push(tab),
                RestoreOutcome::Skipped(reason) => {
                    eprintln!("[session] skipped tab: {reason}");
                    skipped += 1;
                }
            }
        }

        if restored.is_empty() {
            // Nothing recovered — keep the welcome tab the constructor seeded.
            return;
        }

        self.tabs = restored;
        self.active = manifest.active.min(self.tabs.len().saturating_sub(1));

        self.closed_tabs.clear();
        for entry in &manifest.closed_tabs {
            if let Some(closed) = session::restore_closed_tab(entry) {
                self.closed_tabs.push(closed);
            }
        }

        if let Some(theme) = manifest.theme {
            self.theme_mode = theme;
        }
        self.show_whitespace = manifest.show_whitespace;
        self.word_wrap = manifest.word_wrap;

        if skipped > 0 {
            self.set_status(format!(
                "Restored {} tab(s); {} couldn't be reopened",
                self.tabs.len(),
                skipped
            ));
        } else {
            self.set_status(format!("Restored {} tab(s)", self.tabs.len()));
        }

        // Rehydrate the document-switcher MRU from the manifest. Drop any
        // ids that didn't make it into `self.tabs` (closed-since-save or
        // restore-failed tabs); `touch_mru` further normalises by pushing
        // the active tab to the front. With an empty manifest MRU (legacy
        // sessions or fresh installs), `touch_mru` alone produces a one-
        // entry MRU which `cycle_tab_mru` then back-fills lazily from
        // tab-strip order on first use.
        let live: std::collections::HashSet<uuid::Uuid> =
            self.tabs.iter().map(|t| t.session_id).collect();
        self.tab_mru = manifest
            .mru
            .iter()
            .copied()
            .filter(|id| live.contains(id))
            .collect();

        // NPP `New Document ▸ Always open a new document in addition at startup`.
        // Append a fresh blank tab and make it active so users land on a
        // scratch buffer regardless of session contents.
        if crate::settings::snapshot().startup_open_blank {
            let label = crate::buffer::next_new_label_for(
                self.tabs.iter().map(|t| t.buffer.new_label.as_ref()),
            );
            let mut blank = crate::tab::Tab::empty();
            blank.buffer.new_label = Some(label);
            self.tabs.push(blank);
            self.active = self.tabs.len() - 1;
        }
        self.touch_mru();
    }

    /// Create the dialog-specific focus handles on first use. Called from
    /// `open_or_focus_find_dialog` and from any setter that needs to focus
    /// one of the lazy inputs.
    pub(crate) fn ensure_dialog_focus_handles(&mut self, cx: &mut gpui::App) {
        if self.fif_replacement_focus.is_none() {
            self.fif_replacement_focus = Some(cx.focus_handle());
        }
        if self.fif_directory_focus.is_none() {
            self.fif_directory_focus = Some(cx.focus_handle());
        }
        if self.mark_focus.is_none() {
            self.mark_focus = Some(cx.focus_handle());
        }
    }

    /// Clamp `target` into the allowed zoom range, write it to the workspace,
    /// push the new rem size into the window, and mirror to the persisted
    /// config snapshot so it survives the next launch.
    pub(crate) fn set_zoom(
        &mut self,
        target: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clamped = clamp_zoom(target);
        if (clamped - self.zoom_level).abs() < f32::EPSILON {
            return;
        }
        self.zoom_level = clamped;
        window.set_rem_size(px(ZOOM_REM_BASE * clamped));
        record_zoom_level(window, clamped);
        cx.notify();
    }

    pub(crate) fn zoom_in(
        &mut self,
        _: &crate::actions::ViewZoomIn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_zoom(self.zoom_level * ZOOM_STEP, window, cx);
    }

    pub(crate) fn zoom_out(
        &mut self,
        _: &crate::actions::ViewZoomOut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_zoom(self.zoom_level / ZOOM_STEP, window, cx);
    }

    pub(crate) fn zoom_reset(
        &mut self,
        _: &crate::actions::ViewZoomReset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_zoom(ZOOM_DEFAULT, window, cx);
    }

    pub(crate) fn scroll_top(&self) -> Pixels {
        self.tabs[self.active].scroll_top
    }

    pub(crate) fn scroll_left(&self) -> Pixels {
        self.tabs[self.active].scroll_left
    }

    /// Upper bound on `scroll_top` for the active tab — total content height
    /// minus the viewport, plus one extra line of bottom padding so a user
    /// can scroll the last line a hair past the top edge (matches every
    /// modern editor).
    pub(crate) fn max_scroll_top(&self) -> Pixels {
        let total_rows = self.total_visual_rows();
        let content = self.viewport.line_height * total_rows as f32;
        // NPP `Editing 1 ▸ Enable scrolling beyond last line`: when on, pad
        // by half the viewport so the last line can sit near the top edge.
        // Default keeps the prior single-line breathing-room.
        let extra = if crate::settings::snapshot().scroll_beyond_last_line {
            self.viewport.bounds.size.height * 0.5
        } else {
            self.viewport.line_height
        };
        let max = content + extra - self.viewport.bounds.size.height;
        if max <= px(0.) { px(0.) } else { max }
    }

    /// Total visual-row count for the active tab — matches logical-line
    /// count when wrap is off, else uses the cached wrap layout. Returns 1
    /// (not 0) when the cache hasn't been built yet so scroll math doesn't
    /// divide by zero on the first frame after wrap is toggled on.
    pub(crate) fn total_visual_rows(&self) -> usize {
        if self.word_wrap {
            if let Some(cache) = self.tabs[self.active].wrap_cache.as_ref() {
                return cache.total_rows.max(1) as usize;
            }
        }
        self.viewport.total_lines.max(1)
    }

    pub(crate) fn max_scroll_left(&self) -> Pixels {
        if self.word_wrap {
            return px(0.);
        }
        let widest = self.viewport.max_line_width;
        let extra = px(20.);
        let max = widest + extra - self.viewport.bounds.size.width;
        if max <= px(0.) { px(0.) } else { max }
    }

    pub(crate) fn set_scroll_top(&mut self, value: Pixels, cx: &mut Context<Self>) {
        let max = self.max_scroll_top();
        let clamped = value.clamp(px(0.), max);
        if self.tabs[self.active].scroll_top != clamped {
            self.tabs[self.active].scroll_top = clamped;
            cx.notify();
        }
    }

    pub(crate) fn set_scroll_left(&mut self, value: Pixels, cx: &mut Context<Self>) {
        let max = self.max_scroll_left();
        let clamped = value.clamp(px(0.), max);
        if self.tabs[self.active].scroll_left != clamped {
            self.tabs[self.active].scroll_left = clamped;
            cx.notify();
        }
    }

    /// Ensure the cursor's logical line sits inside the viewport. Snaps —
    /// no animation. Called after every cursor-mutating action so the user
    /// never types into a hidden region.
    pub(crate) fn ensure_cursor_visible(&mut self, cx: &mut Context<Self>) {
        if self.is_in_markdown_preview() {
            // Preview mode has no source-mode viewport — each block is its
            // own laid-out `TextLayout`. Defer the actual scroll to the
            // next `CaretOverlay::paint`, which has the post-paint layout
            // and can compute the caret's window-space pixel y.
            self.preview_caret_scroll_pending = true;
            cx.notify();
            return;
        }
        if self.viewport.bounds.size.height <= px(0.) || self.viewport.line_height <= px(0.) {
            return;
        }
        let head = self.selection().head.min(self.buffer().rope.len_bytes());
        // When wrap is on, scroll_top is measured in visual rows (each
        // wrapped line takes more than one row of vertical space). Look up
        // the visual-row index for `head` via the cache; if the cache isn't
        // built yet (first frame after wrap toggled on), fall back to
        // logical-line index — close enough for one frame.
        let row_idx = if self.word_wrap {
            self.visual_row_for_byte(head)
                .unwrap_or_else(|| self.buffer().rope.byte_to_line(head))
        } else {
            self.buffer().rope.byte_to_line(head)
        };
        let line_top = self.viewport.line_height * row_idx as f32;
        let line_bottom = line_top + self.viewport.line_height;
        let cur = self.scroll_top();
        let view_h = self.viewport.bounds.size.height;
        let new_top = if line_top < cur {
            line_top
        } else if line_bottom > cur + view_h {
            line_bottom - view_h
        } else {
            cur
        };
        if new_top != cur {
            self.set_scroll_top(new_top, cx);
        }
    }

    /// Scroll the rich-doc pane so the block at `block_idx` is fully on-screen
    /// with a 24px margin between block edge and viewport edge. No-op when the
    /// block is already inside the visible region with margin clearance, or
    /// when the block's painted bounds aren't tracked yet (first Find press
    /// before the pane has painted once — the next press will succeed).
    ///
    /// Margin matches the `edge_zone` used by `rich_doc_edge_auto_scroll`
    /// (`src/ui/rich_doc_view.rs` per-block move handler) so the visual
    /// "leeway" feels consistent whether the user is dragging or jumping
    /// via Find Next / Prev.
    pub(crate) fn rich_doc_scroll_match_into_view(
        &mut self,
        block_idx: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(block_bounds) = self.rich_doc_scroll_handle.bounds_for_item(block_idx) else {
            return;
        };
        let viewport = self.rich_doc_scroll_handle.bounds();
        if viewport.size.height <= px(0.) {
            return;
        }
        let margin = px(24.);
        let block_top = block_bounds.top();
        let block_bottom = block_bounds.bottom();
        let vp_top = viewport.top();
        let vp_bottom = viewport.bottom();
        // GPUI scroll offset is 0 at the top of the content and grows MORE
        // NEGATIVE as the user scrolls down. To make a block move DOWN by
        // `delta` pixels you ADD `delta` to offset.y; to move it UP you
        // subtract. Same sign convention as `rich_doc_edge_auto_scroll`.
        let delta = if block_top < vp_top + margin {
            (vp_top + margin) - block_top
        } else if block_bottom > vp_bottom - margin {
            (vp_bottom - margin) - block_bottom
        } else {
            return;
        };
        let max = self.rich_doc_scroll_handle.max_offset();
        let mut off = self.rich_doc_scroll_handle.offset();
        let new_y = (off.y + delta).min(px(0.)).max(-max.height);
        if new_y != off.y {
            off.y = new_y;
            self.rich_doc_scroll_handle.set_offset(off);
            cx.notify();
        }
    }

    /// Visual-row index of `byte` inside the active tab's wrap cache.
    /// `None` when wrap is off or cache is empty. Linear scan over
    /// rows — buffers with extreme line counts could afford a binary
    /// search over `byte_start`, but keep it simple until profiling
    /// flags it.
    pub(crate) fn visual_row_for_byte(&self, byte: usize) -> Option<usize> {
        let cache = self.tabs[self.active].wrap_cache.as_ref()?;
        let byte_u = byte as u32;
        // Binary search by `byte_start`: find the last row whose start <= byte.
        let rows = &cache.rows;
        if rows.is_empty() {
            return None;
        }
        let mut lo = 0usize;
        let mut hi = rows.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if rows[mid].byte_start <= byte_u {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        // `lo` is the first row whose byte_start > byte; the preceding row
        // contains `byte`. `lo == 0` only when byte < rows[0].byte_start
        // which can happen for byte == 0 only if rows[0].byte_start > 0
        // (impossible — first row always starts at 0), so lo >= 1.
        Some(lo.saturating_sub(1))
    }

    pub(crate) fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
    }

    pub(crate) fn theme(&self) -> Theme {
        let snap = crate::settings::snapshot();
        let base = Theme::resolve(
            self.theme_mode,
            self.system_appearance,
            snap.tone_palette,
            &snap.customized_slots_array(),
        );
        // Layer Style Configurator overrides on top of the base palette so
        // bg/text/syntax colours follow the user's picked theme XML.
        // `active_theme::snapshot` returns None when no theme is picked or
        // the file can't be read — in either case the base palette wins.
        match crate::style_model::active_theme::snapshot() {
            Some(doc) => base.with_xml_overrides(&doc),
            None => base,
        }
    }

    /// Snapshot of the user's persisted preferences. Cheap — clones from
    /// the process-global cache. Future call sites for tab width, the
    /// Split-Lines edge column, and the Date/Time custom format read here
    /// instead of `crate::consts::TAB_SPACES` so the Preferences dialog
    /// drives behaviour live.
    #[allow(dead_code)]
    pub(crate) fn settings(&self) -> crate::settings::Settings {
        crate::settings::snapshot()
    }

    pub(crate) fn buffer(&self) -> &Buffer {
        &self.tabs[self.active].buffer
    }

    /// Active tab's selection, clamped to the rope and floored to char
    /// boundaries. The stored value can be stale (undo restore, session
    /// restore, plugin calls); every reader through here gets offsets ropey
    /// accepts.
    pub(crate) fn selection(&self) -> Selection {
        let tab = &self.tabs[self.active];
        tab.selection.clamped(&tab.buffer.rope)
    }

    /// Active tab's pending Begin/End Select marker as a 0-based line index,
    /// or `None` if no marker is set. Used by the gutter to paint the marker
    /// glyph so the user can see where the selection will extend from on the
    /// second press of Ctrl+Shift+B.
    pub(crate) fn begin_select_marker_line(&self) -> Option<usize> {
        let tab = &self.tabs[self.active];
        let offset = tab.begin_select_marker?;
        let total = tab.buffer.rope.len_bytes();
        Some(tab.buffer.rope.byte_to_line(offset.min(total)))
    }

    pub(crate) fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// True when the active tab is in the WYSIWYG markdown preview pane
    /// (view_mode flipped AND file is `.md` / `.markdown`). Both checks
    /// must pass because non-markdown files can be in MarkdownPreview mode
    /// transiently during a view switch.
    pub(crate) fn is_in_markdown_preview(&self) -> bool {
        let tab = &self.tabs[self.active];
        matches!(tab.view_mode, crate::tab::TabViewMode::MarkdownPreview)
            && crate::ui::markdown_view::is_markdown_path(tab.buffer.file_path.as_deref())
    }

    /// Scroll the markdown preview by `dir * (viewport_height - overlap)`.
    /// Clamped to the handle's `[max_offset.height, 0]` range so we never
    /// scroll above the top or past the bottom. Caret position is left
    /// alone — the preview doesn't yet have visual-line knowledge to
    /// move the caret to a meaningful "one page down" source byte, so
    /// PageUp/PageDown here behave like a browser (scroll only, caret
    /// stays put). Returns true if a scroll was applied.
    pub(crate) fn preview_page_scroll(
        &mut self,
        dir: i32,
        cx: &mut Context<Self>,
    ) -> bool {
        let bounds = self.md_preview_scroll_handle.bounds();
        let viewport_h = bounds.size.height;
        if viewport_h <= px(0.) {
            return false;
        }
        // One-line overlap so the user can pick up reading context. Same
        // convention as page_size_lines() (subtracts 1 line).
        let overlap = px(20.);
        let delta = (viewport_h - overlap).max(px(40.));
        let mut offset = self.md_preview_scroll_handle.offset();
        offset.y -= delta * (dir as f32);
        let max_y = self.md_preview_scroll_handle.max_offset().height;
        offset.y = offset.y.max(-max_y).min(px(0.));
        self.md_preview_scroll_handle.set_offset(offset);
        cx.notify();
        true
    }

    pub(crate) fn offset_at_position(&self, pos: Point<Pixels>) -> usize {
        let (Some(first), Some(last)) = (self.last_lines.first(), self.last_lines.last()) else {
            return 0;
        };
        // Above the first visible line — clamp to its start. That keeps
        // a drag-out-of-window selection extending toward the top of the
        // viewport rather than teleporting to byte 0 (which it would do
        // when more than one screenful is scrolled below).
        if pos.y < first.bounds.top() {
            return first.line_byte_start;
        }
        if pos.y >= last.bounds.bottom() {
            return last.line_byte_start + last.line_byte_len;
        }
        for line in &self.last_lines {
            if pos.y >= line.bounds.top() && pos.y < line.bounds.bottom() {
                let local_x = pos.x - line.bounds.left();
                let local_x = local_x.max(px(0.));
                let idx = if line.line_byte_len == 0 {
                    0
                } else {
                    line.shaped
                        .closest_index_for_x(local_x)
                        .min(line.line_byte_len)
                };
                return line.line_byte_start + idx;
            }
        }
        last.line_byte_start + last.line_byte_len
    }

    pub(crate) fn input_focus(&self, window: &Window) -> InputFocus {
        if self.find_focus.is_focused(window) {
            InputFocus::FindQuery
        } else if self.replace_focus.is_focused(window) {
            InputFocus::Replacement
        } else if self.goto_focus.is_focused(window) {
            InputFocus::Goto
        } else if self.fif_query_focus.is_focused(window) {
            InputFocus::FifQuery
        } else if self.fif_filter_focus.is_focused(window) {
            InputFocus::FifFilter
        } else if self
            .fif_replacement_focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window))
        {
            InputFocus::FifReplacement
        } else if self
            .fif_directory_focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window))
        {
            InputFocus::FifDirectory
        } else if self
            .mark_focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window))
        {
            InputFocus::MarkQuery
        } else if self
            .macro_focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window))
        {
            InputFocus::MacroPrompt
        } else if self
            .tab_rename_focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window))
        {
            InputFocus::TabRename
        } else if self
            .folder_inline_create_focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window))
        {
            InputFocus::FolderInlineCreate
        } else if self
            .rich_doc_link_focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window))
        {
            InputFocus::RichDocLinkUrl
        } else if self
            .rich_doc_color_focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window))
        {
            InputFocus::RichDocColorHex
        } else if self
            .rich_doc_body_focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window))
        {
            InputFocus::RichDocBody
        } else if {
            // Active tab's CSV cell input — owned by the per-tab CsvModel.
            let tab_id = self.tabs[self.active].session_id;
            self.csv_models
                .get(&tab_id)
                .and_then(|m| m.cell_input.as_ref())
                .is_some_and(|i| i.focus_handle.is_focused(window))
        } {
            InputFocus::CsvCell
        } else if {
            // CSV find bar query input.
            let tab_id = self.tabs[self.active].session_id;
            self.csv_models
                .get(&tab_id)
                .and_then(|m| m.find_input.as_ref())
                .is_some_and(|i| i.focus_handle.is_focused(window))
        } {
            InputFocus::CsvFindBar
        } else if {
            // CSV replace bar input — only present when find_show_replace.
            let tab_id = self.tabs[self.active].session_id;
            self.csv_models
                .get(&tab_id)
                .and_then(|m| m.replace_input.as_ref())
                .is_some_and(|i| i.focus_handle.is_focused(window))
        } {
            InputFocus::CsvReplaceBar
        } else if {
            // CSV grid container — focused when the user clicks the grid
            // background or any non-cell element. Lets workspace-level
            // actions (arrow keys / F2 / Cmd+A / Esc / typed chars) route
            // through grid-specific handlers instead of the rope.
            let tab_id = self.tabs[self.active].session_id;
            self.csv_models
                .get(&tab_id)
                .and_then(|m| m.grid_focus.as_ref())
                .is_some_and(|h| h.is_focused(window))
        } {
            InputFocus::CsvGrid
        } else {
            InputFocus::Editor
        }
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn clamp_zoom(zoom: f32) -> f32 {
    if zoom.is_nan() {
        ZOOM_DEFAULT
    } else {
        zoom.clamp(ZOOM_MIN, ZOOM_MAX)
    }
}

/// HsvHost wiring for the rich-doc colour bar's More-colours popover. All
/// HSV state lives on `rich_doc_color_bar` so it disappears with the bar;
/// when the bar is absent the host degrades to "zeros, no drag" reads and
/// no-op writes (defensive — the HSV surface is only rendered inside the
/// popover, which is only rendered when the bar exists, so writes against
/// a missing bar shouldn't happen in practice). The hex buffer is the
/// bar's `text` field; rewriting it triggers `apply_rich_doc_color_buffer`
/// for the live block-colour preview.
impl crate::ui::style_configurator::hsv_surface::HsvHost for Workspace {
    fn picker_hue(&self) -> f32 {
        self.rich_doc_color_bar
            .as_ref()
            .map(|s| s.picker_hue)
            .unwrap_or(0.0)
    }
    fn picker_sat(&self) -> f32 {
        self.rich_doc_color_bar
            .as_ref()
            .map(|s| s.picker_sat)
            .unwrap_or(0.0)
    }
    fn picker_val(&self) -> f32 {
        self.rich_doc_color_bar
            .as_ref()
            .map(|s| s.picker_val)
            .unwrap_or(0.0)
    }
    fn picker_hsv_drag(&self) -> crate::ui::style_configurator::hsv_surface::HsvDrag {
        self.rich_doc_color_bar
            .as_ref()
            .map(|s| s.picker_hsv_drag)
            .unwrap_or(crate::ui::style_configurator::hsv_surface::HsvDrag::None)
    }
    fn set_picker_hue(&mut self, h: f32) {
        if let Some(state) = self.rich_doc_color_bar.as_mut() {
            state.picker_hue = h;
        }
    }
    fn set_picker_sat(&mut self, s: f32) {
        if let Some(state) = self.rich_doc_color_bar.as_mut() {
            state.picker_sat = s;
        }
    }
    fn set_picker_val(&mut self, v: f32) {
        if let Some(state) = self.rich_doc_color_bar.as_mut() {
            state.picker_val = v;
        }
    }
    fn set_picker_hsv_drag(
        &mut self,
        drag: crate::ui::style_configurator::hsv_surface::HsvDrag,
    ) {
        if let Some(state) = self.rich_doc_color_bar.as_mut() {
            state.picker_hsv_drag = drag;
        }
    }
    fn picker_hex_text(&self) -> &str {
        self.rich_doc_color_bar
            .as_ref()
            .map(|s| s.text.as_str())
            .unwrap_or("")
    }
    fn set_picker_hex_text(&mut self, text: String) {
        if let Some(state) = self.rich_doc_color_bar.as_mut() {
            state.text_cursor = text.len();
            state.text_anchor = text.len();
            state.text = text;
        }
    }
    fn on_hsv_picker_buffer_changed(&mut self, cx: &mut Context<Self>) {
        // Live-apply through the existing path — same code that runs on
        // every typed character and quick-pick swatch click.
        self.apply_rich_doc_color_buffer(cx);
    }
}
