// Notepad++-style right-click context menu for the tab strip.
// Behaviour reference: PowerEditor/src/Notepad_plus.rc:464,469,811-816 —
// Rename, Close All but Active, and the tab-colour menu.
//
// Menu layout reproduces NPP 1:1:
//   Close
//   Close Multiple Tabs ▶
//   ───
//   Save
//   Save As…
//   Open into ▶
//   Rename
//   Move to Trash
//   Reload
//   Print
//   ───
//   Read-Only
//   Clear Read-Only Flag
//   ───
//   Copy to Clipboard ▶
//   Move Document ▶
//   Apply Color to Tab ▶
//
// Submenus cascade to the right of their parent row. The whole structure
// is rebuilt each frame from `Workspace::tab_context_menu` state — there's
// no entity for the menu itself, only state on the workspace.

use gpui::{
    Corner, CursorStyle, Entity, IntoElement, MouseButton, Point, anchored, deferred, div,
    prelude::*, px, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::{TabContextMenuState, TabSubmenu, Workspace};

const MENU_WIDTH_REM: f32 = 230. / 16.;
const SUBMENU_WIDTH_REM: f32 = 230. / 16.;
const ROW_HEIGHT_REM: f32 = 22. / 16.;

/// Snapshot of per-tab data the menu needs for enabled-state checks. Pulled
/// once at the top of `tab_context_menu` so the closure-heavy click
/// handlers don't have to keep re-reading.
pub(crate) struct TabFlags {
    has_path: bool,
    dirty: bool,
    is_read_only: bool,
    tab_color: Option<u8>,
    tabs_total: usize,
    is_first: bool,
    is_last: bool,
    has_clean_others: bool,
    can_close_left: bool,
    can_close_right: bool,
}

pub(crate) fn flags_for(workspace: &Workspace, idx: usize) -> Option<TabFlags> {
    let tab = workspace.tabs.get(idx)?;
    let total = workspace.tabs.len();
    let has_clean_others = workspace
        .tabs
        .iter()
        .enumerate()
        .any(|(i, t)| i != idx && !t.buffer.dirty);
    Some(TabFlags {
        has_path: tab.buffer.file_path.is_some(),
        dirty: tab.buffer.dirty,
        is_read_only: tab.buffer.read_only,
        tab_color: tab.buffer.tab_color,
        tabs_total: total,
        is_first: idx == 0,
        is_last: idx + 1 == total,
        has_clean_others,
        can_close_left: idx > 0,
        can_close_right: idx + 1 < total,
    })
}

/// Render the entire tab right-click menu (top-level panel + any open
/// submenu cascade). Caller spawns this from the workspace render's
/// `when_some(self.tab_context_menu, …)` branch and passes pre-computed
/// flags so we don't double-lease the workspace entity (we're already
/// inside its `update`).
pub(crate) fn tab_context_menu(
    theme: &Theme,
    state: TabContextMenuState,
    flags: TabFlags,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let backdrop_workspace = workspace.clone();
    let backdrop = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .on_mouse_down(MouseButton::Left, {
            let ws = backdrop_workspace.clone();
            move |_e, _w, cx| {
                ws.update(cx, |w, cx| {
                    if resolve_popup_target(w, state, cx).is_some() { w.hide_tab_context_menu(cx); }
                });
            }
        })
        .on_mouse_down(MouseButton::Right, move |_e, _w, cx| {
            backdrop_workspace.update(cx, |w, cx| {
                if resolve_popup_target(w, state, cx).is_some() { w.hide_tab_context_menu(cx); }
            });
        });

    let main_panel = main_menu_panel(theme, &state, &flags, workspace.clone());

    // Submenus cascade to the right of the main panel. We pin the cascade's
    // x to the right edge of the main panel — `submenu_anchor.x = main.x +
    // panel_width`. Y matches the row that opened it.
    let submenu = state.open_submenu.map(|which| {
        submenu_panel(theme, which, state, &flags, workspace.clone())
    });

    let menu_x = state.position.x;
    let menu_y = state.position.y;

    deferred(
        div().absolute().top_0().left_0().size_full().child(
            div()
                .size_full()
                .child(backdrop)
                .child(
                    anchored()
                        .position(Point::new(menu_x, menu_y))
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(px(4.))
                        .child(main_panel),
                )
                .when_some(submenu, |this, sub| {
                    this.child(
                        anchored()
                            .position(state.submenu_anchor)
                            .anchor(Corner::TopLeft)
                            .snap_to_window_with_margin(px(4.))
                            .child(sub),
                    )
                }),
        ),
    )
    .with_priority(2)
}

#[derive(Clone, Copy)]
enum MenuItem {
    Sep,
    Leaf {
        label: &'static str,
        action: LeafAction,
        enabled: bool,
        checkmark: bool,
    },
    Submenu {
        label: &'static str,
        which: TabSubmenu,
        enabled: bool,
    },
}

#[derive(Clone, Copy)]
enum LeafAction {
    Close,
    Save,
    SaveAs,
    Rename,
    MoveToTrash,
    Reload,
    Print,
    ToggleReadOnly,
    ClearReadOnlyFlag,
}

fn main_items(flags: &TabFlags) -> Vec<MenuItem> {
    vec![
        MenuItem::Leaf {
            label: "Close",
            action: LeafAction::Close,
            enabled: true,
            checkmark: false,
        },
        MenuItem::Submenu {
            label: "Close Multiple Tabs",
            which: TabSubmenu::CloseMultiple,
            enabled: flags.tabs_total > 1,
        },
        MenuItem::Sep,
        MenuItem::Leaf {
            label: "Save",
            action: LeafAction::Save,
            enabled: flags.dirty && flags.has_path,
            checkmark: false,
        },
        MenuItem::Leaf {
            label: "Save As…",
            action: LeafAction::SaveAs,
            enabled: true,
            checkmark: false,
        },
        MenuItem::Submenu {
            label: "Open into",
            which: TabSubmenu::OpenInto,
            enabled: flags.has_path,
        },
        MenuItem::Leaf {
            label: "Rename",
            action: LeafAction::Rename,
            enabled: flags.has_path,
            checkmark: false,
        },
        MenuItem::Leaf {
            label: trash_label(),
            action: LeafAction::MoveToTrash,
            enabled: flags.has_path,
            checkmark: false,
        },
        MenuItem::Leaf {
            label: "Reload",
            action: LeafAction::Reload,
            enabled: flags.has_path,
            checkmark: false,
        },
        MenuItem::Leaf {
            // Print needs platform-specific impl (NSPrintOperation, GDI,
            // CUPS) — explicitly deferred to v2; greyed out here.
            label: "Print",
            action: LeafAction::Print,
            enabled: false,
            checkmark: false,
        },
        MenuItem::Sep,
        MenuItem::Leaf {
            label: "Read-Only",
            action: LeafAction::ToggleReadOnly,
            enabled: true,
            checkmark: flags.is_read_only,
        },
        MenuItem::Leaf {
            label: "Clear Read-Only Flag",
            action: LeafAction::ClearReadOnlyFlag,
            enabled: flags.is_read_only && flags.has_path,
            checkmark: false,
        },
        MenuItem::Sep,
        MenuItem::Submenu {
            label: "Copy to Clipboard",
            which: TabSubmenu::CopyToClipboard,
            enabled: flags.has_path,
        },
        MenuItem::Submenu {
            label: "Move Document",
            which: TabSubmenu::MoveDocument,
            enabled: flags.tabs_total > 1,
        },
        MenuItem::Submenu {
            label: "Apply Color to Tab",
            which: TabSubmenu::ApplyColor,
            enabled: true,
        },
    ]
}

fn trash_label() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Move to Trash"
    }
    #[cfg(target_os = "windows")]
    {
        "Move to Recycle Bin"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "Move to Trash"
    }
}

