// Real editor prepaint and completion row layout at narrow viewport widths.
use super::*;
use gpui::{AppContext, Render, TestAppContext, WindowAppearance};

struct Fixture {
    workspace: Entity<Workspace>,
    width: Pixels,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .track_focus(&self.workspace.read(cx).focus_handle)
            .child(
                div()
                    .absolute()
                    .left(px(20.))
                    .top(px(20.))
                    .w(self.width)
                    .h(px(400.))
                    .child(crate::editor_element::EditorElement {
                        workspace: self.workspace.clone(),
                    }),
            )
            .child(autocomplete_popup(
                &Theme::for_mode(crate::theme::ThemeMode::Dark),
                self.workspace.clone(),
            ))
    }
}

fn painted_row_is_readable_or_menu_is_dismissed(rem: f32, enough_width: bool) {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let mut cx = TestAppContext::single();
    let width = px(if enough_width {
        rem * 8.
    } else {
        rem * 1.5 + 1.
    });
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
            w.active_tab_mut().buffer.rope = ropey::Rope::from_str("a\nalpha");
            w.active_tab_mut().selection.collapse_to(1);
            w.window_closed = true;
            w
        });
        Fixture { workspace, width }
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.set_rem_size(px(rem));
        let workspace = fixture.read(cx).workspace.clone();
        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
        window.refresh();
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        fixture.read(cx).workspace.clone().update(cx, |w, cx| {
            w.autocomplete_word(&crate::actions::EditAutocompleteWord, window, cx);
            assert!(
                w.autocomplete
                    .as_ref()
                    .unwrap()
                    .candidates
                    .iter()
                    .any(|name| name == "alpha")
            );
        });
    });
    visual.run_until_parked();
    let menu = visual.debug_bounds("autocomplete-menu");
    if enough_width {
        assert!(
            menu.is_some(),
            "wide positive must retain menu at rem={rem}"
        );
    }
    if let Some(menu) = menu {
        let label = visual
            .debug_bounds("autocomplete-row-label")
            .expect("actual painted row label");
        visual.update(|window, cx| {
            let w = fixture.read(cx).workspace.read(cx);
            let caret = w.autocomplete_caret_bounds().unwrap();
            assert!(!menu.intersects(&caret));
            assert!(menu.left() >= w.viewport.bounds.left() && menu.right() <= w.viewport.bounds.right());
            let style = window.text_style();
            let runs = [TextRun { len: "…".len(), font: style.font(), color: style.color,
                background_color: None, underline: None, strikethrough: None }];
            let ellipsis = window.text_system().shape_line("…".into(), px(rem * 12. / 16.), &runs, None);
            assert!(label.intersect(&menu).size.width >= ellipsis.width,
                "retained row must fit a complete readable ellipsis: rem={rem}, viewport={width:?}, menu={menu:?}, label={label:?}, ellipsis={:?}", ellipsis.width);
        });
    } else {
        visual.update(|_, cx| assert!(fixture.read(cx).workspace.read(cx).autocomplete.is_none()));
    }
    cx.quit();
}

#[test]
fn narrow_painted_completion_rows_are_readable_or_dismissed_at_each_zoom() {
    for rem in [8., 16., 32., 48.] {
        painted_row_is_readable_or_menu_is_dismissed(rem, false);
    }
}

#[test]
fn sufficient_width_painted_completion_rows_remain_readable_at_each_zoom() {
    for rem in [8., 16., 32., 48.] {
        painted_row_is_readable_or_menu_is_dismissed(rem, true);
    }
}
