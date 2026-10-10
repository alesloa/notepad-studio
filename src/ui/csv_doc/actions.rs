// CSV grid action handlers — cell click, commit, navigate, emit.
//
// All entry points take `&mut Workspace` so they can mutate both the
// `CsvModel` and the underlying rope (via `replace_byte_range`) inside
// the same update. Called from grid mouse / key handlers and from the
// workspace-level on_action listeners for the toolbar toggles.

use gpui::{Context, FocusHandle, Pixels, Point, SharedString, TextRun, Window};
use uuid::Uuid;

use crate::tab::InputFocus;
use crate::workspace::Workspace;

use super::context_menu::{CsvContextMenuState, CsvMenuAction, CsvMenuAxis};
use super::model::{CellRange, CellRangeDrag, CsvCellInput};

/// Body-cell mousedown handler. Excel semantics:
///
/// * **Plain click** — activates the cell, drops any range / axis
///   selection, arms a cell-range drag in case the cursor moves past
///   `DRAG_THRESHOLD_PX` before mouseup.
/// * **Shift-click** — extends the existing `active_cell` into a
///   rectangle range to the clicked cell. No drag is armed because the
///   anchor is already fixed.
/// * **Cmd / Ctrl-click** — currently behaves like a plain click; per-
///   cell discontiguous selection is not modelled here yet (and Excel's
///   model is awkward enough that we'd rather keep the simpler one
///   until a user actually asks for it).
/// * **Double-click** — opens the cell editor.
///
/// The drag itself is followed in `grid.rs`'s window-global mousemove /
/// mouseup. Mouseup with `crossed_threshold == false` discards the
/// single-cell range and leaves `active_cell` set — the click path.
pub(crate) fn on_cell_mousedown(
    w: &mut Workspace,
    row: usize,
    col: usize,
    click_count: usize,
    shift: bool,
    _cmd_or_ctrl: bool,
    mouse_x: Pixels,
    mouse_y: Pixels,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    // Commit any in-progress edit BEFORE deciding what this click does.
    commit_cell_edit(w, cx);
    if click_count >= 2 {
        open_cell_editor(w, row, col, window, cx);
        return;
    }
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    if row >= model.row_count() || col >= model.col_count() {
        return;
    }

    if shift {
        // Shift-click extends from the current `active_cell` if set,
        // otherwise from the existing range's anchor, otherwise just
        // activates the clicked cell.
        let anchor = model
            .active_cell
            .or(model.cell_range.map(|r| r.anchor))
            .unwrap_or((row, col));
        model.select_cell_range(anchor, (row, col));
        model.active_cell = Some((row, col));
    } else {
        model.clear_axis_selections();
        model.active_cell = Some((row, col));
        // Arm a fresh range + drag. The range starts as 1×1 (anchor =
        // focus = clicked cell). If the cursor stays put through
        // mouseup the drag handler discards the range so the click
        // path is indistinguishable from the pre-range behaviour.
        model.cell_range = Some(CellRange::new((row, col), (row, col)));
        model.cell_range_drag = Some(CellRangeDrag {
            anchor: (row, col),
            start_mouse_x: mouse_x,
            start_mouse_y: mouse_y,
            crossed_threshold: false,
        });
    }

    if let Some(handle) = model.grid_focus.clone() {
        window.focus(&handle, cx);
    }
    cx.notify();
}

/// Tab / Shift+Tab navigation with row wrap. `forward = true` moves one
/// column right and wraps to col 0 of the next row when at the last
/// column. `forward = false` moves one column left and wraps to the
/// last column of the previous row when at col 0. At the boundary
/// (last cell forward, (0,0) backward) the active cell stays put.
/// Always commits any in-progress edit first; never opens the editor on
/// the new cell — single-click semantics, user invokes F2 / typing to
/// edit.
pub(crate) fn move_active_cell_tab(
    w: &mut Workspace,
    forward: bool,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let row_count = model.row_count();
    let col_count = model.col_count();
    if row_count == 0 || col_count == 0 {
        return;
    }
    let (cur_r, cur_c) = model.active_cell.unwrap_or((0, 0));
    let (new_r, new_c) = if forward {
        if cur_c + 1 < col_count {
            (cur_r, cur_c + 1)
        } else if cur_r + 1 < row_count {
            (cur_r + 1, 0)
        } else {
            (cur_r, cur_c)
        }
    } else if cur_c > 0 {
        (cur_r, cur_c - 1)
    } else if cur_r > 0 {
        (cur_r - 1, col_count - 1)
    } else {
        (cur_r, cur_c)
    };
    model.active_cell = Some((new_r, new_c));
    super::grid::scroll_cell_into_view(model, new_r, new_c);
    cx.notify();
}

/// Enter / Shift+Enter navigation. `down = true` moves one row down;
/// `down = false` moves one row up. Stays put at the row boundary.
/// Commits any in-progress edit first; never opens the editor on the
/// new cell.
pub(crate) fn move_active_cell_enter(
    w: &mut Workspace,
    down: bool,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let row_count = model.row_count();
    let col_count = model.col_count();
    if row_count == 0 || col_count == 0 {
        return;
    }
    let (cur_r, cur_c) = model.active_cell.unwrap_or((0, 0));
    let new_r = if down {
        (cur_r + 1).min(row_count - 1)
    } else {
        cur_r.saturating_sub(1)
    };
    model.active_cell = Some((new_r, cur_c));
    super::grid::scroll_cell_into_view(model, new_r, cur_c);
    cx.notify();
}

/// Open the inline cell editor at `(row, col)`. Seeds the editor's
/// buffer with the current cell value and focuses the input.
pub(crate) fn open_cell_editor(
    w: &mut Workspace,
    row: usize,
    col: usize,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let focus_handle = cx.focus_handle();
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    if row >= model.row_count() || col >= model.col_count() {
        return;
    }
    let seed = model.cell(row, col).to_string();
    model.active_cell = Some((row, col));
    model.editing_cell = Some((row, col));
    model.cell_input = Some(CsvCellInput::new(focus_handle.clone(), seed));
    window.focus(&focus_handle, cx);
    cx.notify();
}

/// Open an inline editor over the column `col`'s header label. Uses the
/// row == `usize::MAX` sentinel on `editing_cell` so the grid render
/// path and `commit_cell_edit` know to target `model.headers` instead
/// of a body cell. Single click on a header → rename.
pub(crate) fn open_header_editor(
    w: &mut Workspace,
    col: usize,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let focus_handle = cx.focus_handle();
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    if col >= model.col_count() {
        return;
    }
    let seed = model.header(col).to_string();
    model.editing_cell = Some((usize::MAX, col));
    model.cell_input = Some(CsvCellInput::new(focus_handle.clone(), seed));
    window.focus(&focus_handle, cx);
    cx.notify();
}