fn open_into_label_file_manager() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Open Containing Folder in Finder"
    }
    #[cfg(target_os = "windows")]
    {
        "Open Containing Folder in Explorer"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "Open Containing Folder in File Manager"
    }
}

fn open_into_label_terminal() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Open Containing Folder in Terminal"
    }
    #[cfg(target_os = "windows")]
    {
        "Open Containing Folder in cmd"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "Open Containing Folder in Terminal"
    }
}

fn main_menu_panel(
    theme: &Theme,
    state: &TabContextMenuState,
    flags: &TabFlags,
    workspace: Entity<Workspace>,
) -> gpui::Div {
    let items = main_items(flags);
    let mut panel = panel_base(theme, MENU_WIDTH_REM);

    // Close any open submenu when the cursor enters a non-submenu row so
    // the cascade doesn't linger over a row that no longer wants it.
    for (idx, item) in items.iter().enumerate() {
        match *item {
            MenuItem::Sep => {
                panel = panel.child(separator_row(theme));
            }
            MenuItem::Leaf {
                label,
                action,
                enabled,
                checkmark,
            } => {
                panel = panel.child(leaf_row(
                    theme,
                    label,
                    enabled,
                    checkmark,
                    {
                        let ws = workspace.clone();
                        let painted = *state;
                        move |window, cx| {
                            run_leaf_action(action, painted, &ws, window, cx);
                        }
                    },
                    {
                        let ws = workspace.clone();
                        let painted = *state;
                        move |_w, cx| {
                            ws.update(cx, |w, cx| {
                                if resolve_popup_target(w, painted, cx).is_some() { w.close_tab_submenu(cx); }
                            });
                        }
                    },
                ));
            }
            MenuItem::Submenu {
                label,
                which,
                enabled,
            } => {
                let row_y = px(4.) + px(idx as f32 * (ROW_HEIGHT_REM * 16.));
                let anchor_x = state.position.x + rems(MENU_WIDTH_REM).to_pixels(px(16.));
                let anchor_y = state.position.y + row_y;
                panel = panel.child(submenu_row(
                    theme,
                    label,
                    enabled,
                    state.open_submenu == Some(which),
                    {
                        let ws = workspace.clone();
                        let painted = *state;
                        move |_w, cx| {
                            ws.update(cx, |w, cx| {
                                if resolve_popup_target(w, painted, cx).is_none() { return; }
                                w.open_tab_submenu(
                                    which,
                                    Point::new(anchor_x, anchor_y),
                                    cx,
                                );
                            });
                        }
                    },
                ));
            }
        }
    }

    panel
}

