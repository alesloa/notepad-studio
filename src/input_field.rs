use gpui::{
    App, Bounds, DispatchPhase, ElementId, ElementInputHandler, Entity, FocusHandle,
    GlobalElementId, Hsla, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Rgba, ShapedLine, SharedString, Style, TextRun, Window, fill, point,
    px, relative, rgb, rgba, size,
};

use crate::consts::CARET_WIDTH_PX;
use crate::tab::InputFocus;
use crate::workspace::{InputDrag, Workspace};

/// A minimal text input. Renders the text + a cursor, and registers an
/// IME-aware text input handler against the workspace's matching focus
/// handle so typed characters route into the right `InputFocus` field.
pub(crate) struct InputFieldElement {
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) target: InputFocus,
    pub(crate) placeholder: Option<String>,
}

impl IntoElement for InputFieldElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for InputFieldElement {
    type RequestLayoutState = ();
    /// Prepaint state: (focus handle, shaped line if any, cursor byte,
    /// anchor byte). Anchor is needed to paint the selection rect spanning
    /// anchor..cursor — collapsed selections have anchor == cursor and the
    /// rect skips painting.
    type PrepaintState = (FocusHandle, Option<ShapedLine>, Option<usize>, Option<usize>);

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let workspace = self.workspace.read(cx);
        let theme = workspace.theme();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());

        let (text_str, cursor_byte, anchor_byte) = match self.target {
            InputFocus::FindQuery => (
                workspace.find_state.query.clone(),
                workspace.find_state.query_cursor,
                workspace.find_state.query_anchor,
            ),
            InputFocus::Replacement => (
                workspace.find_state.replacement.clone(),
                workspace.find_state.replacement_cursor,
                workspace.find_state.replacement_anchor,
            ),
            InputFocus::Goto => (
                workspace.goto_state.text.clone(),
                workspace.goto_state.text_cursor,
                workspace.goto_state.text_anchor,
            ),
            InputFocus::FifQuery => (
                workspace.find_in_files.query.clone(),
                workspace.find_in_files.query_cursor,
                workspace.find_in_files.query_anchor,
            ),
            InputFocus::FifReplacement => (
                workspace.find_in_files.replacement.clone(),
                workspace.find_in_files.replacement_cursor,
                workspace.find_in_files.replacement_anchor,
            ),
            InputFocus::FifFilter => (
                workspace.find_in_files.filter.clone(),
                workspace.find_in_files.filter_cursor,
                workspace.find_in_files.filter_anchor,
            ),
            InputFocus::FifDirectory => (
                workspace.find_in_files.directory_text.clone(),
                workspace.find_in_files.directory_text_cursor,
                workspace.find_in_files.directory_text_anchor,
            ),
            InputFocus::MarkQuery => (
                workspace.mark_state.query.clone(),
                workspace.mark_state.query_cursor,
                workspace.mark_state.query_anchor,
            ),
            InputFocus::MacroPrompt => workspace
                .macro_prompt
                .as_ref()
                .map(|s| (s.text.clone(), s.text_cursor, s.text_anchor))
                .unwrap_or((String::new(), 0, 0)),
            InputFocus::TabRename => workspace
                .tab_rename
                .as_ref()
                .map(|s| (s.text.clone(), s.text_cursor, s.text_anchor))
                .unwrap_or((String::new(), 0, 0)),
            InputFocus::FolderInlineCreate => workspace
                .folder_inline_create
                .as_ref()
                .map(|s| (s.text.clone(), s.text_cursor, s.text_anchor))
                .unwrap_or((String::new(), 0, 0)),
            InputFocus::RichDocLinkUrl => workspace
                .rich_doc_link_bar
                .as_ref()
                .map(|s| (s.text.clone(), s.text_cursor, s.text_anchor))
                .unwrap_or((String::new(), 0, 0)),
            InputFocus::RichDocColorHex => workspace
                .rich_doc_color_bar
                .as_ref()
                .map(|s| (s.text.clone(), s.text_cursor, s.text_anchor))
                .unwrap_or((String::new(), 0, 0)),
            // RichDocBody, CsvCell, CsvGrid, and the CSV find-bar all
            // route through Workspace's EntityInputHandler / custom
            // elements directly — they never target a plain InputField.
            InputFocus::Editor
            | InputFocus::RichDocBody
            | InputFocus::CsvCell
            | InputFocus::CsvGrid
            | InputFocus::CsvFindBar
            | InputFocus::CsvReplaceBar => (String::new(), 0, 0),
        };

        let handle = match self.target {
            InputFocus::FindQuery => workspace.find_focus.clone(),
            InputFocus::Replacement => workspace.replace_focus.clone(),
            InputFocus::Goto => workspace.goto_focus.clone(),
            InputFocus::FifQuery => workspace.fif_query_focus.clone(),
            InputFocus::FifReplacement => workspace
                .fif_replacement_focus
                .clone()
                .unwrap_or_else(|| workspace.focus_handle.clone()),
            InputFocus::FifFilter => workspace.fif_filter_focus.clone(),
            InputFocus::FifDirectory => workspace
                .fif_directory_focus
                .clone()
                .unwrap_or_else(|| workspace.focus_handle.clone()),
            InputFocus::MarkQuery => workspace
                .mark_focus
                .clone()
                .unwrap_or_else(|| workspace.focus_handle.clone()),
            InputFocus::MacroPrompt => workspace
                .macro_focus
                .clone()
                .unwrap_or_else(|| workspace.focus_handle.clone()),
            InputFocus::TabRename => workspace
                .tab_rename_focus
                .clone()
                .unwrap_or_else(|| workspace.focus_handle.clone()),
            InputFocus::FolderInlineCreate => workspace
                .folder_inline_create_focus
                .clone()
                .unwrap_or_else(|| workspace.focus_handle.clone()),
            InputFocus::RichDocLinkUrl => workspace
                .rich_doc_link_focus
                .clone()
                .unwrap_or_else(|| workspace.focus_handle.clone()),
            InputFocus::RichDocColorHex => workspace
                .rich_doc_color_focus
                .clone()
                .unwrap_or_else(|| workspace.focus_handle.clone()),
            InputFocus::Editor
            | InputFocus::RichDocBody
            | InputFocus::CsvCell
            | InputFocus::CsvGrid
            | InputFocus::CsvFindBar
            | InputFocus::CsvReplaceBar => workspace.focus_handle.clone(),
        };

        // If empty and a placeholder is set, paint that in a dimmed color.
        let display: SharedString = if text_str.is_empty() {
            self.placeholder.clone().unwrap_or_default().into()
        } else {
            text_str.clone().into()
        };
        let display_color: Hsla = if text_str.is_empty() {
            // Approximate gutter_text — placeholder shouldn't shout.
            Rgba::from(rgb(theme.gutter_text)).into()
        } else {
            Rgba::from(rgb(theme.text)).into()
        };

        let runs = if display.is_empty() {
            Vec::new()
        } else {
            vec![TextRun {
                len: display.len(),
                font: style.font(),
                color: display_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }]
        };

        let shaped = if display.is_empty() {
            None
        } else {
            Some(window.text_system().shape_line(
                display.clone(),
                font_size,
                &runs,
                None,
            ))
        };

        let cursor_local = if text_str.is_empty() {
            Some(0usize)
        } else {
            Some(cursor_byte.min(text_str.len()))
        };
        let anchor_local = if text_str.is_empty() {
            Some(0usize)
        } else {
            Some(anchor_byte.min(text_str.len()))
        };

        let _ = bounds;
        (handle, shaped, cursor_local, anchor_local)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (handle, shaped, cursor_local, anchor_local) = prepaint;
        let line_height = window.line_height();

        // Vertically center the text in the field.
        let baseline_y = bounds.top() + (bounds.size.height - line_height).max(px(0.)) / 2.0;

        // Re-shape the current text so we can compute x positions for both
        // the selection rect and the caret without threading the ShapedLine
        // through the prepaint tuple. shape_line is cheap for a single-line
        // input — far less work than the dispatch overhead saved.
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let theme = self.workspace.read(cx).theme();
        let text_str = match self.target {
            InputFocus::FindQuery => self.workspace.read(cx).find_state.query.clone(),
            InputFocus::Replacement => {
                self.workspace.read(cx).find_state.replacement.clone()
            }
            InputFocus::Goto => self.workspace.read(cx).goto_state.text.clone(),
            InputFocus::FifQuery => self.workspace.read(cx).find_in_files.query.clone(),
            InputFocus::FifReplacement => {
                self.workspace.read(cx).find_in_files.replacement.clone()
            }
            InputFocus::FifFilter => self.workspace.read(cx).find_in_files.filter.clone(),
            InputFocus::FifDirectory => {
                self.workspace.read(cx).find_in_files.directory_text.clone()
            }
            InputFocus::MarkQuery => self.workspace.read(cx).mark_state.query.clone(),
            InputFocus::MacroPrompt => self
                .workspace
                .read(cx)
                .macro_prompt
                .as_ref()
                .map(|s| s.text.clone())
                .unwrap_or_default(),
            InputFocus::TabRename => self
                .workspace
                .read(cx)
                .tab_rename
                .as_ref()
                .map(|s| s.text.clone())
                .unwrap_or_default(),
            InputFocus::FolderInlineCreate => self
                .workspace
                .read(cx)
                .folder_inline_create
                .as_ref()
                .map(|s| s.text.clone())
                .unwrap_or_default(),
            InputFocus::RichDocLinkUrl => self
                .workspace
                .read(cx)
                .rich_doc_link_bar
                .as_ref()
                .map(|s| s.text.clone())
                .unwrap_or_default(),
            InputFocus::RichDocColorHex => self
                .workspace
                .read(cx)
                .rich_doc_color_bar
                .as_ref()
                .map(|s| s.text.clone())
                .unwrap_or_default(),
            InputFocus::Editor
            | InputFocus::RichDocBody
            | InputFocus::CsvCell
            | InputFocus::CsvGrid
            | InputFocus::CsvFindBar
            | InputFocus::CsvReplaceBar => String::new(),
        };
        let display: SharedString = text_str.clone().into();
        let measure_line = if display.is_empty() {
            None
        } else {
            let runs = vec![TextRun {
                len: display.len(),
                font: style.font(),
                color: Rgba::from(rgb(theme.text)).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            Some(
                window
                    .text_system()
                    .shape_line(display.clone(), font_size, &runs, None),
            )
        };

        // Horizontal scroll: if the caret would land past the right edge,
        // shift the painted text left so the caret stays visible. Closures
        // capture this so click coordinates resolve back to the correct
        // byte under whatever's currently on screen.
        let usable_w = bounds.size.width.max(px(1.));
        let scroll_pad = px(4.);
        let cursor_x_for_scroll = if let Some(line2) = measure_line.as_ref() {
            let cur = cursor_local.unwrap_or(0).min(line2.len);
            line2.x_for_index(cur)
        } else {
            px(0.)
        };
        let h_scroll = if cursor_x_for_scroll > usable_w - scroll_pad {
            cursor_x_for_scroll - usable_w + scroll_pad
        } else {
            px(0.)
        };

        // 1. Paint the selection rect first so the text glyphs render on top
        //    of it. Only when the input is focused — we don't want stale
        //    selection bg painted in unfocused fields.
        if handle.is_focused(window) {
            if let (Some(cur), Some(anchor)) = (*cursor_local, *anchor_local) {
                if cur != anchor {
                    if let Some(line2) = measure_line.as_ref() {
                        let lo = cur.min(anchor).min(line2.len);
                        let hi = cur.max(anchor).min(line2.len);
                        if lo < hi {
                            let x_lo = bounds.left() + line2.x_for_index(lo) - h_scroll;
                            let x_hi = bounds.left() + line2.x_for_index(hi) - h_scroll;
                            // Reuse `theme.selection` — same colour as the
                            // editor pane's selection — so the dialog input
                            // matches the editor's visual language.
                            let sel_color: Hsla = rgba(theme.selection).into();
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(x_lo, baseline_y),
                                    size(x_hi - x_lo, line_height),
                                ),
                                sel_color,
                            ));
                        }
                    }
                }
            }
        }

        // 2. Paint the text glyphs (placeholder text or real text).
        if let Some(shaped) = shaped.take() {
            let _ = shaped.paint(
                point(bounds.left() - h_scroll, baseline_y),
                line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        }

        // 3. Caret on top.
        if handle.is_focused(window) {
            if let Some(cur) = *cursor_local {
                let caret_color: Hsla = Rgba::from(rgb(theme.text)).into();
                let cx_pos = if let Some(line2) = measure_line.as_ref() {
                    let cur = cur.min(line2.len);
                    bounds.left() + line2.x_for_index(cur) - h_scroll
                } else {
                    bounds.left()
                };
                window.paint_quad(fill(
                    Bounds::new(
                        point(cx_pos, baseline_y),
                        size(px(CARET_WIDTH_PX), line_height),
                    ),
                    caret_color,
                ));
            }
        }

        // 4. Route IME / OS keyboard text into the workspace via the matching
        //    focus handle. gpui only fires the input handler whose handle is
        //    currently focused, so this is safe to register every frame.
        window.handle_input(
            handle,
            ElementInputHandler::new(bounds, self.workspace.clone()),
            cx,
        );

        // 5. Mouse: click + drag = caret + selection. The mouse-down listener
        //    is element-local (only fires on hits inside `bounds`); the move
        //    + up listeners are window-level so a drag that wanders outside
        //    the field still updates the selection (mirrors the editor pane).
        //    All three listeners are re-registered every paint, so they
        //    don't accumulate across frames.
        let target = self.target;
        let click_workspace = self.workspace.clone();
        let click_bounds_left = bounds.left();
        let click_text = text_str.clone();
        let style_for_listener = style.clone();
        let font_size_for_listener = font_size;
        let click_h_scroll = h_scroll;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            // Double-click (or beyond) selects the entire field — matches
            // native text-field behaviour. Skip seeding `input_drag` so a
            // hold-and-drag afterwards doesn't mangle the selection.
            if event.click_count >= 2 {
                click_workspace.update(cx, |w, cx| {
                    w.input_set_caret_byte(target, 0, false);
                    w.input_set_caret_byte(target, click_text.len(), true);
                    cx.notify();
                });
                return;
            }
            let extend = event.modifiers.shift;
            click_workspace.update(cx, |w, cx| {
                let byte = byte_for_x(
                    &click_text,
                    event.position.x - click_bounds_left + click_h_scroll,
                    font_size_for_listener,
                    &style_for_listener,
                    window,
                );
                w.input_set_caret_byte(target, byte, extend);
                w.input_drag = Some(InputDrag {
                    focus: target,
                    bounds_left: click_bounds_left,
                });
                cx.notify();
            });
        });

        let move_workspace = self.workspace.clone();
        let move_text = text_str.clone();
        let style_for_move = style.clone();
        let font_size_for_move = font_size;
        let move_h_scroll = h_scroll;
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            // Only extend on a real drag — a sub-pixel nudge between
            // mouse-down and mouse-up shouldn't grow the selection.
            if !event.dragging() {
                return;
            }
            move_workspace.update(cx, |w, cx| {
                let Some(drag) = w.input_drag else {
                    return;
                };
                if drag.focus != target {
                    return;
                }
                let byte = byte_for_x(
                    &move_text,
                    event.position.x - drag.bounds_left + move_h_scroll,
                    font_size_for_move,
                    &style_for_move,
                    window,
                );
                // Drag = extend. Only `cursor` moves; `anchor` stays seated
                // at the click origin so the user can drag in either
                // direction.
                w.input_set_caret_byte(target, byte, true);
                cx.notify();
            });
        });

        let up_workspace = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            up_workspace.update(cx, |w, cx| {
                if w.input_drag.is_some() {
                    w.input_drag = None;
                    cx.notify();
                }
            });
        });
    }
}

/// Translate a window-relative x offset into a byte index inside `text_str`.
/// Used by both the click and drag mouse listeners. Empty inputs always
/// resolve to 0; anything past the end clamps to the string's byte length.
fn byte_for_x(
    text_str: &str,
    x_offset: Pixels,
    font_size: Pixels,
    style: &gpui::TextStyle,
    window: &mut Window,
) -> usize {
    if text_str.is_empty() {
        return 0;
    }
    let display: SharedString = text_str.to_string().into();
    let runs = vec![TextRun {
        len: display.len(),
        font: style.font(),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let line = window
        .text_system()
        .shape_line(display, font_size, &runs, None);
    let local_x = if x_offset < px(0.) { px(0.) } else { x_offset };
    line.closest_index_for_x(local_x).min(line.len)
}

