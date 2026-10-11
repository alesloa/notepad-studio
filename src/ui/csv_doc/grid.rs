// CSV grid preview pane.
//
// Stateless render function — mirrors `markdown_preview_pane`. The
// caller (workspace/render.rs) extracts a read-only snapshot of the
// active tab's `CsvModel` and passes it in. Mutations route through
// `entity.update(cx, …)` inside event closures, which write back to
// the model on `Workspace::csv_models`.
//
// V1 does NOT virtualize — every row is rendered. For files under
// ~5k rows this is comfortable; beyond that the scroll lag becomes
// visible. Virtualization is a follow-up; the architectural seam is
// the inner row loop in `render_body` which can switch to a
// `uniform_list`-style visible-rows-only render without disturbing the
// outer layout.

use std::collections::{BTreeSet, HashSet};

use gpui::{
    AnyElement, CursorStyle, Entity, FocusHandle, IntoElement, MouseButton, ParentElement, Pixels,
    Point, ScrollHandle, SharedString, Styled, div, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::Workspace;

use super::cell_input::cell_input_wrapper;
use super::model::{
    CellRange, ColDragState, ColResizeState, CsvModel, DEFAULT_COL_WIDTH_PX, DRAG_THRESHOLD_PX,
    FillDragState, RESIZE_HANDLE_WIDTH_PX, ROW_GUTTER_WIDTH_PX, ROW_HEIGHT_PX, RowDragState,
};

/// Snapshot of the data we need to lay out the grid. The caller builds
/// this from the workspace's `CsvModel` before delegating to
/// `csv_preview_pane`. Stays cheap to build — clones the cell strings
/// rather than holding a borrow, so GPUI event closures can call back
/// into `Workspace::update` without tripping the borrow checker.
pub(crate) struct CsvGridSnapshot {
    pub(crate) col_count: usize,
    pub(crate) row_count: usize,
    pub(crate) col_widths: Vec<Pixels>,
    pub(crate) headers: Vec<String>,
    pub(crate) cells: Vec<Vec<String>>,
    pub(crate) active_cell: Option<(usize, usize)>,
    pub(crate) editing_cell: Option<(usize, usize)>,
    pub(crate) cell_input_handle: Option<FocusHandle>,
    pub(crate) detached_draft: bool,
    pub(crate) grid_focus: Option<FocusHandle>,
    pub(crate) col_drag: Option<ColDragState>,
    pub(crate) row_drag: Option<RowDragState>,
    pub(crate) col_resize: Option<ColResizeState>,
    pub(crate) scroll_handle: ScrollHandle,
    pub(crate) total_content_width: Pixels,
    pub(crate) selected_rows: BTreeSet<usize>,
    pub(crate) selected_cols: BTreeSet<usize>,
    pub(crate) cell_range: Option<CellRange>,
    pub(crate) find_bar_open: bool,
    pub(crate) find_input_handle: Option<FocusHandle>,
    pub(crate) find_query: String,
    pub(crate) find_case_sensitive: bool,
    pub(crate) find_whole_cell: bool,
    pub(crate) find_match_count: usize,
    pub(crate) find_current_match: Option<usize>,
    pub(crate) find_show_replace: bool,
    pub(crate) replace_input_handle: Option<FocusHandle>,
    pub(crate) replace_summary: Option<String>,
    /// Whether the global filter row chevrons are visible. Cmd+Shift+L
    /// toggles this on the model.
    pub(crate) filters_visible: bool,
    /// Logical columns that have at least one hidden value — used to tint
    /// the funnel button when active.
    pub(crate) filtered_cols: BTreeSet<usize>,
    /// Pre-computed set of logical row indices to skip in the body. Built
    /// once from `row_is_visible` so the row loop walks a Vec of visible
    /// rows directly rather than re-checking per cell.
    pub(crate) hidden_rows: HashSet<usize>,
    /// In-flight Excel fill-series drag, or `None` when no drag is
    /// active. Drives both the dashed preview overlay and the
    /// suppression of the regular cell-range drag handler.
    pub(crate) fill_drag: Option<FillDragState>,
}

impl CsvGridSnapshot {
    pub(crate) fn from_model(model: &CsvModel) -> Self {
        let col_count = model.col_count();
        let row_count = model.row_count();
        let col_widths: Vec<Pixels> = (0..col_count).map(|c| model.col_width(c)).collect();
        let headers: Vec<String> = (0..col_count).map(|c| model.header(c).to_string()).collect();
        let cells: Vec<Vec<String>> = (0..row_count)
            .map(|r| (0..col_count).map(|c| model.cell(r, c).to_string()).collect())
            .collect();
        let cell_input_handle = model.cell_input.as_ref().map(|i| i.focus_handle.clone());
        let grid_focus = model.grid_focus.clone();
        let total_content_width = model.total_content_width();
        Self {
            col_count,
            row_count,
            col_widths,
            headers,
            cells,
            active_cell: model.active_cell,
            editing_cell: if model.draft_needs_strip() { None } else { model.editing_cell },
            detached_draft: model.draft_needs_strip(),
            cell_input_handle,
            grid_focus,
            col_drag: model.col_drag.clone(),
            row_drag: model.row_drag.clone(),
            col_resize: model.col_resize,
            scroll_handle: model.scroll_handle.clone(),
            total_content_width,
            selected_rows: model.selected_rows.clone(),
            selected_cols: model.selected_cols.clone(),
            cell_range: model.cell_range,
            find_bar_open: model.find_bar_open,
            find_input_handle: model.find_input_handle.clone(),
            find_query: model.find_query.clone(),
            find_case_sensitive: model.find_case_sensitive,
            find_whole_cell: model.find_whole_cell,
            find_match_count: model.find_matches.len(),
            find_current_match: model.find_current_match,
            find_show_replace: model.find_show_replace,
            replace_input_handle: model.replace_input_handle.clone(),
            replace_summary: model.replace_summary.clone(),
            filters_visible: model.filters_visible,
            filtered_cols: model
                .col_filters
                .iter()
                .filter_map(|(c, s)| if s.is_empty() { None } else { Some(*c) })
                .collect(),
            hidden_rows: (0..row_count)
                .filter(|&r| !model.row_is_visible(r))
                .collect(),
            fill_drag: model.fill_drag,
        }
    }
}

/// Translucent overlay applied on top of selected rows / columns. The
/// theme accent already has full alpha so we knock it down here to keep
/// the underlying text legible.
fn selection_overlay_color(theme: &Theme) -> u32 {
    // theme.toggle_active is the brand accent. We want a translucent
    // wash — same hue, ~35% alpha. The rgba helper takes 0xRRGGBBAA;
    // bit-or the alpha byte onto the rgb-only base. Top byte of an u32
    // colour in this codebase is unused (rgb constructor zeros it).
    let base = theme.toggle_active & 0x00FF_FFFF;
    base
}

pub(crate) fn csv_preview_pane(
    theme: &Theme,
    snapshot: &CsvGridSnapshot,
    workspace: Entity<Workspace>,
) -> AnyElement {
    let grid_focus = snapshot.grid_focus.clone();
    let input_bridge = grid_focus.as_ref().map(|focus| super::grid_input::grid_input_bridge(workspace.clone(), focus.clone()));
    let draft_strip = recovered_draft_strip(theme, snapshot, workspace.clone());
    if snapshot.col_count == 0 {
        // Empty-schema placeholder. Still tracks the grid focus so
        // Cmd-A / arrows / typed chars don't fall through to the rope
        // editor while the user is staring at an empty file. The
        // status bar is appended below so the row/column totals stay
        // visible even on an empty file.
        let mut wrapper = div()
            .id("csv-preview-pane-empty")
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .text_color(rgb(theme.disabled_text))
            .text_size(rems(13. / 16.));
        if let Some(handle) = grid_focus.as_ref() {
            wrapper = wrapper.track_focus(handle);
        }
        return wrapper
            .when_some(input_bridge, |pane, bridge| pane.child(bridge))
            .when_some(draft_strip, |pane, strip| pane.child(strip))
            .child(
                div()
                    .flex_grow()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .child(SharedString::from(
                        "Empty CSV — add a column to get started.",
                    )),
            )
            .child(super::status_bar::csv_status_bar(theme, snapshot))
            .into_any_element();
    }

    let scroll_handle = snapshot.scroll_handle.clone();
    let body_x_offset = scroll_handle.offset().x;
    let header = render_header(theme, snapshot, workspace.clone());
    let body = render_body(theme, snapshot, workspace.clone());

    let column_drop_indicator = column_drop_indicator(theme, snapshot);
    let row_drop_indicator = row_drop_indicator(theme, snapshot);
    let fill_handle = fill_handle_overlay(theme, snapshot, workspace.clone());
    let fill_preview = fill_preview_overlay(theme, snapshot);
    let body_width = snapshot.total_content_width + px(ROW_GUTTER_WIDTH_PX);

    let ws_for_bg = workspace.clone();
    let grid_focus_for_bg = grid_focus.clone();

    let mut pane = div()
        .id("csv-preview-pane")
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .bg(rgb(theme.bg))
        .text_color(rgb(theme.text))
        .text_size(rems(13. / 16.));
    if let Some(handle) = grid_focus.as_ref() {
        pane = pane.track_focus(handle);
    }
    let find_bar = super::find_bar::csv_find_bar(theme, snapshot, workspace.clone());

    // Header strip frozen at the top of the pane. Its inner row mirrors
    // the body's horizontal scroll offset so columns stay aligned with
    // body cells when the user scrolls right. Vertical scroll does not
    // touch it — the strip lives OUTSIDE the body scroller.
    let header_strip = div()
        .id("csv-grid-header-strip")
        .relative()
        .h(px(ROW_HEIGHT_PX + 2.0))
        .w_full()
        .flex_shrink_0()
        .overflow_hidden()
        .bg(rgb(theme.chrome))
        .child(
            div()
                .absolute()
                .top(px(0.))
                .left(body_x_offset)
                .w(body_width)
                .child(header),
        );

    pane.when_some(input_bridge, |pane, bridge| pane.child(bridge)).when_some(draft_strip, |pane, strip| pane.child(strip)).child(header_strip)
        .child(
            div()
                .id("csv-grid-body")
                .flex_grow()
                .overflow_y_scroll()
                .overflow_x_scroll()
                .track_scroll(&scroll_handle)
                // Click on grid background — areas below the last row,
                // beyond the last column — clears the axis selection
                // and steals focus back to the grid handle. Matches
                // Numbers / Sheets: clicking outside the table dismisses
                // any row/col highlight.
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    if let Some(handle) = grid_focus_for_bg.as_ref() {
                        window.focus(handle, cx);
                    }
                    ws_for_bg.update(cx, |w, cx| {
                        super::actions::clear_grid_selection_and_edit(w, cx);
                    });
                })
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .w(body_width)
                        .min_w_full()
                        .child(body)
                        .when_some(column_drop_indicator, |this, ind| this.child(ind))
                        .when_some(row_drop_indicator, |this, ind| this.child(ind))
                        .when_some(fill_preview, |this, ind| this.child(ind))
                        .when_some(fill_handle, |this, ind| this.child(ind)),
                ),
        )
        .when_some(find_bar, |p, bar| p.child(bar))
        .child(super::status_bar::csv_status_bar(theme, snapshot))
        .into_any_element()
}