fn submenu_panel(
    theme: &Theme,
    which: TabSubmenu,
    state: TabContextMenuState,
    flags: &TabFlags,
    workspace: Entity<Workspace>,
) -> gpui::Div {
    match which {
        TabSubmenu::CloseMultiple => close_multiple_panel(theme, state, flags, workspace),
        TabSubmenu::OpenInto => open_into_panel(theme, state, flags, workspace),
        TabSubmenu::CopyToClipboard => copy_panel(theme, state, flags, workspace),
        TabSubmenu::MoveDocument => move_document_panel(theme, state, flags, workspace),
        TabSubmenu::ApplyColor => apply_color_panel(theme, state, flags, workspace),
    }
}

fn close_multiple_panel(
    theme: &Theme,
    state: TabContextMenuState,
    flags: &TabFlags,
    workspace: Entity<Workspace>,
) -> gpui::Div {
    let mut panel = panel_base(theme, SUBMENU_WIDTH_REM);
    panel = panel.child(leaf_row(
        theme,
        "Close All BUT This",
        flags.tabs_total > 1,
        false,
        {
            let ws = workspace.clone();
            move |window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    w.close_all_but(target, window, cx);
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(leaf_row(
        theme,
        "Close All to the Left",
        flags.can_close_left,
        false,
        {
            let ws = workspace.clone();
            move |window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    w.close_all_to_left(target, window, cx);
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(leaf_row(
        theme,
        "Close All to the Right",
        flags.can_close_right,
        false,
        {
            let ws = workspace.clone();
            move |window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    w.close_all_to_right(target, window, cx);
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(leaf_row(
        theme,
        "Close All Unchanged",
        flags.has_clean_others || (!flags.dirty && flags.tabs_total > 1),
        false,
        {
            let ws = workspace.clone();
            move |window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(_target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    w.close_all_unchanged(window, cx);
                });
            }
        },
        |_, _| {},
    ));
    panel
}

fn open_into_panel(
    theme: &Theme,
    state: TabContextMenuState,
    _flags: &TabFlags,
    workspace: Entity<Workspace>,
) -> gpui::Div {
    let mut panel = panel_base(theme, SUBMENU_WIDTH_REM + 60. / 16.);

    panel = panel.child(leaf_row(
        theme,
        open_into_label_file_manager(),
        true,
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    if let Some(path) = w
                        .tabs
                        .get(target)
                        .and_then(|t| t.buffer.file_path.clone())
                    {
                        if let Err(err) = crate::shell::reveal_in_file_manager(&path) {
                            w.set_status(format!("Reveal failed: {err}"));
                        }
                    }
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(leaf_row(
        theme,
        open_into_label_terminal(),
        true,
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    if let Some(path) = w
                        .tabs
                        .get(target)
                        .and_then(|t| t.buffer.file_path.clone())
                    {
                        let dir = path.parent().unwrap_or_else(|| std::path::Path::new(".")).to_path_buf();
                        if let Err(err) = crate::shell::open_terminal_at(&dir) {
                            w.set_status(format!("Open terminal failed: {err}"));
                        }
                    }
                });
            }
        },
        |_, _| {},
    ));
    // Open Containing Folder as Workspace — adds the active tab's parent
    // dir to the folder-panel root list and shows the panel.
    panel = panel.child(leaf_row(
        theme,
        "Open Containing Folder as Workspace",
        true,
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    if let Some(parent) = w
                        .tabs
                        .get(target)
                        .and_then(|t| t.buffer.file_path.as_ref())
                        .and_then(|p| p.parent())
                        .map(std::path::Path::to_path_buf)
                    {
                        let n = crate::ui::folder_panel::add_root(parent.clone());
                        crate::ui::folder_panel::set_visible(true);
                        w.set_status(format!(
                            "Added '{}' as workspace root ({n} total)",
                            parent.display()
                        ));
                        cx.notify();
                    }
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(separator_row(theme));
    panel = panel.child(leaf_row(
        theme,
        "Open in Default Viewer",
        true,
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    if let Some(path) = w
                        .tabs
                        .get(target)
                        .and_then(|t| t.buffer.file_path.clone())
                    {
                        if let Err(err) = crate::shell::open_with_default_viewer(&path) {
                            w.set_status(format!("Open in default viewer failed: {err}"));
                        }
                    }
                });
            }
        },
        |_, _| {},
    ));
    panel
}

fn copy_panel(
    theme: &Theme,
    state: TabContextMenuState,
    _flags: &TabFlags,
    workspace: Entity<Workspace>,
) -> gpui::Div {
    let mut panel = panel_base(theme, SUBMENU_WIDTH_REM);

    panel = panel.child(leaf_row(
        theme,
        "Copy Full File Path",
        true,
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    if let Some(p) = w
                        .tabs
                        .get(target)
                        .and_then(|t| t.buffer.file_path.clone())
                    {
                        w.cancel_bookmark_copy(cx);
                        crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(
                            p.display().to_string(),
                        ));
                        w.set_status("Copied full path");
                    }
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(leaf_row(
        theme,
        "Copy Filename",
        true,
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    if let Some(p) = w
                        .tabs
                        .get(target)
                        .and_then(|t| t.buffer.file_path.clone())
                    {
                        if let Some(name) = p.file_name() {
                            w.cancel_bookmark_copy(cx);
                            crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(
                                name.to_string_lossy().into_owned(),
                            ));
                            w.set_status("Copied filename");
                        }
                    }
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(leaf_row(
        theme,
        "Copy Current Dir. Path",
        true,
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    if let Some(p) = w
                        .tabs
                        .get(target)
                        .and_then(|t| t.buffer.file_path.clone())
                    {
                        if let Some(parent) = p.parent() {
                            w.cancel_bookmark_copy(cx);
                            crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(
                                parent.display().to_string(),
                            ));
                            w.set_status("Copied directory path");
                        }
                    }
                });
            }
        },
        |_, _| {},
    ));
    panel
}

