// Settings submenu. Mirrors Notepad++'s `Settings` menu — Preferences,
// Shortcut Mapper, future entries (Style Configurator, Import, …) slot
// in below.

use gpui::{Menu, MenuItem};

use crate::actions::*;

pub(crate) fn build() -> Menu {
    Menu {
        name: "Settings".into(),
        items: vec![
            MenuItem::action("Preferences\u{200B}…", OpenPreferences),
            MenuItem::action("Style Configurator…", OpenStyleConfigurator),
            MenuItem::action("Shortcut Mapper…", OpenShortcutMapper),
        ],
    }
}