fn recovered_draft_strip(theme: &Theme, snapshot: &CsvGridSnapshot, workspace: Entity<Workspace>) -> Option<AnyElement> {
    if !snapshot.detached_draft { return None; }
    let handle = snapshot.cell_input_handle.clone()?;
    let cancel = workspace.clone();
    Some(div().id("csv-recovered-draft").debug_selector(|| "csv-recovered-draft".to_string()).flex().flex_col().flex_shrink_0().w_full()
        .text_color(rgb(theme.text)).text_size(rems(13. / 16.))
        .child(div().flex().items_center().justify_between().px(rems(0.5))
            .child("Recovered cell draft — copy or edit the text before cancelling")
            .child(div().id("csv-cancel-recovered-draft").cursor(CursorStyle::PointingHand).px(rems(0.5))
                .child("Cancel draft").on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    cancel.update(cx, |w, cx| {
                        super::actions::cancel_cell_edit(w, cx);
                        if let Some(handle) = w.csv_models.get(&w.tabs[w.active].session_id).and_then(|model| model.grid_focus.as_ref()) {
                            window.focus(handle, cx);
                        }
                    });
                })))
        .child(div().h(rems(1.75)).w_full().child(cell_input_wrapper(theme, workspace.downgrade(), handle,
            usize::MAX, usize::MAX, px(0.), px(0.))))
        .into_any_element())
}

