// PowerEditor/src/Notepad_plus.rc:1281-1308 — Tools hashes, order and labels.
use crate::actions::*;
use gpui::{Menu, MenuItem};
pub(crate) fn build() -> Menu {
    Menu { name: "T&ools".into(), items: vec![
        MenuItem::Submenu(Menu { name: "MD5".into(), items: vec![
            MenuItem::action("Generate...", ToolsMd5Generate),
            MenuItem::action("Generate from files...", ToolsMd5GenerateFromFiles),
            MenuItem::action("Generate from selection into clipboard", ToolsMd5GenerateSelection),
        ] }),
        MenuItem::Submenu(Menu { name: "SHA-1".into(), items: vec![
            MenuItem::action("Generate...", ToolsSha1Generate),
            MenuItem::action("Generate from files...", ToolsSha1GenerateFromFiles),
            MenuItem::action("Generate from selection into clipboard", ToolsSha1GenerateSelection),
        ] }),
        MenuItem::Submenu(Menu { name: "SHA-256".into(), items: vec![
            MenuItem::action("Generate...", ToolsSha256Generate),
            MenuItem::action("Generate from files...", ToolsSha256GenerateFromFiles),
            MenuItem::action("Generate from selection into clipboard", ToolsSha256GenerateSelection),
        ] }),
        MenuItem::Submenu(Menu { name: "SHA-512".into(), items: vec![
            MenuItem::action("Generate...", ToolsSha512Generate),
            MenuItem::action("Generate from files...", ToolsSha512GenerateFromFiles),
            MenuItem::action("Generate from selection into clipboard", ToolsSha512GenerateSelection),
        ] }),
    ] }
}
