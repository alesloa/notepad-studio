// CSV row / column right-click context menu.
//
// Same shape as `ui::context_menu` — a deferred anchored panel with a
// transparent backdrop that dismisses on outside click. The menu is
// rebuilt every frame from `Workspace::csv_context_menu`; the
// workspace stores only the click position and the target axis, not an
// entity handle, so toggling the menu is cheap (no allocation of GPUI
// widgets up front).

use gpui::{
    Corner, CursorStyle, Entity, IntoElement, MouseButton, Pixels, Point, anchored, deferred, div,
    prelude::*, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::Workspace;

/// Which axis the right-click landed on. Drives the menu's labels and
/// which model operations it invokes. Cell variant carries both row +
/// col since cell-targeted ops can affect either axis or just the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CsvMenuAxis {
    Row,
    Column,
    Cell { row: usize, col: usize },
}

/// State backing the CSV context menu. The workspace owns one of these
/// at a time — opening on a new target replaces the previous menu.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CsvContextMenuState {
    pub(crate) axis: CsvMenuAxis,
    /// Logical index of the row / column that was right-clicked. Unused
    /// for Cell-axis menus — the row/col live on the axis variant.
    pub(crate) target_index: usize,
    /// Window-space coordinate at which to anchor the menu's top-left.
    pub(crate) position: Point<Pixels>,
    /// How many items are in the user's current axis selection. Drives
    /// "Delete Row" vs "Delete N Rows" wording and greys out single-only
    /// items like "Insert Above".
    pub(crate) selection_count: usize,
    /// Snapshot of "this column has a filter" at menu-open time. Drives
    /// the greyed-out state of the Clear Filter entry. Captured rather
    /// than recomputed every paint to keep the menu render stateless.
    pub(crate) col_has_filter: bool,
    /// Snapshot of "ANY column has a filter" at menu-open time. Drives
    /// the greyed-out state of the Clear All Filters entry.
    pub(crate) any_filter_active: bool,
    /// Snapshot of `filters_visible` at menu-open time. Drives the check-
    /// mark / label state on the Toggle Filter Row entry.
    pub(crate) filters_visible: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    DeleteRow,
    DeleteColumn,
    InsertRowAbove,
    InsertRowBelow,
    InsertColumnLeft,
    InsertColumnRight,
    ClearContents,
    Cut,
    Copy,
    Paste,
    SortAscending,
    SortDescending,
    AutoFitColumn,
    AutoFitAllColumns,
    ClearFilter,
    ClearAllFilters,
    ToggleFilterRow,
    Sep,
}

impl Entry {
    fn label(self, selection_count: usize) -> String {
        match self {
            Self::DeleteRow => {
                if selection_count > 1 {
                    format!("Delete {selection_count} Rows")
                } else {
                    "Delete Row".to_string()
                }
            }
            Self::DeleteColumn => {
                if selection_count > 1 {
                    format!("Delete {selection_count} Columns")
                } else {
                    "Delete Column".to_string()
                }
            }
            Self::InsertRowAbove => "Insert Row Above".to_string(),
            Self::InsertRowBelow => "Insert Row Below".to_string(),
            Self::InsertColumnLeft => "Insert Column Left".to_string(),
            Self::InsertColumnRight => "Insert Column Right".to_string(),
            Self::ClearContents => "Clear Contents".to_string(),
            Self::Cut => "Cut".to_string(),
            Self::Copy => "Copy".to_string(),
            Self::Paste => "Paste".to_string(),
            Self::SortAscending => "Sort Ascending".to_string(),
            Self::SortDescending => "Sort Descending".to_string(),
            Self::AutoFitColumn => "Auto-fit Column".to_string(),
            Self::AutoFitAllColumns => "Auto-fit All Columns".to_string(),
            Self::ClearFilter => "Clear Filter".to_string(),
            Self::ClearAllFilters => "Clear All Filters".to_string(),
            Self::ToggleFilterRow => "Toggle Filter Row".to_string(),
            Self::Sep => String::new(),
        }
    }
}

const ROW_ENTRIES: &[Entry] = &[
    Entry::DeleteRow,
    Entry::Sep,
    Entry::InsertRowAbove,
    Entry::InsertRowBelow,
    Entry::Sep,
    Entry::ClearContents,
];

const COLUMN_ENTRIES: &[Entry] = &[
    Entry::SortAscending,
    Entry::SortDescending,
    Entry::Sep,
    Entry::ToggleFilterRow,
    Entry::ClearFilter,
    Entry::ClearAllFilters,
    Entry::Sep,
    Entry::AutoFitColumn,
    Entry::AutoFitAllColumns,
    Entry::Sep,
    Entry::DeleteColumn,
    Entry::Sep,
    Entry::InsertColumnLeft,
    Entry::InsertColumnRight,
    Entry::Sep,
    Entry::ClearContents,
];

const CELL_ENTRIES: &[Entry] = &[
    Entry::Cut,
    Entry::Copy,
    Entry::Paste,
    Entry::Sep,
    Entry::ClearContents,
    Entry::Sep,
    Entry::InsertRowAbove,
    Entry::InsertRowBelow,
    Entry::InsertColumnLeft,
    Entry::InsertColumnRight,
    Entry::Sep,
    Entry::DeleteRow,
    Entry::DeleteColumn,
];