fn render_header(
    theme: &Theme,
    snap: &CsvGridSnapshot,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let mut row = div()
        .flex()
        .flex_row()
        .h(px(ROW_HEIGHT_PX + 2.0))
        .flex_shrink_0()
        .bg(rgb(theme.chrome))
        .border_b_1()
        .border_color(rgb(theme.border));

    row = row.child(
        div()
            .w(px(ROW_GUTTER_WIDTH_PX))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(rgb(theme.border)),
    );

    for col in 0..snap.col_count {
        let width = snap.col_widths.get(col).copied().unwrap_or(px(DEFAULT_COL_WIDTH_PX));
        let label = snap.headers.get(col).cloned().unwrap_or_default();
        let active = snap.active_cell.map(|(_, c)| c) == Some(col);
        let selected = snap.selected_cols.contains(&col);
        let is_editing_header = snap.editing_cell == Some((usize::MAX, col));
        let ws_for_header = workspace.clone();
        let ws_for_resize = workspace.clone();
        let ws_for_resize_right = workspace.clone();
        let ws_for_right = workspace.clone();

        let label_display: SharedString = if label.is_empty() {
            SharedString::from(format!("Column {}", col + 1))
        } else {
            SharedString::from(label)
        };

        // Every path that sets `editing_cell` sets `cell_input` with it;
        // if they ever disagree the header shows its label.
        let editor_handle = snap.cell_input_handle.clone().filter(|_| is_editing_header);
        let label_child: AnyElement = if let Some(handle) = editor_handle {
            // Inline editor mounted over the header label. Same widget as
            // a body-cell edit; the row == usize::MAX sentinel in
            // editing_cell tells commit_cell_edit to route the buffer
            // back into model.headers rather than model.rows.
            div()
                .w(width)
                .h_full()
                .flex_shrink_0()
                .child(cell_input_wrapper(
                    theme,
                    workspace.downgrade(),
                    handle,
                    usize::MAX,
                    col,
                    width,
                    px(ROW_HEIGHT_PX),
                ))
                .into_any_element()
        } else {
            div()
                .flex_grow()
                .min_w_0()
                .text_color(rgb(theme.chrome_text))
                .child(div().truncate().child(label_display))
                .into_any_element()
        };

        let mut cell = div()
            .id(SharedString::from(format!("csv-h-{col}")))
            .flex()
            .items_center()
            .px(px(8.))
            .w(width)
            .h_full()
            .flex_shrink_0()
            .relative()
            .when(selected && !is_editing_header, |d| {
                d.bg(rgb(selection_overlay_color(theme)))
            })
            .when(!selected && active && !is_editing_header, |d| {
                d.bg(rgb(theme.toggle_active))
            })
            .border_r_1()
            .border_color(rgb(theme.border))
            .child(label_child);

        // AutoFilter funnel chevron — inline button to the right of the
        // header label when the global filter row is visible. Clicking
        // opens the filter dropdown for this column. The button sits
        // inside the header's flex row so it doesn't collide with the
        // absolute resize handle on the far right; reserve a small
        // right-side margin so the chevron and the handle don't overlap.
        if snap.filters_visible && !is_editing_header {
            let active_filter = snap.filtered_cols.contains(&col);
            let ws_for_chevron = workspace.clone();
            let glyph: SharedString = if active_filter {
                SharedString::from("▼")
            } else {
                SharedString::from("▾")
            };
            let chevron_color = if active_filter {
                theme.toggle_active
            } else {
                theme.chrome_text
            };
            cell = cell.child(
                div()
                    .id(SharedString::from(format!("csv-h-{col}-filter")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(16.))
                    .h(px(16.))
                    .mr(px(RESIZE_HANDLE_WIDTH_PX + 2.0))
                    .text_size(rems(10. / 16.))
                    .text_color(rgb(chevron_color))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(rgb(theme.toggle_active)).text_color(rgb(theme.bg)))
                    .child(glyph)
                    .on_mouse_down(MouseButton::Left, move |e, _window, cx| {
                        let position = e.position;
                        cx.stop_propagation();
                        ws_for_chevron.update(cx, |w, cx| {
                            super::actions::open_filter_dropdown(w, col, position, cx);
                        });
                    }),
            );
        }

        if !is_editing_header {
            cell = cell
                .child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .right(px(0.))
                        .w(px(RESIZE_HANDLE_WIDTH_PX))
                        .h_full()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                            let mouse_x = e.position.x;
                            let click_count = e.click_count;
                            cx.stop_propagation();
                            // Double-click auto-fits — never arms drag.
                            // Excel/Sheets parity. When the grid is in a
                            // full-selection state (Cmd+A selected every
                            // row, or the user shift-clicked multiple
                            // column headers), fit every column instead
                            // of just this one.
                            if click_count >= 2 {
                                ws_for_resize.update(cx, |w, cx| {
                                    let tab_id = w.tabs[w.active].session_id;
                                    let fit_all = w
                                        .csv_models
                                        .get(&tab_id)
                                        .map(|m| {
                                            let row_count = m.row_count();
                                            let col_count = m.col_count();
                                            let all_rows = row_count > 0
                                                && m.selected_rows.len() == row_count;
                                            let multi_cols = m.selected_cols.len() > 1;
                                            let all_cols = col_count > 0
                                                && m.selected_cols.len() == col_count;
                                            all_rows || multi_cols || all_cols
                                        })
                                        .unwrap_or(false);
                                    if fit_all {
                                        super::actions::auto_fit_all_columns(w, window, cx);
                                    } else {
                                        super::actions::auto_fit_column(
                                            w, col, window, cx,
                                        );
                                    }
                                });
                                return;
                            }
                            ws_for_resize.update(cx, |w, cx| {
                                let tab_id = w.tabs[w.active].session_id;
                                if let Some(model) = w.csv_models.get_mut(&tab_id) {
                                    let start_width = model.col_width(col);
                                    // One undo grain per drag-resize.
                                    // mousemove writes set_col_width
                                    // repeatedly without pushing — push
                                    // once at the start of the gesture.
                                    model.push_undo();
                                    model.col_resize = Some(ColResizeState {
                                        col_logical: col,
                                        start_mouse_x: mouse_x,
                                        start_width,
                                    });
                                    cx.notify();
                                }
                            });
                        })
                        .on_mouse_down(MouseButton::Right, move |e, _w, cx| {
                            let position = e.position;
                            cx.stop_propagation();
                            ws_for_resize_right.update(cx, |w, cx| {
                                super::actions::open_col_context_menu(w, col, position, cx);
                            });
                        }),
                )
                .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                    let mouse_x = e.position.x;
                    let mods = e.modifiers;
                    let cmd_or_ctrl = mods.platform || mods.control;
                    let shift = mods.shift;
                    let click_count = e.click_count;
                    cx.stop_propagation();
                    ws_for_header.update(cx, |w, cx| {
                        // Cmd / Shift means the user is composing a multi-
                        // select — handle the click immediately and skip
                        // drag-reorder arming so an accidental 1-px wiggle
                        // doesn't turn a toggle into a move.
                        if cmd_or_ctrl || shift {
                            super::actions::click_col_select(w, col, cmd_or_ctrl, shift, cx);
                            let tab_id = w.tabs[w.active].session_id;
                            if let Some(handle) = w
                                .csv_models
                                .get(&tab_id)
                                .and_then(|m| m.grid_focus.clone())
                            {
                                window.focus(&handle, cx);
                            }
                            return;
                        }
                        // Double click → open the inline rename editor and
                        // skip drag arming. The first click of the pair
                        // already ran its own mouseup (which selected the
                        // column); the second click upgrades to rename.
                        if click_count >= 2 {
                            if !super::actions::commit_cell_edit(w, cx) { return; }
                            super::actions::open_header_editor(w, col, window, cx);
                            return;
                        }
                        if !super::actions::commit_cell_edit(w, cx) { return; }
                        let tab_id = w.tabs[w.active].session_id;
                        let Some(model) = w.csv_models.get_mut(&tab_id) else {
                            return;
                        };
                        // Multi-drag when the clicked col is already part of
                        // a multi-selection — capture the snapshot now so a
                        // mid-drag selection change can't desync the block.
                        // Otherwise replace selection on mousedown so a missed
                        // mouseup (release outside the header strip) doesn't
                        // leave stale selection on the previous column.
                        let dragged: Vec<usize> = if model.selected_cols.contains(&col)
                            && model.selected_cols.len() > 1
                        {
                            model.selected_cols.iter().copied().collect()
                        } else {
                            model.select_col_plain(col);
                            vec![col]
                        };
                        model.col_drag = Some(ColDragState {
                            dragged,
                            primary: col,
                            start_mouse_x: mouse_x,
                            current_mouse_x: mouse_x,
                            drop_logical: col,
                            crossed_threshold: false,
                        });
                        // Focus the grid so subsequent outside clicks
                        // trigger the blur listener and clear selection.
                        let grid_handle = model.grid_focus.clone();
                        cx.notify();
                        if let Some(handle) = grid_handle {
                            window.focus(&handle, cx);
                        }
                    });
                })
                .on_mouse_down(MouseButton::Right, move |e, _w, cx| {
                    let position = e.position;
                    cx.stop_propagation();
                    ws_for_right.update(cx, |w, cx| {
                        super::actions::open_col_context_menu(w, col, position, cx);
                    });
                });
        }

        row = row.child(cell);
    }

    // Trailing filler — extends the header background to the right edge
    // when the wrapper is wider than the sum of column widths. No
    // border-r, no content. Without this the gutter+columns end and the
    // remaining wrapper width shows the pane background, which looks
    // broken next to the body rows that also have their own fillers.
    row = row.child(
        div()
            .flex_grow()
            .h_full()
            .bg(rgb(theme.chrome)),
    );

    let ws_for_move = workspace.clone();
    let ws_for_up = workspace;
    let total_w = snap.total_content_width;
    let col_widths = snap.col_widths.clone();
    div()
        .relative()
        .child(row)
        .on_mouse_move(move |event, _w, cx| {
            let mouse_x = event.position.x;
            ws_for_move.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let Some(model) = w.csv_models.get_mut(&tab_id) else {
                    return;
                };
                if let Some(state) = model.col_resize.as_mut() {
                    let delta = mouse_x - state.start_mouse_x;
                    let new_w = state.start_width + delta;
                    let col = state.col_logical;
                    model.set_col_width(col, new_w);
                    cx.notify();
                    return;
                }
                if let Some(drag) = model.col_drag.as_mut() {
                    drag.current_mouse_x = mouse_x;
                    let delta = f32::from(mouse_x - drag.start_mouse_x).abs();
                    if delta >= DRAG_THRESHOLD_PX {
                        drag.crossed_threshold = true;
                    }
                    if drag.crossed_threshold {
                        let drop_logical = drop_logical_for_x(
                            mouse_x - drag.start_mouse_x,
                            drag.primary,
                            &col_widths,
                            total_w,
                        );
                        drag.drop_logical = drop_logical;
                    }
                    cx.notify();
                }
            });
        })
        .on_mouse_up(MouseButton::Left, move |_event, _window, cx| {
            ws_for_up.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let mut emit = false;
                let mut click_col: Option<usize> = None;
                if let Some(model) = w.csv_models.get_mut(&tab_id) {
                    if let Some(state) = model.col_resize.take() {
                        // Width changes are view-only — no source emit.
                        // But check whether the drag actually changed the
                        // width: a click on the resize handle that didn't
                        // produce any mouse move would otherwise leave a
                        // spurious undo entry pushed at mousedown.
                        let final_w = model.col_width(state.col_logical);
                        if final_w == state.start_width {
                            model.pop_undo_no_restore();
                        }
                    }
                    if let Some(drag) = model.col_drag.take() {
                        if !drag.crossed_threshold {
                            // The pointer never moved far enough — treat
                            // this as a plain click. Defer to after this
                            // borrow ends so click_col_select can take
                            // its own &mut Workspace.
                            click_col = Some(drag.primary);
                        } else {
                            // move_col_block returns (new_primary, did_change).
                            // did_change is false when the drop position
                            // matches the original order (single-col drop-
                            // at-start, or multi-col contiguous drop-at-
                            // start). Only emit when something moved.
                            let (_, did_change) = model.move_col_block(
                                &drag.dragged,
                                drag.primary,
                                drag.drop_logical,
                            );
                            if did_change {
                                emit = true;
                            }
                        }
                    }
                }
                if let Some(col) = click_col {
                    // Single-click on a column header selects the column.
                    // Double-click (handled in mousedown above) opens the
                    // rename editor. Right-click opens the context menu.
                    super::actions::click_col_select(w, col, false, false, cx);
                }
                if emit {
                    super::actions::emit_to_source(w, cx);
                }
                cx.notify();
            });
        })
}

