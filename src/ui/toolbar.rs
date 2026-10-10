// Notepad++-style icon toolbar that sits above the tab strip.
//
// Replaces the old "● Untitled / Notepad Studio vX.Y.Z" title row. Layout:
// `File | Edit | Search | View | Macro | Plugins`, each group separated by a
// 1px vertical bar.
//
// Each button dispatches an existing Workspace action method directly via
// `entity.update`, mirroring the pattern in `context_menu.rs`. Disabled
// buttons (no selection for Cut/Copy, no undo history, etc.) lower opacity
// and skip the click handler.
//
// SVG glyphs come from `src/assets.rs` (embedded via `include_bytes!`) and
// are tinted via `text_color` — gpui's svg renderer treats `text.color` as
// the fill so we get monochrome icons that follow the theme automatically.

use gpui::{
    AnyView, App, BoxShadow, Context, CursorStyle, Entity, IntoElement, MouseButton,
    ParentElement, Render, SharedString, Window, div, hsla, point, prelude::*, px, rems, rgb, svg,
};

use crate::actions::*;
use crate::shortcuts;
use crate::theme::Theme;
use crate::workspace::Workspace;

/// Hover-tooltip view rendered above each toolbar button. Holds the colors
/// captured at toolbar build time so the popup follows the active theme
/// without holding a borrow on the workspace.
struct ToolbarTooltip {
    label: SharedString,
    chord: Option<SharedString>,
    bg: u32,
    text: u32,
    border: u32,
    chord_color: u32,
}

impl Render for ToolbarTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Match Zed's `elevation_2` styling (`ui::traits::styled_ext`):
        // `rounded_lg` (8 px) + `border_1` + two-layer drop shadow. Uses
        // our own palette (`theme.toolbar_bg` / `theme.border`) instead of
        // Zed's `elevated_surface_background` so it follows the active
        // theme. Padding bumped to `py(6) px(10)` to match the airier feel
        // of Zed's `py_1 px_2`.
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(10.))
            .py(px(6.))
            .bg(rgb(self.bg))
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(self.border))
            .shadow(vec![
                BoxShadow {
                    color: hsla(0., 0., 0., 0.12),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(3.),
                    spread_radius: px(0.),
                },
                BoxShadow {
                    color: hsla(0., 0., 0., 0.06),
                    offset: point(px(1.), px(1.)),
                    blur_radius: px(0.),
                    spread_radius: px(0.),
                },
            ])
            .text_size(px(12.))
            .text_color(rgb(self.text))
            .child(self.label.clone());
        if let Some(c) = &self.chord {
            col = col.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(self.chord_color))
                    .child(c.clone()),
            );
        }
        // GPUI anchors tooltips at `mouse_position + (1, 1)`, which places
        // the popup directly underneath the cursor body. Match Zed's own
        // `tooltip_container` pattern (`ui::components::tooltip` →
        // `div().pl_2().pt_2p5()`): wrap the visible popup in transparent
        // left + top padding so it sits down-and-to-the-right past the
        // cursor — readable, with the cursor never sitting on the text.
        div().pl(px(8.)).pt(px(10.)).child(col)
    }
}

fn format_key(k: &str, mac: bool) -> String {
    match k {
        "left" => if mac { "←" } else { "Left" }.into(),
        "right" => if mac { "→" } else { "Right" }.into(),
        "up" => if mac { "↑" } else { "Up" }.into(),
        "down" => if mac { "↓" } else { "Down" }.into(),
        "enter" => if mac { "↩" } else { "Enter" }.into(),
        "escape" => if mac { "⎋" } else { "Esc" }.into(),
        "tab" => if mac { "⇥" } else { "Tab" }.into(),
        "backspace" => if mac { "⌫" } else { "Backspace" }.into(),
        "delete" => if mac { "⌦" } else { "Delete" }.into(),
        "space" => "Space".into(),
        "pageup" => if mac { "⇞" } else { "PgUp" }.into(),
        "pagedown" => if mac { "⇟" } else { "PgDn" }.into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "insert" => "Insert".into(),
        s if s.len() <= 3 && s.starts_with('f') && s[1..].chars().all(|c| c.is_ascii_digit()) => {
            s.to_uppercase()
        }
        s if s.chars().count() == 1 => s.to_uppercase(),
        s => s.into(),
    }
}

/// Render a gpui keystroke string (e.g. `cmd-shift-s`, `f3`) as a
/// human-readable shortcut label per platform. macOS uses Apple glyphs
/// (`⌘⇧S`); other OSes spell it (`Ctrl+Shift+S`).
fn format_chord(chord: &str) -> String {
    let (mods, key) = shortcuts::split_chord(chord);
    let mac = cfg!(target_os = "macos");

    if mac {
        let mut s = String::new();
        for m in &mods {
            match *m {
                "ctrl" => s.push('⌃'),
                "alt" => s.push('⌥'),
                "shift" => s.push('⇧'),
                "cmd" => s.push('⌘'),
                _ => {}
            }
        }
        s.push_str(&format_key(key, true));
        s
    } else {
        let mut out: Vec<String> = Vec::new();
        for m in &mods {
            match *m {
                "cmd" => continue,
                "ctrl" => out.push("Ctrl".into()),
                "alt" => out.push("Alt".into()),
                "shift" => out.push("Shift".into()),
                _ => {}
            }
        }
        out.push(format_key(key, false));
        out.join("+")
    }
}

/// Build the tooltip display text for a toolbar item: looks up the catalog
/// row by action id, takes the first default chord for this platform, and
/// renders it. Returns `None` for items that don't appear in the catalog
/// (whitespace toggle, word-wrap toggle, plugins folder) or have no
/// default chord here.
fn chord_for(action_id: &str) -> Option<String> {
    let entry = shortcuts::catalog().iter().find(|e| e.id == action_id)?;
    let chord = shortcuts::platform_default_keys(entry).first()?;
    Some(format_chord(chord))
}

