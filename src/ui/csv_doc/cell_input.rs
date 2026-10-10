// CSV cell text-input element.
//
// Mirrors the `HexInput` Element pattern from
// `src/ui/style_configurator/colour_picker.rs`: the Element calls
// `window.handle_input(&handle, ElementInputHandler::new(bounds,
// workspace), cx)` in `paint` so IME / paste / typed chars route into
// the active tab's `CsvModel::cell_input.buffer` via
// `Workspace::EntityInputHandler`. Mouse-down / move / up are
// window-global so click+drag selection tracks past the cell's bounds,
// matching how every other text input in the app behaves.
//
// One CRITICAL difference from HexInput: this Element does NOT own its
// own focus handle. The caller (`grid.rs`) wraps the Element in a div
// that calls `.id("csv-cell-{r}-{c}").track_focus(&handle)`. The handle
// itself lives on the workspace's `CsvModel::cell_input` so the focus
// id is stable across the editing-cell's lifetime — anchoring the focus
// to a dedicated dispatch-tree node avoids the GPUI focus-handle trap
// where a shared node silently drops `handle_input` registration.

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, ElementInputHandler, FocusHandle, Hsla,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Rgba, SharedString, StyleRefinement, Styled, TextRun, WeakEntity, Window, div, fill, point,
    prelude::*, px, rgb, rgba, size,
};

use crate::consts::CARET_WIDTH_PX;
use crate::theme::Theme;
use crate::workspace::Workspace;

/// Insets inside the cell bounds — the input glyphs are nudged in so they
/// line up with the static cell text underneath them when the user opens
/// the editor.
const CELL_TEXT_PADDING_X: f32 = 6.0;

/// Render a CSV cell as an editable text input. The wrapper div lives on
/// the caller — this returns the inner `IntoElement` (the custom
/// Element that owns paint + mouse handling).
/// Which `CsvCellInput` slot on the model this element edits — the
/// in-grid cell editor or the inline find-bar query input. Both share
/// the same mouse / paint / IME plumbing; only the buffer source
/// differs. Keeps the find bar from reinventing 250 lines of element
/// code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellInputTarget {
    Cell,
    FindBar,
    ReplaceBar,
}

pub(crate) fn cell_input_element(
    theme: &Theme,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
) -> CsvCellInputElement {
    CsvCellInputElement {
        workspace,
        focus_handle,
        text_color: theme.text,
        selection_color: theme.selection,
        caret_color: theme.text,
        editing_bg: theme.chrome,
        target: CellInputTarget::Cell,
    }
}

/// Variant that reads/writes `model.find_input` instead of
/// `model.cell_input`. Same Element type — see [`CellInputTarget`].
pub(crate) fn find_input_element(
    theme: &Theme,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
) -> CsvCellInputElement {
    CsvCellInputElement {
        workspace,
        focus_handle,
        text_color: theme.text,
        selection_color: theme.selection,
        caret_color: theme.text,
        editing_bg: theme.bg,
        target: CellInputTarget::FindBar,
    }
}

/// Variant that reads/writes `model.replace_input`. Same plumbing as
/// the find input — only the buffer target differs.
pub(crate) fn replace_input_element(
    theme: &Theme,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
) -> CsvCellInputElement {
    CsvCellInputElement {
        workspace,
        focus_handle,
        text_color: theme.text,
        selection_color: theme.selection,
        caret_color: theme.text,
        editing_bg: theme.bg,
        target: CellInputTarget::ReplaceBar,
    }
}

/// Build the wrapper div around the input Element. The caller (grid
/// renderer) hands us the cell's logical (row, col) so the wrapper has
/// a stable `id` per cell — required so `track_focus` lands a
/// dedicated dispatch-tree node per editor instance instead of
/// stamping the same node every paint (GPUI focus-handle caveat).
pub(crate) fn cell_input_wrapper(
    theme: &Theme,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    row: usize,
    col: usize,
    _width: Pixels,
    _height: Pixels,
) -> gpui::Stateful<gpui::Div> {
    let id: SharedString = SharedString::from(format!("csv-cell-{row}-{col}"));
    // Fill the outer cell div instead of pinning to explicit ROW_HEIGHT_PX —
    // the row's `border_b_1` consumes 1px of the row's inner content area in
    // border-box layout, so an h-pinned wrapper overflows by 1px and the
    // bottom focus border lands on/over the row separator, looking clipped.
    // h_full / w_full matches the static cell's sizing.
    div()
        .id(id)
        .track_focus(&focus_handle)
        .cursor(CursorStyle::IBeam)
        .w_full()
        .h_full()
        .bg(rgb(theme.chrome))
        .border_2()
        .border_color(rgb(theme.link))
        .child(cell_input_element(theme, workspace, focus_handle))
}