fn move_document_panel(
    theme: &Theme,
    state: TabContextMenuState,
    flags: &TabFlags,
    workspace: Entity<Workspace>,
) -> gpui::Div {
    let mut panel = panel_base(theme, SUBMENU_WIDTH_REM);

    panel = panel.child(leaf_row(
        theme,
        "Move to Start",
        !flags.is_first,
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    w.move_tab_to_start(target, cx);
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(leaf_row(
        theme,
        "Move to End",
        !flags.is_last,
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    w.move_tab_to_end(target, cx);
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(separator_row(theme));
    // Split-view items — Notepad Studio has no split-view concept yet; greyed
    // pending v2.
    for label in ["Move to Other View", "Clone to Other View"] {
        panel = panel.child(leaf_row(theme, label, false, false, |_, _| {}, |_, _| {}));
    }
    panel = panel.child(leaf_row(
        theme,
        "Move to New Instance",
        flags.tabs_total > 1,
        false,
        {
            let ws = workspace.clone();
            move |window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    move_tab_to_new_window(w, target, true, window, cx);
                });
            }
        },
        |_, _| {},
    ));
    panel = panel.child(leaf_row(
        theme,
        "Open in New Instance",
        true,
        false,
        {
            let ws = workspace.clone();
            move |window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    move_tab_to_new_window(w, target, false, window, cx);
                });
            }
        },
        |_, _| {},
    ));
    panel
}