fn render_body(
    theme: &Theme,
    snap: &CsvGridSnapshot,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let mut body = div().flex().flex_col();

    for row in 0..snap.row_count {
        // AutoFilter — skip rendering rows hidden by an active filter.
        // Row identity (the logical index) is preserved so cell coords
        // anywhere else in the code base stay valid; we just don't
        // paint the row strip itself.
        if snap.hidden_rows.contains(&row) {
            continue;
        }
        let row_selected = snap.selected_rows.contains(&row);
        let mut row_div = div()
            .flex()
            .flex_row()
            .h(px(ROW_HEIGHT_PX))
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(theme.border));

        let ws_for_row = workspace.clone();
        let ws_for_row_right = workspace.clone();
        row_div = row_div.child(
            div()
                .id(SharedString::from(format!("csv-rowgutter-{row}")))
                .w(px(ROW_GUTTER_WIDTH_PX))
                .h_full()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .when(row_selected, |d| d.bg(rgb(selection_overlay_color(theme))))
                .when(!row_selected, |d| d.bg(rgb(theme.chrome)))
                .border_r_1()
                .border_color(rgb(theme.border))
                .text_color(rgb(theme.gutter_text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(format!("{}", row + 1)))
                .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                    let mouse_y = e.position.y;
                    let mods = e.modifiers;
                    let cmd_or_ctrl = mods.platform || mods.control;
                    let shift = mods.shift;
                    cx.stop_propagation();
                    ws_for_row.update(cx, |w, cx| {
                        if cmd_or_ctrl || shift {
                            super::actions::click_row_select(w, row, cmd_or_ctrl, shift, cx);
                            // Take focus so the grid's blur listener fires
                            // when the user later clicks outside.
                            let tab_id = w.tabs[w.active].session_id;
                            if let Some(handle) = w
                                .csv_models
                                .get(&tab_id)
                                .and_then(|m| m.grid_focus.clone())
                            {
                                window.focus(&handle, cx);
                            }
                            return;
                        }
                        if !super::actions::commit_cell_edit(w, cx) { return; }
                        let tab_id = w.tabs[w.active].session_id;
                        let Some(model) = w.csv_models.get_mut(&tab_id) else {
                            return;
                        };
                        let dragged: Vec<usize> = if model.selected_rows.contains(&row)
                            && model.selected_rows.len() > 1
                        {
                            model.selected_rows.iter().copied().collect()
                        } else {
                            model.select_row_plain(row);
                            vec![row]
                        };
                        model.row_drag = Some(RowDragState {
                            dragged,
                            primary: row,
                            start_mouse_y: mouse_y,
                            current_mouse_y: mouse_y,
                            drop_logical: row,
                            crossed_threshold: false,
                        });
                        // Focus the grid so subsequent click-outside fires
                        // on_blur and clears the selection. Without this
                        // the gutter click never gives the grid keyboard
                        // focus, so the blur listener has nothing to fire
                        // on — row selection persisted past outside clicks.
                        let grid_handle = model.grid_focus.clone();
                        cx.notify();
                        if let Some(handle) = grid_handle {
                            window.focus(&handle, cx);
                        }
                    });
                })
                .on_mouse_down(MouseButton::Right, move |e, _w, cx| {
                    let position = e.position;
                    cx.stop_propagation();
                    ws_for_row_right.update(cx, |w, cx| {
                        super::actions::open_row_context_menu(w, row, position, cx);
                    });
                }),
        );

        for col in 0..snap.col_count {
            let width = snap.col_widths.get(col).copied().unwrap_or(px(DEFAULT_COL_WIDTH_PX));
            let value = snap
                .cells
                .get(row)
                .and_then(|r| r.get(col))
                .cloned()
                .unwrap_or_default();
            let is_active = snap.active_cell == Some((row, col));
            let is_editing = snap.editing_cell == Some((row, col));
            let col_selected = snap.selected_cols.contains(&col);
            let in_range = snap
                .cell_range
                .map(|r| r.contains(row, col))
                .unwrap_or(false);
            let any_selected = row_selected || col_selected || in_range;

            // As in the header: without an input the cell renders static.
            let editor_handle = snap.cell_input_handle.clone().filter(|_| is_editing);
            let cell = if let Some(handle) = editor_handle {
                div()
                    .w(width)
                    .h_full()
                    .flex_shrink_0()
                    .child(cell_input_wrapper(
                        theme,
                        workspace.downgrade(),
                        handle,
                        row,
                        col,
                        width,
                        px(ROW_HEIGHT_PX),
                    ))
                    .into_any_element()
            } else {
                let ws_for_click = workspace.clone();
                let ws_for_right = workspace.clone();
                let numeric = super::model::is_numeric(&value);
                div()
                    .id(SharedString::from(format!("csv-cell-static-{row}-{col}")))
                    .w(width)
                    .h_full()
                    .flex_shrink_0()
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .when(numeric, |d| d.justify_end())
                    .when(any_selected, |d| d.bg(rgb(selection_overlay_color(theme))))
                    .when(!any_selected && is_active, |d| d.bg(rgb(theme.toggle_active)))
                    .border_r_1()
                    .border_color(rgb(theme.border))
                    .text_color(rgb(theme.text))
                    .overflow_hidden()
                    .cursor(CursorStyle::IBeam)
                    .child(div().truncate().min_w_0().child(SharedString::from(value)))
                    .on_mouse_down(MouseButton::Left, move |e, w, cx| {
                        let click_count = e.click_count;
                        let mods = e.modifiers;
                        let mouse_x = e.position.x;
                        let mouse_y = e.position.y;
                        cx.stop_propagation();
                        ws_for_click.update(cx, |ws, cx| {
                            super::actions::on_cell_mousedown(
                                ws,
                                row,
                                col,
                                click_count,
                                mods.shift,
                                mods.platform || mods.control,
                                mouse_x,
                                mouse_y,
                                w,
                                cx,
                            );
                        });
                    })
                    .on_mouse_down(MouseButton::Right, move |e, _w, cx| {
                        let position = e.position;
                        cx.stop_propagation();
                        ws_for_right.update(cx, |ws, cx| {
                            super::actions::open_cell_context_menu(
                                ws, row, col, position, cx,
                            );
                        });
                    })
                    .into_any_element()
            };

            row_div = row_div.child(cell);
        }

        // Trailing filler — same as the header filler but tinted with
        // the row-selection overlay when this row is in selected_rows,
        // so the highlight reads as a full-width band like Excel.
        row_div = row_div.child(
            div()
                .flex_grow()
                .h_full()
                .when(row_selected, |d| d.bg(rgb(selection_overlay_color(theme)))),
        );

        body = body.child(row_div);
    }

    // Row drag move + up — window-global so the drag tracks past the
    // gutter's bounds. Same pattern as the column drag handlers above.
    // The cell-range drag path runs alongside it: at most one of
    // (row_drag, cell_range_drag) is set at a time, so the handlers are
    // additive rather than conflicting.
    let ws_for_row_move = workspace.clone();
    let ws_for_row_up = workspace.clone();
    let col_widths_for_move = snap.col_widths.clone();
    let row_count_for_move = snap.row_count;
    let col_count_for_move = snap.col_count;
    let ws_for_cell_move = workspace.clone();
    let ws_for_cell_up = workspace;
    let total_rows = snap.row_count;
    div()
        .relative()
        .child(body)
        .on_mouse_move(move |event, _w, cx| {
            let mouse_x = event.position.x;
            let mouse_y = event.position.y;
            ws_for_row_move.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let Some(model) = w.csv_models.get_mut(&tab_id) else {
                    return;
                };
                if let Some(drag) = model.row_drag.as_mut() {
                    drag.current_mouse_y = mouse_y;
                    let delta_y = mouse_y - drag.start_mouse_y;
                    if f32::from(delta_y).abs() >= DRAG_THRESHOLD_PX {
                        drag.crossed_threshold = true;
                    }
                    if drag.crossed_threshold {
                        let from = drag.primary;
                        let from_center_y = from as f32 * ROW_HEIGHT_PX + ROW_HEIGHT_PX / 2.0;
                        let drag_center = from_center_y + f32::from(delta_y);
                        let mut drop = total_rows.saturating_sub(1);
                        for i in 0..total_rows {
                            let cell_center = i as f32 * ROW_HEIGHT_PX + ROW_HEIGHT_PX / 2.0;
                            if drag_center < cell_center {
                                drop = i;
                                break;
                            }
                        }
                        drag.drop_logical = drop;
                    }
                    cx.notify();
                }
            });
            // Fill-series drag takes priority over the cell-range drag.
            // Window-global so the cursor can leave the body bounds and
            // still continue the gesture (Excel-style — release outside
            // the viewport commits at the last hovered cell).
            let fill_active = ws_for_cell_move.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let fill_active = w
                    .csv_models
                    .get(&tab_id)
                    .map(|m| m.fill_drag.is_some())
                    .unwrap_or(false);
                if fill_active {
                    super::actions::update_fill_drag(w, mouse_x, mouse_y, cx);
                }
                fill_active
            });
            if fill_active {
                return;
            }
            // Cell range drag — separate path. Walks col_widths +
            // ROW_HEIGHT_PX to translate the absolute mouse coord into
            // a (row, col) cell, then extends the live range.
            ws_for_cell_move.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let Some(model) = w.csv_models.get_mut(&tab_id) else {
                    return;
                };
                let Some(drag) = model.cell_range_drag.as_mut() else {
                    return;
                };
                let dx = f32::from(mouse_x - drag.start_mouse_x);
                let dy = f32::from(mouse_y - drag.start_mouse_y);
                if !drag.crossed_threshold
                    && (dx.abs() >= DRAG_THRESHOLD_PX || dy.abs() >= DRAG_THRESHOLD_PX)
                {
                    drag.crossed_threshold = true;
                }
                if drag.crossed_threshold {
                    let anchor = drag.anchor;
                    // Compute target cell from the anchor's screen
                    // position + delta. Anchor cell's centre is at:
                    //   y = anchor_row * ROW_HEIGHT_PX + ROW_HEIGHT_PX/2
                    //   x = sum(col_widths[0..anchor_col]) + col_widths[anchor_col]/2
                    let anchor_y =
                        anchor.0 as f32 * ROW_HEIGHT_PX + ROW_HEIGHT_PX / 2.0;
                    let mut anchor_x = 0.0_f32;
                    for w in col_widths_for_move.iter().take(anchor.1) {
                        anchor_x += f32::from(*w);
                    }
                    if let Some(w) = col_widths_for_move.get(anchor.1) {
                        anchor_x += f32::from(*w) / 2.0;
                    }
                    let target_y = anchor_y + dy;
                    let target_x = anchor_x + dx;
                    let target_row = if target_y < 0.0 {
                        0
                    } else {
                        ((target_y / ROW_HEIGHT_PX) as usize)
                            .min(row_count_for_move.saturating_sub(1))
                    };
                    // Walk col widths cumulatively to find the column
                    // containing target_x.
                    let mut acc = 0.0_f32;
                    let mut target_col = col_count_for_move.saturating_sub(1);
                    for (i, w) in col_widths_for_move.iter().enumerate() {
                        acc += f32::from(*w);
                        if target_x < acc {
                            target_col = i;
                            break;
                        }
                    }
                    if target_x < 0.0 {
                        target_col = 0;
                    }
                    model.extend_cell_range_focus((target_row, target_col));
                    model.active_cell = Some((target_row, target_col));
                    cx.notify();
                }
            });
        })
        .on_mouse_up(MouseButton::Left, move |_event, _w, cx| {
            ws_for_row_up.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let mut emit = false;
                let mut click_row: Option<usize> = None;
                if let Some(model) = w.csv_models.get_mut(&tab_id) {
                    if let Some(drag) = model.row_drag.take() {
                        if !drag.crossed_threshold {
                            click_row = Some(drag.primary);
                        } else {
                            let (_, did_change) = model.move_row_block(
                                &drag.dragged,
                                drag.primary,
                                drag.drop_logical,
                            );
                            if did_change {
                                emit = true;
                            }
                        }
                    }
                }
                if let Some(row) = click_row {
                    super::actions::click_row_select(w, row, false, false, cx);
                }
                if emit {
                    super::actions::emit_to_source(w, cx);
                }
                cx.notify();
            });
            // Fill-series commit takes priority — if the user released
            // mid fill-drag, apply the pattern and skip the regular
            // cell-range mouseup cleanup (a fill drag never coexists
            // with an active cell_range_drag).
            let fill_active = ws_for_cell_up.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let active = w
                    .csv_models
                    .get(&tab_id)
                    .map(|m| m.fill_drag.is_some())
                    .unwrap_or(false);
                if active {
                    super::actions::commit_fill_drag(w, cx);
                }
                active
            });
            if fill_active {
                return;
            }
            // Finalize cell range drag — if the cursor never crossed
            // the drag threshold, discard the range (it was just a
            // click that landed in `on_cell_mousedown`'s click branch).
            ws_for_cell_up.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let Some(model) = w.csv_models.get_mut(&tab_id) else {
                    return;
                };
                if let Some(drag) = model.cell_range_drag.take() {
                    if !drag.crossed_threshold {
                        // No drag movement — keep active_cell (set by
                        // mousedown) but drop the single-cell range so
                        // it doesn't paint a redundant tint.
                        if model
                            .cell_range
                            .map(|r| r.is_single_cell())
                            .unwrap_or(false)
                        {
                            model.cell_range = None;
                        }
                    }
                    cx.notify();
                }
            });
        })
}

