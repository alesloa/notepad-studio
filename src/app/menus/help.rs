// Help submenu.

use gpui::{Menu, MenuItem};

use crate::actions::*;

pub(crate) fn build() -> Menu {
    Menu {
        // Notepad++ titles its help menu "?"; macOS keeps "Help" so the
        // system search field attaches to it.
        name: if cfg!(target_os = "macos") { "Help" } else { "&?" }.into(),
        items: vec![MenuItem::action("About", HelpAbout)],
    }
}
