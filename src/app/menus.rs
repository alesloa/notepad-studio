// Menu installer + global action wiring. Each submodule owns one submenu;
// this index assembles them in visual order and installs the global
// (window-less) actions.

mod app_menu;
pub(crate) mod bookmarks;
mod edit;
mod encoding;
mod file;
mod help;
mod language;
mod macros;
mod plugins;
mod run;
mod search;
mod settings;
mod state;
mod view;

pub(crate) use state::{publish as publish_menu_state, MenuState};

use gpui::{App, Menu, MenuItem};

use crate::actions::*;
use crate::app::window::open_editor_window;
use crate::ui::menu_bar::mnemonic;
use crate::workspace::Workspace;

/// Top-level order follows Notepad++ (PowerEditor/src/Notepad_plus.rc,
/// IDR_M30_MENU): File, Edit, Search, View, Encoding, Language, Settings,
/// Tools, Macro, Run, Plugins, Window, ?. Labels carry Win32 `&` access keys
/// for the in-window bar; macOS gets them stripped and keeps its app menu.
pub(crate) fn install_menus(cx: &mut App) {
    let mut menus = Vec::new();
    if cfg!(target_os = "macos") {
        menus.push(app_menu::build());
    }
    let state = state::current();
    menus.extend([
        file::build(&state),
        edit::build(&state),
        search::build(&state),
        view::build(&state),
        encoding::build(&state),
        language::build(&state),
        settings::build(&state),
        macros::build(&state),
        run::build(&state),
        plugins::build(&state),
        help::build(&state),
    ]);
    if cfg!(target_os = "macos") {
        menus = menus.into_iter().map(strip_mnemonics).collect();
    }
    cx.set_menus(menus);
}

/// Drop `&` access-key markers for a menu system that has none (macOS).
fn strip_mnemonics(menu: Menu) -> Menu {
    Menu {
        name: mnemonic::strip(&menu.name).into(),
        items: menu
            .items
            .into_iter()
            .map(|item| match item {
                MenuItem::Action {
                    name,
                    action,
                    os_action,
                    checked,
                } => MenuItem::Action {
                    name: mnemonic::strip(&name).into(),
                    action,
                    os_action,
                    checked,
                },
                MenuItem::Submenu(sub) => MenuItem::Submenu(strip_mnemonics(sub)),
                other => other,
            })
            .collect(),
    }
}

/// Re-emit the full menu tree. Call after any state change that affects
/// menu contents — currently the File ▸ Open Recent submenu is the only
/// dynamic surface (rebuilt on every `recent::record_opened` /
/// `recent::clear_global`), but the API stays generic for future ones.
pub(crate) fn rebuild_menus(cx: &mut App) {
    install_menus(cx);
}

pub(crate) fn install_global_actions(cx: &mut App) {
    // A global handler fired from a key chord or the in-window menu bar runs
    // while gpui holds the dispatching window, so updating that window fails
    // (`WindowHandle::update` errors): About opened a spare window and
    // dialogs lost their centre-on-workspace bounds. Handlers that touch a
    // window run deferred, after dispatch hands the window back.
    cx.on_action(|_: &Quit, cx: &mut App| cx.defer(super::quit::quit_with_recovery));
    cx.on_action(|_: &FileNewWindow, cx: &mut App| cx.defer(open_editor_window));
    // Help ▸ About — route into the focused (or any open) workspace so the
    // modal mounts on a real window. If no workspace exists (rare: macOS
    // dock-icon menu after every window has been closed), spawn a fresh
    // editor window first, then show the dialog inside it.
    cx.on_action(|_: &HelpAbout, cx: &mut App| {
        cx.defer(|cx| {
            if !show_about_in_active_window(cx) {
                open_editor_window(cx);
                show_about_in_active_window(cx);
            }
        })
    });
    cx.on_action(|_: &OpenShortcutMapper, cx: &mut App| {
        cx.defer(crate::ui::shortcut_mapper::open_or_focus);
    });
    cx.on_action(|_: &OpenPreferences, cx: &mut App| {
        cx.defer(crate::ui::preferences::open_or_focus);
    });
    cx.on_action(|_: &OpenStyleConfigurator, cx: &mut App| {
        cx.defer(crate::ui::style_configurator::open_or_focus);
    });
    cx.on_action(|_: &EditOpenTextTools, cx: &mut App| {
        cx.defer(crate::ui::text_tools::open_or_focus);
    });
    cx.on_action(|_: &EditOpenClipboardHistory, cx: &mut App| {
        cx.defer(crate::ui::clipboard_history_panel::open_or_focus);
    });
    cx.on_action(|_: &EditOpenCharacterPanel, cx: &mut App| {
        cx.defer(crate::ui::character_panel::open_or_focus);
    });

    // Open Recent: dispatch into the focused window's Workspace via
    // `open_editor_window_with_path` so a click on a recents item opens the
    // file in the existing window if one is around, or spawns a fresh one if
    // every window is closed (matches Notepad++ on Windows; on macOS the
    // recents stay reachable from the dock-icon File menu after the last
    // window closes).
    //
    // Stale-entry handling: if the file no longer exists on disk, surface an
    // async Yes/No prompt to drop the entry from the MRU. We don't auto-prune
    // on startup because a path might be temporarily unreachable (network
    // share offline, external drive disconnected) — the user gets to decide
    // when they actually click it.
    cx.on_action(|action: &FileOpenRecent, cx: &mut App| {
        let path = action.path.clone();
        if path.exists() {
            cx.defer(move |cx| crate::app::window::open_path_in_active_or_new_window(path, cx));
            return;
        }
        let path_for_async = path.clone();
        cx.spawn(async move |cx| {
            let result = rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title(crate::consts::APP_NAME)
                .set_description(format!(
                    "Could not find {}.\nRemove from recent list?",
                    path_for_async.display()
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                .await;
            if matches!(result, rfd::MessageDialogResult::Yes) {
                cx.update(|cx| {
                    crate::recent::remove_path(&path_for_async);
                    rebuild_menus(cx);
                });
            }
        })
        .detach();
    });

    cx.on_action(|_: &FileClearRecent, cx: &mut App| {
        crate::recent::clear_global();
        rebuild_menus(cx);
    });

    cx.on_action(|_: &PluginsOpenFolder, _cx: &mut App| {
        crate::plugins::open_plugins_folder();
    });
}

/// Find the focused (or any) workspace window and open the About dialog
/// inside it. Returns `false` when no workspace window exists yet so the
/// caller can spawn one and retry.
fn show_about_in_active_window(cx: &mut App) -> bool {
    let workspace_window = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find_map(|h| h.downcast::<Workspace>())
        });
    let Some(handle) = workspace_window else {
        return false;
    };
    handle
        .update(cx, |workspace, _window, cx| {
            workspace.show_about_dialog(cx);
        })
        .is_ok()
}
