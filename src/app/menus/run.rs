// PowerEditor/src/Notepad_plus.rc:1318-1323 — Run menu labels and order.
use super::MenuState;
use crate::actions::{RunCommand, RunSavedCommand, RunValidateShortcuts};
use gpui::{Menu, MenuItem};

pub(crate) fn build(_: &MenuState) -> Menu {
    build_with_saved(&crate::run_command::saved_snapshot())
}

// PowerEditor/src/Notepad_plus.cpp:501-508; RunDlg.cpp:413-454 — saved
// user commands follow the fixed Run items.
fn build_with_saved(saved: &[crate::run_command::SavedRunCommand]) -> Menu {
    let mut items = vec![
        MenuItem::action("&Run...", RunCommand),
        MenuItem::separator(),
        MenuItem::action("Validate shortcuts.xml", RunValidateShortcuts),
    ];
    if !saved.is_empty() {
        items.push(MenuItem::separator());
        for command in saved {
            items.push(MenuItem::action(
                crate::ui::menu_bar::mnemonic::escape(&command.name),
                RunSavedCommand {
                    id: command.id.clone(),
                },
            ));
        }
    }
    Menu {
        name: "&Run".into(),
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_menu_commands_match_catalog_and_f5_default() {
        let menu = build_with_saved(&[]);
        assert_eq!(menu.name.as_ref(), "&Run");
        assert_eq!(menu.items.len(), 3);
        let ids = ["RunCommand", "RunValidateShortcuts"];
        for (index, id) in [(0, ids[0]), (2, ids[1])] {
            let MenuItem::Action { action, .. } = &menu.items[index] else {
                panic!("expected action");
            };
            let entry = crate::shortcuts::catalog()
                .iter()
                .find(|entry| entry.id == id)
                .unwrap();
            assert_eq!(action.name(), (entry.action)().name());
            assert_eq!(entry.context, Some("Workspace"));
        }
        let run = crate::shortcuts::catalog()
            .iter()
            .find(|entry| entry.id == "RunCommand")
            .unwrap();
        assert_eq!(run.keys, ["f5"]);
        assert_eq!(run.mac_keys, ["f5"]);
        assert_eq!(
            crate::shortcuts::catalog()
                .iter()
                .filter(|entry| entry.keys.contains(&"f5"))
                .count(),
            1
        );
    }

    #[test]
    fn named_menu_entries_preserve_order_and_escape_literal_ampersands() {
        let saved = [
            crate::run_command::SavedRunCommand {
                id: "first".into(),
                name: "Build & café".into(),
                command: "one".into(),
            },
            crate::run_command::SavedRunCommand {
                id: "second".into(),
                name: "中".into(),
                command: "two".into(),
            },
        ];
        let menu = build_with_saved(&saved);
        assert_eq!(menu.items.len(), 6);
        for (index, expected) in [(4, &saved[0]), (5, &saved[1])] {
            let MenuItem::Action { name, action, .. } = &menu.items[index] else {
                panic!("expected saved action");
            };
            assert_eq!(crate::ui::menu_bar::mnemonic::strip(name), expected.name);
            let actual = action
                .as_any()
                .downcast_ref::<RunSavedCommand>()
                .expect("saved Run action");
            assert_eq!(actual.id, expected.id);
        }
    }
}
