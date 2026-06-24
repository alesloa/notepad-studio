// macOS-style app submenu — the leftmost menu bearing the app name. Holds the
// About entry plus the system Quit. On Linux/Windows this still appears first
// in the menu bar (gpui's native menus mirror the same layout).

use gpui::{Menu, MenuItem};

use crate::actions::*;
use crate::consts::APP_NAME;

pub(crate) fn build() -> Menu {
    Menu {
        name: APP_NAME.into(),
        items: vec![
            MenuItem::action(format!("About {APP_NAME}"), HelpAbout),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ],
    }
}