fn drop_logical_for_x(
    delta_x: Pixels,
    from: usize,
    col_widths: &[Pixels],
    _total_w: Pixels,
) -> usize {
    let mut cumulative: Vec<f32> = Vec::with_capacity(col_widths.len() + 1);
    cumulative.push(0.0);
    let mut total = 0.0;
    for w in col_widths {
        total += f32::from(*w);
        cumulative.push(total);
    }
    let last = col_widths.len().saturating_sub(1);
    // `from` was captured at mousedown; the width list is from the latest
    // render and can be shorter by now. Without its edges there is no
    // centre to measure from, so the drop stays at the nearest column.
    let (Some(&left), Some(&right)) =
        (cumulative.get(from), cumulative.get(from.saturating_add(1)))
    else {
        return from.min(last);
    };
    let from_center = (left + right) / 2.0;
    let drag_center = from_center + f32::from(delta_x);
    for (i, edges) in cumulative.windows(2).enumerate() {
        let &[l, r] = edges else { continue };
        let cell_center = (l + r) / 2.0;
        if drag_center < cell_center {
            return i;
        }
    }
    last
}

fn column_drop_indicator(theme: &Theme, snap: &CsvGridSnapshot) -> Option<AnyElement> {
    let drag = snap.col_drag.as_ref()?;
    if !drag.crossed_threshold {
        return None;
    }
    // Single-col no-op suppression. Multi-col always renders an
    // indicator because even drop-at-primary collapses the block.
    if drag.dragged.len() == 1 && drag.drop_logical == drag.primary {
        return None;
    }
    let mut x = ROW_GUTTER_WIDTH_PX;
    for (i, w) in snap.col_widths.iter().enumerate() {
        if i >= drag.drop_logical {
            break;
        }
        x += f32::from(*w);
    }
    Some(
        div()
            .absolute()
            .top(px(0.))
            .left(px(x - 1.0))
            .w(px(2.))
            .h_full()
            .bg(rgb(theme.link))
            .into_any_element(),
    )
}

