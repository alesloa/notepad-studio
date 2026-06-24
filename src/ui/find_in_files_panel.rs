// Find-in-Files results panel — single status row + NPP-style grouped
// results list. The Find dialog owns query/filter/options entry, so the
// panel itself is purely a viewer (+ a close button).

use gpui::{
    Bounds, CursorStyle, DispatchPhase, Element, ElementId, Entity, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, StatefulInteractiveElement, Style,
    Styled, WeakEntity, Window, div, prelude::*, px, rgb,
};

use crate::actions::FindInFilesClose;
use crate::find_in_files::FindInFilesState;
use crate::theme::Theme;
use crate::workspace::{FIF_PANEL_HANDLE_HEIGHT, FifPanelDrag, Workspace};

const ROW_HEIGHT: f32 = 22.0;

pub(crate) fn find_in_files_panel(
    theme: &Theme,
    state: &FindInFilesState,
    workspace: Entity<Workspace>,
    height: Pixels,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .h(height)
        .bg(rgb(theme.chrome))
        .border_t_1()
        .border_color(rgb(theme.border))
        .child(status_row(theme, state, workspace.clone()))
        .child(results_list(theme, state, workspace))
}

/// Splitter handle painted just above the Find-in-Files results panel.
/// Mirrors `PreviewPaneResizeHandle`: a thin chrome strip with a top
/// border, registered as a hitbox so the cursor switches to ResizeUpDown
/// and so window-global mouse handlers can drive the drag.
pub(crate) struct FifPanelResizeHandle {
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) theme_chrome: u32,
    pub(crate) theme_border: u32,
}

impl IntoElement for FifPanelResizeHandle {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FifPanelResizeHandle {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = px(FIF_PANEL_HANDLE_HEIGHT).into();
        style.flex_shrink = 0.;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) {
        window.set_cursor_style(CursorStyle::ResizeUpDown, hitbox);
        // Chrome-coloured strip with a single-pixel separator line at the
        // top so the splitter visually butts up against the editor pane.
        window.paint_quad(gpui::fill(bounds, rgb(self.theme_chrome)));
        let top_line = Bounds::new(bounds.origin, gpui::size(bounds.size.width, px(1.)));
        window.paint_quad(gpui::fill(top_line, rgb(self.theme_border)));

        // Small grip — a short horizontal bar centred in the handle so the
        // user can see there's something to drag. Without this the handle
        // reads as a plain border and people miss it.
        let grip_w = px(40.);
        let grip_h = px(2.);
        let cx_x = bounds.origin.x + (bounds.size.width / 2.) - (grip_w / 2.);
        let cy_y = bounds.origin.y + (bounds.size.height / 2.) - (grip_h / 2.);
        let grip = Bounds::new(gpui::point(cx_x, cy_y), gpui::size(grip_w, grip_h));
        window.paint_quad(gpui::fill(grip, rgb(self.theme_border)));

        let viewport_h = window.viewport_size().height;

        let ws_for_down = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            let _ = ws_for_down.update(cx, |w, cx| {
                w.fif_panel_drag = Some(FifPanelDrag {
                    start_mouse_y: event.position.y,
                    start_height: w.fif_panel_height,
                });
                cx.notify();
            });
        });

        let ws_for_move = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let _ = ws_for_move.update(cx, |w, cx| {
                let Some(drag) = w.fif_panel_drag else {
                    return;
                };
                // Drag UP → cursor.y decreases → panel grows. New height =
                // start_height + (start_y - current_y).
                let dy = drag.start_mouse_y - event.position.y;
                let next = drag.start_height + dy;
                w.set_fif_panel_height(next, viewport_h, cx);
            });
        });

        let ws_for_up = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            let _ = ws_for_up.update(cx, |w, cx| {
                if w.fif_panel_drag.is_some() {
                    w.fif_panel_drag = None;
                    cx.notify();
                }
            });
        });
    }
}

fn status_row(
    theme: &Theme,
    state: &FindInFilesState,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    // NPP's docked-panel title bar reads `Search results - (N hits)` with a
    // small × close button at the right edge. Errors override the title so
    // the user sees the bad-regex / missing-directory message; everything
    // else is derived from the latest result set.
    let total = state.results.len();
    let (text, color) = if let Some(err) = &state.error {
        (err.clone(), theme.error)
    } else if state.searching {
        ("Searching…".to_string(), theme.chrome_text)
    } else {
        (
            format!("Search results - ({total} hit{})", if total == 1 { "" } else { "s" }),
            theme.chrome_text,
        )
    };
    let close_ws = workspace.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(20.))
        .px(px(8.))
        .bg(rgb(theme.chrome))
        .border_b_1()
        .border_color(rgb(theme.border))
        .text_size(px(11.))
        .text_color(rgb(color))
        .child(div().flex_grow().child(text))
        .child(
            div()
                .id("fif-panel-close")
                .h(px(18.))
                .w(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(rgb(theme.chrome_text))
                .cursor(CursorStyle::PointingHand)
                .child("×")
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    close_ws.update(cx, |w, cx| {
                        w.close_find_in_files(&FindInFilesClose, window, cx)
                    });
                }),
        )
}

