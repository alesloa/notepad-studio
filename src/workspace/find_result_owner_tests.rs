use gpui::VisualContext;
use super::super::syntax_refresh_tests::with_sources;
use crate::actions::{EditFindAllCurrent, EditFindAllOpened};
use crate::tab::{ClosedTab, Tab};

fn search(entity: &gpui::Entity<super::super::Workspace>, visual: &mut gpui::VisualTestContext, all: bool) {
    visual.update_window_entity(entity, |w, window, cx| {
        w.tabs[0].buffer.rope = ropey::Rope::from_str("alpha hit caf\u{e9}\r\n");
        w.tabs[1].buffer.rope = ropey::Rope::from_str("beta hit \u{4e2d}\r\n");
        for tab in &mut w.tabs { tab.buffer.language = None; tab.selection = Default::default(); }
        w.find_state.query = "hit".into();
        if all { w.find_all_in_all_opened(&EditFindAllOpened, window, cx); }
        else { w.find_all_current(&EditFindAllCurrent, window, cx); }
        assert_eq!(w.find_in_files.results.len(), if all { 2 } else { 1 });
    });
    visual.run_until_parked();
}

#[test]
fn raw_result_owner_follows_original_tab_after_reorder() {
    with_sources(|entity, visual| {
        search(entity, visual, false);
        let original = visual.update(|_, cx| entity.read(cx).tabs[0].session_id);
        visual.update_window_entity(entity, |w, _, cx| {
            w.tabs.swap(0, 1); w.active = 1;
            w.jump_to_find_result(0, cx);
            assert_eq!(w.tabs[w.active].session_id, original);
            assert_eq!(w.buffer().slice_bytes(w.selection().ordered()), "hit");
            assert!(w.tabs[0].selection.is_empty());
        });
    });
}

#[test]
fn raw_result_owner_closed_source_does_not_select_its_successor() {
    with_sources(|entity, visual| {
        search(entity, visual, true);
        visual.update_window_entity(entity, |w, _, cx| {
            w.tabs.remove(0); w.active = 0;
            let before = w.selection();
            w.jump_to_find_result(0, cx);
            assert_eq!((w.selection().anchor,w.selection().head),(before.anchor,before.head));
        });
    });
}

#[test]
fn raw_result_owner_changed_source_does_not_use_old_offsets() {
    with_sources(|entity, visual| {
        search(entity, visual, false);
        visual.simulate_input("prefix ");
        visual.update_window_entity(entity, |w, _, cx| {
            let before = w.selection(); let revision = w.buffer().raw_revision.clone();
            w.jump_to_find_result(0, cx);
            assert_eq!((w.selection().anchor,w.selection().head),(before.anchor,before.head));
            assert_eq!(w.buffer().raw_revision,revision);
            assert_eq!(w.buffer().rope.to_string(),"prefix alpha hit caf\u{e9}\r\n");
        });
    });
}

#[test]
fn raw_result_owner_reopened_incarnation_does_not_adopt_old_result() {
    with_sources(|entity, visual| {
        search(entity, visual, false);
        visual.update_window_entity(entity, |w, _, cx| {
            let closed = w.tabs.remove(0);
            let reopened = Tab::from_closed(ClosedTab { buffer: closed.buffer, session_id: closed.session_id });
            w.tabs.insert(0,reopened);
            w.jump_to_find_result(0,cx);
            assert!(w.selection().is_empty());
        });
    });
}

#[test]
fn raw_result_owner_old_painted_row_cannot_navigate_a_replaced_result() {
    with_sources(|entity, visual| {
        search(entity, visual, false);
        let bounds=visual.debug_bounds("fif-result-0").unwrap();
        visual.update(|window,cx| {
            entity.update(cx,|w,cx| {
                w.find_state.query="alpha".into();
                w.find_all_current(&EditFindAllCurrent,window,cx);
            });
            let before=entity.read(cx).selection();
            window.simulate_mouse_down(gpui::MouseDownEvent { position:bounds.center(),..Default::default() },cx);
            let w=entity.read(cx);
            assert_eq!((w.selection().anchor,w.selection().head),(before.anchor,before.head));
            assert_eq!(w.find_in_files.last_search_query,"alpha");
        });
        visual.run_until_parked();
        let bounds=visual.debug_bounds("fif-result-0").unwrap();
        visual.simulate_click(bounds.center(),gpui::Modifiers::none());
        visual.update(|_,cx| assert_eq!(entity.read(cx).buffer().slice_bytes(entity.read(cx).selection().ordered()),"alpha"));
    });
}

#[test]
fn raw_result_owner_undo_return_does_not_revive_old_coordinates() {
    with_sources(|entity,visual| {
        search(entity,visual,false);
        let revision=visual.update(|_,cx| entity.read(cx).buffer().raw_revision.clone());
        visual.simulate_input("x");
        visual.dispatch_action(crate::actions::EditUndo);
        visual.update_window_entity(entity,|w,_,cx| {
            assert_eq!(w.buffer().rope.to_string(),"alpha hit caf\u{e9}\r\n");
            assert_eq!(w.buffer().raw_revision,revision);
            let before=w.selection();
            w.jump_to_find_result(0,cx);
            assert_eq!((w.selection().anchor,w.selection().head),(before.anchor,before.head));
        });
    });
}

#[test]
fn raw_result_owner_other_document_edit_and_caret_changes_keep_original_valid() {
    with_sources(|entity,visual| {
        search(entity,visual,false);
        visual.update_window_entity(entity,|w,window,cx| {
            w.set_active_tab(1,cx);
            window.focus(&w.focus_handle,cx);
        });
        visual.simulate_input("other edit ");
        visual.update_window_entity(entity,|w,_,cx| {
            let other=w.tabs[1].buffer.rope.clone();
            w.tabs[0].selection.collapse_to(3);
            w.tabs[0].buffer.file_path=Some(std::path::PathBuf::from("neutral-renamed.txt"));
            w.jump_to_find_result(0,cx);
            assert_eq!(w.active,0);
            assert_eq!(w.buffer().slice_bytes(w.selection().ordered()),"hit");
            assert!(w.tabs[1].buffer.rope.is_instance(&other));
            assert!(w.tabs[0].undo_stack.is_empty());
        });
        visual.run_until_parked();
        let bounds=visual.debug_bounds("fif-result-0").unwrap();
        visual.simulate_click(bounds.center(),gpui::Modifiers::none());
        visual.update(|_,cx| assert_eq!(entity.read(cx).buffer().slice_bytes(entity.read(cx).selection().ordered()),"hit"));
    });
}

#[test]
fn raw_result_owner_all_opened_surviving_row_follows_tab_after_another_closes() {
    with_sources(|entity,visual| {
        search(entity,visual,true);
        let source=visual.update(|_,cx| entity.read(cx).tabs[1].session_id);
        visual.update_window_entity(entity,|w,_,cx| {
            w.tabs.remove(0);w.active=0;
            w.jump_to_find_result(1,cx);
            assert_eq!(w.tabs[w.active].session_id,source);
            assert_eq!(w.buffer().slice_bytes(w.selection().ordered()),"hit");
            assert!(w.tabs[0].undo_stack.is_empty());
        });
    });
}
