// PowerEditor/src/Notepad_plus.rc:789-818 — View > Tab, then Word wrap.

use gpui::{Menu, MenuItem};

use crate::actions::*;
use super::MenuState;

pub(crate) fn build(state: &MenuState) -> Menu {
    Menu {
        name: "&View".into(),
        items: vec![
            MenuItem::action("Toggle Theme", ViewToggleTheme),
            MenuItem::separator(),
            MenuItem::action("Zoom In", ViewZoomIn),
            MenuItem::action("Zoom Out", ViewZoomOut),
            MenuItem::action("Reset Zoom", ViewZoomReset),
            MenuItem::separator(),
            MenuItem::submenu(tab_menu(state)),
            MenuItem::action("Word Wrap", ViewWordWrap).checked(state.word_wrap),
            MenuItem::action("Show Whitespace", ViewShowWhitespace).checked(state.show_whitespace),
            MenuItem::separator(),
            MenuItem::action("Folder as Workspace", ViewToggleFolderPanel),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", ViewToggleFullScreen),
        ],
    }
}

fn tab_menu(state: &MenuState) -> Menu {
    let selected = |color| state.has_document && state.tab_color == Some(color);
    Menu {
        name: "Ta&b".into(),
        items: vec![
            MenuItem::action("1st Tab", TabGoto1),
            MenuItem::action("2nd Tab", TabGoto2),
            MenuItem::action("3rd Tab", TabGoto3),
            MenuItem::action("4th Tab", TabGoto4),
            MenuItem::action("5th Tab", TabGoto5),
            MenuItem::action("6th Tab", TabGoto6),
            MenuItem::action("7th Tab", TabGoto7),
            MenuItem::action("8th Tab", TabGoto8),
            MenuItem::action("9th Tab", TabGoto9),
            MenuItem::separator(),
            MenuItem::action("First Tab", TabFirst),
            MenuItem::action("Last Tab", TabLast),
            MenuItem::action("Next Tab", TabNextSequential),
            MenuItem::action("Previous Tab", TabPrevSequential),
            MenuItem::separator(),
            MenuItem::action("Move to Start", TabMoveToStart),
            MenuItem::action("Move to End", TabMoveToEnd),
            MenuItem::action("Move Tab Forward", TabMoveForward),
            MenuItem::action("Move Tab Backward", TabMoveBackward),
            MenuItem::separator(),
            MenuItem::action("Apply Color 1", TabColor1).checked(selected(1)),
            MenuItem::action("Apply Color 2", TabColor2).checked(selected(2)),
            MenuItem::action("Apply Color 3", TabColor3).checked(selected(3)),
            MenuItem::action("Apply Color 4", TabColor4).checked(selected(4)),
            MenuItem::action("Apply Color 5", TabColor5).checked(selected(5)),
            MenuItem::action("Remove Color", TabClearColor),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_menu_follows_the_oracle_order_and_uses_rebindable_actions() {
        let menu = build(&MenuState::default());
        let tab = menu.items.iter().find_map(|item| match item {
            MenuItem::Submenu(menu) if menu.name.replace('&', "") == "Tab" => Some(menu),
            _ => None,
        }).expect("View > Tab menu");
        let labels: Vec<&str> = tab.items.iter().filter_map(|item| match item {
            MenuItem::Action { name, .. } => Some(name.as_ref()),
            _ => None,
        }).collect();
        assert_eq!(labels, vec![
            "1st Tab", "2nd Tab", "3rd Tab", "4th Tab", "5th Tab", "6th Tab",
            "7th Tab", "8th Tab", "9th Tab", "First Tab", "Last Tab", "Next Tab",
            "Previous Tab", "Move to Start", "Move to End", "Move Tab Forward",
            "Move Tab Backward", "Apply Color 1", "Apply Color 2", "Apply Color 3",
            "Apply Color 4", "Apply Color 5", "Remove Color",
        ]);
        for index in [9, 14, 19] {
            assert!(matches!(tab.items.get(index), Some(MenuItem::Separator)));
        }
        for item in &tab.items {
            let MenuItem::Action { action, .. } = item else { continue; };
            let entry = crate::shortcuts::catalog().iter()
                .find(|entry| (entry.action)().name() == action.name())
                .expect("Tab menu action missing from Shortcut Mapper");
            assert!(entry.npp.is_some(), "{} needs its oracle command id", entry.id);
        }
    }

    #[test]
    fn only_the_active_tabs_color_is_checked() {
        for color in [None, Some(1), Some(2), Some(3), Some(4), Some(5), Some(6)] {
            let menu = tab_menu(&MenuState {
                has_document: true, tab_color: color, ..MenuState::default()
            });
            let checked: Vec<&str> = menu.items.iter().filter_map(|item| match item {
                MenuItem::Action { name, checked: true, .. } => Some(name.as_ref()),
                _ => None,
            }).collect();
            let expected = color.filter(|color| (1..=5).contains(color))
                .map(|color| format!("Apply Color {color}"));
            assert_eq!(checked, expected.as_deref().into_iter().collect::<Vec<_>>());
        }
        assert!(tab_menu(&MenuState { tab_color: Some(2), ..MenuState::default() })
            .items.iter().all(|item| !matches!(item, MenuItem::Action { checked: true, .. })));
    }
}
