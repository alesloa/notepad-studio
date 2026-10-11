// Actual registered PageDown dispatch, including its event-time text style.
use super::*;
use gpui::{AppContext, Render, TestAppContext, WindowAppearance};

struct Fixture {
    workspace: Entity<Workspace>,
    width: Pixels,
    event_font: Option<gpui::SharedString>,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .key_context("Workspace")
            .font_family(crate::fonts::mono())
            .track_focus(&self.workspace.read(cx).focus_handle)
            .on_action(
                cx.listener(|this, action: &crate::actions::EditPageDown, window, cx| {
                    this.event_font = Some(window.text_style().font_family);
                    this.workspace
                        .update(cx, |w, cx| w.page_down(action, window, cx));
                }),
            )
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

#[test]
fn registered_page_down_preserves_near_threshold_mono_popup_and_editor_caret() {
    for rem in [8., 16., 32., 48.] {
        let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
        let mut cx = TestAppContext::single();
        cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
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
                w.active_tab_mut().buffer.rope = ropey::Rope::from_str("a\nalpha alpine alps");
                w.active_tab_mut().selection.collapse_to(1);
                w.window_closed = true;
                w
            });
            Fixture {
                workspace,
                width: px(200.),
                event_font: None,
            }
        });
        let (mono_width, event_width) = visual.update(|window, cx| {
            window.activate_window();
            window.set_rem_size(px(rem));
            let event_width = readable_label_width(window);
            let style = window.text_style();
            let mut font = style.font();
            font.family = crate::fonts::mono();
            let runs = [TextRun {
                len: "…".len(),
                font,
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let mono_width = window
                .text_system()
                .shape_line("…".into(), px(rem * 12. / 16.), &runs, None)
                .width;
            fixture.update(cx, |f, _| f.width = px(rem * 1.5 + 4. + 1.) + mono_width);
            window.focus(
                &fixture.read(cx).workspace.read(cx).focus_handle.clone(),
                cx,
            );
            window.refresh();
            (mono_width, event_width)
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            fixture.read(cx).workspace.clone().update(cx, |w, cx| {
                w.autocomplete_word(&crate::actions::EditAutocompleteWord, window, cx);
                assert_eq!(w.autocomplete.as_ref().unwrap().candidates.len(), 3);
            });
        });
        visual.run_until_parked();
        let menu = visual
            .debug_bounds("autocomplete-menu")
            .expect("valid near-threshold rendered popup");
        let label = visual.debug_bounds("autocomplete-row-label").unwrap();
        assert!(
            label.intersect(&menu).size.width >= mono_width,
            "fixture must first prove readable painted row"
        );
        let entry = crate::shortcuts::catalog()
            .iter()
            .find(|entry| entry.id == "EditPageDown")
            .unwrap();
        let keys = crate::shortcuts::platform_default_keys(entry);
        visual.simulate_keystrokes(keys.first().unwrap());
        visual.update(|_, cx| {
            let f = fixture.read(cx);
            assert!(f.event_font.is_some(), "registered event route must fire");
            let w = f.workspace.read(cx);
            assert_eq!(w.selection().head, 1, "PageDown must navigate the list rather than the editor");
            assert_eq!(w.selection().anchor, 1);
            let state = w.autocomplete.as_ref().expect("readable opening must survive PageDown");
            assert_eq!(state.selected, 2);
            assert_eq!(w.buffer().rope.to_string(), "a\nalpha alpine alps");
            println!("rem={rem} rendered_font={} event_font={} mono_ellipsis={mono_width:?} event_ellipsis={event_width:?}",
                crate::fonts::mono(), f.event_font.as_ref().unwrap());
        });
        assert!(visual.debug_bounds("autocomplete-menu").is_some());
        cx.quit();
    }
}