pub(crate) struct CsvCellInputElement {
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) text_color: u32,
    pub(crate) selection_color: u32,
    pub(crate) caret_color: u32,
    pub(crate) editing_bg: u32,
    pub(crate) target: CellInputTarget,
}

impl IntoElement for CsvCellInputElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

pub(crate) struct CellPrepaint {
    pub(crate) buffer: String,
    pub(crate) cursor: usize,
    pub(crate) anchor: usize,
    pub(crate) shaped: Option<gpui::ShapedLine>,
}

impl gpui::Element for CsvCellInputElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<CellPrepaint>;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let workspace = self.workspace.upgrade()?;
        let target = self.target;
        let (buffer, cursor, anchor) = workspace.read_with(cx, |w, _cx| {
            let tab = &w.tabs[w.active];
            let model = w.csv_models.get(&tab.session_id)?;
            let input = match target {
                CellInputTarget::Cell => model.cell_input.as_ref()?,
                CellInputTarget::FindBar => model.find_input.as_ref()?,
                CellInputTarget::ReplaceBar => model.replace_input.as_ref()?,
            };
            Some((
                input.buffer.clone(),
                input.cursor.min(input.buffer.len()),
                input.anchor.min(input.buffer.len()),
            ))
        })?;

        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let color: Hsla = Rgba::from(rgb(self.text_color)).into();
        let runs = vec![TextRun {
            len: buffer.len().max(1),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let display_text = if buffer.is_empty() {
            String::from(" ")
        } else {
            single_line_display(&buffer)
        };
        let shaped = Some(window.text_system().shape_line(
            SharedString::from(display_text),
            font_size,
            &runs,
            None,
        ));

        Some(CellPrepaint {
            buffer,
            cursor,
            anchor,
            shaped,
        })
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let Some(CellPrepaint {
            buffer,
            cursor,
            anchor,
            shaped,
        }) = prepaint.as_mut()
        else {
            return;
        };

        // Editing-cell background (in case the wrapper div didn't paint).
        let bg: Hsla = Rgba::from(rgb(self.editing_bg)).into();
        window.paint_quad(fill(bounds, bg));

        let line_height = window.line_height();
        let text_left = bounds.left() + px(CELL_TEXT_PADDING_X);
        let baseline_y = bounds.top() + (bounds.size.height - line_height).max(px(0.)) / 2.0;

        let lo = (*cursor).min(*anchor);
        let hi = (*cursor).max(*anchor);
        if hi > lo && !buffer.is_empty() {
            if let Some(line) = shaped.as_ref() {
                let lo_disp = lo.min(line.len);
                let hi_disp = hi.min(line.len);
                let x_lo = text_left + line.x_for_index(lo_disp);
                let x_hi = text_left + line.x_for_index(hi_disp);
                let sel_color: Hsla = rgba(self.selection_color).into();
                window.paint_quad(fill(
                    Bounds::new(point(x_lo, baseline_y), size(x_hi - x_lo, line_height)),
                    sel_color,
                ));
            }
        }

        if let Some(line) = shaped.take() {
            let _ = line.paint(
                point(text_left, baseline_y),
                line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        }

        if self.focus_handle.is_focused(window) {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let caret_color: Hsla = Rgba::from(rgb(self.caret_color)).into();
            let runs = vec![TextRun {
                len: buffer.len().max(1),
                font: style.font(),
                color: caret_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let display_text = if buffer.is_empty() {
                String::from(" ")
            } else {
                single_line_display(buffer)
            };
            let line = window.text_system().shape_line(
                SharedString::from(display_text),
                font_size,
                &runs,
                None,
            );
            let cx_idx = if buffer.is_empty() { 0 } else { (*cursor).min(line.len) };
            let cx_pos = text_left + line.x_for_index(cx_idx);
            window.paint_quad(fill(
                Bounds::new(
                    point(cx_pos, baseline_y),
                    size(px(CARET_WIDTH_PX), line_height),
                ),
                caret_color,
            ));
        }

        // IME / OS text input router. Without this call, typed chars
        // and paste do nothing.
        window.handle_input(
            &self.focus_handle,
            ElementInputHandler::new(bounds, workspace.clone()),
            cx,
        );

        // Mouse-down: focus the cell input and place cursor at the
        // clicked byte. Double-click selects entire buffer.
        let ws_for_down = workspace.clone();
        let focus_for_down = self.focus_handle.clone();
        let target_for_down = self.target;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            cx.stop_propagation();
            let click_count = event.click_count;
            window.focus(&focus_for_down, cx);
            let click_x_local = event.position.x - bounds.left() - px(CELL_TEXT_PADDING_X);
            ws_for_down.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let Some(model) = w.csv_models.get_mut(&tab_id) else {
                    return;
                };
                let Some(input) = (match target_for_down {
                    CellInputTarget::Cell => model.cell_input.as_mut(),
                    CellInputTarget::FindBar => model.find_input.as_mut(),
                    CellInputTarget::ReplaceBar => model.replace_input.as_mut(),
                }) else {
                    return;
                };
                if click_count >= 2 {
                    input.anchor = 0;
                    input.cursor = input.buffer.len();
                    input.drag_active = false;
                } else {
                    let buf_byte = byte_for_x_in_text(&input.buffer, click_x_local, window);
                    input.cursor = buf_byte;
                    input.anchor = buf_byte;
                    input.drag_active = true;
                }
                cx.notify();
            });
        });

        // Mouse-move while dragging — extend selection to current x.
        let ws_for_move = workspace.clone();
        let target_for_move = self.target;
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if !event.dragging() {
                return;
            }
            ws_for_move.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                let Some(model) = w.csv_models.get_mut(&tab_id) else {
                    return;
                };
                let Some(input) = (match target_for_move {
                    CellInputTarget::Cell => model.cell_input.as_mut(),
                    CellInputTarget::FindBar => model.find_input.as_mut(),
                    CellInputTarget::ReplaceBar => model.replace_input.as_mut(),
                }) else {
                    return;
                };
                if !input.drag_active {
                    return;
                }
                let click_x_local =
                    event.position.x - bounds.left() - px(CELL_TEXT_PADDING_X);
                let buf_byte = byte_for_x_in_text(&input.buffer, click_x_local, window);
                if input.cursor != buf_byte {
                    input.cursor = buf_byte;
                    cx.notify();
                }
            });
        });

        // Mouse-up — close drag.
        let ws_for_up = workspace.clone();
        let target_for_up = self.target;
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            ws_for_up.update(cx, |w, _cx| {
                let tab_id = w.tabs[w.active].session_id;
                let Some(model) = w.csv_models.get_mut(&tab_id) else {
                    return;
                };
                let Some(input) = (match target_for_up {
                    CellInputTarget::Cell => model.cell_input.as_mut(),
                    CellInputTarget::FindBar => model.find_input.as_mut(),
                    CellInputTarget::ReplaceBar => model.replace_input.as_mut(),
                }) else {
                    return;
                };
                input.drag_active = false;
            });
        });
    }
}

