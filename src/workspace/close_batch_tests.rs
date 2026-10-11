use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext};
use crate::{actions::*, settings::Settings, tab::Tab, workspace::Workspace};

fn press(visual: &mut VisualTestContext, id: &str) {
    let entry = crate::shortcuts::catalog().iter().find(|entry| entry.id == id).unwrap();
    visual.simulate_keystrokes(crate::shortcuts::platform_default_keys(entry)[0]);
}

fn with_window(count: usize, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext, &[uuid::Uuid])) {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false, tab_exit_on_last_close: false,
        ..Default::default()
    });
    let mut cx = TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.tabs = (0..count).map(|index| {
            let mut tab = Tab::empty();
            tab.buffer.rope = ropey::Rope::from_str(&format!("clean {index} 中😀\n"));
            tab.buffer.file_path = Some(std::env::temp_dir().join(format!("close-batch-{index}.txt")));
            tab
        }).collect();
        w.active = count.saturating_sub(1);
        w.window_closed = true;
        w.touch_mru();
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    let ids = visual.update(|_, cx| entity.read(cx).tabs.iter().map(|tab| tab.session_id).collect::<Vec<_>>());
    test(&entity, visual, &ids);
    cx.quit();
}

fn assert_one_new_blank(entity: &Entity<Workspace>, visual: &mut VisualTestContext, old_ids: &[uuid::Uuid]) {
    visual.run_until_parked();
    visual.update(|_, cx| {
        let w = entity.read(cx);
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.active, 0);
        assert!(!old_ids.contains(&w.tabs[0].session_id));
        assert_eq!(w.buffer().rope.len_bytes(), 0);
        assert!(!w.buffer().dirty);
        assert_eq!(w.buffer().new_label.as_ref().map(|label| label.to_string()).as_deref(), Some("new 1"));
        assert_eq!(w.closed_tabs.len(), old_ids.len());
        assert_eq!(w.tab_mru.front(), Some(&w.tabs[0].session_id));
    });
}

#[test]
fn close_batch_unchanged_never_exposes_an_empty_rendered_strip() {
    for count in [1, 2, 5] {
        for file_action in [false, true] {
            with_window(count, |entity, visual, old_ids| {
                if file_action { visual.dispatch_action(FileCloseAllUnchanged); }
                else { visual.update_window_entity(entity, |w, window, cx| w.close_all_unchanged(window, cx)); }
                assert_one_new_blank(entity, visual, old_ids);
            });
        }
    }
}

#[test]
fn close_batch_single_last_close_can_create_its_blank_without_a_csv_preflight() {
    with_window(1, |entity, visual, old_ids| {
        press(visual, "FileCloseTab");
        assert_one_new_blank(entity, visual, old_ids);
    });
}

#[test]
fn close_batch_close_all_keeps_one_blank_and_all_closed_documents() {
    for count in [1, 2, 5] {
        with_window(count, |entity, visual, old_ids| {
            press(visual, "FileCloseAllTabs");
            assert_one_new_blank(entity, visual, old_ids);
        });
    }
}

#[test]
fn close_batch_close_all_keeps_a_document_opened_after_dispatch() {
    with_window(2, |entity, visual, old_ids| {
        let kept = visual.update_window_entity(entity, |w, window, cx| {
            w.close_all_tabs(&FileCloseAllTabs, window, cx);
            let mut tab = Tab::empty();
            let id = tab.session_id;
            tab.buffer.rope = ropey::Rope::from_str("opened later 中😀");
            w.tabs.push(tab);
            w.active = w.tabs.len() - 1;
            w.touch_mru();
            id
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs.len(), 1);
            assert_eq!(w.tabs[w.active].session_id, kept);
            assert_eq!(w.buffer().rope.to_string(), "opened later 中😀");
            assert!(old_ids.iter().all(|id| w.closed_tabs.iter().any(|tab| tab.session_id == *id)));
        });
    });
}