/// Returns a closure compatible with gpui's `Stateful<Div>::tooltip` that
/// spawns a fresh `ToolbarTooltip` view on demand.
fn make_tooltip(
    label: SharedString,
    chord: Option<SharedString>,
    bg: u32,
    text: u32,
    border: u32,
    chord_color: u32,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_window, cx| {
        let label = label.clone();
        let chord = chord.clone();
        cx.new(move |_| ToolbarTooltip {
            label,
            chord,
            bg,
            text,
            border,
            chord_color,
        })
        .into()
    }
}

/// Snapshot of editor state used to compute disabled / pressed states for the
/// toolbar buttons. Built in `Workspace::render` and handed to `toolbar`.
#[derive(Clone, Copy)]
pub(crate) struct ToolbarFlags {
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
    pub(crate) has_selection: bool,
    pub(crate) active_dirty: bool,
    pub(crate) any_dirty: bool,
    pub(crate) is_recording: bool,
    pub(crate) has_recorded_macro: bool,
    pub(crate) word_wrap: bool,
    pub(crate) show_whitespace: bool,
    /// Active tab's file extension is `.md` / `.markdown` etc. Drives the
    /// enable/disable state of the markdown view-toggle pair.
    pub(crate) is_markdown: bool,
    /// Active tab is currently rendered as preview. Highlights the preview
    /// button when on, the source button when off.
    pub(crate) markdown_preview_active: bool,
    /// Active tab is a `.csv` / `.tsv` file — drives enable/disable of the
    /// CSV view-toggle pair.
    pub(crate) is_csv: bool,
    /// Active CSV tab is in spreadsheet preview mode. Highlights the
    /// preview button when on, the source button when off.
    pub(crate) csv_preview_active: bool,
    /// Active tab loaded from a rich-document format (Phase 1: DOCX). Drives
    /// the enable/disable state of the Doc / Raw view-toggle pair.
    pub(crate) is_rich_doc: bool,
    /// Active tab's view_mode is RichDocView (rendered). Highlights Doc View
    /// when on, Raw View when off.
    pub(crate) rich_doc_view_active: bool,
    /// `RichDocPayload::active_block.is_some()` — the user has clicked into a
    /// block so toolbar formatting buttons have a target. False disables the
    /// Bold / Italic / Underline buttons to avoid silent no-ops.
    pub(crate) rich_doc_has_active_block: bool,
    pub(crate) rich_doc_block_bold: bool,
    pub(crate) rich_doc_block_italic: bool,
    pub(crate) rich_doc_block_underline: bool,
    pub(crate) rich_doc_block_strike: bool,
    /// `Some` when every run in the active block shares a color; drives the
    /// pressed-state swatch on the Color button. `None` for default/mixed.
    pub(crate) rich_doc_block_color: Option<u32>,
    /// True when every run in the active block carries the same link URL.
    /// Drives the pressed-state on the Link button.
    pub(crate) rich_doc_block_link: bool,
    /// Alignment of the active block (Left when no block / mixed). Drives the
    /// pressed-state of the four alignment buttons; only one ever lights up.
    pub(crate) rich_doc_block_alignment: crate::rich_doc::ast::Alignment,
    /// Indent level of the active block (0..=16). Drives a subtle hint on the
    /// indent buttons — the Decrease button greys out at 0; Increase greys at 16.
    pub(crate) rich_doc_block_indent_level: u8,
    /// List membership of the active block. `Some(.. ordered: false)` lights
    /// the Bulleted List button; `Some(.. ordered: true)` lights the Numbered
    /// List button. `None` leaves both unpressed.
    pub(crate) rich_doc_block_list: Option<crate::rich_doc::ast::ListMarker>,
    /// Heading level of the active block (1..=6) when it is a `Heading`, or
    /// `None` for paragraphs / non-text blocks. Drives the Promote disabled
    /// state (Promote greys out at H1) and is reserved for future per-level
    /// pressed-state on the Heading buttons.
    pub(crate) rich_doc_block_heading_level: Option<u8>,
}

#[derive(Clone, Copy)]
enum Cmd {
    FileNew,
    FileOpen,
    FileSave,
    FileSaveAll,
    FileClose,
    FileCloseAll,
    EditCut,
    EditCopy,
    EditPaste,
    EditUndo,
    EditRedo,
    Find,
    Replace,
    FindInFiles,
    Mark,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ShowWhitespace,
    WordWrap,
    FullScreen,
    MacroStart,
    MacroStop,
    MacroPlay,
    MacroRunMany,
    MacroSave,
    PluginsFolder,
    TextTools,
    /// Universal "View Source" toggle — works for markdown, rich-doc, and
    /// CSV/TSV. Replaces three separate per-type Source buttons; dispatch
    /// fans out to the right `set_*_view_mode` based on the active tab's
    /// file type.
    ViewSource,
    /// Universal "View Preview" toggle — counterpart to `ViewSource`.
    ViewPreview,
    RdBold,
    RdItalic,
    RdUnderline,
    RdStrike,
    RdColor,
    RdLink,
    RdAlignLeft,
    RdAlignCenter,
    RdAlignRight,
    RdAlignJustify,
    RdIndentInc,
    RdIndentDec,
    RdListBullet,
    RdListNumber,
    RdHeadingPromote,
    RdHeadingDemote,
    MdBold,
    MdItalic,
    MdStrikethrough,
    MdInlineCode,
    MdHeading1,
    MdHeading2,
    MdHeading3,
    MdBulletList,
    MdOrderedList,
    MdTaskList,
    MdBlockQuote,
    MdCodeBlock,
    MdLink,
    MdImage,
    MdHorizontalRule,
}

