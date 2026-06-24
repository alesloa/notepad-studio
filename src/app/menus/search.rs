// Search submenu.

use gpui::{Menu, MenuItem};

use crate::actions::*;

pub(crate) fn build() -> Menu {
    Menu {
        name: "Search".into(),
        items: vec![
            MenuItem::action("Find…", EditFind),
            MenuItem::action("Find Next", EditFindNext),
            MenuItem::action("Find Previous", EditFindPrev),
            MenuItem::action("Find All in Current Document", EditFindAllCurrent),
            MenuItem::action("Count", EditFindCount),
            MenuItem::separator(),
            MenuItem::action("Replace…", EditReplace),
            MenuItem::separator(),
            MenuItem::action("Go to Line…", EditGotoLine),
            MenuItem::separator(),
            MenuItem::action("Find in Files…", SearchFindInFiles),
            MenuItem::action("Find in Projects…", SearchFindInProjects),
            MenuItem::separator(),
            MenuItem::action("Mark…", SearchMark),
            MenuItem::action("Clear all marks", MarkClearAll),
            MenuItem::action("Copy Marked Text", MarkCopy),
            MenuItem::separator(),
            super::bookmarks::build(),
        ],
    }
}
