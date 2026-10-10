// View submenu.

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
            MenuItem::action("Word Wrap", ViewWordWrap).checked(state.word_wrap),
            MenuItem::action("Show Whitespace", ViewShowWhitespace).checked(state.show_whitespace),
            MenuItem::separator(),
            MenuItem::action("Folder as Workspace", ViewToggleFolderPanel),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", ViewToggleFullScreen),
        ],
    }
}
