use super::*;
use crate::actions::{MoveDown, MoveLineStart, MoveRight, SelectLineEnd, SelectRight};
use crate::{settings::Settings, tab::Tab};
use gpui::TestAppContext;

#[derive(Clone, Copy)]
enum SearchButton { All, Count, Next }

fn painted_current(scoped: bool, button: SearchButton, word_scope: bool, query: &str, expected: usize) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default()
    });
    let mut cx = TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            1., gpui::WindowAppearance::Dark,
        );
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str("outside hit\r\n\u{4e2d} hit caf\u{e9}\r\noutside hit");
        w.tabs = vec![tab];
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = query.into();
        w.find_state.regex = word_scope;
        w.dialog_tab = DialogTab::Find;
        w.find_state.open = true;
        w
    });
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        window.activate_window();
        window.focus(&w.focus_handle, cx);
    }));
    visual.run_until_parked();
    if scoped {
        visual.dispatch_action(MoveDown);
        visual.dispatch_action(MoveLineStart);
        if word_scope {
            visual.dispatch_action(MoveRight);
            visual.dispatch_action(MoveRight);
            for _ in 0..3 { visual.dispatch_action(SelectRight); }
        } else {
            visual.dispatch_action(SelectLineEnd);
        }
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.buffer().slice_bytes(w.selection().ordered()),
                if word_scope { "hit" } else { "\u{4e2d} hit caf\u{e9}" });
        });
    }
    let before = visual.update(|_, cx| {
        let w = entity.read(cx);
        (w.buffer().rope.clone(), w.buffer().raw_revision.clone(), w.buffer().dirty,
            w.selection(), w.tabs[0].undo_stack.len(), w.tabs[0].redo_stack.len())
    });
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| {
        crate::ui::find_dialog::FindDialog::new(weak, window, cx)
    });
    dialog.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).find_focus.clone(), cx);
    });
    dialog.run_until_parked();
    if scoped {
        let checkbox = dialog.debug_bounds("dialog-checkbox-In selection").unwrap();
        dialog.simulate_click(checkbox.center(), gpui::Modifiers::none());
        dialog.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.find_state.in_selection);
            assert_eq!(w.find_state.in_selection_scope, Some(before.3.ordered()));
        });
    }
    let bounds = dialog.debug_bounds(match button {
        SearchButton::Count => "dialog-button-Count",
        SearchButton::Next => "dialog-button-Find Next",
        SearchButton::All => "dialog-button-Find All in Current Document",
    }).unwrap();
    dialog.simulate_click(bounds.center(), gpui::Modifiers::none());
    visual.update(|_, cx| {
        let w = entity.read(cx);
        assert!(w.buffer().rope.is_instance(&before.0));
        assert_eq!(w.buffer().raw_revision, before.1);
        assert_eq!(w.buffer().dirty, before.2);
        assert_eq!(w.selection().anchor, before.3.anchor);
        assert_eq!(w.selection().head, before.3.head);
        assert_eq!(w.selection().is_dragging, before.3.is_dragging);
        assert_eq!(w.tabs[0].undo_stack.len(), before.4);
        assert_eq!(w.tabs[0].redo_stack.len(), before.5);
        if matches!(button, SearchButton::Count) {
            let status = format!("{expected} match{}", if expected == 1 { "" } else { "es" });
            assert_eq!(w.find_state.status.as_deref(), Some(status.as_str()));
            assert_eq!(w.find_state.matches.len(), expected);
        } else if matches!(button, SearchButton::Next) {
            assert_eq!(w.find_state.matches.len(), expected);
        } else {
            assert_eq!(w.find_in_files.results.len(), expected);
            if scoped && expected == 1 {
                let hit = &w.find_in_files.results[0];
                assert_eq!(hit.line, 1);
                assert_eq!(hit.match_in_line, 4..7);
                assert_eq!(hit.line_text, "\u{4e2d} hit caf\u{e9}");
            }
        }
    });
    cx.quit();
}

#[test]
fn painted_find_all_current_limits_results_to_real_selected_line() {
    painted_current(true, SearchButton::All, false, "hit", 1);
}
#[test]
fn painted_find_all_current_full_document_control() {
    painted_current(false, SearchButton::All, false, "hit", 3);
}
#[test]
fn painted_count_real_selected_line_control() {
    painted_current(true, SearchButton::Count, false, "hit", 1);
}
#[test]
fn painted_find_all_current_preserves_midline_anchor_context() {
    painted_current(true, SearchButton::All, true, "^hit", 0);
}
#[test]
fn painted_find_all_current_bounds_greedy_match_to_selected_word() {
    painted_current(true, SearchButton::All, true, "hit.*", 1);
}
#[test]
fn painted_find_all_current_treats_selection_end_as_target_end() {
    painted_current(true, SearchButton::All, true, "hit$", 1);
}

#[test]
fn painted_count_preserves_midline_anchor_context() {
    painted_current(true, SearchButton::Count, true, "^hit", 0);
}
#[test]
fn painted_find_next_preserves_midline_anchor_context() {
    painted_current(true, SearchButton::Next, true, "^hit", 0);
}
#[test]
fn painted_count_bounds_greedy_match_to_selected_word() {
    painted_current(true, SearchButton::Count, true, "hit.*", 1);
}
#[test]
fn painted_find_next_treats_selection_end_as_target_end() {
    painted_current(true, SearchButton::Next, true, "hit$", 1);
}
