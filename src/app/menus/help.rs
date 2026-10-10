// Help submenu.

use gpui::{Menu, MenuItem};

use crate::actions::*;
use super::MenuState;

pub(crate) fn build(_state: &MenuState) -> Menu {
    Menu {
        // Notepad++ titles its help menu "?"; macOS keeps "Help" so the
        // system search field attaches to it.
        name: if cfg!(target_os = "macos") { "Help" } else { "&?" }.into(),
        items: vec![MenuItem::action("About", HelpAbout)],
    }
}
