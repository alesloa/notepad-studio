// PowerEditor/src/ScintillaComponent/FunctionCallTip.cpp:410-449 —
// original themed signature presentation, active argument and overload controls.
use gpui::{
    deferred, div, point, prelude::*, px, rems, rgb, size, AnyElement, App, Bounds, CursorStyle,
    Element, ElementId, Entity, GlobalElementId, HighlightStyle, IntoElement, LayoutId,
    MouseButton, Pixels, Point, Position, Style, StyledText, TextRun, Window,
};
use std::ops::Range;

use crate::api_registry::ApiEntry;
use crate::theme::Theme;
use crate::workspace::{HintSnapshot, Workspace};

const ROW_REM: f32 = 20. / 16.;

pub(crate) fn function_hint(theme: &Theme, workspace: Entity<Workspace>) -> impl IntoElement {
    deferred(CaretHint {
        theme: *theme,
        workspace,
    })
    .with_priority(1)
}

struct CaretHint {
    theme: Theme,
    workspace: Entity<Workspace>,
}
struct HintPrepaint {
    element: AnyElement,
    bounds: Bounds<Pixels>,
}

impl IntoElement for CaretHint {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for CaretHint {
    type RequestLayoutState = ();
    type PrepaintState = Option<HintPrepaint>;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (
            window.request_layout(
                Style {
                    position: Position::Absolute,
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let (snapshot, caret, viewport, occupied) = {
            let w = self.workspace.read(cx);
            let snapshot = w.function_hint_snapshot(window)?;
            let Some(caret) = w.autocomplete_caret_bounds() else {
                let session = snapshot.session;
                self.workspace
                    .update(cx, |w, cx| w.dismiss_function_hint_session(session, cx));
                return None;
            };
            (
                snapshot,
                caret,
                w.viewport
                    .bounds
                    .intersect(&Bounds::new(Point::default(), window.viewport_size())),
                super::autocomplete_popup::occupied_bounds(w, window),
            )
        };
        let presentation = presentation(&snapshot)?;
        let width = hint_width(&presentation, window);
        let Some((bounds, rows)) = placement(
            caret,
            viewport,
            width,
            !presentation.description.is_empty(),
            occupied,
            f32::from(window.rem_size()),
        ) else {
            // A simultaneous completion can temporarily use the available side.
            if occupied.is_none() {
                self.workspace.update(cx, |w, cx| {
                    w.dismiss_function_hint_session(snapshot.session, cx)
                });
            }
            return None;
        };
        let session = snapshot.session;
        let theme = self.theme;
        let dismiss = self.workspace.clone();
        let previous = self.workspace.clone();
        let next = self.workspace.clone();
        let count = presentation.count;
        let mut header = div()
            .flex()
            .items_center()
            .h(rems(ROW_REM))
            .gap(rems(4. / 16.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(presentation.header),
            );
        if count > 1 {
            header = header
                .child(
                    div()
                        .debug_selector(|| "hint-previous".into())
                        .px(rems(2. / 16.))
                        .cursor(CursorStyle::PointingHand)
                        .hover(|style| style.bg(rgb(theme.button_hover_bg)))
                        .child("▲")
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            previous.update(cx, |w, cx| {
                                w.navigate_function_hint_session(session, -1, window, cx)
                            });
                        }),
                )
                .child(format!("{} of {}", presentation.overload + 1, count))
                .child(
                    div()
                        .debug_selector(|| "hint-next".into())
                        .px(rems(2. / 16.))
                        .cursor(CursorStyle::PointingHand)
                        .hover(|style| style.bg(rgb(theme.button_hover_bg)))
                        .child("▼")
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            next.update(cx, |w, cx| {
                                w.navigate_function_hint_session(session, 1, window, cx)
                            });
                        }),
                );
        }
        let mut highlights = Vec::new();
        if let Some(range) = presentation.highlight {
            highlights.push((
                range,
                HighlightStyle {
                    background_color: Some(rgb(theme.selection).into()),
                    font_weight: Some(gpui::FontWeight::BOLD),
                    ..Default::default()
                },
            ));
        }
        let parameters = StyledText::new(presentation.parameters).with_highlights(highlights);
        let mut element = div()
            .id("function-hint")
            .debug_selector(|| "function-hint".into())
            .flex()
            .flex_col()
            .w(bounds.size.width)
            .h(bounds.size.height)
            .px(rems(6. / 16.))
            .py(rems(2. / 16.))
            .bg(rgb(theme.chrome))
            .border_1()
            .border_color(rgb(theme.border))
            .text_size(rems(12. / 16.))
            .text_color(rgb(theme.text))
            .overflow_hidden()
            .occlude()
            .on_mouse_down_out(move |event, window, cx| {
                dismiss.update(cx, |w, cx| {
                    if super::autocomplete_popup::occupied_bounds(w, window)
                        .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        return;
                    }
                    w.dismiss_function_hint_session(session, cx);
                });
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(header)
            .child(
                div()
                    .debug_selector(|| "hint-parameters".into())
                    .h(rems(ROW_REM))
                    .overflow_hidden()
                    .child(parameters),
            );
        if rows > 2 && !presentation.description.is_empty() {
            element = element.child(
                div()
                    .h(rems(ROW_REM))
                    .truncate()
                    .text_color(rgb(theme.chrome_text))
                    .child(presentation.description),
            );
        }
        let mut element = element.into_any_element();
        element.layout_as_root(bounds.size.into(), window, cx);
        element.prepaint_at(bounds.origin, window, cx);
        Some(HintPrepaint { element, bounds })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(state) = state {
            window.with_content_mask(
                Some(gpui::ContentMask {
                    bounds: state.bounds,
                }),
                |window| state.element.paint(window, cx),
            );
        }
    }
}

struct Presentation {
    header: String,
    parameters: String,
    highlight: Option<Range<usize>>,
    description: String,
    overload: usize,
    count: usize,
}

fn label(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut text: String = chars
        .by_ref()
        .take(max_chars)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}

fn presentation(snapshot: &HintSnapshot) -> Option<Presentation> {
    let entry = snapshot.api.registry.find(&snapshot.call.name)?;
    signature(
        entry,
        snapshot.overload,
        snapshot.call.argument,
        &snapshot.api.registry.environment,
    )
}

fn signature(
    entry: &ApiEntry,
    overload_index: usize,
    argument: usize,
    environment: &crate::api_registry::ApiEnvironment,
) -> Option<Presentation> {
    let overload = entry.overloads.get(overload_index)?;
    let header = format!(
        "{} {} · argument {}",
        label(&overload.return_text, 32),
        label(&entry.name, 64),
        argument.saturating_add(1)
    );
    let mut parameters = environment.start.to_string();
    let mut highlight = None;
    let first = argument.min(overload.parameters.len().saturating_sub(1));
    if first > 0 {
        parameters.push('…');
        parameters.push(environment.separator);
        parameters.push(' ');
    }
    for (index, parameter) in overload.parameters.iter().enumerate().skip(first).take(2) {
        if index > first {
            parameters.push(environment.separator);
            parameters.push(' ');
        }
        let start = parameters.len();
        parameters.push_str(&label(parameter, if index == argument { 96 } else { 64 }));
        if index == argument && parameters.len() > start {
            highlight = Some(start..parameters.len());
        }
    }
    if overload.parameters.len() > first.saturating_add(2) {
        parameters.push(environment.separator);
        parameters.push_str(" …");
    }
    parameters.push(environment.stop);
    Some(Presentation {
        header,
        parameters,
        highlight,
        description: label(&overload.description, 240),
        overload: overload_index,
        count: entry.overloads.len(),
    })
}

fn hint_width(presentation: &Presentation, window: &mut Window) -> Pixels {
    let mut width = window.rem_size() * 12.;
    let font = window.text_style().font();
    for text in [
        &presentation.header,
        &presentation.parameters,
        &presentation.description,
    ] {
        let run = TextRun {
            len: text.len(),
            font: font.clone(),
            color: window.text_style().color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(
            text.clone().into(),
            window.rem_size() * (12. / 16.),
            &[run],
            None,
        );
        width = width.max(line.width + window.rem_size() * 7.);
    }
    width.min(window.rem_size() * 40.)
}

fn placement(
    caret: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    width: Pixels,
    description: bool,
    occupied: Option<Bounds<Pixels>>,
    rem: f32,
) -> Option<(Bounds<Pixels>, usize)> {
    if !rem.is_finite()
        || rem <= 0.
        || ![
            width,
            caret.left(),
            caret.top(),
            caret.bottom(),
            viewport.left(),
            viewport.right(),
            viewport.top(),
            viewport.bottom(),
        ]
        .iter()
        .all(|value| f32::from(*value).is_finite())
    {
        return None;
    }
    let margin = px(rem * 4. / 16.);
    let gap = px(rem * 2. / 16.);
    let left = viewport.left() + margin;
    let right = viewport.right() - margin;
    let top = viewport.top() + margin;
    let bottom = viewport.bottom() - margin;
    if caret.top() < viewport.top()
        || caret.bottom() > viewport.bottom()
        || caret.left() < viewport.left()
        || caret.left() > viewport.right()
    {
        return None;
    }
    let width = width.max(px(0.)).min(right - left);
    if width < px(rem * 4.) {
        return None;
    }
    let x = caret.left().min(right - width).max(left);
    for rows in [if description { 3 } else { 2 }, 2] {
        let height = px(rem * (rows as f32 * ROW_REM + 4. / 16.) + 2.);
        for y in [caret.bottom() + gap, caret.top() - gap - height] {
            let bounds = Bounds::new(point(x, y), size(width, height));
            if y >= top
                && bounds.bottom() <= bottom
                && occupied.is_none_or(|occupied| !occupied.intersects(&bounds))
            {
                return Some((bounds, rows));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    #[test]
    fn placement_excludes_caret_line_completion_and_viewport_edges() {
        let viewport = bounds(0., 0., 500., 200.);
        for caret in [bounds(20., 20., 1., 20.), bounds(480., 170., 1., 20.)] {
            let (placed, _) = placement(caret, viewport, px(640.), true, None, 16.).unwrap();
            assert!(placed.left() >= viewport.left() && placed.right() <= viewport.right());
            assert!(placed.top() >= viewport.top() && placed.bottom() <= viewport.bottom());
            assert!(!placed.intersects(&caret));
        }
        let viewport = bounds(0., 0., 500., 400.);
        let caret = bounds(100., 180., 1., 20.);
        let (below, _) = placement(caret, viewport, px(300.), true, None, 16.).unwrap();
        let (above, _) = placement(caret, viewport, px(300.), true, Some(below), 16.).unwrap();
        assert!(below.top() >= caret.bottom() && above.bottom() <= caret.top());
        assert!(!above.intersects(&below));
        assert!(placement(
            bounds(0., 0., 1., 20.),
            bounds(0., 0., 30., 30.),
            px(200.),
            true,
            None,
            16.
        )
        .is_none());
        assert!(placement(
            bounds(0., 0., 1., 20.),
            viewport,
            px(f32::NAN),
            true,
            None,
            16.
        )
        .is_none());
    }

    #[test]
    fn signature_windows_keep_active_unicode_parameter_and_bound_hostile_labels() {
        let xml = format!("<NotepadPlus><AutoComplete><KeyWord name='draw' func='yes'><Overload retVal='{}' descr='line&#10;next'><Param name='first'/><Param name='é中{}'/><Param name='third'/><Param name='last'/></Overload></KeyWord></AutoComplete></NotepadPlus>", "x".repeat(8000), "z".repeat(7000));
        let registry = crate::api_registry::ApiRegistry::parse(&xml).unwrap();
        let entry = registry.find("draw").unwrap();
        let display = signature(entry, 0, 1, &registry.environment).unwrap();
        assert!(
            display.header.len() < 180
                && display.parameters.len() < 700
                && display.description.len() < 1000
        );
        assert!(display
            .parameters
            .get(display.highlight.unwrap())
            .unwrap()
            .starts_with("é中"));
        assert!(!display.description.contains('\n'));
        assert!(signature(entry, 0, 99, &registry.environment)
            .unwrap()
            .highlight
            .is_none());
        assert_eq!(entry.overloads[0].parameters[1].len(), 7005);
    }
}
