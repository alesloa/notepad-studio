use crate::edit_ops::clipboard_all::tests::{
    clipboard, edit_state, expected_paths, fixture, sample_tabs,
};
use gpui::{Action, MenuItem, TestAppContext};

fn action(label: &str) -> Box<dyn Action> {
    super::copy_to_clipboard_menu()
        .items
        .iter()
        .find_map(|item| match item {
            MenuItem::Action { name, action, .. } if name.as_ref() == label => {
                Some(action.boxed_clone())
            }
            _ => None,
        })
        .expect("actual Copy to Clipboard menu action")
}

#[test]
fn clipboard_all_menu_order_and_unassigned_shortcuts_match_oracle() {
    let menu = super::copy_to_clipboard_menu();
    let labels: Vec<_> = menu
        .items
        .iter()
        .map(|item| match item {
            MenuItem::Action { name, .. } => name.as_ref(),
            MenuItem::Separator => "separator",
            _ => "unexpected",
        })
        .collect();
    assert_eq!(
        labels,
        [
            "Current Full File path to Clipboard",
            "Current Filename to Clipboard",
            "Current Dir. Path to Clipboard",
            "separator",
            "Copy All Filenames",
            "Copy All File Paths",
        ]
    );
    for (label, id, oracle) in [
        (
            "Copy All Filenames",
            "EditCopyAllFilenames",
            "IDM_EDIT_COPY_ALL_NAMES",
        ),
        (
            "Copy All File Paths",
            "EditCopyAllFilePaths",
            "IDM_EDIT_COPY_ALL_PATHS",
        ),
    ] {
        assert_eq!(action(label).name(), format!("notepad_studio::{id}"));
        let rows: Vec<_> = crate::shortcuts::catalog()
            .iter()
            .filter(|entry| entry.id == id)
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].npp, Some(oracle));
        assert_eq!(rows[0].context, Some("Workspace"));
        assert!(rows[0].keys.is_empty());
        assert!(rows[0].mac_keys.is_empty());
        assert_eq!((rows[0].action)().name(), action(label).name());
    }
}

#[test]
fn clipboard_all_actual_menu_actions_dispatch_to_live_workspace() {
    let _settings = crate::settings::override_for_test(Default::default());
    let _recorder = crate::macros::override_execution_for_test(false);
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, sample_tabs());
    let before = visual.update(|_, cx| edit_state(workspace.read(cx)));
    for (label, expected) in [
        (
            "Copy All Filenames",
            "café.txt\r\nnew Ω 7\r\ncafé.txt\r\ncafé.txt\r\n".into(),
        ),
        ("Copy All File Paths", expected_paths()),
    ] {
        visual.update(|window, cx| window.dispatch_action(action(label), cx));
        visual.run_until_parked();
        visual.update(|_, cx| {
            assert_eq!(clipboard(cx), expected);
            assert_eq!(edit_state(workspace.read(cx)), before);
        });
    }
    cx.quit();
}
