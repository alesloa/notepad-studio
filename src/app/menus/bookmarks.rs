// Bookmark submenu — nested under the Search menu (Notepad++ convention).
// PowerEditor/src/Notepad_plus.rc:731–740 — order and labels, no separators.

use gpui::{Menu, MenuItem};

use crate::actions::*;

pub(crate) fn build() -> MenuItem {
    MenuItem::submenu(Menu {
        name: "Bookmark".into(),
        items: vec![
            MenuItem::action("Toggle Bookmark", BookmarkToggle),
            MenuItem::action("Next Bookmark", BookmarkNext),
            MenuItem::action("Previous Bookmark", BookmarkPrev),
            MenuItem::action("Clear All Bookmarks", BookmarkClearAll),
            MenuItem::action("Copy Bookmarked Lines", CopyBookmarkedLines),
        ],
    })
}