/// Commit any pending cell edit and write the new value back to the
/// model + source. No-op when no cell is being edited. Used by every
/// "switch focus away" code path — clicking another cell, dragging a
/// column, opening the toolbar, etc.
pub(crate) fn commit_cell_edit(
    w: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let (row, col, value) = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        let Some((r, c)) = model.editing_cell.take() else {
            return;
        };
        let value = model
            .cell_input
            .take()
            .map(|i| i.buffer)
            .unwrap_or_default();
        (r, c, value)
    };

    // Write back to model. The usize::MAX row sentinel marks a header
    // rename — commit through set_header rather than set_cell so the
    // value lands in `model.headers` and round-trips through the csv
    // writer's header row on the next emit.
    let changed = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        if row == usize::MAX {
            model.set_header(col, value)
        } else {
            model.set_cell(row, col, value)
        }
    };
    if changed {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

/// Discard any pending cell edit without writing back. Bound to Esc
/// inside the cell editor.
pub(crate) fn cancel_cell_edit(
    w: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    model.editing_cell = None;
    model.cell_input = None;
    cx.notify();
}

/// Serialize the active model and replace the rope with the result.
/// One rope op → one undo entry → matches user expectation.
pub(crate) fn emit_to_source(
    w: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get(&tab_id) else {
        return;
    };
    let Some(new_text) = model.emit_source() else {
        w.set_status("CSV grid edit not written: the grid could not be written back as CSV text");
        cx.notify();
        return;
    };
    let total = w.tabs[w.active].buffer.rope.len_bytes();
    let current_text = w.tabs[w.active].buffer.rope.to_string();
    if current_text == new_text {
        cx.notify();
        return;
    }
    w.replace_byte_range(0..total, &new_text, cx);
    // Bump the model's source_version so the next preview render
    // doesn't unnecessarily rebuild — it just emitted the source
    // itself.
    if let Some(model) = w.csv_models.get_mut(&tab_id) {
        // The autosave_generation bumped during apply_edit; mirror that
        // by reading the post-edit value if exposed, else simply
        // increment so subsequent checks compare-equal.
        model.source_version = w.autosave_generation;
    }
}

/// Ensure a `CsvModel` exists for the active tab and is in sync with the
/// rope's current contents. Called at the top of `Workspace::render`
/// whenever the active tab is in `CsvPreview` mode.
pub(crate) fn ensure_model(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let source = w.tabs[w.active].buffer.rope.to_string();
    let path = w.tabs[w.active].buffer.file_path.clone();
    let generation = w.autosave_generation;

    let existing = w.csv_models.contains_key(&tab_id);
    if !existing {
        let default_delim = super::default_delimiter_for_path(path.as_deref());
        let delim = super::detect_delimiter(&source, default_delim);
        let grid_focus = cx.focus_handle();
        let mut model = super::CsvModel::empty(delim);
        model.rebuild_from_source(&source, generation);
        model.grid_focus = Some(grid_focus.clone());
        w.csv_models.insert(tab_id, model);
        register_grid_blur_listener(w, tab_id, &grid_focus, window, cx);
        cx.notify();
        return;
    }

    let needs_rebuild = w
        .csv_models
        .get(&tab_id)
        .map(|m| m.source_version != generation)
        .unwrap_or(true);
    let needs_grid_focus = w
        .csv_models
        .get(&tab_id)
        .map(|m| m.grid_focus.is_none())
        .unwrap_or(false);
    let grid_focus = if needs_grid_focus {
        Some(cx.focus_handle())
    } else {
        None
    };
    if needs_rebuild || grid_focus.is_some() {
        let new_focus_for_listener = grid_focus.clone();
        if let Some(model) = w.csv_models.get_mut(&tab_id) {
            if needs_rebuild {
                model.rebuild_from_source(&source, generation);
            }
            if let Some(handle) = grid_focus {
                model.grid_focus = Some(handle);
            }
            cx.notify();
        }
        if let Some(handle) = new_focus_for_listener {
            register_grid_blur_listener(w, tab_id, &handle, window, cx);
        }
    }
}

/// Register a blur listener on the grid's focus handle so clicking
/// outside the grid (toolbar, tab bar, another tab's editor, another
/// app) clears the selection — Excel parity. Guards against the
/// "grid → own cell editor" focus transition: when the cell input
/// takes focus next, the listener returns without clearing so the
/// editing session is preserved.
fn register_grid_blur_listener(
    w: &mut Workspace,
    tab_id: Uuid,
    handle: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let subscription = cx.on_blur(handle, window, move |w, window, cx| {
        // Skip clearing if focus transitioned to this tab's own cell
        // editor — single/double-click on a cell first focuses the cell
        // input which blurs the grid handle, and we don't want that to
        // wipe the just-set active_cell.
        let cell_input_focused = w
            .csv_models
            .get(&tab_id)
            .and_then(|m| m.cell_input.as_ref())
            .is_some_and(|c| c.focus_handle.is_focused(window));
        if cell_input_focused {
            return;
        }
        let active_is_same = w
            .tabs
            .get(w.active)
            .map(|t| t.session_id == tab_id)
            .unwrap_or(false);
        if active_is_same {
            // Goes through commit_cell_edit → clear path; correct
            // because tab matches.
            clear_grid_selection_and_edit(w, cx);
        } else if let Some(model) = w.csv_models.get_mut(&tab_id) {
            // User switched tabs. Drop selection on the background
            // tab's model directly. Pending edit (if any) stays in
            // cell_input and will be picked up by the normal flow
            // (mode switch / next focus) — losing it silently here
            // would be worse than leaving it pending.
            let dirty = !model.selected_rows.is_empty()
                || !model.selected_cols.is_empty()
                || model.active_cell.is_some();
            if dirty {
                model.clear_axis_selections();
                cx.notify();
            }
        }
    });
    w.csv_grid_blur_subscriptions.insert(tab_id, subscription);
}

/// True when typed characters should route to the active tab's CSV
/// cell editor instead of any other input target. Used by the
/// workspace's `EntityInputHandler` impl.
pub(crate) fn is_cell_input_focused(w: &Workspace, window: &Window) -> bool {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get(&tab_id) else {
        return false;
    };
    let Some(input) = model.cell_input.as_ref() else {
        return false;
    };
    input.focus_handle.is_focused(window)
}

/// Mirror of `InputFocus::CsvCell` — exposed for places that want to
/// label the active focus during dispatch without importing the
/// internal `tab` module.
pub(crate) const FOCUS_TARGET: InputFocus = InputFocus::CsvCell;

/// Handle a left-click that landed on a row gutter without producing a
/// drag (mouseup before the drag-distance threshold). Applies plain /
/// toggle / range selection depending on which modifier — if any — was
/// down when the mouse went up.
pub(crate) fn click_row_select(
    w: &mut Workspace,
    row: usize,
    cmd_or_ctrl: bool,
    shift: bool,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    if row >= model.row_count() {
        return;
    }
    if shift {
        model.select_row_range(row);
    } else if cmd_or_ctrl {
        model.select_row_toggle(row);
    } else {
        model.select_row_plain(row);
    }
    cx.notify();
}

pub(crate) fn click_col_select(
    w: &mut Workspace,
    col: usize,
    cmd_or_ctrl: bool,
    shift: bool,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    if col >= model.col_count() {
        return;
    }
    if shift {
        model.select_col_range(col);
    } else if cmd_or_ctrl {
        model.select_col_toggle(col);
    } else {
        model.select_col_plain(col);
    }
    cx.notify();
}

/// Open the row context menu at `position`. If `row` is NOT already in
/// the row selection, treat the right-click as a fresh single-row
/// selection — same behaviour as Finder / Excel / Numbers.
pub(crate) fn open_row_context_menu(
    w: &mut Workspace,
    row: usize,
    position: gpui::Point<gpui::Pixels>,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let selection_count = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        if row >= model.row_count() {
            return;
        }
        if !model.selected_rows.contains(&row) {
            model.select_row_plain(row);
        }
        model.selected_rows.len()
    };
    let (col_has_filter, any_filter_active, filters_visible) = {
        let model = w.csv_models.get(&w.tabs[w.active].session_id);
        (
            false,
            model.map(|m| m.any_filter_active()).unwrap_or(false),
            model.map(|m| m.filters_visible).unwrap_or(false),
        )
    };
    w.show_csv_context_menu(
        CsvContextMenuState {
            axis: CsvMenuAxis::Row,
            target_index: row,
            position,
            selection_count,
            col_has_filter,
            any_filter_active,
            filters_visible,
        },
        cx,
    );
}

