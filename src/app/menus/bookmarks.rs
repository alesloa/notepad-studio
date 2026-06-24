// Bookmark submenu — nested under the Search menu (Notepad++ convention).

use gpui::{Menu, MenuItem};

use crate::actions::*;

pub(crate) fn build() -> MenuItem {
    MenuItem::submenu(Menu {
        name: "Bookmark".into(),
        items: vec![
            MenuItem::action("Toggle Bookmark", BookmarkToggle),
            MenuItem::action("Next Bookmark", BookmarkNext),
            MenuItem::action("Previous Bookmark", BookmarkPrev),
            MenuItem::separator(),
            MenuItem::action("Clear All Bookmarks", BookmarkClearAll),
        ],
    })
}