/// Map a click x-offset (relative to the text baseline left) to a byte
/// position inside `text`. Returns 0 on click before the first char,
/// `len` on click past the end.
fn byte_for_x_in_text(text: &str, x: Pixels, window: &mut Window) -> usize {
    if text.is_empty() || x <= px(0.) {
        return 0;
    }
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let runs = vec![TextRun {
        len: text.len(),
        font: style.font(),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let line = window.text_system().shape_line(
        SharedString::from(single_line_display(text)),
        font_size,
        &runs,
        None,
    );
    if x >= line.width {
        return text.len();
    }
    line.index_for_x(x).unwrap_or(text.len())
}

/// `shape_line` takes a single line and debug-asserts it holds no `\n`,
/// but a quoted CSV field can carry line breaks (`"line one\nline two"`).
/// Show each `\n` and `\r` as a space: all three are one byte, so the
/// cursor and selection byte offsets into the buffer still index the
/// shaped line.
pub(crate) fn single_line_display(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// Helper used by the wrapper div builder above to keep external paths
/// simple — exposes `StyleRefinement` so callers can add minor tweaks
/// without re-implementing the wrapper's defaults.
#[allow(dead_code)]
pub(crate) fn default_wrapper_style(theme: &Theme) -> StyleRefinement {
    let mut s = StyleRefinement::default();
    s.background = Some(gpui::Fill::Color(Rgba::from(rgb(theme.chrome)).into()));
    s
}

#[cfg(test)]
mod tests {
    use super::single_line_display;

    #[test]
    fn single_line_display_drops_line_breaks_and_keeps_offsets() {
        let cell = "é\r\n中\n👩‍👩‍👧\rend";
        let shown = single_line_display(cell);
        assert!(!shown.contains(['\n', '\r']));
        assert_eq!(shown.len(), cell.len());
        assert_eq!(shown, "é  中 👩‍👩‍👧 end");
        for (i, _) in cell.char_indices() {
            assert!(shown.is_char_boundary(i), "offset {i} must stay a char boundary");
        }
    }
}