fn results_list(
    theme: &Theme,
    state: &FindInFilesState,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let mut list = div()
        .id("find-in-files-results")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .bg(rgb(theme.bg));

    if state.results.is_empty() {
        let placeholder = if state.searching {
            "Searching…"
        } else if state.error.is_some() {
            ""
        } else if state.status.is_some() {
            "No matches"
        } else {
            "Type a query and click Find All"
        };
        if !placeholder.is_empty() {
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(11.))
                    .text_color(rgb(theme.gutter_text))
                    .child(placeholder),
            );
        }
        return list;
    }

    // Top-of-list header — NPP's "Search results - (N hits)" + summary.
    list = list.child(results_summary_header(theme, state));

    // Group consecutive results by the file they came from. `results` is
    // already in path/tab order from each source (find_all_current is one
    // file; find_all_in_all_opened iterates tabs in order; the FIF engine
    // walks `WalkDir` deterministically). Streaming the group break that way
    // avoids an allocation-heavy regroup.
    let mut current_key: Option<String> = None;
    let mut current_buffer: Vec<(usize, &crate::find_in_files::FindResult)> = Vec::new();
    let mut groups: Vec<(String, Vec<(usize, &crate::find_in_files::FindResult)>)> = Vec::new();
    for (idx, r) in state.results.iter().enumerate() {
        let key = group_key(r);
        if current_key.as_ref().map(|k| k != &key).unwrap_or(true) {
            if let Some(prev) = current_key.take() {
                groups.push((prev, std::mem::take(&mut current_buffer)));
            }
            current_key = Some(key);
        }
        current_buffer.push((idx, r));
    }
    if let Some(prev) = current_key.take() {
        groups.push((prev, current_buffer));
    }

    for (key, rows) in groups {
        list = list.child(file_group_header(theme, &key, rows.len()));
        for (idx, r) in rows {
            list = list.child(result_row(theme, idx, r, workspace.clone()));
        }
    }
    if state.truncated {
        list = list.child(
            div()
                .px(px(12.))
                .py(px(6.))
                .text_size(px(11.))
                .text_color(rgb(theme.gutter_text))
                .child("(truncated at 1000 — refine your search)"),
        );
    }

    list
}

fn group_key(r: &crate::find_in_files::FindResult) -> String {
    if let Some(label) = &r.display_label {
        label.clone()
    } else {
        r.path.display().to_string()
    }
}

fn results_summary_header(theme: &Theme, state: &FindInFilesState) -> impl IntoElement {
    let total = state.results.len();
    // Count distinct files (group_key collisions = same file).
    let mut seen: Vec<String> = Vec::new();
    for r in &state.results {
        let k = group_key(r);
        if !seen.contains(&k) {
            seen.push(k);
        }
    }
    let files_with_hits = seen.len();
    let files_searched = state.last_search_files_searched.max(files_with_hits);
    let query_quoted = format!("\"{}\"", state.last_search_query);
    let mode = if state.last_search_mode.is_empty() {
        "Normal".to_string()
    } else {
        state.last_search_mode.clone()
    };
    // NPP's body summary — the docked title bar already shows the
    // `Search results - (N hits)` line, so we only need the second line
    // here: `Search "query" (N hits in M files of K searched) [Mode]`.
    let summary = format!(
        "Search {query_quoted} ({total} hit{} in {files_with_hits} file{} of {files_searched} searched) [{mode}]",
        if total == 1 { "" } else { "s" },
        if files_with_hits == 1 { "" } else { "s" },
    );
    div()
        .px(px(8.))
        .py(px(2.))
        .text_size(px(11.))
        .text_color(rgb(theme.text))
        .child(summary)
}

fn file_group_header(theme: &Theme, label: &str, hits: usize) -> impl IntoElement {
    let text = format!("{label} ({hits} hit{})", if hits == 1 { "" } else { "s" });
    div()
        .pl(px(12.))
        .pr(px(8.))
        .py(px(2.))
        .text_size(px(11.))
        .text_color(rgb(theme.toggle_active))
        .child(text)
}

fn result_row(
    theme: &Theme,
    idx: usize,
    result: &crate::find_in_files::FindResult,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    // NPP paints the row in three styled chunks: the literal word `Line`
    // (subdued chrome colour), the right-aligned line number (syntax-number
    // accent, italic-feeling), then the snippet text. Keep them as separate
    // children so each can carry its own colour.
    let number_text = format!("{:>6}", result.line + 1);

    let (pre, hit, post) = split_match(&result.line_text, &result.match_in_line);

    let click_idx = idx;
    let click_ws = workspace.clone();

    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(ROW_HEIGHT))
        .pl(px(36.))
        .pr(px(8.))
        .text_size(px(11.))
        .cursor(CursorStyle::PointingHand)
        .text_color(rgb(theme.text))
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(theme.chrome_text))
                .child("Line"),
        )
        .child(
            div()
                .flex_shrink_0()
                .pl(px(4.))
                .text_color(rgb(theme.syntax.number))
                .child(number_text),
        )
        .child(
            div()
                .flex_shrink_0()
                .pr(px(8.))
                .text_color(rgb(theme.chrome_text))
                .child(":"),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(div().text_color(rgb(theme.text)).child(pre))
                .child(
                    div()
                        .bg(rgb(theme.match_highlight))
                        .text_color(rgb(theme.bg))
                        .child(hit),
                )
                .child(div().text_color(rgb(theme.text)).child(post)),
        )
        .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
            click_ws.update(cx, |w, cx| w.jump_to_find_result(click_idx, cx));
        })
}

/// Split a line into the text before the match, the match itself, and
/// the text after. The whole line is trimmed of any leading whitespace
/// uniformly (so very-indented lines don't push the match off-screen),
/// preserving the relative offsets.
fn split_match(line: &str, span: &std::ops::Range<usize>) -> (String, String, String) {
    let lead = line.len() - line.trim_start().len();
    let trimmed = &line[lead..];
    let s = span.start.saturating_sub(lead).min(trimmed.len());
    let e = span.end.saturating_sub(lead).min(trimmed.len()).max(s);
    let pre = trimmed[..s].to_string();
    let hit = trimmed[s..e].to_string();
    let post = trimmed[e..].to_string();
    (clamp_chars(pre, 100), hit, clamp_chars(post, 200))
}

fn clamp_chars(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

