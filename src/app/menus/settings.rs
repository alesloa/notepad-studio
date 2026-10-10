// Settings submenu. Mirrors Notepad++'s `Settings` menu — Preferences,
// Shortcut Mapper, future entries (Style Configurator, Import, …) slot
// in below.

use gpui::{Menu, MenuItem};

use crate::actions::*;
use super::MenuState;

pub(crate) fn build(_state: &MenuState) -> Menu {
    Menu {
        name: "Se&ttings".into(),
        items: vec![
            MenuItem::action("Preferences\u{200B}…", OpenPreferences),
            MenuItem::action("Style Configurator…", OpenStyleConfigurator),
            MenuItem::action("Shortcut Mapper…", OpenShortcutMapper),
        ],
    }
}
