// Plugins menu. Notepad++'s top-level "Plugins" menu — one submenu per
// loaded plugin with each command as a leaf action. "Open Plugins Folder"
// lives at the top so the user can drop a fresh dylib in without hunting
// for the config dir.
//
// Rebuilt on every `install_menus` / `rebuild_menus` call, so when the
// scan-and-load pass at startup populates `plugins::snapshot()` the menu
// reflects whatever's there. v1 doesn't support hot-reload — restart to
// pick up new plugins.

use gpui::{Menu, MenuItem};

use crate::actions::*;
use super::MenuState;
use crate::plugins;
use crate::ui::menu_bar::mnemonic;

pub(crate) fn build(_state: &MenuState) -> Menu {
    let entries = plugins::snapshot();
    let mut items: Vec<MenuItem> = Vec::with_capacity(2 + entries.len());
    items.push(MenuItem::action("Open Plugins Folder", PluginsOpenFolder));
    items.push(MenuItem::separator());

    if entries.is_empty() {
        // gpui's Menu API doesn't ship a "disabled item" primitive, so the
        // empty state surfaces as a no-op submenu. The label still tells
        // the user where to look.
        items.push(MenuItem::submenu(Menu {
            name: "No plugins installed".into(),
            items: vec![MenuItem::action(
                "Drop a .dylib / .so / .dll in the plugins folder",
                PluginsOpenFolder,
            )],
        }));
        return Menu {
            name: "&Plugins".into(),
            items,
        };
    }

    for entry in entries {
        let plugin_name = entry.name.clone();
        let mut sub_items: Vec<MenuItem> = Vec::with_capacity(entry.commands.len());
        for cmd in entry.commands {
            sub_items.push(MenuItem::action(
                mnemonic::escape(&cmd.name),
                PluginRunCommand {
                    plugin: plugin_name.clone(),
                    command_id: cmd.id,
                },
            ));
        }
        items.push(MenuItem::submenu(Menu {
            name: mnemonic::escape(&entry.name).into(),
            items: sub_items,
        }));
    }

    Menu {
        name: "&Plugins".into(),
        items,
    }
}