fn apply_color_panel(
    theme: &Theme,
    state: TabContextMenuState,
    flags: &TabFlags,
    workspace: Entity<Workspace>,
) -> gpui::Div {
    let mut panel = panel_base(theme, SUBMENU_WIDTH_REM);

    let palette: &[(&str, u8, u32)] = &[
        ("Apply Color 1", 1, theme.tab_color_1),
        ("Apply Color 2", 2, theme.tab_color_2),
        ("Apply Color 3", 3, theme.tab_color_3),
        ("Apply Color 4", 4, theme.tab_color_4),
        ("Apply Color 5", 5, theme.tab_color_5),
    ];
    for (label, idx, swatch) in palette {
        let label = *label;
        let idx = *idx;
        let swatch = *swatch;
        let checkmark = flags.tab_color == Some(idx);
        let row = color_row(theme, label, swatch, checkmark, {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    w.set_tab_color(target, Some(idx), cx);
                });
            }
        });
        panel = panel.child(row);
    }
    panel = panel.child(separator_row(theme));
    panel = panel.child(leaf_row(
        theme,
        "Remove Color",
        flags.tab_color.is_some(),
        false,
        {
            let ws = workspace.clone();
            move |_window, cx| {
                ws.update(cx, |w, cx| {
                    let Some(target) = resolve_submenu_target(w, state, cx) else { return };
                    w.hide_tab_context_menu(cx);
                    w.set_tab_color(target, None, cx);
                });
            }
        },
        |_, _| {},
    ));
    panel
}

fn resolve_popup_target(
    workspace: &mut Workspace,
    painted: TabContextMenuState,
    cx: &mut gpui::Context<Workspace>,
) -> Option<usize> {
    let current = workspace.tab_context_menu?;
    if current.popup_id != painted.popup_id || current.target_tab_id != painted.target_tab_id {
        return None;
    }
    let target = workspace.tabs.iter().position(|tab| tab.session_id == painted.target_tab_id);
    if target.is_none() { workspace.hide_tab_context_menu(cx); }
    target
}

fn resolve_submenu_target(
    workspace: &mut Workspace,
    painted: TabContextMenuState,
    cx: &mut gpui::Context<Workspace>,
) -> Option<usize> {
    let current = workspace.tab_context_menu?;
    if painted.submenu_id.is_none() || current.submenu_id != painted.submenu_id
        || current.open_submenu != painted.open_submenu {
        return None;
    }
    resolve_popup_target(workspace, painted, cx)
}

fn run_leaf_action(
    action: LeafAction,
    painted: TabContextMenuState,
    workspace: &Entity<Workspace>,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) {
    workspace.update(cx, |w, cx| {
        let Some(target) = resolve_popup_target(w, painted, cx) else { return };
        w.hide_tab_context_menu(cx);
        match action {
            LeafAction::Close => w.close_tab_at(target, window, cx),
            LeafAction::Save => w.save_tab_at(target, cx),
            LeafAction::SaveAs => {
                // Notepad++ selects the right-clicked tab (TabBar.cpp:894
                // "rightclick selects tab as well"), so Save As runs on the
                // active tab. `save_active_as` resolves the tab again after
                // its async dialog, so restoring the previous active tab here
                // would write the wrong document to the chosen path.
                if target < w.tabs.len() {
                    w.set_active_tab(target, cx);
                    w.save_active_as(&crate::actions::FileSaveAs, window, cx);
                }
            }
            LeafAction::Rename => w.open_tab_rename(target, cx),
            LeafAction::MoveToTrash => w.move_tab_to_trash(target, window, cx),
            LeafAction::Reload => w.reload_tab(target, window, cx),
            LeafAction::Print => {
                w.set_status("Print not yet implemented");
            }
            LeafAction::ToggleReadOnly => {
                let cur = w
                    .tabs
                    .get(target)
                    .map(|t| t.buffer.read_only)
                    .unwrap_or(false);
                w.set_tab_read_only(target, !cur, cx);
            }
            LeafAction::ClearReadOnlyFlag => w.clear_tab_read_only_flag(target, cx),
        }
    });
}

