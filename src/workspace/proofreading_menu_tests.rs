// Observe App's native-menu input, not native macOS rendering.
use super::*;
use crate::{actions::{EditToggleGrammar, EditToggleSpelling}, settings::Settings, tab::Tab};
use gpui::{Action, OwnedMenu, OwnedMenuItem, TestAppContext};

fn checked<A: Action>(menus: &[OwnedMenu]) -> Option<bool> {
    for menu in menus {
        for item in &menu.items {
            match item {
                OwnedMenuItem::Action { action, checked, .. } if action.as_any().is::<A>() => return Some(*checked),
                OwnedMenuItem::Submenu(menu) => {
                    if let Some(value) = checked::<A>(std::slice::from_ref(menu)) { return Some(value); }
                }
                _ => {}
            }
        }
    }
    None
}

fn installed_flags(observer: &crate::app::menus::publication_fixture::Guard) -> (usize, (bool, bool)) {
    let (count, menus) = observer.snapshot();
    (count, (checked::<EditToggleSpelling>(&menus).expect("registered spelling menu"),
        checked::<EditToggleGrammar>(&menus).expect("registered grammar menu")))
}

fn toggle_menu(spelling: bool, initial: bool) {
    let _settings = crate::settings::override_for_test(Settings {
        proofread_spelling: initial, proofread_grammar: initial,
        toolbar_hidden: true, folder_panel_visible: false,
        ..Settings::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _macros = crate::macros::override_playback_for_test(Vec::new(), Default::default());
    let observer = crate::app::menus::publication_fixture::observe();
    let writes = crate::settings::persistence_fixture::observe();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        crate::shortcuts::register_catalog(cx, &Default::default());
        crate::app::menus::install_menus(cx);
    });
    let source = "\u{feff}A caf\u{e9} sentence.\r\nhttps://example.com\r\n";
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(source);
        tab.buffer.encoding = crate::text::encoding::Encoding::Utf8Bom;
        tab.buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
        tab.buffer.mark_saved();
        tab.selection.anchor = 3; tab.selection.head = 10;
        w.tabs = vec![tab]; w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w
    });
    visual.update(|window, cx| {
        window.activate_window(); window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    assert_eq!(installed_flags(&observer).1, (initial, initial));
    for value in [!initial, initial] {
        let before = observer.snapshot().0;
        if spelling { visual.dispatch_action(EditToggleSpelling); }
        else { visual.dispatch_action(EditToggleGrammar); }
        visual.run_until_parked();
        visual.update(|_, cx| {
            let prefs = crate::settings::snapshot();
            let expected = if spelling { (value, initial) } else { (initial, value) };
            assert_eq!((prefs.proofread_spelling, prefs.proofread_grammar), expected, "registered handler updated preferences");
            let (published, flags) = installed_flags(&observer);
            assert!(published > before, "registered handler publishes a refreshed menu without reopening it");
            assert_eq!(flags, expected, "published checkmarks follow the toggle");
            assert_eq!(writes.count(), 0, "scoped toggles do not enter real settings persistence");
            entity.update(cx, |w, _| {
                assert_eq!(w.buffer().rope.to_string(), source);
                assert_eq!(w.selection().anchor, 3); assert_eq!(w.selection().head, 10);
                assert_eq!(w.buffer().encoding, crate::text::encoding::Encoding::Utf8Bom);
                assert_eq!(w.buffer().line_ending, crate::text::line_ending::LineEnding::Crlf);
                assert!(!w.buffer().dirty);
                assert!(w.tabs[0].undo_stack.is_empty()); assert!(w.tabs[0].redo_stack.is_empty());
            });
        });
    }
    visual.update(|_, cx| entity.update(cx, |w, _| { w.proofreader.close(); w.window_closed = true; }));
    cx.quit();
}

#[test] fn registered_spelling_toggle_refreshes_installed_checkmark() { toggle_menu(true, Settings::default().proofread_spelling); }
#[test] fn registered_grammar_toggle_refreshes_installed_checkmark() { toggle_menu(false, Settings::default().proofread_grammar); }
#[test] fn disabled_checks_can_be_enabled_without_editing_a_document() {
    toggle_menu(true, false); toggle_menu(false, false);
}
