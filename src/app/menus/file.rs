// File submenu.

use gpui::{Menu, MenuItem};

use crate::actions::*;
use crate::recent;
use crate::settings;

pub(crate) fn build() -> Menu {
    let snap = settings::snapshot();
    let mut items: Vec<MenuItem> = vec![
        MenuItem::action("New Window", FileNewWindow),
        MenuItem::separator(),
        MenuItem::action("New", FileNew),
        MenuItem::action("Open…", FileOpen),
        MenuItem::action("Open Folder as Workspace…", FileOpenFolderAsWorkspace),
    ];

    if snap.recent_files_in_submenu {
        items.push(MenuItem::submenu(build_open_recent_submenu(
            snap.recent_files_show_full_path,
        )));
    } else {
        // Flat list directly in File menu — NPP default. Each recent path
        // becomes a sibling of New / Open / Save instead of a nested submenu.
        let snapshot = recent::snapshot();
        if snapshot.is_empty() {
            items.push(MenuItem::action("Open Recent (Empty)", FileClearRecent));
        } else {
            for path in &snapshot {
                let label = recent::display_label(path, snap.recent_files_show_full_path);
                items.push(MenuItem::action(
                    label,
                    FileOpenRecent { path: path.clone() },
                ));
            }
            items.push(MenuItem::action("Clear Recent List", FileClearRecent));
        }
    }

    items.extend([
        MenuItem::separator(),
        MenuItem::action("Save", FileSave),
        MenuItem::action("Save As…", FileSaveAs),
        MenuItem::action("Save As .docx…", rich_doc::RichDocSaveAsDocx),
        MenuItem::action("Save All", FileSaveAll),
        MenuItem::separator(),
        MenuItem::action("Close Tab", FileCloseTab),
        MenuItem::action("Close All Tabs", FileCloseAllTabs),
        MenuItem::action("Restore Recent Closed Tab", FileRestoreClosedTab),
        MenuItem::separator(),
        MenuItem::action("Exit", Quit),
    ]);

    Menu {
        name: "File".into(),
        items,
    }
}

/// Build the `File ▸ Open Recent` submenu from the current global MRU
/// snapshot. Each persisted path becomes one `FileOpenRecent { path }`
/// menu item; the submenu trails with a separator and a "Clear List" entry.
/// When the MRU is empty we still emit the submenu (with a single placeholder
/// "(Empty)" entry that points at `FileClearRecent` — a no-op on an already-
/// empty list) so the submenu stays present and discoverable across launches.
///
/// Built fresh on every `install_menus` call, so any push to the global MRU
/// followed by `cx.set_menus(...)` reflects on the next menu reveal.
fn build_open_recent_submenu(full_path: bool) -> Menu {
    let snapshot = recent::snapshot();
    let mut items: Vec<MenuItem> = snapshot
        .iter()
        .map(|path| {
            let label = recent::display_label(path, full_path);
            MenuItem::action(label, FileOpenRecent { path: path.clone() })
        })
        .collect();
    if items.is_empty() {
        items.push(MenuItem::action("(Empty)", FileClearRecent));
    } else {
        items.push(MenuItem::separator());
        items.push(MenuItem::action("Clear List", FileClearRecent));
    }
    Menu {
        name: "Open Recent".into(),
        items,
    }
}