/// Spawn a fresh Notepad Studio window for tab `idx`.
///
/// * `move_it = true`  — **Move to New Instance**: the tab is removed from
///   the source window (carrying its rope, dirty state, undo history,
///   bookmarks, marks, read-only flag, tab colour, view mode, …) and
///   appended to the new window. Source then settles its active-tab
///   pointer + MRU via `settle_after_tab_removal` (same path as
///   `finish_close`, minus the closed-tab stack push — the tab isn't
///   closed, it's alive in another window). If the source ends up empty
///   the existing `tab_exit_on_last_close` policy decides whether to
///   close the window or replace with a fresh blank tab.
/// * `move_it = false` — **Open in New Instance**: for a saved buffer the
///   path is re-opened in the new window (NPP semantics — fresh from disk;
///   in-memory dirty edits stay only in the source). For an untitled
///   buffer the rope + dirty + label are cloned via
///   `Buffer::clone_for_transfer` since there's no path to round-trip
///   through.
///
/// In-flight streaming loads (`loading.is_some()`) are refused — the
/// shared `Arc<AtomicBool>` cancel flag can't safely tie two windows'
/// Cancel buttons together. The user gets a status nudge to wait.
fn move_tab_to_new_window(
    workspace: &mut Workspace,
    idx: usize,
    move_it: bool,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) {
    if workspace.defer_automatic_title_tab(idx, "transfer", window, cx,
        move |w, index, window, cx| move_tab_to_new_window(w, index, move_it, window, cx)) { return; }
    let Some(tab) = workspace.tabs.get(idx) else {
        return;
    };
    if tab.buffer.loading.is_some() {
        workspace.set_status("Cannot transfer a tab whose file is still loading");
        return;
    }

    if move_it {
        // Take ownership — the tab is alive in the new window after this,
        // so it does NOT go onto the closed-tab stack.
        workspace.invalidate_automatic_name_context();
        let tab = workspace.tabs.remove(idx);
        workspace.settle_after_tab_removal(idx, window, cx);
        if let Some((tab, error)) = crate::app::window::open_tab_in_new_window(tab, cx) {
            workspace.adopt_transferred_tab(tab, cx);
            workspace.set_status(format!("Document kept in this window: {error}"));
        }
        return;
    }

    // Open in New Instance — source untouched.
    let Some(buffer) = workspace.tabs.get(idx).map(|tab| &tab.buffer) else { return; };
    if let Some(path) = buffer.file_path.clone() {
        if let Err(error) = crate::app::window::open_editor_window_with_initial_path(path, cx) {
            workspace.set_status(format!("Could not open a destination window: {error}. Try Open in New Instance again."));
        }
    } else {
        let cloned_buffer = buffer.clone_for_transfer();
        let mut new_tab = crate::tab::Tab::empty();
        new_tab.buffer = cloned_buffer;
        if let Some((tab, error)) = crate::app::window::open_tab_in_new_window(new_tab, cx) {
            workspace.adopt_transferred_tab(tab, cx);
            workspace.set_status(format!("Document copy kept in this window: {error}"));
        }
    }
}

// ---------- Reusable row + panel primitives -------------------------------

pub(crate) fn panel_base(theme: &Theme, width_rem: f32) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .py(rems(4. / 16.))
        .min_w(rems(width_rem))
        .max_w(rems(width_rem + 80. / 16.))
        .bg(rgb(theme.chrome))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(12. / 16.))
        .text_color(rgb(theme.text))
}

