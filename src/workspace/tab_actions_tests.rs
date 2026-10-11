use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

use super::Workspace;
use crate::{actions::*, tab::Tab, text::{encoding::Encoding, line_ending::LineEnding}};

#[derive(Debug, PartialEq)]
struct DocumentState {
    id: uuid::Uuid,
    text: String,
    encoding: Encoding,
    eol: LineEnding,
    revision: crate::buffer::ContentRevision,
    dirty: bool,
    read_only: bool,
    anchor: usize,
    head: usize,
    undo: usize,
    redo: usize,
}

fn documents(w: &Workspace) -> Vec<DocumentState> {
    let mut states: Vec<_> = w.tabs.iter().map(|tab| DocumentState {
        id: tab.session_id, text: tab.buffer.rope.to_string(),
        encoding: tab.buffer.encoding, eol: tab.buffer.line_ending,
        revision: tab.buffer.raw_revision.clone(), dirty: tab.buffer.dirty,
        read_only: tab.buffer.read_only, anchor: tab.selection.anchor,
        head: tab.selection.head, undo: tab.undo_stack.len(), redo: tab.redo_stack.len(),
    }).collect();
    states.sort_by_key(|state| state.id);
    states
}

fn fixture(cx: &mut TestAppContext, count: usize) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        w.window_closed = true;
        w.tabs = (0..count).map(|index| {
            let mut tab = Tab::empty();
            tab.buffer.rope = ropey::Rope::from_str(&format!("tab {index} 中😀\nrow"));
            tab.buffer.encoding = if index % 2 == 0 { Encoding::Utf16Le } else { Encoding::Utf8Bom };
            tab.buffer.line_ending = if index % 2 == 0 { LineEnding::Crlf } else { LineEnding::Lf };
            tab.buffer.mark_saved();
            tab.selection.collapse_to(9);
            tab
        }).collect();
        w.active = 0;
        w
    });
    visual.update_window_entity(&entity, |w, window, cx| {
        window.activate_window();
        window.focus(&w.focus_handle, cx);
        w.replace_byte_range(0..0, "unsaved ", cx);
    });
    visual.run_until_parked();
    (entity, visual)
}

fn dispatch(visual: &mut VisualTestContext, id: &str) {
    let entry = crate::shortcuts::catalog().iter().find(|entry| entry.id == id).expect("catalog action");
    visual.update(|window, cx| window.dispatch_action((entry.action)(), cx));
    visual.run_until_parked();
}

fn current_id(visual: &mut VisualTestContext, entity: &Entity<Workspace>) -> uuid::Uuid {
    visual.update(|_, cx| entity.read(cx).tabs[entity.read(cx).active].session_id)
}

#[test]
fn menu_tab_slots_and_positional_cycling_dispatch_to_the_visible_document() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let mut cx = TestAppContext::single();
    let (entity, visual) = fixture(&mut cx, 9);
    let (ids, before) = visual.update(|_, cx| {
        let w = entity.read(cx);
        (w.tabs.iter().map(|tab| tab.session_id).collect::<Vec<_>>(), documents(w))
    });
    for (index, id) in ids.iter().enumerate() {
        dispatch(visual, &format!("TabGoto{}", index + 1));
        assert_eq!(current_id(visual, &entity), *id);
    }
    for (action, index) in [
        ("TabLast", 8), ("TabNextSequential", 0), ("TabPrevSequential", 8),
        ("TabFirst", 0), ("TabFirst", 0), ("TabLast", 8),
    ] {
        dispatch(visual, action);
        assert_eq!(current_id(visual, &entity), ids[index]);
    }
    visual.update(|_, cx| assert_eq!(documents(entity.read(cx)), before));
    cx.quit();
}