struct Item {
    cmd: Cmd,
    icon: &'static str,
    tooltip: &'static str,
    /// Catalog id used to look up the bound chord for the tooltip popup.
    /// `None` for items that don't have a keyboard shortcut (Show Whitespace,
    /// Word Wrap, Open Plugins Folder).
    action_id: Option<&'static str>,
}

const ITEMS_FILE: &[Item] = &[
    Item { cmd: Cmd::FileNew, icon: "icons/plus.svg", tooltip: "New", action_id: Some("FileNew") },
    Item { cmd: Cmd::FileOpen, icon: "icons/folder_open.svg", tooltip: "Open…", action_id: Some("FileOpen") },
    Item { cmd: Cmd::FileSave, icon: "icons/save.svg", tooltip: "Save", action_id: Some("FileSave") },
    Item { cmd: Cmd::FileSaveAll, icon: "icons/save_all.svg", tooltip: "Save All", action_id: Some("FileSaveAll") },
    Item { cmd: Cmd::FileClose, icon: "icons/close.svg", tooltip: "Close", action_id: Some("FileCloseTab") },
    Item { cmd: Cmd::FileCloseAll, icon: "icons/close.svg", tooltip: "Close All", action_id: Some("FileCloseAllTabs") },
];
const ITEMS_EDIT: &[Item] = &[
    Item { cmd: Cmd::EditCut, icon: "icons/scissors.svg", tooltip: "Cut", action_id: Some("EditCut") },
    Item { cmd: Cmd::EditCopy, icon: "icons/copy.svg", tooltip: "Copy", action_id: Some("EditCopy") },
    Item { cmd: Cmd::EditPaste, icon: "icons/paste.svg", tooltip: "Paste", action_id: Some("EditPaste") },
    Item { cmd: Cmd::EditUndo, icon: "icons/undo.svg", tooltip: "Undo", action_id: Some("EditUndo") },
    Item { cmd: Cmd::EditRedo, icon: "icons/rotate_cw.svg", tooltip: "Redo", action_id: Some("EditRedo") },
];
const ITEMS_SEARCH: &[Item] = &[
    Item { cmd: Cmd::Find, icon: "icons/magnifying_glass.svg", tooltip: "Find…", action_id: Some("EditFind") },
    Item { cmd: Cmd::Replace, icon: "icons/replace.svg", tooltip: "Replace…", action_id: Some("EditReplace") },
    Item { cmd: Cmd::FindInFiles, icon: "icons/folder_search.svg", tooltip: "Find in Files…", action_id: Some("SearchFindInFiles") },
    Item { cmd: Cmd::Mark, icon: "icons/book.svg", tooltip: "Mark…", action_id: Some("SearchMark") },
];
const ITEMS_VIEW: &[Item] = &[
    Item { cmd: Cmd::ZoomIn, icon: "icons/zoom_in.svg", tooltip: "Zoom In", action_id: Some("ViewZoomIn") },
    Item { cmd: Cmd::ZoomOut, icon: "icons/zoom_out.svg", tooltip: "Zoom Out", action_id: Some("ViewZoomOut") },
    Item { cmd: Cmd::ZoomReset, icon: "icons/zoom_reset.svg", tooltip: "Reset Zoom", action_id: Some("ViewZoomReset") },
    Item { cmd: Cmd::ShowWhitespace, icon: "icons/whitespace.svg", tooltip: "Show Whitespace", action_id: None },
    Item { cmd: Cmd::WordWrap, icon: "icons/word_wrap.svg", tooltip: "Word Wrap", action_id: None },
    Item { cmd: Cmd::FullScreen, icon: "icons/maximize.svg", tooltip: "Toggle Full Screen", action_id: Some("ViewToggleFullScreen") },
];
// Mirrors Notepad++ macOS's five separate Macro toolbar buttons
// (TB_StartRecord, TB_StopRecord, TB_PlayRecord, TB_PlayRecordM, TB_SaveRecord).
// Start and Stop are split so the user can see at a glance which state the
// recorder is in: Start dims while recording, Stop dims when idle.
const ITEMS_MACRO: &[Item] = &[
    Item { cmd: Cmd::MacroStart, icon: "icons/circle.svg", tooltip: "Start Recording", action_id: Some("MacroStartStopRecording") },
    Item { cmd: Cmd::MacroStop, icon: "icons/stop.svg", tooltip: "Stop Recording", action_id: Some("MacroStartStopRecording") },
    Item { cmd: Cmd::MacroPlay, icon: "icons/play_filled.svg", tooltip: "Playback", action_id: Some("MacroPlayback") },
    Item { cmd: Cmd::MacroRunMany, icon: "icons/play_outlined.svg", tooltip: "Run Macro Multiple Times…", action_id: Some("MacroRunMultipleTimes") },
    Item { cmd: Cmd::MacroSave, icon: "icons/save.svg", tooltip: "Save Current Macro…", action_id: Some("MacroSaveCurrent") },
];
const ITEMS_PLUGINS: &[Item] = &[
    Item { cmd: Cmd::PluginsFolder, icon: "icons/folder.svg", tooltip: "Open Plugins Folder", action_id: None },
];
const ITEMS_TOOLS: &[Item] = &[
    Item { cmd: Cmd::TextTools, icon: "icons/wrench.svg", tooltip: "Text Tools", action_id: None },
];
// Universal Source / Preview pair — one button for source view, one for
// rendered preview. Works for markdown, rich-doc (DOCX), and CSV/TSV.
// Enabled whenever the active tab's file type supports preview;
// pressed-state highlights whichever mode the tab is currently in.
//
// The previous build shipped three separate pairs (markdown, csv, rich
// doc) — collapsed into one because they all expressed the same
// semantic "toggle between raw text and rendered view" and the toolbar
// got cluttered with adjacent greyed-out duplicates. Dispatch in
// `Workspace::view_source` / `view_preview` fans out by file type.
const ITEMS_VIEW_TOGGLE: &[Item] = &[
    Item { cmd: Cmd::ViewSource, icon: "icons/code.svg", tooltip: "View Source", action_id: Some("ViewSource") },
    Item { cmd: Cmd::ViewPreview, icon: "icons/book_open.svg", tooltip: "View Preview", action_id: Some("ViewPreview") },
];
// Rich-document inline formatting. Visible only when the active tab is a
// rich-doc in Doc View (`is_rich_doc && rich_doc_view_active`). Each button
// targets the active block (last block the user clicked into) — buttons are
// disabled until the user clicks into a paragraph or heading.
const ITEMS_RICH_DOC_FORMAT: &[Item] = &[
    Item { cmd: Cmd::RdBold, icon: "icons/format_bold.svg", tooltip: "Bold (whole paragraph)", action_id: Some("RichDocBold") },
    Item { cmd: Cmd::RdItalic, icon: "icons/format_italic.svg", tooltip: "Italic (whole paragraph)", action_id: Some("RichDocItalic") },
    Item { cmd: Cmd::RdUnderline, icon: "icons/format_underline.svg", tooltip: "Underline (whole paragraph)", action_id: Some("RichDocUnderline") },
    Item { cmd: Cmd::RdStrike, icon: "icons/format_strike.svg", tooltip: "Strikethrough (whole paragraph)", action_id: Some("RichDocStrikethrough") },
    Item { cmd: Cmd::RdColor, icon: "icons/format_color.svg", tooltip: "Font Color… (whole paragraph)", action_id: Some("RichDocColor") },
    Item { cmd: Cmd::RdLink, icon: "icons/link.svg", tooltip: "Hyperlink… (whole paragraph)", action_id: Some("RichDocLink") },
    Item { cmd: Cmd::RdAlignLeft, icon: "icons/format_align_left.svg", tooltip: "Align Left", action_id: Some("RichDocAlignLeft") },
    Item { cmd: Cmd::RdAlignCenter, icon: "icons/format_align_center.svg", tooltip: "Align Center", action_id: Some("RichDocAlignCenter") },
    Item { cmd: Cmd::RdAlignRight, icon: "icons/format_align_right.svg", tooltip: "Align Right", action_id: Some("RichDocAlignRight") },
    Item { cmd: Cmd::RdAlignJustify, icon: "icons/format_align_justify.svg", tooltip: "Justify", action_id: Some("RichDocAlignJustify") },
    Item { cmd: Cmd::RdIndentInc, icon: "icons/format_indent_increase.svg", tooltip: "Increase Indent", action_id: Some("RichDocIndentInc") },
    Item { cmd: Cmd::RdIndentDec, icon: "icons/format_indent_decrease.svg", tooltip: "Decrease Indent", action_id: Some("RichDocIndentDec") },
    Item { cmd: Cmd::RdListBullet, icon: "icons/list_bullet.svg", tooltip: "Bulleted list", action_id: None },
    Item { cmd: Cmd::RdListNumber, icon: "icons/list_ordered.svg", tooltip: "Numbered list", action_id: None },
    Item { cmd: Cmd::RdHeadingPromote, icon: "icons/heading_promote.svg", tooltip: "Promote heading (Paragraph → H3 → H2 → H1)", action_id: Some("RichDocPromoteHeading") },
    Item { cmd: Cmd::RdHeadingDemote, icon: "icons/heading_demote.svg", tooltip: "Demote heading (H1 → H2 → … → H6 → Paragraph)", action_id: Some("RichDocDemoteHeading") },
];
// Inline / line / block formatting buttons. Visible only when the active tab
// is a markdown file in source-edit mode (`is_markdown && !preview_active`)
// — switched off entirely while previewing because the helpers operate on
// source bytes the user can't see.
const ITEMS_MARKDOWN_FORMAT: &[Item] = &[
    Item { cmd: Cmd::MdBold, icon: "icons/format_bold.svg", tooltip: "Bold", action_id: Some("MdBold") },
    Item { cmd: Cmd::MdItalic, icon: "icons/format_italic.svg", tooltip: "Italic", action_id: Some("MdItalic") },
    Item { cmd: Cmd::MdStrikethrough, icon: "icons/format_strike.svg", tooltip: "Strikethrough", action_id: Some("MdStrikethrough") },
    Item { cmd: Cmd::MdInlineCode, icon: "icons/format_inline_code.svg", tooltip: "Inline Code", action_id: Some("MdInlineCode") },
    Item { cmd: Cmd::MdHeading1, icon: "icons/heading_1.svg", tooltip: "Heading 1", action_id: Some("MdHeading1") },
    Item { cmd: Cmd::MdHeading2, icon: "icons/heading_2.svg", tooltip: "Heading 2", action_id: Some("MdHeading2") },
    Item { cmd: Cmd::MdHeading3, icon: "icons/heading_3.svg", tooltip: "Heading 3", action_id: Some("MdHeading3") },
    Item { cmd: Cmd::MdBulletList, icon: "icons/list_bullet.svg", tooltip: "Bullet List", action_id: Some("MdBulletList") },
    Item { cmd: Cmd::MdOrderedList, icon: "icons/list_ordered.svg", tooltip: "Numbered List", action_id: Some("MdOrderedList") },
    Item { cmd: Cmd::MdTaskList, icon: "icons/list_checks.svg", tooltip: "Task List", action_id: Some("MdTaskList") },
    Item { cmd: Cmd::MdBlockQuote, icon: "icons/quote.svg", tooltip: "Block Quote", action_id: Some("MdBlockQuote") },
    Item { cmd: Cmd::MdCodeBlock, icon: "icons/code_block.svg", tooltip: "Code Block", action_id: Some("MdCodeBlock") },
    Item { cmd: Cmd::MdLink, icon: "icons/link.svg", tooltip: "Link", action_id: Some("MdLink") },
    Item { cmd: Cmd::MdImage, icon: "icons/image.svg", tooltip: "Image", action_id: Some("MdImage") },
    Item { cmd: Cmd::MdHorizontalRule, icon: "icons/horizontal_rule.svg", tooltip: "Horizontal Rule", action_id: Some("MdHorizontalRule") },
];

