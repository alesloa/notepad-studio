use super::*;
use crate::actions::{MoveDown, MoveLineStart, SelectLineEnd};
use crate::{settings::Settings, tab::{Tab, TabViewMode}};
use gpui::TestAppContext;

#[derive(Clone, Copy)]
enum Button { Count, Next, All }
fn raw_selected(button: Button) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings { toolbar_hidden: true,
        folder_panel_visible: false, autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default() });
    let mut cx = TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rich_doc = Some(crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi visible only}".to_vec()).unwrap());
        tab.buffer.rope = ropey::Rope::from_str("<raw>outside hit\r\n中 hit café\r\noutside hit</raw>");
        tab.view_mode = TabViewMode::Edit;
        w.tabs = vec![tab]; w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = "hit".into(); w.find_state.open = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window(); window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    visual.dispatch_action(MoveDown); visual.dispatch_action(MoveLineStart); visual.dispatch_action(SelectLineEnd);
    let before = visual.update(|_, cx| {
        let w = entity.read(cx);
        assert_eq!(w.buffer().slice_bytes(w.selection().ordered()), "中 hit café");
        (w.buffer().rope.clone(), w.buffer().raw_revision.clone(), w.buffer().rich_doc.as_ref().unwrap().selection,
            w.tabs[0].undo_stack.len(), w.selection().ordered())
    });
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| crate::ui::find_dialog::FindDialog::new(weak, window, cx));
    dialog.update(|window, cx| {
        window.activate_window(); window.focus(&entity.read(cx).find_focus.clone(), cx);
    });
    dialog.run_until_parked();
    let bounds = dialog.debug_bounds("dialog-checkbox-In selection").unwrap();
    dialog.simulate_click(bounds.center(), gpui::Modifiers::none());
    dialog.update(|_, cx| assert_eq!(entity.read(cx).find_state.in_selection_scope, Some(before.4.clone())));
    let bounds = dialog.debug_bounds(match button {
        Button::Count => "dialog-button-Count", Button::Next => "dialog-button-Find Next",
        Button::All => "dialog-button-Find All in Current Document",
    }).unwrap();
    dialog.simulate_click(bounds.center(), gpui::Modifiers::none());
    visual.update(|_, cx| {
        let w = entity.read(cx);
        assert_eq!(w.find_state.matches.len(), 1);
        assert!(w.find_state.rich_doc_block_offsets.is_none());
        if matches!(button, Button::Count) { assert_eq!(w.find_state.status.as_deref(), Some("1 match")); }
        if matches!(button, Button::Next) { assert_eq!(w.buffer().slice_bytes(w.selection().ordered()), "hit"); }
        if matches!(button, Button::All) {
            assert_eq!(w.find_in_files.results.len(), 1);
            assert_eq!(w.find_in_files.results[0].line, 1);
            assert_eq!(w.find_in_files.results[0].match_in_line, 4..7);
        }
        assert!(w.buffer().rope.is_instance(&before.0));
        assert_eq!(w.buffer().raw_revision, before.1);
        assert_eq!(w.buffer().rich_doc.as_ref().unwrap().selection, before.2);
        assert_eq!(w.tabs[0].undo_stack.len(), before.3);
    });
    if matches!(button, Button::All) {
        visual.run_until_parked();
        visual.update(|window, _| window.activate_window());
        let bounds = visual.debug_bounds("fif-result-0").unwrap();
        visual.simulate_click(bounds.center(), gpui::Modifiers::none());
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.buffer().slice_bytes(w.selection().ordered()), "hit");
            assert_eq!(w.buffer().rich_doc.as_ref().unwrap().selection, before.2);
        });
        visual.update(|window, cx| window.focus(&entity.read(cx).focus_handle.clone(), cx));
        visual.dispatch_action(MoveLineStart);
        let raw_selection = visual.update(|_, cx| entity.read(cx).selection());
        visual.dispatch_action(crate::actions::rich_doc::RichDocView);
        visual.run_until_parked();
        let bounds = visual.debug_bounds("fif-result-0").unwrap();
        visual.simulate_click(bounds.center(), gpui::Modifiers::none());
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.selection().anchor, raw_selection.anchor);
            assert_eq!(w.selection().head, raw_selection.head);
            assert_eq!(w.buffer().rich_doc.as_ref().unwrap().selection, before.2);
            assert_eq!(w.status_message.as_deref(), Some("This result is from Raw View; switch to Raw View to select it"));
        });
    }
    cx.quit();
}
#[test] fn raw_rich_source_selected_count() { raw_selected(Button::Count); }
#[test] fn raw_rich_source_selected_next() { raw_selected(Button::Next); }
#[test] fn raw_rich_source_selected_all_and_real_result_click() { raw_selected(Button::All); }