pub(crate) fn csv_context_menu(
    theme: &Theme,
    state: CsvContextMenuState,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let dismiss_workspace = workspace.clone();
    let dismiss_workspace_right = workspace.clone();
    let backdrop = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
            dismiss_workspace.update(cx, |w, cx| w.hide_csv_context_menu(cx));
        })
        .on_mouse_down(MouseButton::Right, move |_e, _window, cx| {
            dismiss_workspace_right.update(cx, |w, cx| w.hide_csv_context_menu(cx));
        });

    let bg = theme.chrome;
    let border = theme.border;
    let text = theme.text;
    let chrome_text = theme.chrome_text;
    let hover = theme.toggle_active;
    let sep_color = theme.border;

    let mut menu = div()
        .flex()
        .flex_col()
        .py(rems(4. / 16.))
        .min_w(rems(180. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(rems(12. / 16.))
        .text_color(rgb(text));

    let entries = match state.axis {
        CsvMenuAxis::Row => ROW_ENTRIES,
        CsvMenuAxis::Column => COLUMN_ENTRIES,
        CsvMenuAxis::Cell { .. } => CELL_ENTRIES,
    };
    let multi = state.selection_count > 1;

    for entry in entries {
        let entry = *entry;
        if entry == Entry::Sep {
            menu = menu.child(
                div()
                    .h(rems(1. / 16.))
                    .my(rems(4. / 16.))
                    .mx(rems(8. / 16.))
                    .bg(rgb(sep_color)),
            );
            continue;
        }

        let enabled = match entry {
            // Single-target inserts grey out while multi-select is active —
            // the user's intent is ambiguous when several rows / columns
            // are selected.
            Entry::InsertRowAbove
            | Entry::InsertRowBelow
            | Entry::InsertColumnLeft
            | Entry::InsertColumnRight => !multi,
            // Sort acts on the right-clicked column regardless of the
            // axis selection size, so keep it enabled even when several
            // columns are selected (sort still picks the menu's target
            // column — same Excel behaviour as right-clicking with a
            // multi-col selection then picking Sort).
            Entry::SortAscending | Entry::SortDescending => true,
            // Auto-fit acts on the right-clicked column or every column —
            // both are always enabled regardless of selection size.
            Entry::AutoFitColumn | Entry::AutoFitAllColumns => true,
            // Clear Filter is only meaningful when this column has one.
            Entry::ClearFilter => state.col_has_filter,
            // Clear All Filters is meaningful when ANY filter is active.
            Entry::ClearAllFilters => state.any_filter_active,
            _ => true,
        };

        let row_text = if enabled { text } else { chrome_text };
        let label = match entry {
            // Toggle Filter Row reflects current visibility — Excel/Sheets
            // both show a checkmark prefix to indicate the active state.
            Entry::ToggleFilterRow => {
                if state.filters_visible {
                    "✓ Filter Row".to_string()
                } else {
                    "  Filter Row".to_string()
                }
            }
            _ => entry.label(state.selection_count),
        };
        let mut row = div()
            .flex()
            .items_center()
            .h(rems(22. / 16.))
            .px(rems(12. / 16.))
            .text_color(rgb(row_text))
            .child(label);

        if enabled {
            let row_workspace = workspace.clone();
            row = row
                .cursor(CursorStyle::PointingHand)
                .hover(|s| s.bg(rgb(hover)).text_color(rgb(theme.bg)))
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    row_workspace.update(cx, |w, cx| {
                        w.hide_csv_context_menu(cx);
                        super::actions::run_csv_menu_action(w, state, entry, window, cx);
                    });
                });
        }

        menu = menu.child(row);
    }

    deferred(
        div().absolute().top_0().left_0().size_full().child(
            div()
                .size_full()
                .child(backdrop)
                .child(
                    anchored()
                        .position(state.position)
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(gpui::px(4.))
                        .child(menu),
                ),
        ),
    )
    .with_priority(2)
}

/// Concrete menu action identifier — what `run_csv_menu_action` receives
/// from the click handler. Mirrors the variants in [`Entry`] but is
/// `pub(crate)` because the actions module needs to dispatch on it.
pub(crate) type CsvMenuAction = Entry;

impl CsvMenuAction {
    pub(crate) const DELETE_ROW: Self = Self::DeleteRow;
    pub(crate) const DELETE_COLUMN: Self = Self::DeleteColumn;
    pub(crate) const INSERT_ROW_ABOVE: Self = Self::InsertRowAbove;
    pub(crate) const INSERT_ROW_BELOW: Self = Self::InsertRowBelow;
    pub(crate) const INSERT_COLUMN_LEFT: Self = Self::InsertColumnLeft;
    pub(crate) const INSERT_COLUMN_RIGHT: Self = Self::InsertColumnRight;
    pub(crate) const CLEAR_CONTENTS: Self = Self::ClearContents;
    pub(crate) const CUT: Self = Self::Cut;
    pub(crate) const COPY: Self = Self::Copy;
    pub(crate) const PASTE: Self = Self::Paste;
    pub(crate) const SORT_ASCENDING: Self = Self::SortAscending;
    pub(crate) const SORT_DESCENDING: Self = Self::SortDescending;
    pub(crate) const AUTO_FIT_COLUMN: Self = Self::AutoFitColumn;
    pub(crate) const AUTO_FIT_ALL_COLUMNS: Self = Self::AutoFitAllColumns;
    pub(crate) const CLEAR_FILTER: Self = Self::ClearFilter;
    pub(crate) const CLEAR_ALL_FILTERS: Self = Self::ClearAllFilters;
    pub(crate) const TOGGLE_FILTER_ROW: Self = Self::ToggleFilterRow;
}
