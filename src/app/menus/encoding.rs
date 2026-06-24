// Encoding submenu — top-level menu in the menu bar (matches Notepad++).

use gpui::{Menu, MenuItem};

use crate::actions::*;

pub(crate) fn build() -> Menu {
    Menu {
        name: "Encoding".into(),
        items: vec![
            MenuItem::action("UTF-8", EncodingUtf8),
            MenuItem::action("UTF-8 BOM", EncodingUtf8Bom),
            MenuItem::action("UTF-16 LE", EncodingUtf16Le),
            MenuItem::action("UTF-16 BE", EncodingUtf16Be),
            MenuItem::action("Latin-1", EncodingLatin1),
        ],
    }
}