fn with_rich_views(test: impl FnOnce(&gpui::Entity<Workspace>, &mut gpui::VisualTestContext)) {
    super::super::syntax_refresh_tests::with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.tabs[0].buffer.rich_doc = Some(crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi visible only}".to_vec()).unwrap());
            w.tabs[0].buffer.rope = ropey::Rope::from_str("<raw>raw only</raw>");
            w.tabs[0].buffer.language = None;
            w.find_state.query = "visible".into();
        }));
        test(entity, visual);
    });
}
#[test]
fn raw_rich_source_doc_view_count_and_next_keep_rich_coordinates() {
    with_rich_views(|entity, visual| {
        visual.dispatch_action(crate::actions::rich_doc::RichDocView);
        visual.dispatch_action(EditFindCount);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.find_state.status.as_deref(), Some("1 match"));
            assert!(w.find_state.rich_doc_block_offsets.is_some());
            assert!(w.selection().is_empty());
        });
        visual.dispatch_action(EditFindNext);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.buffer().rich_doc.as_ref().unwrap().selected_text(), "visible");
            assert!(w.selection().is_empty());
        });
    });
}
#[test]
fn raw_rich_source_view_switch_retires_both_scope_coordinate_systems() {
    with_rich_views(|entity, visual| {
        visual.dispatch_action(SelectLineEnd);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.find_state.query = "raw".into(); w.toggle_find_in_selection(cx);
            assert!(w.find_state.in_selection_scope.is_some());
        }));
        visual.dispatch_action(crate::actions::rich_doc::RichDocView);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert!(!w.find_state.in_selection);
            assert!(w.find_state.in_selection_scope.is_none());
            assert!(w.find_state.matches.is_empty());
            let payload = w.tabs[0].buffer.rich_doc.as_mut().unwrap();
            payload.selection = Some(crate::rich_doc::TextSelection {
                anchor: crate::rich_doc::TextPos { block: 0, run: 0, byte: 0 },
                cursor: crate::rich_doc::TextPos { block: 0, run: 0, byte: 7 },
            });
            w.find_state.query = "visible".into(); w.toggle_find_in_selection(cx);
            assert_eq!(w.find_state.in_selection_scope, Some(0..7));
            assert_eq!(w.find_state.matches, vec![0..7]);
        }));
        visual.dispatch_action(crate::actions::rich_doc::RichDocRaw);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(!w.find_state.in_selection);
            assert!(w.find_state.in_selection_scope.is_none());
            assert!(w.find_state.matches.is_empty());
            assert!(w.find_state.rich_doc_block_offsets.is_none());
        });
        visual.dispatch_action(EditFindCount);
        visual.update(|_, cx| assert_eq!(entity.read(cx).find_state.status.as_deref(), Some("0 matches")));
    });
}
#[test]
fn raw_rich_source_scope_owner_rejects_reopened_incarnation_and_other_tab() {
    with_rich_views(|entity, visual| {
        visual.dispatch_action(SelectLineEnd);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.find_state.query = "raw".into(); w.toggle_find_in_selection(cx);
            let closed = w.tabs.remove(0);
            let mut reopened = Tab::from_closed(crate::tab::ClosedTab {
                buffer: closed.buffer, session_id: closed.session_id,
            });
            reopened.view_mode = TabViewMode::Edit;
            w.tabs.insert(0, reopened);
            w.refresh_matches();
            assert!(!w.find_state.in_selection);
            assert!(w.find_state.in_selection_scope.is_none());
            w.toggle_find_in_selection(cx);
            w.active = 1; w.refresh_matches();
            assert!(!w.find_state.in_selection);
            assert!(w.find_state.in_selection_scope.is_none());
        }));
    });
}