/// Mutate `model.scroll_handle` so the cell at `(row, col)` is visible
/// inside the current viewport. No-op when the viewport hasn't been laid
/// out yet (`bounds().size == 0`) or when the cell is already on-screen.
///
/// The CSV grid renders the column header inside the same scroll
/// container as the body, so a positive Y offset (scrolled-to-top of the
/// content) shows the header. To keep the header visible when scrolling
/// up to an offscreen row, the "above viewport" branch clamps the offset
/// so the cell sits *below* the header band rather than under it.
/// Horizontal scroll mirrors the logic for the row gutter.
///
/// `row == usize::MAX` is the header sentinel — only the column is
/// scrolled into view; vertical offset is left alone.
pub(crate) fn scroll_cell_into_view(model: &CsvModel, row: usize, col: usize) {
    let handle = &model.scroll_handle;
    let viewport = handle.bounds();
    let viewport_h = f32::from(viewport.size.height);
    let viewport_w = f32::from(viewport.size.width);
    if viewport_h <= 0.0 || viewport_w <= 0.0 {
        return;
    }

    let offset = handle.offset();
    let off_y = f32::from(offset.y);
    let off_x = f32::from(offset.x);

    let mut new_off_y = off_y;
    let mut new_off_x = off_x;

    // Vertical — skip when targeting the header row (col is what matters
    // for header find matches). Body coordinates start at Y=0 for row 0
    // since the header sits in a separate frozen strip above the scroller.
    if row != usize::MAX {
        let cell_top = row as f32 * ROW_HEIGHT_PX;
        let cell_bottom = cell_top + ROW_HEIGHT_PX;
        let visible_top = -off_y;
        let visible_bottom = visible_top + viewport_h;

        if cell_top < visible_top {
            new_off_y = (-cell_top).min(0.0);
        } else if cell_bottom > visible_bottom {
            new_off_y = viewport_h - cell_bottom;
        }
    }

    // Horizontal.
    let mut cell_left = ROW_GUTTER_WIDTH_PX;
    for c in 0..col {
        cell_left += f32::from(model.col_width(c));
    }
    let cell_w = f32::from(model.col_width(col));
    let cell_right = cell_left + cell_w;
    let visible_left = -off_x;
    let visible_right = visible_left + viewport_w;

    if cell_left < visible_left + ROW_GUTTER_WIDTH_PX {
        new_off_x = (ROW_GUTTER_WIDTH_PX - cell_left).min(0.0);
    } else if cell_right > visible_right {
        new_off_x = viewport_w - cell_right;
    }

    if (new_off_y - off_y).abs() < 0.5 && (new_off_x - off_x).abs() < 0.5 {
        return;
    }
    handle.set_offset(Point::new(px(new_off_x), px(new_off_y)));
}