const GROUPS: &[&[Item]] = &[
    ITEMS_FILE,
    ITEMS_EDIT,
    ITEMS_SEARCH,
    ITEMS_VIEW,
    ITEMS_VIEW_TOGGLE,
    ITEMS_MACRO,
    ITEMS_PLUGINS,
    ITEMS_TOOLS,
];

fn enabled_for(cmd: Cmd, flags: &ToolbarFlags) -> bool {
    match cmd {
        Cmd::EditCut | Cmd::EditCopy => flags.has_selection,
        Cmd::EditUndo => flags.can_undo,
        Cmd::EditRedo => flags.can_redo,
        Cmd::FileSave => flags.active_dirty,
        Cmd::FileSaveAll => flags.any_dirty,
        // Mirrors NPP macOS's MainWindowController updateMacroToolbarButtons.
        Cmd::MacroStart => !flags.is_recording,
        Cmd::MacroStop => flags.is_recording,
        Cmd::MacroPlay | Cmd::MacroSave => !flags.is_recording && flags.has_recorded_macro,
        Cmd::MacroRunMany => !flags.is_recording && flags.has_recorded_macro,
        Cmd::ViewSource | Cmd::ViewPreview => {
            flags.is_markdown || flags.is_csv || flags.is_rich_doc
        }
        Cmd::RdBold
        | Cmd::RdItalic
        | Cmd::RdUnderline
        | Cmd::RdStrike
        | Cmd::RdColor
        | Cmd::RdLink
        | Cmd::RdAlignLeft
        | Cmd::RdAlignCenter
        | Cmd::RdAlignRight
        | Cmd::RdAlignJustify
        | Cmd::RdListBullet
        | Cmd::RdListNumber => {
            flags.is_rich_doc && flags.rich_doc_view_active && flags.rich_doc_has_active_block
        }
        Cmd::RdHeadingPromote => {
            flags.is_rich_doc
                && flags.rich_doc_view_active
                && flags.rich_doc_has_active_block
                && flags.rich_doc_block_heading_level != Some(1)
        }
        Cmd::RdHeadingDemote => {
            flags.is_rich_doc
                && flags.rich_doc_view_active
                && flags.rich_doc_has_active_block
        }
        Cmd::RdIndentInc => {
            flags.is_rich_doc
                && flags.rich_doc_view_active
                && flags.rich_doc_has_active_block
                && flags.rich_doc_block_indent_level < 16
        }
        Cmd::RdIndentDec => {
            flags.is_rich_doc
                && flags.rich_doc_view_active
                && flags.rich_doc_has_active_block
                && flags.rich_doc_block_indent_level > 0
        }
        _ => true,
    }
}

