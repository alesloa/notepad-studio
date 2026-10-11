// Completion list placement uses current prepaint geometry, never a cached
// opening point. Keyboard handling lives in workspace::autocomplete.

use gpui::{
    AnyElement, App, Bounds, CursorStyle, Element, ElementId, Entity, GlobalElementId, IntoElement,
    LayoutId, MouseButton, Pixels, Point, Position, Style, TextRun, Window, deferred, div, point,
    prelude::*, px, rems, rgb, size,
};

use crate::theme::Theme;
use crate::workspace::Workspace;

const MAX_VISIBLE_ROWS: usize = 8;
const ROW_HEIGHT_REM: f32 = 22. / 16.;

pub(crate) fn autocomplete_popup(
    theme: &Theme,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    deferred(CaretPopup {
        theme: *theme,
        workspace,
    })
    .with_priority(2)
}

struct CaretPopup {
    theme: Theme,
    workspace: Entity<Workspace>,
}

struct PopupPrepaint {
    menu: AnyElement,
    bounds: Bounds<Pixels>,
}

impl IntoElement for CaretPopup {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for CaretPopup {
    type RequestLayoutState = ();
    type PrepaintState = Option<PopupPrepaint>;

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
                    ..Style::default()
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
        let snapshot = {
            let w = self.workspace.read(cx);
            if w.autocomplete_context_valid(window) {
                w.autocomplete_caret_bounds()
                    .zip(w.autocomplete.as_ref())
                    .map(|(caret, state)| {
                        (caret, w.viewport.bounds, state.candidates.clone(), state.selected, state.session_id)
                    })
            } else {
                None
            }
        };
        let Some((caret, viewport, candidates, selected, session_id)) = snapshot else {
            self.workspace.update(cx, |w, cx| w.close_autocomplete(cx));
            return None;
        };
        let viewport = viewport.intersect(&Bounds::new(Point::default(), window.viewport_size()));
        let rem = f32::from(window.rem_size());
        let width = candidate_width(&candidates, window);
        let Some(placement) = popup_placement(caret, viewport, width, candidates.len(), rem)
        else {
            self.workspace.update(cx, |w, cx| w.close_autocomplete(cx));
            return None;
        };
        let (start, end) = visible_window(candidates.len(), selected, placement.rows);
        let theme = self.theme;
        let dismiss = self.workspace.clone();
        let scroll = self.workspace.clone();
        let mut menu = div()
            .id("autocomplete-menu")
            .debug_selector(|| "autocomplete-menu".into())
            .flex()
            .flex_col()
            .py(rems(2. / 16.))
            .w(placement.bounds.size.width)
            .bg(rgb(theme.chrome))
            .border_1()
            .border_color(rgb(theme.border))
            .text_size(rems(12. / 16.))
            .text_color(rgb(theme.text))
            .overflow_hidden()
            .occlude()
            .on_mouse_down_out(move |_, _, cx| {
                dismiss.update(cx, |w, cx| w.dismiss_autocomplete_session(session_id, cx));
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(move |event, window, cx| {
                let delta = event.delta.pixel_delta(window.line_height()).y;
                let step = if delta > px(0.) { -1 } else if delta < px(0.) { 1 } else { 0 };
                scroll.update(cx, |w, cx| w.scroll_autocomplete_session(session_id, step, window, cx));
                cx.stop_propagation();
            });
        for (idx, candidate) in candidates
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            let selected = idx == selected;
            let workspace = self.workspace.clone();
            let pick = candidate.clone();
            let row = div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .h(rems(ROW_HEIGHT_REM))
                .px(rems(8. / 16.))
                .border_l_2()
                .border_color(rgb(if selected {
                    theme.toggle_active
                } else {
                    theme.chrome
                }))
                .when(selected, |row| row.bg(rgb(theme.button_hover_bg)))
                .hover(|s| s.bg(rgb(theme.button_hover_bg)))
                .cursor(CursorStyle::PointingHand)
                .child(div().min_w_0().truncate().child(candidate_label(candidate)))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    workspace.update(cx, |w, cx| w.accept_autocomplete_session(session_id, &pick, window, cx));
                });
            menu = menu.child(row);
        }
        let mut menu = menu.into_any_element();
        menu.layout_as_root(placement.bounds.size.into(), window, cx);
        menu.prepaint_at(placement.bounds.origin, window, cx);
        Some(PopupPrepaint {
            menu,
            bounds: placement.bounds,
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(prepaint) = prepaint {
            window.with_content_mask(
                Some(gpui::ContentMask {
                    bounds: prepaint.bounds,
                }),
                |window| {
                    prepaint.menu.paint(window, cx);
                },
            );
        }
    }
}

/// Fresh geometry for a simultaneous call hint; no previous-frame bounds.
pub(crate) fn occupied_bounds(workspace: &Workspace, window: &mut Window) -> Option<Bounds<Pixels>> {
    if !workspace.autocomplete_context_valid(window) { return None; }
    let state = workspace.autocomplete.as_ref()?;
    let caret = workspace.autocomplete_caret_bounds()?;
    let viewport = workspace.viewport.bounds.intersect(&Bounds::new(Point::default(), window.viewport_size()));
    let width = candidate_width(&state.candidates, window);
    popup_placement(caret, viewport, width, state.candidates.len(), f32::from(window.rem_size()))
        .map(|placement| placement.bounds)
}

fn candidate_width(candidates: &[String], window: &mut Window) -> Pixels {
    let font = window.text_style().font();
    let font_size = window.rem_size() * (12. / 16.);
    let mut width = window.rem_size() * 8.;
    for candidate in candidates {
        // Width is capped anyway; don't shape unbounded file-derived text.
        let label = candidate_label(candidate);
        let runs = [TextRun {
            len: label.len(),
            font: font.clone(),
            color: window.text_style().color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let shaped = window
            .text_system()
            .shape_line(label.into(), font_size, &runs, None);
        width = width.max(shaped.width + window.rem_size() * (20. / 16.) + px(2.));
    }
    width.min(window.rem_size() * 24.)
}

fn candidate_label(candidate: &str) -> String {
    let mut chars = candidate.chars();
    let mut label: String = chars.by_ref().take(80).collect();
    if chars.next().is_some() {
        label.push('…');
    }
    label
}

#[derive(Debug)]
struct PopupPlacement {
    bounds: Bounds<Pixels>,
    rows: usize,
}

/// Fit complete rows on one side of the active line. Sliding a tall popup
/// upward from the window bottom would cover the text being completed.
fn popup_placement(
    caret: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    width: Pixels,
    len: usize,
    rem: f32,
) -> Option<PopupPlacement> {
    if len == 0 || !rem.is_finite() || rem <= 0. || !f32::from(width).is_finite() {
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
        || right <= left
    {
        return None;
    }
    let row_height = px(rem * ROW_HEIGHT_REM);
    let chrome = px(rem * 4. / 16. + 2.);
    let rows_that_fit = |space: Pixels| {
        ((f32::from(space - chrome) / f32::from(row_height))
            .floor()
            .max(0.) as usize)
            .min(MAX_VISIBLE_ROWS)
            .min(len)
    };
    let below = rows_that_fit(bottom - caret.bottom() - gap);
    let above = rows_that_fit(caret.top() - gap - top);
    let use_above = above > below && below < len.min(MAX_VISIBLE_ROWS);
    let rows = if use_above { above } else { below };
    if rows == 0 {
        return None;
    }
    let width = width.max(px(0.)).min(right - left);
    if width <= px(rem) {
        return None;
    }
    let height = row_height * rows as f32 + chrome;
    let x = caret.left().min(right - width).max(left);
    let y = if use_above {
        caret.top() - gap - height
    } else {
        caret.bottom() + gap
    };
    Some(PopupPlacement {
        bounds: Bounds::new(point(x, y), size(width, height)),
        rows,
    })
}

fn visible_window(len: usize, selected: usize, capacity: usize) -> (usize, usize) {
    let selected = selected.min(len.saturating_sub(1));
    let capacity = capacity.min(MAX_VISIBLE_ROWS);
    let start = (selected.saturating_add(1))
        .saturating_sub(capacity)
        .min(len);
    (start, start.saturating_add(capacity).min(len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn window_follows_selection() {
        assert_eq!(visible_window(0, 0, 8), (0, 0));
        assert_eq!(visible_window(3, 2, 8), (0, 3));
        assert_eq!(visible_window(50, 0, 8), (0, 8));
        assert_eq!(visible_window(50, 7, 8), (0, 8));
        assert_eq!(visible_window(50, 8, 8), (1, 9));
        assert_eq!(visible_window(50, 49, 8), (42, 50));
        assert_eq!(visible_window(50, 49, 2), (48, 50));
    }

    #[test]
    fn selection_past_the_list_stays_in_range() {
        for capacity in [0, 1, 2, 8] {
            for (len, selected) in [(50, 60), (9, 100), (9, usize::MAX), (0, usize::MAX)] {
                let (start, end) = visible_window(len, selected, capacity);
                assert!(
                    start <= end && end <= len,
                    "{len} {selected} -> {start}..{end}"
                );
                assert!(end - start <= capacity);
            }
        }
    }

    #[test]
    fn screenshot_sized_pane_keeps_the_first_line_clear() {
        let caret = bounds(440., 117., 1., 22.);
        let viewport = bounds(63., 114., 467., 119.);
        let popup = popup_placement(caret, viewport, px(160.), 50, 16.).expect("room below");
        assert!(popup.bounds.top() > caret.bottom());
        assert!(popup.bounds.right() <= viewport.right());
        assert!(popup.bounds.bottom() <= viewport.bottom());
        assert_eq!(popup.rows, 3);
    }

    #[test]
    fn bottom_line_flips_above_instead_of_covering_the_caret() {
        let caret = bounds(400., 280., 1., 20.);
        let popup = popup_placement(caret, bounds(80., 100., 450., 210.), px(180.), 50, 16.)
            .expect("room above");
        assert_eq!(popup.rows, 7);
        assert!(popup.bounds.bottom() < caret.top());
    }

    #[test]
    fn no_geometry_or_no_room_means_no_popup() {
        let viewport = bounds(80., 100., 450., 26.);
        assert!(popup_placement(bounds(100., 102., 1., 22.), viewport, px(180.), 1, 16.).is_none());
        for caret in [
            bounds(100., 90., 1., 22.),
            bounds(100., 120., 1., 22.),
            bounds(60., 102., 1., 22.),
        ] {
            assert!(popup_placement(caret, viewport, px(180.), 1, 16.).is_none());
        }
    }

    #[test]
    fn huge_candidates_have_bounded_unicode_display_labels() {
        assert_eq!(candidate_label("alphabet"), "alphabet");
        let candidate = "中".repeat(4_000_000);
        let label = candidate_label(&candidate);
        assert_eq!(label.chars().count(), 81);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn placement_stays_in_pane_at_all_zoom_levels_and_positions() {
        let viewport = bounds(80., 100., 450., 250.);
        for rem in [8., 12., 16., 24., 48.] {
            for y in (100..329).step_by(3) {
                for x in [80., 120., 400., 529.] {
                    let caret = bounds(x, y as f32, 1., 22.);
                    if let Some(popup) = popup_placement(caret, viewport, px(1000.), 50, rem) {
                        assert!(popup.bounds.left() >= viewport.left());
                        assert!(popup.bounds.right() <= viewport.right());
                        assert!(popup.bounds.top() >= viewport.top());
                        assert!(popup.bounds.bottom() <= viewport.bottom());
                        assert!(
                            popup.bounds.bottom() < caret.top()
                                || popup.bounds.top() > caret.bottom()
                        );
                    }
                }
            }
        }
    }

    struct Fixture {
        workspace: Entity<Workspace>,
        height: f32,
        outside_clicks: usize,
        _settings: crate::settings::TestSettingsGuard,
    }

    impl gpui::Render for Fixture {
        fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
            div()
                .relative()
                .size_full()
                .track_focus(&self.workspace.read(cx).focus_handle)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.outside_clicks += 1),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(80.))
                        .top(px(110.))
                        .w(px(450.))
                        .h(px(self.height))
                        .child(crate::editor_element::EditorElement {
                            workspace: self.workspace.clone(),
                        }),
                )
                .child(deferred(CaretPopup {
                    theme: Theme::for_mode(crate::theme::ThemeMode::Dark),
                    workspace: self.workspace.clone(),
                }))
        }
    }

    fn fixture(
        cx: &mut gpui::TestAppContext,
        height: f32,
    ) -> (Entity<Fixture>, &mut gpui::VisualTestContext) {
        use gpui::{AppContext, WindowAppearance};
        let settings = crate::settings::override_for_test(crate::settings::Settings::default());
        let (fixture, visual) = cx.add_window_view(|_, cx| {
            let workspace = cx.new(|cx| {
                let mut w = Workspace::new(
                    cx.focus_handle(),
                    cx.focus_handle(),
                    cx.focus_handle(),
                    cx.focus_handle(),
                    cx.focus_handle(),
                    cx.focus_handle(),
                    1.,
                    WindowAppearance::Dark,
                );
                w.active_tab_mut().buffer.rope =
                    ropey::Rope::from_str("al\nalpha albatross alpine");
                w.active_tab_mut().selection = crate::buffer::Selection {
                    anchor: 2,
                    head: 2,
                    ..Default::default()
                };
                w.window_closed = true;
                w
            });
            Fixture {
                workspace,
                height,
                outside_clicks: 0,
                _settings: settings,
            }
        });
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        visual.update(|window, cx| {
            fixture.read(cx).workspace.clone().update(cx, |w, cx| {
                w.focus_handle.focus(window, cx);
                w.autocomplete_word(&crate::actions::EditAutocompleteWord, window, cx);
                assert!(w.autocomplete.is_some());
            });
        });
        visual.run_until_parked();
        (fixture, visual)
    }

    #[test]
    fn gpui_test_window_lays_out_the_real_menu_clear_of_the_caret() {
        let mut cx = gpui::TestAppContext::single();
        let (fixture, visual) = fixture(&mut cx, 120.);
        let popup = visual
            .debug_bounds("autocomplete-menu")
            .expect("real GPUI menu laid out and painted");
        visual.update(|_, cx| {
            let w = fixture.read(cx).workspace.read(cx);
            let caret = w
                .autocomplete_caret_bounds()
                .expect("current caret geometry");
            assert!(popup.top() > caret.bottom());
            assert!(popup.bottom() <= w.viewport.bounds.bottom());
            assert!(popup.left() >= w.viewport.bounds.left());
            assert!(popup.right() <= w.viewport.bounds.right());
        });
        cx.quit();
    }

    #[test]
    fn outside_click_dismisses_and_reaches_the_underlying_view() {
        let mut cx = gpui::TestAppContext::single();
        let (fixture, visual) = fixture(&mut cx, 120.);
        assert!(visual.debug_bounds("autocomplete-menu").is_some());
        visual.simulate_click(point(px(100.), px(112.)), gpui::Modifiers::none());
        visual.update(|_, cx| {
            let fixture = fixture.read(cx);
            assert_eq!(fixture.outside_clicks, 1);
            assert!(fixture.workspace.read(cx).autocomplete.is_none());
        });
        cx.quit();
    }

    #[test]
    fn clicking_a_suggestion_accepts_without_clicking_the_editor() {
        let mut cx = gpui::TestAppContext::single();
        let (fixture, visual) = fixture(&mut cx, 120.);
        let popup = visual
            .debug_bounds("autocomplete-menu")
            .expect("visible menu");
        visual.simulate_click(
            popup.origin + point(px(12.), px(12.)),
            gpui::Modifiers::none(),
        );
        visual.update(|_, cx| {
            let fixture = fixture.read(cx);
            let w = fixture.workspace.read(cx);
            assert_eq!(fixture.outside_clicks, 0);
            assert!(w.autocomplete.is_none());
            assert_eq!(
                w.buffer().rope.to_string(),
                "albatross\nalpha albatross alpine"
            );
        });
        cx.quit();
    }

    #[test]
    fn clicking_menu_padding_does_not_move_the_editor_caret() {
        let mut cx = gpui::TestAppContext::single();
        let (fixture, visual) = fixture(&mut cx, 120.);
        let popup = visual
            .debug_bounds("autocomplete-menu")
            .expect("visible menu");
        visual.simulate_click(
            popup.origin + point(px(2.), px(1.)),
            gpui::Modifiers::none(),
        );
        visual.update(|_, cx| {
            let fixture = fixture.read(cx);
            let w = fixture.workspace.read(cx);
            assert_eq!(fixture.outside_clicks, 0);
            assert!(w.autocomplete.is_some());
            assert_eq!(w.selection().head, 2);
        });
        cx.quit();
    }

    #[test]
    fn scrolling_the_list_moves_selection_without_scrolling_the_editor() {
        let mut cx = gpui::TestAppContext::single();
        let (fixture, visual) = fixture(&mut cx, 120.);
        let popup = visual
            .debug_bounds("autocomplete-menu")
            .expect("visible menu");
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: popup.origin + point(px(12.), px(12.)),
            delta: gpui::ScrollDelta::Lines(point(0., -1.)),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        visual.update(|_, cx| {
            let w = fixture.read(cx).workspace.read(cx);
            assert_eq!(w.autocomplete.as_ref().expect("visible list").selected, 1);
            assert_eq!(w.tabs[w.active].scroll_top, px(0.));
        });
        cx.quit();
    }

    #[test]
    fn invisible_popup_does_not_keep_an_acceptance_state() {
        let mut cx = gpui::TestAppContext::single();
        let (fixture, visual) = fixture(&mut cx, 20.);
        assert!(visual.debug_bounds("autocomplete-menu").is_none());
        visual.update(|_, cx| assert!(fixture.read(cx).workspace.read(cx).autocomplete.is_none()));
        cx.quit();
    }

    #[test]
    fn scrolling_the_caret_offscreen_dismisses_the_list() {
        let mut cx = gpui::TestAppContext::single();
        let (fixture, visual) = fixture(&mut cx, 120.);
        assert!(visual.debug_bounds("autocomplete-menu").is_some());
        visual.update(|window, cx| {
            fixture.read(cx).workspace.clone().update(cx, |w, _| {
                let line_height = w.viewport.line_height;
                w.active_tab_mut().scroll_top = line_height;
            });
            window.refresh();
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("autocomplete-menu").is_none());
        visual.update(|_, cx| assert!(fixture.read(cx).workspace.read(cx).autocomplete.is_none()));
        cx.quit();
    }

    #[test]
    fn wrap_boundary_uses_the_following_visual_row() {
        let mut cx = gpui::TestAppContext::single();
        let (fixture, visual) = fixture(&mut cx, 120.);
        visual.update(|window, cx| {
            fixture.read(cx).workspace.clone().update(cx, |w, cx| {
                w.close_autocomplete(cx);
                w.word_wrap = true;
                w.active_tab_mut().buffer.rope = ropey::Rope::from_str(&"alpha ".repeat(60));
                w.active_tab_mut().wrap_cache = None;
            });
            window.refresh();
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            fixture.read(cx).workspace.clone().update(cx, |w, _| {
                let row = w.last_lines.get(1).expect("wrapped second visual row");
                let head = row.line_byte_start;
                let expected = row.bounds;
                w.active_tab_mut().selection = crate::buffer::Selection {
                    anchor: head,
                    head,
                    ..Default::default()
                };
                let caret = w
                    .autocomplete_caret_bounds()
                    .expect("wrap-boundary geometry");
                assert_eq!(caret.top(), expected.top());
                assert_eq!(caret.left(), expected.left());
            });
        });
        cx.quit();
    }
}