/// Bottom-right corner-square that the user grabs to start a fill-series
/// drag. Rendered only when `cell_range` is set and no fill drag is in
/// flight (during the drag itself the preview rect takes over).
///
/// Sized 6×6 — same dimension as RESIZE_HANDLE_WIDTH_PX — and anchored
/// over the source range's bottom-right cell corner. Z-order above the
/// trailing filler and cell content thanks to GPUI's absolute children
/// painting after their relative siblings.
fn fill_handle_overlay(
    theme: &Theme,
    snap: &CsvGridSnapshot,
    workspace: Entity<Workspace>,
) -> Option<AnyElement> {
    if snap.fill_drag.is_some() {
        return None;
    }
    let range = snap.cell_range?;
    if range.max_row() >= snap.row_count || range.max_col() >= snap.col_count {
        return None;
    }
    // Suppress the handle when the range overlaps a hidden row at its
    // bottom edge — the corner anchor would land in invisible space and
    // confuse the user.
    if snap.hidden_rows.contains(&range.max_row()) {
        return None;
    }
    // Body-wrapper coordinates start at the gutter top-left (header lives
    // in a frozen sibling strip, not inside the scroller).
    let mut left_px = ROW_GUTTER_WIDTH_PX;
    for c in 0..=range.max_col() {
        left_px += f32::from(snap.col_widths.get(c).copied().unwrap_or(px(DEFAULT_COL_WIDTH_PX)));
    }
    let top_px = (range.max_row() + 1) as f32 * ROW_HEIGHT_PX;

    let handle_size = 7.0_f32;
    let half = handle_size / 2.0;

    let ws_for_handle = workspace;
    Some(
        div()
            .id("csv-fill-handle")
            .absolute()
            .left(px(left_px - half))
            .top(px(top_px - half))
            .w(px(handle_size))
            .h(px(handle_size))
            .bg(rgb(theme.toggle_active))
            .border_1()
            .border_color(rgb(theme.bg))
            .cursor(CursorStyle::Crosshair)
            .on_mouse_down(MouseButton::Left, move |e, _window, cx| {
                let position = e.position;
                cx.stop_propagation();
                ws_for_handle.update(cx, |w, cx| {
                    super::actions::begin_fill_drag(w, position, cx);
                });
            })
            .into_any_element(),
    )
}

