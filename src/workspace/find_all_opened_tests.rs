use super::*;
use crate::{settings::Settings, tab::Tab};
use gpui::TestAppContext;

fn painted_all_opened(read_only: bool, extended: bool, all_read_only: bool, same_text: bool) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        confirm_replace_all_in_all_docs: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default()
    });
    let mut cx = TestAppContext::single();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            1., gpui::WindowAppearance::Dark,
        );
        w.tabs = [all_read_only, read_only].into_iter().map(|read_only| {
            let mut tab = Tab::empty();
            tab.buffer.rope = ropey::Rope::from_str("a");
            tab.buffer.read_only = read_only;
            tab
        }).collect();
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = "a".into();
        w.find_state.replacement = if same_text { "a" } else if extended { "\\t" } else { "x" }.into();
        w.find_state.extended = extended;
        w.dialog_tab = DialogTab::Replace;
        w.find_state.open = true;
        w
    });
    visual.run_until_parked();
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| {
        crate::ui::find_dialog::FindDialog::new(weak, window, cx)
    });
    dialog.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).find_focus.clone(), cx);
    });
    dialog.run_until_parked();
    let button = dialog.debug_bounds("dialog-button-Replace All in All Opened Documents").unwrap();
    dialog.simulate_click(button.center(), gpui::Modifiers::none());
    visual.update(|_, cx| {
        let w = entity.read(cx);
        let replacement = if same_text { "a" } else if extended { "\t" } else { "x" };
        assert_eq!(w.tabs[0].buffer.rope.to_string(), if all_read_only { "a" } else { replacement });
        assert_eq!(w.tabs[1].buffer.rope.to_string(), if read_only { "a" } else { replacement });
        assert_eq!(w.tabs[0].undo_stack.len(), usize::from(!all_read_only));
        assert_eq!(w.tabs[1].undo_stack.len(), usize::from(!read_only));
        assert_eq!(w.tabs[1].buffer.dirty, !read_only && !same_text);
        assert_eq!(w.active, 0);
        eprintln!("painted all-opened: read_only={read_only} extended={extended} status={:?}", w.find_state.status);
        assert_eq!(w.find_state.status.as_deref(), Some(if all_read_only {
            "No matches in any open document"
        } else if read_only {
            "Replaced 1 occurrence across 1 document"
        } else {
            "Replaced 2 occurrences across 2 documents"
        }));
    });
    cx.quit();
}

#[test]
fn painted_normal_all_opened_reports_only_actual_writable_replacements() {
    painted_all_opened(true, false, false, false);
}

#[test]
fn painted_extended_all_opened_reports_only_actual_writable_replacements() {
    painted_all_opened(true, true, false, false);
}

#[test]
fn painted_normal_all_opened_writable_control_counts_both_documents() {
    painted_all_opened(false, false, false, false);
}

#[test]
fn painted_extended_all_opened_writable_control_counts_both_documents() {
    painted_all_opened(false, true, false, false);
}

#[test]
fn painted_normal_all_opened_read_only_only_reports_no_replacement() {
    painted_all_opened(true, false, true, false);
}

#[test]
fn painted_extended_all_opened_read_only_only_reports_no_replacement() {
    painted_all_opened(true, true, true, false);
}

#[test]
fn painted_normal_all_opened_same_text_control_counts_processed_occurrences() {
    painted_all_opened(false, false, false, true);
}

#[test]
fn painted_extended_all_opened_same_text_control_counts_processed_occurrences() {
    painted_all_opened(false, true, false, true);
}