/// Returns true if the command is currently in an "active / on" state. The
/// toolbar paints the hover background under such buttons so toggle features
/// (Word Wrap, Show Whitespace, recording) show their state at a glance.
fn pressed_for(cmd: Cmd, flags: &ToolbarFlags) -> bool {
    match cmd {
        Cmd::ShowWhitespace => flags.show_whitespace,
        Cmd::WordWrap => flags.word_wrap,
        // Highlight the Start button while a recording is in flight so the
        // toolbar visually pulses with the recorder state even though Start
        // is technically disabled.
        Cmd::MacroStart => flags.is_recording,
        // ViewSource lights up whenever the active tab is in any source/edit
        // mode for a preview-capable file type. ViewPreview lights up in any
        // preview mode (MarkdownPreview, RichDocView, CsvPreview).
        Cmd::ViewSource => {
            (flags.is_markdown && !flags.markdown_preview_active)
                || (flags.is_csv && !flags.csv_preview_active)
                || (flags.is_rich_doc && !flags.rich_doc_view_active)
        }
        Cmd::ViewPreview => {
            (flags.is_markdown && flags.markdown_preview_active)
                || (flags.is_csv && flags.csv_preview_active)
                || (flags.is_rich_doc && flags.rich_doc_view_active)
        }
        Cmd::RdBold => flags.rich_doc_block_bold,
        Cmd::RdItalic => flags.rich_doc_block_italic,
        Cmd::RdUnderline => flags.rich_doc_block_underline,
        Cmd::RdStrike => flags.rich_doc_block_strike,
        Cmd::RdColor => flags.rich_doc_block_color.is_some(),
        Cmd::RdLink => flags.rich_doc_block_link,
        Cmd::RdAlignLeft => {
            flags.rich_doc_block_alignment == crate::rich_doc::ast::Alignment::Left
        }
        Cmd::RdAlignCenter => {
            flags.rich_doc_block_alignment == crate::rich_doc::ast::Alignment::Center
        }
        Cmd::RdAlignRight => {
            flags.rich_doc_block_alignment == crate::rich_doc::ast::Alignment::Right
        }
        Cmd::RdAlignJustify => {
            flags.rich_doc_block_alignment == crate::rich_doc::ast::Alignment::Justify
        }
        Cmd::RdListBullet => matches!(
            flags.rich_doc_block_list,
            Some(crate::rich_doc::ast::ListMarker { ordered: false, .. })
        ),
        Cmd::RdListNumber => matches!(
            flags.rich_doc_block_list,
            Some(crate::rich_doc::ast::ListMarker { ordered: true, .. })
        ),
        _ => false,
    }
}