pub(crate) fn separator_row(theme: &Theme) -> gpui::Div {
    div()
        .h(rems(1. / 16.))
        .my(rems(4. / 16.))
        .mx(rems(8. / 16.))
        .bg(rgb(theme.border))
}

pub(crate) fn leaf_row(
    theme: &Theme,
    label: &str,
    enabled: bool,
    checkmark: bool,
    on_click: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
    on_hover: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Div {
    let chrome_text = theme.chrome_text;
    let text = theme.text;
    let hover = theme.toggle_active;
    let bg = theme.bg;
    let mark = if checkmark { "✓" } else { " " };
    let row_text = if enabled { text } else { chrome_text };
    let label = label.to_string();

    let mut row = div()
        .flex()
        .items_center()
        .h(rems(ROW_HEIGHT_REM))
        .px(rems(8. / 16.))
        .text_color(rgb(row_text))
        .child(
            div()
                .w(rems(14. / 16.))
                .pr(rems(4. / 16.))
                .text_color(rgb(text))
                .child(mark.to_string()),
        )
        .child(div().flex_grow().child(label));

    if enabled {
        row = row
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(hover)).text_color(rgb(bg)))
            .on_mouse_move(move |_e, w, cx| on_hover(w, cx))
            .on_mouse_down(MouseButton::Left, move |_e, w, cx| on_click(w, cx));
    }
    row
}

#[cfg(test)]
#[path = "tab_reload_tests.rs"]
mod tab_reload_tests;

#[cfg(test)]
#[path = "tab_popup_tests.rs"]
mod tab_popup_tests;

#[cfg(test)]
#[path = "tab_trash_tests.rs"]
mod tab_trash_tests;

fn color_row(
    theme: &Theme,
    label: &str,
    swatch: u32,
    checkmark: bool,
    on_click: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Div {
    let text = theme.text;
    let hover = theme.toggle_active;
    let bg = theme.bg;
    let mark = if checkmark { "✓" } else { " " };
    let label = label.to_string();

    let mut row = div()
        .flex()
        .items_center()
        .h(rems(ROW_HEIGHT_REM))
        .px(rems(8. / 16.))
        .text_color(rgb(text))
        .child(
            div()
                .w(rems(14. / 16.))
                .pr(rems(4. / 16.))
                .child(mark.to_string()),
        )
        .child(
            div()
                .w(rems(14. / 16.))
                .h(rems(12. / 16.))
                .mr(rems(8. / 16.))
                .bg(rgb(swatch))
                .border_1()
                .border_color(rgb(theme.border)),
        )
        .child(div().flex_grow().child(label));

    row = row
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(hover)).text_color(rgb(bg)))
        .on_mouse_down(MouseButton::Left, move |_e, w, cx| on_click(w, cx));
    row
}

fn submenu_row(
    theme: &Theme,
    label: &str,
    enabled: bool,
    is_open: bool,
    on_click_or_hover: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Div {
    let chrome_text = theme.chrome_text;
    let text = theme.text;
    let hover = theme.toggle_active;
    let bg = theme.bg;
    let row_text = if enabled { text } else { chrome_text };
    let label = label.to_string();
    let on_click_or_hover = std::rc::Rc::new(on_click_or_hover);
    let on_click = on_click_or_hover.clone();
    let on_hover = on_click_or_hover.clone();

    let mut row = div()
        .flex()
        .items_center()
        .h(rems(ROW_HEIGHT_REM))
        .px(rems(8. / 16.))
        .text_color(rgb(row_text))
        .child(div().w(rems(14. / 16.)))
        .child(div().flex_grow().child(label))
        .child(div().pl(rems(8. / 16.)).child("▶"));

    if is_open {
        row = row.bg(rgb(hover)).text_color(rgb(bg));
    }

    if enabled {
        row = row
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(hover)).text_color(rgb(bg)))
            .on_mouse_down(MouseButton::Left, move |_e, w, cx| on_click(w, cx))
            .on_mouse_move(move |_e, w, cx| on_hover(w, cx));
    }
    row
}

#[cfg(test)]
pub(crate) fn transfer_for_startup_test(workspace: &mut Workspace, idx: usize, move_it: bool,
    window: &mut gpui::Window, cx: &mut gpui::Context<Workspace>) {
    move_tab_to_new_window(workspace, idx, move_it, window, cx);
}