#[test]
fn tab_move_commands_keep_uuid_selection_history_and_saved_state_at_edges() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let mut cx = TestAppContext::single();
    let (entity, visual) = fixture(&mut cx, 3);
    dispatch(visual, "TabGoto2");
    let active_id = current_id(visual, &entity);
    let before = visual.update(|_, cx| documents(entity.read(cx)));
    for (action, index) in [
        ("TabMoveForward", 2), ("TabMoveForward", 2), ("TabMoveBackward", 1),
        ("TabMoveToStart", 0), ("TabMoveBackward", 0), ("TabMoveToEnd", 2),
        ("TabMoveToEnd", 2),
    ] {
        dispatch(visual, action);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.active, index);
            assert_eq!(w.tabs[w.active].session_id, active_id);
            assert_eq!(documents(w), before);
        });
    }
    visual.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-shift-pageup" } else { "ctrl-shift-pageup" });
    visual.update(|_, cx| assert_eq!(entity.read(cx).active, 1));
    visual.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-shift-pagedown" } else { "ctrl-shift-pagedown" });
    visual.update(|_, cx| assert_eq!(entity.read(cx).active, 2));
    dispatch(visual, "TabGoto9"); // An absent slot does not select another document.
    assert_eq!(current_id(visual, &entity), active_id);
    cx.quit();
}

#[test]
fn tab_colors_apply_only_to_the_selected_read_only_document_without_an_edit() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let mut cx = TestAppContext::single();
    let (entity, visual) = fixture(&mut cx, 3);
    dispatch(visual, "TabGoto2");
    visual.update_window_entity(&entity, |w, _, _| w.active_tab_mut().buffer.read_only = true);
    let before = visual.update(|_, cx| documents(entity.read(cx)));
    for color in 1..=5 {
        dispatch(visual, &format!("TabColor{color}"));
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.buffer().tab_color, Some(color));
            assert!(w.tabs.iter().enumerate().all(|(index, tab)| index == w.active || tab.buffer.tab_color.is_none()));
            assert_eq!(documents(w), before);
        });
    }
    dispatch(visual, "TabClearColor");
    visual.update(|_, cx| {
        assert_eq!(entity.read(cx).buffer().tab_color, None);
        assert_eq!(documents(entity.read(cx)), before);
    });
    cx.quit();
}

#[test]
fn first_last_tab_refuse_a_cell_draft_without_losing_it_during_reorder() {
    crate::ui::csv_doc::actions::savepoint_tests::with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        crate::ui::csv_doc::actions::open_cell_editor(w, 0, 0, window, cx);
        let id = w.tabs[w.active].session_id;
        w.csv_models.get_mut(&id).unwrap().cell_input.as_mut().unwrap().buffer = "draft 中😀".into();
        w.active_tab_mut().buffer.read_only = true;
        w.tabs.push(Tab::empty());
        let before = documents(w);
        w.last_tab(&TabLast, window, cx);
        assert_eq!(w.tabs[w.active].session_id, id);
        w.move_active_tab_to_end(&TabMoveToEnd, window, cx);
        assert_eq!(w.active, 1);
        w.first_tab(&TabFirst, window, cx);
        assert_eq!(w.active, 1);
        assert_eq!(w.csv_models.get(&id).unwrap().cell_input.as_ref().unwrap().buffer, "draft 中😀");
        assert_eq!(documents(w), before);
    });
}

#[test]
fn tab_move_and_color_actions_ignore_empty_or_invalid_targets() {
    use gpui::{AppContext, VisualContext};
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let mut cx = TestAppContext::single();
    let entity = cx.new(|cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        w.window_closed = true;
        w
    });
    {
        let visual = cx.add_empty_window();
        visual.update_window_entity(&entity, |w, window, cx| {
            for empty in [true, false] {
                w.tabs = if empty { Vec::new() } else { vec![Tab::empty()] };
                w.active = usize::MAX;
                w.move_active_tab_to_start(&TabMoveToStart, window, cx);
                w.move_active_tab_to_end(&TabMoveToEnd, window, cx);
                w.move_active_tab_forward(&TabMoveForward, window, cx);
                w.move_active_tab_backward(&TabMoveBackward, window, cx);
                w.tab_color_1(&TabColor1, window, cx);
                w.tab_clear_color(&TabClearColor, window, cx);
                assert_eq!(w.active, usize::MAX);
                assert!(w.tabs.iter().all(|tab| tab.buffer.tab_color.is_none()));
            }
        });
    }
    cx.quit();
}
