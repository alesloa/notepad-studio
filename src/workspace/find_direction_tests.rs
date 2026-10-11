// Behavior oracle: NppCommands.cpp:1598-1615; FindReplaceDlg.cpp:2080-2110.
// These are actual GPUI action/keyboard and painted dialog controls, not native UI tests.
use super::*;
use crate::{settings::Settings, tab::{Tab, TabViewMode}};
use gpui::TestAppContext;

#[derive(Clone, Copy, Debug)]
enum Route { Next, Previous, HeldShiftNext, HeldShiftPrevious, F3, ShiftF3,
    Button, ShiftButton, Enter, ShiftEnter, Arrow, ShiftArrow, DialogF3, DialogShiftF3 }

fn direction(route: Route, backward: bool, rich: bool, expected: usize) {
    direction_in_tab(route, backward, rich, expected, DialogTab::Find, false);
}

fn direction_in_tab(route: Route, backward: bool, rich: bool, expected: usize,
    dialog_tab: DialogTab, replacement_focus: bool) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default()
    });
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        crate::shortcuts::register_catalog(cx, &Default::default());
        crate::app::keymap::register_dialog_only(cx);
    });
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::empty();
        if rich {
            let mut payload = crate::rich_doc::rtf::open_bytes(
                br"{\rtf1\ansi hit caf\u233?\par hit caf\u233?\par hit caf\u233?}".to_vec()).unwrap();
            let pos = payload.block_start(1);
            payload.selection = Some(crate::rich_doc::TextSelection { anchor: pos, cursor: pos });
            tab.buffer.rope = ropey::Rope::from_str(&payload.raw_view_text);
            tab.buffer.rich_doc = Some(payload);
            tab.view_mode = TabViewMode::RichDocView;
        } else {
            tab.buffer.rope = ropey::Rope::from_str("hit caf\u{e9}\r\nhit \u{4e2d}\r\nhit");
            tab.buffer.encoding = crate::text::encoding::Encoding::Utf8Bom;
            tab.buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
        }
        w.tabs = vec![tab];
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = "hit".into();
        w.find_state.backward = backward;
        w.find_state.wrap_around = false;
        w.refresh_matches();
        assert_eq!(w.find_state.matches.len(), 3);
        w.find_state.current = Some(1);
        if !rich {
            let range = w.find_state.matches[1].clone();
            w.tabs[0].selection.anchor = range.start;
            w.tabs[0].selection.head = range.end;
        }
        w
    });
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        window.activate_window(); window.focus(&w.focus_handle, cx);
    }));
    visual.run_until_parked();
    match route {
        Route::Next | Route::HeldShiftNext => {
            visual.simulate_modifiers_change(gpui::Modifiers {
                shift: matches!(route, Route::HeldShiftNext), ..Default::default()
            });
            visual.dispatch_action(crate::actions::EditFindNext);
            visual.simulate_modifiers_change(gpui::Modifiers::none());
        }
        Route::Previous | Route::HeldShiftPrevious => {
            visual.simulate_modifiers_change(gpui::Modifiers {
                shift: matches!(route, Route::HeldShiftPrevious), ..Default::default()
            });
            visual.dispatch_action(crate::actions::EditFindPrev);
            visual.simulate_modifiers_change(gpui::Modifiers::none());
        }
        Route::F3 => visual.simulate_keystrokes("f3"),
        Route::ShiftF3 => {
            visual.simulate_modifiers_change(gpui::Modifiers { shift: true, ..Default::default() });
            visual.simulate_keystrokes("shift-f3");
            visual.simulate_modifiers_change(gpui::Modifiers::none());
        }
        Route::Button | Route::ShiftButton | Route::Enter | Route::ShiftEnter
            | Route::Arrow | Route::ShiftArrow | Route::DialogF3 | Route::DialogShiftF3 => {
            visual.update(|_, cx| entity.update(cx, |w, _| {
                w.find_state.open = true; w.dialog_tab = dialog_tab;
            }));
            let weak = entity.downgrade();
            let (_, dialog) = visual.cx.add_window_view(|window, cx| {
                crate::ui::find_dialog::FindDialog::new(weak, window, cx)
            });
            dialog.update(|window, cx| {
                let focus = if replacement_focus { entity.read(cx).replace_focus.clone() }
                    else { entity.read(cx).find_focus.clone() };
                window.activate_window(); window.focus(&focus, cx);
            });
            dialog.run_until_parked();
            match route {
                Route::Button | Route::ShiftButton => {
                    let bounds = dialog.debug_bounds("dialog-button-Find Next").unwrap();
                    let modifiers = gpui::Modifiers {
                        shift: matches!(route, Route::ShiftButton), ..Default::default()
                    };
                    dialog.simulate_modifiers_change(modifiers);
                    dialog.simulate_click(bounds.center(), modifiers);
                    dialog.simulate_modifiers_change(gpui::Modifiers::none());
                }
                Route::Enter => dialog.simulate_keystrokes("enter"),
                Route::ShiftEnter => {
                    // KeyDown's chord and the window's held modifiers are
                    // separate inputs in TestWindow, as on native platforms.
                    dialog.simulate_modifiers_change(gpui::Modifiers {
                        shift: true, ..Default::default()
                    });
                    dialog.simulate_keystrokes("shift-enter");
                    dialog.simulate_modifiers_change(gpui::Modifiers::none());
                }
                Route::Arrow | Route::ShiftArrow => {
                    let bounds = dialog.debug_bounds("dialog-direction-previous").unwrap();
                    let modifiers = gpui::Modifiers {
                        shift: matches!(route, Route::ShiftArrow), ..Default::default()
                    };
                    dialog.simulate_modifiers_change(modifiers);
                    dialog.simulate_click(bounds.center(), modifiers);
                    dialog.simulate_modifiers_change(gpui::Modifiers::none());
                }
                Route::DialogF3 => dialog.simulate_keystrokes("f3"),
                Route::DialogShiftF3 => {
                    dialog.simulate_modifiers_change(gpui::Modifiers { shift: true, ..Default::default() });
                    dialog.simulate_keystrokes("shift-f3");
                    dialog.simulate_modifiers_change(gpui::Modifiers::none());
                }
                _ => unreachable!(),
            }
        }
    }
    visual.run_until_parked();
    visual.update(|_, cx| entity.update(cx, |w, _| {
        eprintln!("{route:?} replace_tab={} backward={backward} rich={rich}: actual={:?} expected={expected}",
            dialog_tab == DialogTab::Replace, w.find_state.current);
        assert_eq!(w.find_state.current, Some(expected));
        assert_eq!(w.find_state.backward, backward, "one-shot command preserves checkbox");
        if rich {
            let selection = w.buffer().rich_doc.as_ref().unwrap().selection.unwrap();
            assert_eq!(selection.anchor.block, expected);
            assert_eq!(selection.cursor.block, expected);
        } else {
            assert_eq!(w.selection().ordered(), w.find_state.matches[expected]);
            assert_eq!(w.buffer().rope.to_string(), "hit caf\u{e9}\r\nhit \u{4e2d}\r\nhit");
            assert_eq!(w.buffer().encoding, crate::text::encoding::Encoding::Utf8Bom);
            assert_eq!(w.buffer().line_ending, crate::text::line_ending::LineEnding::Crlf);
        }
        assert!(!w.buffer().dirty);
        assert!(w.tabs[0].undo_stack.is_empty());
        w.window_closed = true;
    }));
    cx.quit();
}