pub(crate) fn toolbar(
    theme: &Theme,
    flags: ToolbarFlags,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let bg = theme.toolbar_bg;
    let icon_color = theme.toolbar_icon;
    let icon_disabled = theme.toolbar_icon_disabled;
    let hover_bg = theme.toolbar_hover_bg;
    let separator = theme.toolbar_separator;
    let border = theme.border;
    // Tooltip popup palette — captured by value into the per-button closures.
    // Reuses the chrome surface so the popup matches the toolbar visually.
    let tip_bg = theme.chrome;
    let tip_text = theme.chrome_text;
    let tip_border = theme.border;
    let tip_chord = theme.disabled_text;

    let mut row = div()
        .h(rems(30. / 16.))
        .flex_shrink_0()
        .px(rems(6. / 16.))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.))
        .overflow_hidden()
        .bg(rgb(bg))
        .border_b_1()
        .border_color(rgb(border));

    for (group_idx, group) in GROUPS.iter().enumerate() {
        if group_idx > 0 {
            row = row.child(
                div()
                    .w(px(1.))
                    .h(rems(18. / 16.))
                    .mx(px(4.))
                    .bg(rgb(separator)),
            );
        }
        for (item_idx, item) in group.iter().enumerate() {
            let enabled = enabled_for(item.cmd, &flags);
            let pressed = pressed_for(item.cmd, &flags);
            let cmd = item.cmd;
            let path = item.icon;
            let label: SharedString = SharedString::new_static(item.tooltip);
            let chord_text: Option<SharedString> = item
                .action_id
                .and_then(|id| chord_for(id))
                .map(SharedString::from);
            let id_str: SharedString =
                SharedString::from(format!("toolbar-btn-{}-{}", group_idx, item_idx));

            let icon_fg = if enabled { icon_color } else { icon_disabled };
            let mut button = div()
                .id(id_str)
                .flex()
                .items_center()
                .justify_center()
                .w(px(26.))
                .h(px(24.))
                .rounded(px(2.))
                .when(pressed, |d| d.bg(rgb(hover_bg)))
                .text_color(rgb(icon_fg))
                .child(svg().path(path).size(px(16.)).text_color(rgb(icon_fg)))
                .tooltip(make_tooltip(
                    label,
                    chord_text,
                    tip_bg,
                    tip_text,
                    tip_border,
                    tip_chord,
                ));

            if enabled {
                let click_workspace = workspace.clone();
                button = button
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(rgb(hover_bg)))
                    .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                        let entity = click_workspace.clone();
                        entity.update(cx, |w, cx| match cmd {
                            Cmd::FileNew => w.open_new_tab(&FileNew, window, cx),
                            Cmd::FileOpen => w.open_dialog(&FileOpen, window, cx),
                            Cmd::FileSave => w.save_active(&FileSave, window, cx),
                            Cmd::FileSaveAll => w.save_all(&FileSaveAll, window, cx),
                            Cmd::FileClose => w.close_active_tab(&FileCloseTab, window, cx),
                            Cmd::FileCloseAll => {
                                w.close_all_tabs(&FileCloseAllTabs, window, cx)
                            }
                            Cmd::EditCut => w.cut(&EditCut, window, cx),
                            Cmd::EditCopy => w.copy(&EditCopy, window, cx),
                            Cmd::EditPaste => w.paste(&EditPaste, window, cx),
                            Cmd::EditUndo => w.undo(&EditUndo, window, cx),
                            Cmd::EditRedo => w.redo(&EditRedo, window, cx),
                            Cmd::Find => w.open_find(&EditFind, window, cx),
                            Cmd::Replace => w.toggle_replace_bar(&EditReplace, window, cx),
                            Cmd::FindInFiles => {
                                w.open_find_in_files(&SearchFindInFiles, window, cx)
                            }
                            Cmd::Mark => w.open_mark(&SearchMark, window, cx),
                            Cmd::ZoomIn => w.zoom_in(&ViewZoomIn, window, cx),
                            Cmd::ZoomOut => w.zoom_out(&ViewZoomOut, window, cx),
                            Cmd::ZoomReset => w.zoom_reset(&ViewZoomReset, window, cx),
                            Cmd::ShowWhitespace => {
                                w.toggle_show_whitespace(&ViewShowWhitespace, window, cx)
                            }
                            Cmd::WordWrap => w.toggle_word_wrap(&ViewWordWrap, window, cx),
                            Cmd::FullScreen => {
                                w.toggle_full_screen(&ViewToggleFullScreen, window, cx)
                            }
                            Cmd::MacroStart => w.macro_start_recording(
                                &MacroStartRecording,
                                window,
                                cx,
                            ),
                            Cmd::MacroStop => w.macro_stop_recording(
                                &MacroStopRecording,
                                window,
                                cx,
                            ),
                            Cmd::MacroPlay => w.macro_playback(&MacroPlayback, window, cx),
                            Cmd::MacroRunMany => w.macro_run_multiple_times(
                                &MacroRunMultipleTimes,
                                window,
                                cx,
                            ),
                            Cmd::MacroSave => {
                                w.macro_save_current(&MacroSaveCurrent, window, cx)
                            }
                            Cmd::PluginsFolder => {
                                crate::plugins::open_plugins_folder();
                            }
                            Cmd::TextTools => {
                                crate::ui::text_tools::open_or_focus(cx);
                            }
                            Cmd::ViewSource => {
                                w.view_source(&ViewSource, window, cx);
                            }
                            Cmd::ViewPreview => {
                                w.view_preview(&ViewPreview, window, cx);
                            }
                            Cmd::RdBold => w.rich_doc_bold(&RichDocBold, window, cx),
                            Cmd::RdItalic => w.rich_doc_italic(&RichDocItalic, window, cx),
                            Cmd::RdUnderline => {
                                w.rich_doc_underline(&RichDocUnderline, window, cx)
                            }
                            Cmd::RdStrike => {
                                w.rich_doc_strikethrough(&RichDocStrikethrough, window, cx)
                            }
                            Cmd::RdColor => w.open_rich_doc_color_bar(window, cx),
                            Cmd::RdLink => w.open_rich_doc_link_bar(window, cx),
                            Cmd::RdAlignLeft => w.rich_doc_align_left(window, cx),
                            Cmd::RdAlignCenter => w.rich_doc_align_center(window, cx),
                            Cmd::RdAlignRight => w.rich_doc_align_right(window, cx),
                            Cmd::RdAlignJustify => w.rich_doc_align_justify(window, cx),
                            Cmd::RdIndentInc => w.rich_doc_indent_increase(window, cx),
                            Cmd::RdIndentDec => w.rich_doc_indent_decrease(window, cx),
                            Cmd::RdListBullet => w.rich_doc_toggle_list_bullet(window, cx),
                            Cmd::RdListNumber => w.rich_doc_toggle_list_number(window, cx),
                            Cmd::RdHeadingPromote => w.rich_doc_promote_heading(
                                &RichDocPromoteHeading,
                                window,
                                cx,
                            ),
                            Cmd::RdHeadingDemote => w.rich_doc_demote_heading(
                                &RichDocDemoteHeading,
                                window,
                                cx,
                            ),
                            Cmd::MdBold => w.md_bold(&MdBold, window, cx),
                            Cmd::MdItalic => w.md_italic(&MdItalic, window, cx),
                            Cmd::MdStrikethrough => {
                                w.md_strikethrough(&MdStrikethrough, window, cx)
                            }
                            Cmd::MdInlineCode => {
                                w.md_inline_code(&MdInlineCode, window, cx)
                            }
                            Cmd::MdHeading1 => w.md_heading_1(&MdHeading1, window, cx),
                            Cmd::MdHeading2 => w.md_heading_2(&MdHeading2, window, cx),
                            Cmd::MdHeading3 => w.md_heading_3(&MdHeading3, window, cx),
                            Cmd::MdBulletList => {
                                w.md_bullet_list(&MdBulletList, window, cx)
                            }
                            Cmd::MdOrderedList => {
                                w.md_ordered_list_action(&MdOrderedList, window, cx)
                            }
                            Cmd::MdTaskList => w.md_task_list(&MdTaskList, window, cx),
                            Cmd::MdBlockQuote => {
                                w.md_block_quote(&MdBlockQuote, window, cx)
                            }
                            Cmd::MdCodeBlock => {
                                w.md_code_block_action(&MdCodeBlock, window, cx)
                            }
                            Cmd::MdLink => w.md_link(&MdLink, window, cx),
                            Cmd::MdImage => w.md_image(&MdImage, window, cx),
                            Cmd::MdHorizontalRule => {
                                w.md_horizontal_rule_action(&MdHorizontalRule, window, cx)
                            }
                        });
                    });
            }

            row = row.child(button);
        }
    }

    row
}

