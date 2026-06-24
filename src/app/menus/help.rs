// Help submenu.

use gpui::{Menu, MenuItem};

use crate::actions::*;

pub(crate) fn build() -> Menu {
    Menu {
        name: "Help".into(),
        items: vec![MenuItem::action("About", HelpAbout)],
    }
}