/// Dashed preview rectangle covering `current_target` during a fill
/// drag. The original `cell_range` stays painted underneath because the
/// body cells still tint to `selection_overlay_color` when they're
/// inside the source range; the dashed border shows the extra cells
/// that would be filled on release.
fn fill_preview_overlay(theme: &Theme, snap: &CsvGridSnapshot) -> Option<AnyElement> {
    let state = snap.fill_drag.as_ref()?;
    let target = state.current_target;
    if target.max_row() >= snap.row_count || target.max_col() >= snap.col_count {
        return None;
    }

    let mut left_px = ROW_GUTTER_WIDTH_PX;
    for c in 0..target.min_col() {
        left_px += f32::from(snap.col_widths.get(c).copied().unwrap_or(px(DEFAULT_COL_WIDTH_PX)));
    }
    let mut width_px = 0.0_f32;
    for c in target.min_col()..=target.max_col() {
        width_px += f32::from(snap.col_widths.get(c).copied().unwrap_or(px(DEFAULT_COL_WIDTH_PX)));
    }
    let top_px = target.min_row() as f32 * ROW_HEIGHT_PX;
    let height_px = (target.max_row() + 1 - target.min_row()) as f32 * ROW_HEIGHT_PX;

    Some(
        div()
            .absolute()
            .left(px(left_px))
            .top(px(top_px))
            .w(px(width_px))
            .h(px(height_px))
            .border_2()
            .border_dashed()
            .border_color(rgb(theme.toggle_active))
            .into_any_element(),
    )
}

fn row_drop_indicator(theme: &Theme, snap: &CsvGridSnapshot) -> Option<AnyElement> {
    let drag = snap.row_drag.as_ref()?;
    if !drag.crossed_threshold {
        return None;
    }
    if drag.dragged.len() == 1 && drag.drop_logical == drag.primary {
        return None;
    }
    // Body coords start at row 0 since the header is a frozen sibling
    // outside the scroller — no header offset needed.
    let y = drag.drop_logical as f32 * ROW_HEIGHT_PX;
    Some(
        div()
            .absolute()
            .left(px(0.))
            .top(px(y - 1.0))
            .h(px(2.))
            .w(px(f32::from(snap.total_content_width) + ROW_GUTTER_WIDTH_PX))
            .bg(rgb(theme.link))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_logical_for_x_picks_column_under_drag_center() {
        let widths = [px(100.), px(100.), px(100.)];
        assert_eq!(drop_logical_for_x(px(0.), 0, &widths, px(300.)), 1);
        assert_eq!(drop_logical_for_x(px(120.), 0, &widths, px(300.)), 2);
        assert_eq!(drop_logical_for_x(px(-500.), 2, &widths, px(300.)), 0);
        assert_eq!(drop_logical_for_x(px(500.), 0, &widths, px(300.)), 2);
    }

    #[test]
    fn drop_logical_for_x_with_stale_source_column_does_not_panic() {
        // The drag's source column can outlive the width list it was
        // captured against (columns deleted, empty sheet).
        assert_eq!(drop_logical_for_x(px(10.), 5, &[px(100.)], px(100.)), 0);
        assert_eq!(drop_logical_for_x(px(10.), 1, &[px(100.)], px(100.)), 0);
        assert_eq!(drop_logical_for_x(px(10.), 0, &[], px(0.)), 0);
        assert_eq!(drop_logical_for_x(px(10.), usize::MAX, &[px(1.)], px(1.)), 0);
    }
}