/// Secondary toolbar — a single horizontal bar dedicated to markdown
/// formatting. Rendered directly under the main toolbar in
/// `Workspace::render` only when the active tab is a markdown file in
/// preview mode. Icons are centered horizontally so the bar reads as a
/// floating control row rather than a left-aligned menu.
pub(crate) fn markdown_format_bar(
    theme: &Theme,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let bg = theme.toolbar_bg;
    let icon_color = theme.toolbar_icon;
    let hover_bg = theme.toolbar_hover_bg;
    let border = theme.border;
    let tip_bg = theme.chrome;
    let tip_text = theme.chrome_text;
    let tip_border = theme.border;
    let tip_chord = theme.disabled_text;

    let mut row = div()
        .h(rems(30. / 16.))
        .flex_shrink_0()
        .px(rems(6. / 16.))
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .overflow_hidden()
        .bg(rgb(bg))
        .border_b_1()
        .border_color(rgb(border));

    for (item_idx, item) in ITEMS_MARKDOWN_FORMAT.iter().enumerate() {
        let cmd = item.cmd;
        let path = item.icon;
        let label: SharedString = SharedString::new_static(item.tooltip);
        let chord_text: Option<SharedString> = item
            .action_id
            .and_then(|id| chord_for(id))
            .map(SharedString::from);
        let id_str: SharedString =
            SharedString::from(format!("md-format-btn-{}", item_idx));
        let icon_fg = icon_color;
        let click_workspace = workspace.clone();

        let button = div()
            .id(id_str)
            .flex()
            .items_center()
            .justify_center()
            .w(px(26.))
            .h(px(24.))
            .rounded(px(2.))
            .text_color(rgb(icon_fg))
            .child(svg().path(path).size(px(16.)).text_color(rgb(icon_fg)))
            .tooltip(make_tooltip(
                label,
                chord_text,
                tip_bg,
                tip_text,
                tip_border,
                tip_chord,
            ))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(hover_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                let entity = click_workspace.clone();
                entity.update(cx, |w, cx| match cmd {
                    Cmd::MdBold => w.md_bold(&MdBold, window, cx),
                    Cmd::MdItalic => w.md_italic(&MdItalic, window, cx),
                    Cmd::MdStrikethrough => {
                        w.md_strikethrough(&MdStrikethrough, window, cx)
                    }
                    Cmd::MdInlineCode => w.md_inline_code(&MdInlineCode, window, cx),
                    Cmd::MdHeading1 => w.md_heading_1(&MdHeading1, window, cx),
                    Cmd::MdHeading2 => w.md_heading_2(&MdHeading2, window, cx),
                    Cmd::MdHeading3 => w.md_heading_3(&MdHeading3, window, cx),
                    Cmd::MdBulletList => w.md_bullet_list(&MdBulletList, window, cx),
                    Cmd::MdOrderedList => {
                        w.md_ordered_list_action(&MdOrderedList, window, cx)
                    }
                    Cmd::MdTaskList => w.md_task_list(&MdTaskList, window, cx),
                    Cmd::MdBlockQuote => w.md_block_quote(&MdBlockQuote, window, cx),
                    Cmd::MdCodeBlock => {
                        w.md_code_block_action(&MdCodeBlock, window, cx)
                    }
                    Cmd::MdLink => w.md_link(&MdLink, window, cx),
                    Cmd::MdImage => w.md_image(&MdImage, window, cx),
                    Cmd::MdHorizontalRule => {
                        w.md_horizontal_rule_action(&MdHorizontalRule, window, cx)
                    }
                    // The bar only renders ITEMS_MARKDOWN_FORMAT, so any
                    // other command would be a programming error in the
                    // const above. Treat it as a no-op for safety.
                    _ => {}
                });
            });

        row = row.child(button);
    }

    row
}