/// Background click on the grid (below last row, beyond last column).
/// Commits any pending cell edit, clears row + column axis selections,
/// and steals focus back to the grid container. The drag bail-out is
/// included so a stale `col_drag` / `row_drag` doesn't outlive a click
/// that landed outside any header / gutter.
pub(crate) fn clear_grid_selection_and_edit(
    w: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let mut changed = false;
    // active_cell counts as selection state too — dead-space click must
    // drop the cell highlight even when no row/col axis is set.
    if !model.selected_rows.is_empty()
        || !model.selected_cols.is_empty()
        || model.active_cell.is_some()
    {
        model.clear_axis_selections();
        changed = true;
    }
    if model.col_drag.take().is_some() {
        changed = true;
    }
    if model.row_drag.take().is_some() {
        changed = true;
    }
    if model.col_resize.take().is_some() {
        changed = true;
    }
    if changed {
        cx.notify();
    }
}

/// Auto-fit a single column's width to the wider of (header text,
/// longest cell). Triggered by double-clicking the column's resize
/// handle. View-only — does NOT emit source.
pub(crate) fn auto_fit_column(
    w: &mut Workspace,
    col_logical: usize,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let measure = build_text_measure(window);
    model.auto_fit_column(col_logical, measure);
    cx.notify();
}

/// Auto-fit every column at once. Used by the resize-handle double-click
/// when the grid is in a select-all state and by the column header
/// context menu's "Auto-fit all" entry. View-only — no source emit.
pub(crate) fn auto_fit_all_columns(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let measure = build_text_measure(window);
    model.auto_fit_all_columns(measure);
    cx.notify();
}