#[test] fn explicit_next_ignores_backward_checkbox() { direction(Route::Next, true, false, 2); }
#[test] fn explicit_previous_ignores_backward_checkbox() { direction(Route::Previous, true, false, 0); }
#[test] fn catalog_f3_ignores_backward_checkbox() { direction(Route::F3, true, false, 2); }
#[test] fn catalog_shift_f3_ignores_backward_checkbox() { direction(Route::ShiftF3, true, false, 0); }
#[test] fn explicit_next_keeps_rich_coordinates() { direction(Route::Next, true, true, 2); }
#[test] fn explicit_previous_keeps_rich_coordinates() { direction(Route::Previous, true, true, 0); }
#[test] fn default_explicit_directions_unchanged() {
    direction(Route::Next, false, false, 2); direction(Route::Previous, false, false, 0);
}
#[test] fn main_dialog_button_obeys_backward_checkbox() { direction(Route::Button, true, false, 0); }
#[test] fn dialog_enter_obeys_backward_checkbox() { direction(Route::Enter, true, false, 0); }
#[test] fn dialog_shift_enter_reverses_backward_checkbox() { direction(Route::ShiftEnter, true, false, 2); }

#[test] fn find_primary_routes_keep_both_checkbox_directions() {
    for backward in [false, true] {
        for route in [Route::Button, Route::Enter] {
            direction(route, backward, false, if backward { 0 } else { 2 });
        }
        for route in [Route::ShiftButton, Route::ShiftEnter] {
            direction(route, backward, false, if backward { 2 } else { 0 });
        }
    }
}

#[test] fn replace_primary_routes_keep_both_checkbox_directions() {
    for backward in [false, true] {
        for route in [Route::Button, Route::Enter, Route::ShiftButton, Route::ShiftEnter] {
            let reverse = matches!(route, Route::ShiftButton | Route::ShiftEnter);
            let expected = if backward == reverse { 2 } else { 0 };
            direction_in_tab(route, backward, false, expected, DialogTab::Replace, false);
        }
    }
}

#[test] fn replacement_input_enter_routes_keep_checkbox_direction() {
    for backward in [false, true] {
        direction_in_tab(Route::Enter, backward, false, if backward { 0 } else { 2 }, DialogTab::Replace, true);
        direction_in_tab(Route::ShiftEnter, backward, false, if backward { 2 } else { 0 }, DialogTab::Replace, true);
    }
}

#[test] fn painted_previous_arrow_ignores_checkbox_in_both_tabs() {
    for dialog_tab in [DialogTab::Find, DialogTab::Replace] {
        for backward in [false, true] {
            for route in [Route::Arrow, Route::ShiftArrow] {
                direction_in_tab(route, backward, false, 0, dialog_tab, false);
            }
        }
    }
}

#[test] fn dialog_f3_commands_ignore_checkbox_in_both_tabs() {
    for dialog_tab in [DialogTab::Find, DialogTab::Replace] {
        for backward in [false, true] {
            direction_in_tab(Route::DialogF3, backward, false, 2, dialog_tab, false);
            direction_in_tab(Route::DialogShiftF3, backward, false, 0, dialog_tab, false);
        }
    }
}

#[test] fn dialog_routes_keep_formatted_text_coordinates() {
    direction(Route::Arrow, true, true, 0);
    direction(Route::Enter, true, true, 0);
    direction(Route::ShiftEnter, true, true, 2);
}

#[test] fn explicit_actions_ignore_held_shift() {
    for backward in [false, true] {
        direction(Route::HeldShiftNext, backward, false, 2);
        direction(Route::HeldShiftPrevious, backward, false, 0);
    }
    direction(Route::HeldShiftNext, true, true, 2);
    direction(Route::HeldShiftPrevious, true, true, 0);
}