/// Rich-document formatting secondary bar. Mirrors `markdown_format_bar` —
/// rendered directly under the main toolbar in `Workspace::render` only when
/// the active tab is a rich-doc in Doc View. Bold / Italic / Underline target
/// the active block, which the user picks by clicking into a rendered
/// paragraph or heading in the Doc View pane.
pub(crate) fn rich_doc_format_bar(
    theme: &Theme,
    flags: ToolbarFlags,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let bg = theme.toolbar_bg;
    let icon_color = theme.toolbar_icon;
    let icon_disabled = theme.toolbar_icon_disabled;
    let hover_bg = theme.toolbar_hover_bg;
    let border = theme.border;
    let tip_bg = theme.chrome;
    let tip_text = theme.chrome_text;
    let tip_border = theme.border;
    let tip_chord = theme.disabled_text;

    let mut row = div()
        .h(rems(30. / 16.))
        .flex_shrink_0()
        .px(rems(6. / 16.))
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .overflow_hidden()
        .bg(rgb(bg))
        .border_b_1()
        .border_color(rgb(border));

    for (item_idx, item) in ITEMS_RICH_DOC_FORMAT.iter().enumerate() {
        let cmd = item.cmd;
        let path = item.icon;
        let label: SharedString = SharedString::new_static(item.tooltip);
        let chord_text: Option<SharedString> = item
            .action_id
            .and_then(|id| chord_for(id))
            .map(SharedString::from);
        let id_str: SharedString = SharedString::from(format!("rd-format-btn-{}", item_idx));
        let enabled = enabled_for(cmd, &flags);
        let pressed = pressed_for(cmd, &flags);
        let icon_fg = if enabled { icon_color } else { icon_disabled };
        let click_workspace = workspace.clone();

        let mut button = div()
            .id(id_str)
            .flex()
            .items_center()
            .justify_center()
            .w(px(26.))
            .h(px(24.))
            .rounded(px(2.))
            .text_color(rgb(icon_fg))
            .child(svg().path(path).size(px(16.)).text_color(rgb(icon_fg)))
            .tooltip(make_tooltip(
                label,
                chord_text,
                tip_bg,
                tip_text,
                tip_border,
                tip_chord,
            ));

        if pressed {
            button = button.bg(rgb(hover_bg));
        }

        if enabled {
            button = button
                .cursor(CursorStyle::PointingHand)
                .hover(|s| s.bg(rgb(hover_bg)))
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    let entity = click_workspace.clone();
                    entity.update(cx, |w, cx| match cmd {
                        Cmd::RdBold => w.rich_doc_bold(&RichDocBold, window, cx),
                        Cmd::RdItalic => w.rich_doc_italic(&RichDocItalic, window, cx),
                        Cmd::RdUnderline => {
                            w.rich_doc_underline(&RichDocUnderline, window, cx)
                        }
                        Cmd::RdStrike => {
                            w.rich_doc_strikethrough(&RichDocStrikethrough, window, cx)
                        }
                        Cmd::RdColor => w.open_rich_doc_color_bar(window, cx),
                        Cmd::RdLink => w.open_rich_doc_link_bar(window, cx),
                        Cmd::RdAlignLeft => w.rich_doc_align_left(window, cx),
                        Cmd::RdAlignCenter => w.rich_doc_align_center(window, cx),
                        Cmd::RdAlignRight => w.rich_doc_align_right(window, cx),
                        Cmd::RdAlignJustify => w.rich_doc_align_justify(window, cx),
                        Cmd::RdIndentInc => w.rich_doc_indent_increase(window, cx),
                        Cmd::RdIndentDec => w.rich_doc_indent_decrease(window, cx),
                        Cmd::RdListBullet => w.rich_doc_toggle_list_bullet(window, cx),
                        Cmd::RdListNumber => w.rich_doc_toggle_list_number(window, cx),
                        Cmd::RdHeadingPromote => {
                            w.rich_doc_promote_heading(&RichDocPromoteHeading, window, cx)
                        }
                        Cmd::RdHeadingDemote => {
                            w.rich_doc_demote_heading(&RichDocDemoteHeading, window, cx)
                        }
                        _ => {}
                    });
                });
        } else {
            button = button.cursor(CursorStyle::Arrow);
        }

        row = row.child(button);
    }

    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_chord_keeps_dash_key() {
        if cfg!(target_os = "macos") {
            assert_eq!(format_chord("cmd--"), "⌘-");
            assert_eq!(format_chord("cmd-shift-s"), "⌘⇧S");
        } else {
            assert_eq!(format_chord("ctrl--"), "Ctrl+-");
            assert_eq!(format_chord("ctrl-shift-s"), "Ctrl+Shift+S");
            assert_eq!(format_chord("ctrl-insert"), "Ctrl+Insert");
        }
    }

    #[test]
    fn chord_for_uses_this_platforms_default() {
        let expected = if cfg!(target_os = "macos") { "⌘S" } else { "Ctrl+S" };
        assert_eq!(chord_for("FileSave").as_deref(), Some(expected));
        // Notepad++ has no default for Preferences on Windows/Linux.
        if !cfg!(target_os = "macos") {
            assert_eq!(chord_for("OpenPreferences"), None);
        }
        assert_eq!(chord_for("NoSuchAction"), None);
    }
}