/// Build a text-width measurer closure backed by `window.text_system()`.
/// Shared by the resize-handle double-click path and the column-header
/// context menu's auto-fit entries — keeps the font / size / shaper setup
/// in one place so both consumers measure identically.
pub(crate) fn build_text_measure(window: &mut Window) -> impl Fn(&str) -> f32 {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let font = style.font();
    let text_system = window.text_system().clone();
    // A quoted field can hold line breaks and `shape_line` takes a single
    // line (it debug-asserts on `\n`), so measure each line and keep the
    // widest.
    move |s: &str| -> f32 {
        s.split(['\n', '\r'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let runs = vec![TextRun {
                    len: part.len(),
                    font: font.clone(),
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                let line = text_system.shape_line(
                    SharedString::from(part.to_string()),
                    font_size,
                    &runs,
                    None,
                );
                f32::from(line.width)
            })
            .fold(0.0, f32::max)
    }
}

/// Open the inline CSV find bar. Allocates the find_input + focus
/// handle on first use; seeds the query from the existing buffer so
/// reopening keeps the previous search. Focuses the find input so
/// typed chars land in the query.
pub(crate) fn open_find_bar(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    // Commit any in-progress cell edit first — switching focus to the
    // find bar mid-edit must not lose the user's typed value.
    let has_cell_edit = model.cell_input.is_some();
    if has_cell_edit {
        // Defer commit to the normal helper.
        commit_cell_edit(w, cx);
    }
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let handle = model
        .find_input_handle
        .get_or_insert_with(|| cx.focus_handle())
        .clone();
    let seed = model.find_query.clone();
    let input = CsvCellInput::new(handle.clone(), seed);
    model.find_input = Some(input);
    model.find_bar_open = true;
    // Recompute matches against the seeded query so the counter shows
    // the right total the moment the bar opens.
    let q = model.find_query.clone();
    if !q.is_empty() {
        model.update_find_query(q);
    }
    window.focus(&handle, cx);
    cx.notify();
}

/// Close the find bar without clearing the query string — reopening
/// restores the previous search. Returns focus to the grid container so
/// arrow keys / typing pick the grid up again.
pub(crate) fn close_find_bar(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    model.find_bar_open = false;
    model.find_input = None;
    model.replace_input = None;
    model.replace_summary = None;
    if let Some(h) = model.grid_focus.clone() {
        window.focus(&h, cx);
    }
    cx.notify();
}

/// Open the find bar in find+replace mode (Cmd+H). Same as `open_find_bar`
/// plus `find_show_replace = true` and a freshly-allocated replace_input.
/// Focuses the replace input so typing lands in the replace string.
pub(crate) fn open_replace_bar(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    // First make sure the find bar is up.
    open_find_bar(w, window, cx);
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    model.find_show_replace = true;
    let handle = model
        .replace_input_handle
        .get_or_insert_with(|| cx.focus_handle())
        .clone();
    let seed = model.replace_text.clone();
    let input = CsvCellInput::new(handle.clone(), seed);
    model.replace_input = Some(input);
    // Focus the replace input — Cmd+H usually means "ready to type a
    // replacement"; the user already has a query from a prior Cmd+F.
    // If there's no query yet, leave focus on the find input instead.
    if model.find_query.is_empty() {
        if let Some(h) = model.find_input_handle.clone() {
            window.focus(&h, cx);
        }
    } else {
        window.focus(&handle, cx);
    }
    cx.notify();
}

/// Tab from find input — focus the replace input when the replace row
/// is up. No-op when find_show_replace is false.
pub(crate) fn focus_replace_input(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    if !model.find_show_replace {
        return;
    }
    if model.replace_input.is_none() {
        let handle = model
            .replace_input_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let seed = model.replace_text.clone();
        model.replace_input = Some(CsvCellInput::new(handle, seed));
    }
    let handle = model
        .replace_input
        .as_ref()
        .map(|i| i.focus_handle.clone());
    if let Some(h) = handle {
        window.focus(&h, cx);
        cx.notify();
    }
}

/// Tab from replace input — focus the find input.
pub(crate) fn focus_find_input(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get(&tab_id) else {
        return;
    };
    let handle = model.find_input.as_ref().map(|i| i.focus_handle.clone());
    if let Some(h) = handle {
        window.focus(&h, cx);
        cx.notify();
    }
}

/// Replace the current find match + advance. Emits source on change.
pub(crate) fn replace_current_action(
    w: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let changed = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        model.replace_current()
    };
    if changed {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

/// Replace every match — one undo grain.
pub(crate) fn replace_all_action(
    w: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let count = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        model.replace_all()
    };
    if count > 0 {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

/// Step to the next (or previous) find match. When the match is a body
/// cell, sets `active_cell` and scrolls so the row is on-screen. Header
/// matches don't move the active cell but still advance the cursor.
pub(crate) fn find_step(
    w: &mut Workspace,
    forward: bool,
    _window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let target = if forward { model.next_match() } else { model.prev_match() };
    if let Some((r, c)) = target {
        if r != usize::MAX {
            model.active_cell = Some((r, c));
        }
        super::grid::scroll_cell_into_view(model, r, c);
    }
    cx.notify();
}

/// Delete / Backspace pressed while the grid (not a cell) is focused.
/// Routes to `CsvModel::handle_delete_key` and emits source on change.
/// Priority: selected cols → selected rows → active-cell clear → no-op.
pub(crate) fn handle_grid_delete_key(
    w: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let changed = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        model.handle_delete_key()
    };
    if changed {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

/// Escape pressed while the grid (not a cell) is focused. Priority:
/// 1. Cancel in-flight col / row drag (clears state, no source emit).
/// 2. Cancel in-flight column resize.
/// 3. Clear axis selection.
/// 4. Drop active_cell highlight.
/// Returns true when something changed so the caller can suppress
/// downstream Esc handlers (close find / dismiss dialogs).
pub(crate) fn handle_grid_escape(
    w: &mut Workspace,
    cx: &mut Context<Workspace>,
) -> bool {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return false;
    };
    if model.col_drag.take().is_some()
        || model.row_drag.take().is_some()
        || model.col_resize.take().is_some()
        || model.fill_drag.take().is_some()
    {
        cx.notify();
        return true;
    }
    let mut changed = false;
    if !model.selected_rows.is_empty() || !model.selected_cols.is_empty() {
        model.clear_axis_selections();
        changed = true;
    }
    if model.active_cell.take().is_some() {
        changed = true;
    }
    if changed {
        cx.notify();
        true
    } else {
        false
    }
}

/// Cmd+A on the grid (no cell being edited): select every row, clear
/// column selection. Mirrors how a spreadsheet treats "Select All".
pub(crate) fn select_all_rows(w: &mut Workspace, cx: &mut Context<Workspace>) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let count = model.row_count();
    if count == 0 {
        return;
    }
    model.selected_cols.clear();
    model.last_clicked_col = None;
    model.selected_rows = (0..count).collect();
    model.last_clicked_row = Some(0);
    cx.notify();
}

/// F2 / Enter on a focused (non-editing) grid: open the cell editor at
/// `active_cell`. When exactly one column is selected (and no rows),
/// route the keystroke into the header rename path instead — Excel
/// parity for "selected column → F2 = rename header". No-op when no
/// cell is active or when the model has no rows yet.
pub(crate) fn open_active_cell_editor(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let header_col = w.csv_models.get(&tab_id).and_then(|m| {
        if m.selected_cols.len() == 1 && m.selected_rows.is_empty() {
            m.selected_cols.iter().copied().next()
        } else {
            None
        }
    });
    if let Some(col) = header_col {
        open_header_editor(w, col, window, cx);
        return;
    }
    let target = w
        .csv_models
        .get(&tab_id)
        .and_then(|m| m.active_cell.or_else(|| {
            if m.row_count() > 0 && m.col_count() > 0 {
                Some((0, 0))
            } else {
                None
            }
        }));
    let Some((r, c)) = target else {
        return;
    };
    open_cell_editor(w, r, c, window, cx);
}

/// Begin a cell edit seeded with `seed` text (the first typed char). The
/// cell editor opens with the buffer pre-populated and the cursor at
/// the end of the seed — matches Excel's "start typing on the active
/// cell to replace its contents". When exactly one column is selected
/// and no rows, the typing seeds the header rename instead — Excel
/// parity for renaming a selected column by just typing.
pub(crate) fn begin_cell_edit_with_seed(
    w: &mut Workspace,
    seed: &str,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let header_col = w.csv_models.get(&tab_id).and_then(|m| {
        if m.selected_cols.len() == 1 && m.selected_rows.is_empty() {
            m.selected_cols.iter().copied().next()
        } else {
            None
        }
    });
    if let Some(col) = header_col {
        let focus_handle = cx.focus_handle();
        {
            let Some(model) = w.csv_models.get_mut(&tab_id) else {
                return;
            };
            if col >= model.col_count() {
                return;
            }
            model.editing_cell = Some((usize::MAX, col));
            let buffer = seed.to_string();
            let len = buffer.len();
            model.cell_input = Some(CsvCellInput {
                focus_handle: focus_handle.clone(),
                buffer,
                cursor: len,
                anchor: len,
                drag_active: false,
            });
        }
        window.focus(&focus_handle, cx);
        cx.notify();
        return;
    }

    let target = w.csv_models.get(&tab_id).and_then(|m| {
        m.active_cell.or_else(|| {
            if m.row_count() > 0 && m.col_count() > 0 {
                Some((0, 0))
            } else {
                None
            }
        })
    });
    let Some((r, c)) = target else {
        return;
    };
    let focus_handle = cx.focus_handle();
    {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        if r >= model.row_count() || c >= model.col_count() {
            return;
        }
        model.active_cell = Some((r, c));
        model.editing_cell = Some((r, c));
        let buffer = seed.to_string();
        let len = buffer.len();
        model.cell_input = Some(CsvCellInput {
            focus_handle: focus_handle.clone(),
            buffer,
            cursor: len,
            anchor: len,
            drag_active: false,
        });
    }
    window.focus(&focus_handle, cx);
    cx.notify();
}

/// Move the highlighted (non-editing) active cell by one step. Mirrors
/// the arrow / page / home-end semantics expected on a focused grid.
/// `extend` is true for shift-modified arrows — extends the row or
/// column selection range from `last_clicked_*` rather than moving the
/// highlight.
pub(crate) fn nudge_active_cell(
    w: &mut Workspace,
    delta_row: isize,
    delta_col: isize,
    extend: bool,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let row_count = model.row_count();
    let col_count = model.col_count();
    if row_count == 0 || col_count == 0 {
        return;
    }

    // Extending with an active cell present: shift-arrow grows a cell-
    // rectangle range from the active cell (or the existing range's
    // anchor) toward the arrow direction. This branch takes precedence
    // over the axis-extension branches below — shift-arrow on a body
    // cell should produce a rectangle selection like Excel, not a
    // whole-row / whole-column extension.
    if extend
        && (model.active_cell.is_some() || model.cell_range.is_some())
        && model.selected_rows.is_empty()
        && model.selected_cols.is_empty()
    {
        let anchor = model
            .cell_range
            .map(|r| r.anchor)
            .or(model.active_cell)
            .unwrap_or((0, 0));
        let focus = model.cell_range.map(|r| r.focus).or(model.active_cell);
        let cur = focus.unwrap_or(anchor);
        let nr = (cur.0 as isize + delta_row).clamp(0, row_count as isize - 1) as usize;
        let nc = (cur.1 as isize + delta_col).clamp(0, col_count as isize - 1) as usize;
        model.select_cell_range(anchor, (nr, nc));
        model.active_cell = Some((nr, nc));
        super::grid::scroll_cell_into_view(model, nr, nc);
        cx.notify();
        return;
    }

    // Plain arrow with a cell range: collapse the range, move the
    // active cell from the range's focus by one step. Matches the way
    // Excel + Sheets behave — arrow keys exit the selection rectangle.
    if !extend && model.cell_range.is_some() {
        let focus = model
            .cell_range
            .map(|r| r.focus)
            .or(model.active_cell)
            .unwrap_or((0, 0));
        let nr = (focus.0 as isize + delta_row).clamp(0, row_count as isize - 1) as usize;
        let nc = (focus.1 as isize + delta_col).clamp(0, col_count as isize - 1) as usize;
        model.cell_range = None;
        model.cell_range_drag = None;
        model.active_cell = Some((nr, nc));
        super::grid::scroll_cell_into_view(model, nr, nc);
        cx.notify();
        return;
    }

    // Extending: shift-arrow nudges the leading edge of the row OR col
    // selection. Vertical motion extends rows; horizontal motion extends
    // cols. Mutually exclusive — picking a new axis clears the other.
    if extend {
        if delta_row != 0 {
            let anchor = model
                .last_clicked_row
                .or_else(|| model.selected_rows.iter().copied().next_back())
                .unwrap_or(0);
            let head = model
                .selected_rows
                .iter()
                .copied()
                .next_back()
                .unwrap_or(anchor);
            let new_head = (head as isize + delta_row).clamp(0, row_count as isize - 1)
                as usize;
            let (lo, hi) = if anchor <= new_head {
                (anchor, new_head)
            } else {
                (new_head, anchor)
            };
            model.selected_cols.clear();
            model.last_clicked_col = None;
            model.selected_rows = (lo..=hi).collect();
            if model.last_clicked_row.is_none() {
                model.last_clicked_row = Some(anchor);
            }
            cx.notify();
            return;
        }
        if delta_col != 0 {
            let anchor = model
                .last_clicked_col
                .or_else(|| model.selected_cols.iter().copied().next_back())
                .unwrap_or(0);
            let head = model
                .selected_cols
                .iter()
                .copied()
                .next_back()
                .unwrap_or(anchor);
            let new_head = (head as isize + delta_col).clamp(0, col_count as isize - 1)
                as usize;
            let (lo, hi) = if anchor <= new_head {
                (anchor, new_head)
            } else {
                (new_head, anchor)
            };
            model.selected_rows.clear();
            model.last_clicked_row = None;
            model.selected_cols = (lo..=hi).collect();
            if model.last_clicked_col.is_none() {
                model.last_clicked_col = Some(anchor);
            }
            cx.notify();
            return;
        }
        return;
    }

    // Plain motion. If a row OR col axis selection is active, the arrow
    // walks the selection along that axis (matches Excel: arrow on a
    // row-selected file moves to the next row). Otherwise it walks the
    // active_cell.
    if !model.selected_rows.is_empty() && delta_row != 0 {
        let head = model
            .last_clicked_row
            .or_else(|| model.selected_rows.iter().copied().next())
            .unwrap_or(0);
        let new = (head as isize + delta_row).clamp(0, row_count as isize - 1) as usize;
        model.selected_rows.clear();
        model.selected_rows.insert(new);
        model.last_clicked_row = Some(new);
        let scroll_col = model.active_cell.map(|(_, c)| c).unwrap_or(0);
        super::grid::scroll_cell_into_view(model, new, scroll_col);
        cx.notify();
        return;
    }
    if !model.selected_cols.is_empty() && delta_col != 0 {
        let head = model
            .last_clicked_col
            .or_else(|| model.selected_cols.iter().copied().next())
            .unwrap_or(0);
        let new = (head as isize + delta_col).clamp(0, col_count as isize - 1) as usize;
        model.selected_cols.clear();
        model.selected_cols.insert(new);
        model.last_clicked_col = Some(new);
        let scroll_row = model.active_cell.map(|(r, _)| r).unwrap_or(0);
        super::grid::scroll_cell_into_view(model, scroll_row, new);
        cx.notify();
        return;
    }

    let cur = model.active_cell.unwrap_or((0, 0));
    let nr = (cur.0 as isize + delta_row).clamp(0, row_count as isize - 1) as usize;
    let nc = (cur.1 as isize + delta_col).clamp(0, col_count as isize - 1) as usize;
    model.active_cell = Some((nr, nc));
    super::grid::scroll_cell_into_view(model, nr, nc);
    cx.notify();
}

/// Cmd+C while the CSV grid is focused (no cell editor open). Serialises
/// the current selection — cell range, selected rows, selected cols, or
/// the active cell — as TSV and writes it to the system clipboard. No
/// mutation; does not push an undo grain.
pub(crate) fn copy_grid_selection(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get(&tab_id) else {
        return;
    };
    let Some(kind) = super::clipboard::current_copy_kind(model) else {
        return;
    };
    let text = super::clipboard::serialize_selection(model, &kind);
    if text.is_empty() {
        return;
    }
    crate::clipboard_history::record_copy(&text);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
}

/// Cmd+X — same selection serialisation as copy, then blanks the
/// selection (or deletes whole rows / cols, matching Excel semantics).
/// One undo grain per cut.
pub(crate) fn cut_grid_selection(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let tab_id = w.tabs[w.active].session_id;
    let kind = {
        let Some(model) = w.csv_models.get(&tab_id) else {
            return;
        };
        super::clipboard::current_copy_kind(model)
    };
    let Some(kind) = kind else {
        return;
    };

    let text = {
        let Some(model) = w.csv_models.get(&tab_id) else {
            return;
        };
        super::clipboard::serialize_selection(model, &kind)
    };
    if !text.is_empty() {
        crate::clipboard_history::record_copy(&text);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
    }

    let mut changed = false;
    if let Some(model) = w.csv_models.get_mut(&tab_id) {
        match kind {
            super::clipboard::CopyKind::Rect(range) => {
                if model.clear_range(range) {
                    changed = true;
                }
            }
            super::clipboard::CopyKind::Rows(rows) => {
                if !rows.is_empty() {
                    model.delete_rows(&rows);
                    changed = true;
                }
            }
            super::clipboard::CopyKind::Cols(cols) => {
                if !cols.is_empty() {
                    model.delete_cols(&cols);
                    changed = true;
                }
            }
            super::clipboard::CopyKind::Cell((r, c)) => {
                if model.set_cell(r, c, String::new()) {
                    changed = true;
                }
            }
        }
    }
    if changed {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

/// Cmd+V — parses the clipboard as TSV (or CSV when no tabs) and writes
/// the cells starting at the anchor. Anchor priority: cell_range's
/// top-left → active_cell → no-op. When the source block is 1×1 and the
/// target is a multi-cell range, the value fills the range. Grows the
/// grid with empty rows / cols when the paste extends past the current
/// bounds. One undo grain.
pub(crate) fn paste_into_grid(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let text = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    if text.is_empty() {
        return;
    }
    let data = super::clipboard::parse_clipboard(&text);
    if data.is_empty() {
        return;
    }

    let tab_id = w.tabs[w.active].session_id;
    let (anchor, fill_target) = {
        let Some(model) = w.csv_models.get(&tab_id) else {
            return;
        };
        if let Some(range) = model.cell_range {
            ((range.min_row(), range.min_col()), Some(range))
        } else if let (Some(&first_row), Some(&last_row)) =
            (model.selected_rows.first(), model.selected_rows.last())
        {
            // Paste against the first selected row, col 0 — the rows
            // act as the fill target so a 1×1 source broadcasts.
            let last_col = model.col_count().saturating_sub(1);
            let range = CellRange::new((first_row, 0), (last_row, last_col));
            ((first_row, 0), Some(range))
        } else if let (Some(&first_col), Some(&last_col)) =
            (model.selected_cols.first(), model.selected_cols.last())
        {
            let last_row = model.row_count().saturating_sub(1);
            let range = CellRange::new((0, first_col), (last_row, last_col));
            ((0, first_col), Some(range))
        } else if let Some(cell) = model.active_cell {
            (cell, None)
        } else {
            return;
        }
    };

    let changed = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        model.paste_at(anchor.0, anchor.1, &data, fill_target)
    };
    if changed {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

/// Cmd+D inside the CSV grid — fill down. Top row of the active range
/// copies into every row below it within the range. With only a single
/// active cell, copies the cell directly above into it (Excel parity).
/// No-op when there's no selection or active cell.
pub(crate) fn fill_down_grid(w: &mut Workspace, cx: &mut Context<Workspace>) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let target = {
        let Some(model) = w.csv_models.get(&tab_id) else {
            return;
        };
        if let Some(range) = model.cell_range {
            FillTarget::Range(range)
        } else if let (Some(&first_row), Some(&last_row)) =
            (model.selected_rows.first(), model.selected_rows.last())
        {
            // Whole-row selection — fill across all columns of the rows.
            let last_col = model.col_count().saturating_sub(1);
            FillTarget::Range(CellRange::new((first_row, 0), (last_row, last_col)))
        } else if let Some((r, c)) = model.active_cell {
            if r == 0 {
                return;
            }
            FillTarget::FromNeighbor(r, c)
        } else {
            return;
        }
    };
    let changed = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        match target {
            FillTarget::Range(range) => model.fill_down_range(range),
            FillTarget::FromNeighbor(r, c) => {
                let value = model.cell(r - 1, c).to_string();
                model.set_cell(r, c, value)
            }
        }
    };
    if changed {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

/// Cmd+R inside the CSV grid — fill right. Left column of the active
/// range copies into every column to its right within the range. With
/// only a single active cell, copies the cell directly to the left.
pub(crate) fn fill_right_grid(w: &mut Workspace, cx: &mut Context<Workspace>) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let target = {
        let Some(model) = w.csv_models.get(&tab_id) else {
            return;
        };
        if let Some(range) = model.cell_range {
            FillTarget::Range(range)
        } else if let (Some(&first_col), Some(&last_col)) =
            (model.selected_cols.first(), model.selected_cols.last())
        {
            let last_row = model.row_count().saturating_sub(1);
            FillTarget::Range(CellRange::new((0, first_col), (last_row, last_col)))
        } else if let Some((r, c)) = model.active_cell {
            if c == 0 {
                return;
            }
            FillTarget::FromNeighbor(r, c)
        } else {
            return;
        }
    };
    let changed = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        match target {
            FillTarget::Range(range) => model.fill_right_range(range),
            FillTarget::FromNeighbor(r, c) => {
                let value = model.cell(r, c - 1).to_string();
                model.set_cell(r, c, value)
            }
        }
    };
    if changed {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

enum FillTarget {
    Range(CellRange),
    FromNeighbor(usize, usize),
}

/// Replace the selection between `cursor` and `anchor` (either order) in
/// `buffer` with `text`, and return the byte offset just past the
/// inserted text, where the caret goes. The offsets are fields other
/// code writes too, so one past the end is taken as the end and one
/// inside a multi-byte char as that char's start.
fn replace_input_selection(buffer: &mut String, cursor: usize, anchor: usize, text: &str) -> usize {
    let char_start = |i: usize| {
        let mut i = i.min(buffer.len());
        // Offset 0 is always a char boundary, so this stops.
        while !buffer.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let lo = char_start(cursor.min(anchor));
    let hi = char_start(cursor.max(anchor));
    buffer.replace_range(lo..hi, text);
    lo.saturating_add(text.len())
}

/// Cmd+; — insert today's date (yyyy-MM-dd) at the active cell. If a
/// cell editor is open, insert at cursor; otherwise overwrite the active
/// cell's value. No-op when there's no active cell or open editor.
pub(crate) fn insert_today_date(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let value = today_iso_string();
    insert_literal_into_active_cell(w, &value, cx);
}

/// Cmd+Shift+; (rendered as `Cmd+Shift+:` on US layout) — insert the
/// current local time (HH:MM:SS).
pub(crate) fn insert_now_time(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let value = now_time_string();
    insert_literal_into_active_cell(w, &value, cx);
}

fn insert_literal_into_active_cell(
    w: &mut Workspace,
    literal: &str,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let editing = w
        .csv_models
        .get(&tab_id)
        .map(|m| m.editing_cell.is_some() && m.cell_input.is_some())
        .unwrap_or(false);
    if editing {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        let Some(input) = model.cell_input.as_mut() else {
            return;
        };
        let new_cursor =
            replace_input_selection(&mut input.buffer, input.cursor, input.anchor, literal);
        input.cursor = new_cursor;
        input.anchor = new_cursor;
        cx.notify();
        return;
    }
    let Some(model) = w.csv_models.get(&tab_id) else {
        return;
    };
    let Some((r, c)) = model.active_cell else {
        return;
    };
    let changed = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        model.set_cell(r, c, literal.to_string())
    };
    if changed {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

fn today_iso_string() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn now_time_string() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

pub(crate) fn open_col_context_menu(
    w: &mut Workspace,
    col: usize,
    position: gpui::Point<gpui::Pixels>,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let (selection_count, col_has_filter, any_filter_active, filters_visible) = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        if col >= model.col_count() {
            return;
        }
        if !model.selected_cols.contains(&col) {
            model.select_col_plain(col);
        }
        let col_has_filter = model
            .col_filters
            .get(&col)
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        (
            model.selected_cols.len(),
            col_has_filter,
            model.any_filter_active(),
            model.filters_visible,
        )
    };
    w.show_csv_context_menu(
        CsvContextMenuState {
            axis: CsvMenuAxis::Column,
            target_index: col,
            position,
            selection_count,
            col_has_filter,
            any_filter_active,
            filters_visible,
        },
        cx,
    );
}

/// Apply a menu pick to the active CSV model. Centralised so the menu
/// element doesn't need to know about model internals.
pub(crate) fn run_csv_menu_action(
    w: &mut Workspace,
    state: CsvContextMenuState,
    action: CsvMenuAction,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    // Menu actions are always taken while NO drag is in flight (the
    // menu only opens via a separate right-click). Belt-and-braces:
    // explicitly drop any stale col_drag / row_drag / col_resize so a
    // following mousedown on the row gutter / header isn't shadowed by
    // an old gesture that survived a prior mouseup race.
    if let Some(model) = w.csv_models.get_mut(&tab_id) {
        model.col_drag = None;
        model.row_drag = None;
        model.col_resize = None;
    }
    // Resolve row + col targets per axis. Row axis: target_index = row,
    // col is meaningless (use 0); Column axis: opposite; Cell axis:
    // both come off the variant.
    let (target_row, target_col) = match state.axis {
        CsvMenuAxis::Row => (state.target_index, 0),
        CsvMenuAxis::Column => (0, state.target_index),
        CsvMenuAxis::Cell { row, col } => (row, col),
    };

    // Filter menu items don't depend on the axis variant — handle them
    // up front against the model and return. ToggleFilterRow is global;
    // ClearFilter / ClearAllFilters touch the same model state.
    match action {
        CsvMenuAction::TOGGLE_FILTER_ROW => {
            toggle_csv_filter_row(w, cx);
            return;
        }
        CsvMenuAction::CLEAR_ALL_FILTERS => {
            clear_all_csv_filters(w, cx);
            return;
        }
        CsvMenuAction::CLEAR_FILTER => {
            // For Cell-axis, target the cell's column; otherwise the
            // already-resolved target_col.
            let col = match state.axis {
                CsvMenuAxis::Cell { col, .. } => col,
                _ => target_col,
            };
            clear_filter_for_col(w, col, cx);
            return;
        }
        _ => {}
    }

    // Cell-axis Cut / Copy / Paste need workspace clipboard ops — handle
    // them before borrowing csv_models mutably below.
    if matches!(state.axis, CsvMenuAxis::Cell { .. }) {
        match action {
            CsvMenuAction::COPY => {
                copy_grid_selection(w, cx);
                cx.notify();
                return;
            }
            CsvMenuAction::CUT => {
                cut_grid_selection(w, cx);
                return;
            }
            CsvMenuAction::PASTE => {
                paste_into_grid(w, cx);
                return;
            }
            _ => {}
        }
    }

    let mut emit = false;
    {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        match action {
            CsvMenuAction::DELETE_ROW => {
                let sel = if !model.selected_rows.is_empty() {
                    model.selected_rows.clone()
                } else if let Some(range) = model.cell_range.as_ref() {
                    (range.min_row()..=range.max_row()).collect()
                } else {
                    let mut s = std::collections::BTreeSet::new();
                    s.insert(target_row);
                    s
                };
                if !sel.is_empty() {
                    model.delete_rows(&sel);
                    emit = true;
                }
            }
            CsvMenuAction::DELETE_COLUMN => {
                let sel = if !model.selected_cols.is_empty() {
                    model.selected_cols.clone()
                } else if let Some(range) = model.cell_range.as_ref() {
                    (range.min_col()..=range.max_col()).collect()
                } else {
                    let mut s = std::collections::BTreeSet::new();
                    s.insert(target_col);
                    s
                };
                if !sel.is_empty() {
                    model.delete_cols(&sel);
                    emit = true;
                }
            }
            CsvMenuAction::SORT_ASCENDING => {
                model.sort_by_col(target_col, true);
                emit = true;
            }
            CsvMenuAction::SORT_DESCENDING => {
                model.sort_by_col(target_col, false);
                emit = true;
            }
            CsvMenuAction::AUTO_FIT_COLUMN => {
                let measure = build_text_measure(window);
                model.auto_fit_column(target_col, measure);
            }
            CsvMenuAction::AUTO_FIT_ALL_COLUMNS => {
                let measure = build_text_measure(window);
                model.auto_fit_all_columns(measure);
            }
            CsvMenuAction::INSERT_ROW_ABOVE => {
                model.insert_row_at(target_row, true);
                model.clear_axis_selections();
                emit = true;
            }
            CsvMenuAction::INSERT_ROW_BELOW => {
                model.insert_row_at(target_row, false);
                model.clear_axis_selections();
                emit = true;
            }
            CsvMenuAction::INSERT_COLUMN_LEFT => {
                model.insert_col_at(target_col, true);
                model.clear_axis_selections();
                emit = true;
            }
            CsvMenuAction::INSERT_COLUMN_RIGHT => {
                model.insert_col_at(target_col, false);
                model.clear_axis_selections();
                emit = true;
            }
            CsvMenuAction::CLEAR_CONTENTS => match state.axis {
                CsvMenuAxis::Row => {
                    let sel = if model.selected_rows.is_empty() {
                        let mut s = std::collections::BTreeSet::new();
                        s.insert(target_row);
                        s
                    } else {
                        model.selected_rows.clone()
                    };
                    if model.clear_rows(&sel) {
                        emit = true;
                    }
                }
                CsvMenuAxis::Column => {
                    let sel = if model.selected_cols.is_empty() {
                        let mut s = std::collections::BTreeSet::new();
                        s.insert(target_col);
                        s
                    } else {
                        model.selected_cols.clone()
                    };
                    if model.clear_cols(&sel) {
                        emit = true;
                    }
                }
                CsvMenuAxis::Cell { row, col } => {
                    if let Some(range) = model.cell_range.clone() {
                        if model.clear_range(range) {
                            emit = true;
                        }
                    } else if model.set_cell(row, col, String::new()) {
                        emit = true;
                    }
                }
            },
            _ => {}
        }
    }
    if emit {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

/// Open the right-click menu on a specific cell. Replaces row + column
/// selections with the clicked cell's coordinates so subsequent Delete
/// Row / Column hits operate predictably even when the user right-clicks
/// outside an existing selection.
pub(crate) fn open_cell_context_menu(
    w: &mut Workspace,
    row: usize,
    col: usize,
    position: gpui::Point<gpui::Pixels>,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let selection_count = {
        let Some(model) = w.csv_models.get_mut(&tab_id) else {
            return;
        };
        if row >= model.row_count() || col >= model.col_count() {
            return;
        }
        let inside_range = model
            .cell_range
            .as_ref()
            .map(|r| r.contains(row, col))
            .unwrap_or(false);
        let inside_axis =
            model.selected_rows.contains(&row) || model.selected_cols.contains(&col);
        // Excel/Sheets: right-click inside an existing selection keeps the
        // selection intact and opens the menu against it. Right-click
        // outside collapses to a single cell.
        if !inside_range && !inside_axis {
            model.cell_range = None;
            model.cell_range_drag = None;
            model.clear_axis_selections();
            model.active_cell = Some((row, col));
        } else if !inside_range {
            model.active_cell = Some((row, col));
        }
        model.selected_rows.len().max(model.selected_cols.len())
    };
    let (col_has_filter, any_filter_active, filters_visible) = {
        let model = w.csv_models.get(&w.tabs[w.active].session_id);
        (
            model
                .and_then(|m| m.col_filters.get(&col).map(|s| !s.is_empty()))
                .unwrap_or(false),
            model.map(|m| m.any_filter_active()).unwrap_or(false),
            model.map(|m| m.filters_visible).unwrap_or(false),
        )
    };
    w.show_csv_context_menu(
        CsvContextMenuState {
            axis: CsvMenuAxis::Cell { row, col },
            target_index: 0,
            position,
            selection_count,
            col_has_filter,
            any_filter_active,
            filters_visible,
        },
        cx,
    );
}

// ---- AutoFilter actions ----------------------------------------------------

/// Build and stash the filter dropdown state for `col` anchored at the
/// click point. Idempotent — calling it again replaces an open dropdown
/// on a different column.
pub(crate) fn open_filter_dropdown(
    w: &mut Workspace,
    col: usize,
    anchor: gpui::Point<gpui::Pixels>,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    model.open_filter_dropdown_for_col(col, anchor);
    cx.notify();
}

/// Apply the staged hidden set on the open filter dropdown to
/// `col_filters` and dismiss. No-op when no dropdown is open.
pub(crate) fn apply_filter_dropdown(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    model.commit_filter_dropdown();
    cx.notify();
}

pub(crate) fn cancel_filter_dropdown(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    model.cancel_filter_dropdown();
    cx.notify();
}

/// Cmd+Shift+L handler — flips the global filter-row visibility flag.
/// Header funnel chevrons appear / disappear; existing filters survive
/// (Excel parity — Cmd+Shift+L only toggles the trigger row, never
/// removes data filters).
pub(crate) fn toggle_csv_filter_row(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    model.toggle_filters_visible();
    cx.notify();
}

pub(crate) fn clear_filter_for_col(
    w: &mut Workspace,
    col: usize,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    model.clear_filter_for_col(col);
    cx.notify();
}

pub(crate) fn clear_all_csv_filters(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    model.clear_all_filters();
    cx.notify();
}

// ---- Fill-series drag ------------------------------------------------------
//
// Excel-style fill handle on the bottom-right corner of `cell_range`.
// Mouse-down arms `fill_drag`; the body's window-global mousemove walks
// `update_fill_drag`; mouseup commits or cancels. The handle itself is
// rendered in `grid.rs`.

pub(crate) fn begin_fill_drag(
    w: &mut Workspace,
    anchor_window_pos: Point<Pixels>,
    cx: &mut Context<Workspace>,
) {
    commit_cell_edit(w, cx);
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    if model.begin_fill_drag(anchor_window_pos) {
        cx.notify();
    }
}

/// Walk the cursor's body-space delta into a logical (row, col) cell
/// and update the preview target. Same coordinate math as
/// `cell_range_drag`'s mousemove handler in `grid.rs`, but anchored at
/// the source range's bottom-right cell centre rather than the original
/// click anchor.
pub(crate) fn update_fill_drag(
    w: &mut Workspace,
    mouse_x: Pixels,
    mouse_y: Pixels,
    cx: &mut Context<Workspace>,
) {
    let tab_id = w.tabs[w.active].session_id;
    let Some(model) = w.csv_models.get_mut(&tab_id) else {
        return;
    };
    let Some(state) = model.fill_drag else {
        return;
    };
    let row_count = model.row_count();
    let col_count = model.col_count();
    if row_count == 0 || col_count == 0 {
        return;
    }

    let src = state.source_range;
    let anchor = state.anchor_window_pos;
    let dx = f32::from(mouse_x - anchor.x);
    let dy = f32::from(mouse_y - anchor.y);

    // Bottom-right cell centre in body-space. The mouse-anchor lives in
    // window-space but the delta is identical (the body wrapper does
    // not translate while a drag is in flight).
    let br_row = src.max_row();
    let br_col = src.max_col();
    let br_center_y =
        br_row as f32 * super::model::ROW_HEIGHT_PX + super::model::ROW_HEIGHT_PX / 2.0;
    let mut br_center_x = 0.0_f32;
    for c in 0..br_col {
        br_center_x += f32::from(model.col_width(c));
    }
    br_center_x += f32::from(model.col_width(br_col)) / 2.0;

    let target_y = br_center_y + dy;
    let target_x = br_center_x + dx;

    let target_row = if target_y < 0.0 {
        0
    } else {
        ((target_y / super::model::ROW_HEIGHT_PX) as usize).min(row_count.saturating_sub(1))
    };
    let mut acc = 0.0_f32;
    let mut target_col = col_count.saturating_sub(1);
    for c in 0..col_count {
        acc += f32::from(model.col_width(c));
        if target_x < acc {
            target_col = c;
            break;
        }
    }
    if target_x < 0.0 {
        target_col = 0;
    }

    model.update_fill_drag((target_row, target_col), dx, dy);
    cx.notify();
}

pub(crate) fn commit_fill_drag(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let tab_id = w.tabs[w.active].session_id;
    let mut emit = false;
    if let Some(model) = w.csv_models.get_mut(&tab_id) {
        emit = model.commit_fill_drag();
    }
    if emit {
        emit_to_source(w, cx);
    } else {
        cx.notify();
    }
}

#[allow(dead_code)]
pub(crate) fn cancel_fill_drag(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let tab_id = w.tabs[w.active].session_id;
    if let Some(model) = w.csv_models.get_mut(&tab_id) {
        model.cancel_fill_drag();
    }
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::replace_input_selection;

    #[test]
    fn replace_input_selection_inserts_and_replaces() {
        let mut b = String::from("abc");
        assert_eq!(replace_input_selection(&mut b, 1, 1, "XY"), 3);
        assert_eq!(b, "aXYbc");
        let mut b = String::from("abcdef");
        assert_eq!(replace_input_selection(&mut b, 4, 1, "-"), 2);
        assert_eq!(b, "a-ef");
    }

    #[test]
    fn replace_input_selection_with_stale_offsets() {
        // "é" is bytes 0..2 and "中" bytes 2..5. Offsets past the end
        // land at the end; offsets inside a char move back to its start.
        let mut b = String::from("é中");
        assert_eq!(replace_input_selection(&mut b, 99, 99, "!"), 6);
        assert_eq!(b, "é中!");
        let mut b = String::from("é中");
        assert_eq!(replace_input_selection(&mut b, 1, 3, "x"), 1);
        assert_eq!(b, "x中");
        let mut b = String::new();
        assert_eq!(replace_input_selection(&mut b, 4, usize::MAX, "2026-10-10"), 10);
        assert_eq!(b, "2026-10-10");
    }
}
