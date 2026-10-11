// Right-click context menu for folder-panel rows. Reuses the panel /
// leaf_row primitives from `ui::tab_context_menu` so visual style and
// hover behaviour stay consistent across the two menus.

use std::path::PathBuf;

use gpui::{
    Corner, Entity, IntoElement, MouseButton, ParentElement, Pixels, Point, Styled, anchored,
    deferred, div, prelude::*, px, rems,
};

use crate::theme::Theme;
use crate::ui::tab_context_menu::{leaf_row, panel_base, separator_row};
use crate::workspace::Workspace;

const MENU_WIDTH_REM: f32 = 230. / 16.;

#[derive(Clone, Debug)]
pub(crate) struct FolderContextMenuState {
    /// Path the menu was opened on. Drives every action's target.
    pub(crate) target: PathBuf,
    /// Window-space anchor.
    pub(crate) position: Point<Pixels>,
    /// Row classification at open time — drives which items are shown
    /// (root-only entries are gated behind `RowKind::Root`).
    pub(crate) kind: RowKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    Root,
    Folder,
    File,
}

/// Render the folder-panel right-click menu (panel + scrim). Caller
/// spawns this from workspace render's
/// `when_some(self.folder_panel.context_menu.as_ref(), ...)` branch.
pub(crate) fn folder_context_menu(
    theme: &Theme,
    state: &FolderContextMenuState,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let backdrop_ws = workspace.clone();
    let backdrop = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .on_mouse_down(MouseButton::Left, {
            let ws = backdrop_ws.clone();
            move |_e, _w, cx| {
                ws.update(cx, |w, cx| w.hide_folder_panel_context_menu(cx));
            }
        })
        .on_mouse_down(MouseButton::Right, move |_e, _w, cx| {
            backdrop_ws.update(cx, |w, cx| w.hide_folder_panel_context_menu(cx));
        });

    let panel = build_panel(theme, state, workspace);

    deferred(
        div().absolute().top_0().left_0().size_full().child(
            div()
                .size_full()
                .child(backdrop)
                .child(
                    anchored()
                        .position(state.position)
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(px(4.))
                        .child(panel),
                ),
        ),
    )
    .with_priority(2)
}

fn build_panel(
    theme: &Theme,
    state: &FolderContextMenuState,
    workspace: Entity<Workspace>,
) -> gpui::Div {
    let mut panel = panel_base(theme, MENU_WIDTH_REM);
    let target = state.target.clone();
    let kind = state.kind;
    let is_dir = matches!(kind, RowKind::Root | RowKind::Folder);

    if !is_dir {
        // File row: Open / Open in Default Viewer / Reveal in Tab.
        let ws = workspace.clone();
        let p = target.clone();
        panel = panel.child(leaf_row(theme, "Open", true, false, move |_w, cx| {
            let p = p.clone();
            ws.update(cx, |w, cx| {
                w.hide_folder_panel_context_menu(cx);
                w.open_path(p, cx);
            });
        }, |_, _| {}));

        let ws = workspace.clone();
        let p = target.clone();
        panel = panel.child(leaf_row(
            theme,
            "Open in Default Viewer",
            true,
            false,
            move |_w, cx| {
                let p = p.clone();
                ws.update(cx, |w, cx| {
                    w.hide_folder_panel_context_menu(cx);
                    if let Err(err) = crate::shell::open_with_default_viewer(&p) {
                        w.set_status(format!("Open in default viewer failed: {err}"));
                    }
                });
            },
            |_, _| {},
        ));

        let ws = workspace.clone();
        let p = target.clone();
        panel = panel.child(leaf_row(
            theme,
            "Reveal in Tab",
            true,
            false,
            move |_w, cx| {
                let p = p.clone();
                ws.update(cx, |w, cx| {
                    w.hide_folder_panel_context_menu(cx);
                    if let Some(idx) = w
                        .tabs
                        .iter()
                        .position(|t| t.buffer.file_path.as_deref() == Some(p.as_path()))
                    {
                        w.set_active_tab(idx, cx);
                    } else {
                        // Not yet open — same as Open.
                        w.open_path(p, cx);
                    }
                });
            },
            |_, _| {},
        ));
        panel = panel.child(separator_row(theme));
    }

    // Open Containing Folder in <Finder/Explorer/...>.
    let ws = workspace.clone();
    let p = target.clone();
    panel = panel.child(leaf_row(
        theme,
        reveal_label(),
        true,
        false,
        move |_w, cx| {
            let p = p.clone();
            ws.update(cx, |w, cx| {
                w.hide_folder_panel_context_menu(cx);
                if let Err(err) = crate::shell::reveal_in_file_manager(&p) {
                    w.set_status(format!("Reveal failed: {err}"));
                }
            });
        },
        |_, _| {},
    ));

    let ws = workspace.clone();
    let p = target.clone();
    panel = panel.child(leaf_row(
        theme,
        terminal_label(),
        true,
        false,
        move |_w, cx| {
            let p = p.clone();
            ws.update(cx, |w, cx| {
                w.hide_folder_panel_context_menu(cx);
                let dir = if p.is_dir() {
                    p.clone()
                } else {
                    p.parent()
                        .map(|x| x.to_path_buf())
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                };
                if let Err(err) = crate::shell::open_terminal_at(&dir) {
                    w.set_status(format!("Open terminal failed: {err}"));
                }
            });
        },
        |_, _| {},
    ));

    // Copy Full Path / Copy Filename.
    panel = panel.child(separator_row(theme));
    let ws = workspace.clone();
    let p = target.clone();
    panel = panel.child(leaf_row(
        theme,
        "Copy Full Path",
        true,
        false,
        move |_w, cx| {
            let p = p.clone();
            ws.update(cx, |w, cx| {
                w.hide_folder_panel_context_menu(cx);
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                    p.display().to_string(),
                ));
                w.set_status("Copied full path");
            });
        },
        |_, _| {},
    ));
    let ws = workspace.clone();
    let p = target.clone();
    panel = panel.child(leaf_row(
        theme,
        "Copy Filename",
        true,
        false,
        move |_w, cx| {
            let p = p.clone();
            ws.update(cx, |w, cx| {
                w.hide_folder_panel_context_menu(cx);
                if let Some(name) = p.file_name() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        name.to_string_lossy().into_owned(),
                    ));
                    w.set_status("Copied filename");
                }
            });
        },
        |_, _| {},
    ));

    if is_dir {
        // New File / New Folder — open the inline-create draft row inside
        // the target directory. Both auto-expand the folder so the typing
        // row is visible.
        panel = panel.child(separator_row(theme));
        let ws = workspace.clone();
        let p = target.clone();
        panel = panel.child(leaf_row(
            theme,
            "New File…",
            true,
            false,
            move |window, cx| {
                let p = p.clone();
                ws.update(cx, |w, cx| {
                    w.hide_folder_panel_context_menu(cx);
                    w.begin_folder_inline_create(
                        p,
                        crate::find_state::FolderInlineCreateKind::File,
                        window,
                        cx,
                    );
                });
            },
            |_, _| {},
        ));
        let ws = workspace.clone();
        let p = target.clone();
        panel = panel.child(leaf_row(
            theme,
            "New Folder…",
            true,
            false,
            move |window, cx| {
                let p = p.clone();
                ws.update(cx, |w, cx| {
                    w.hide_folder_panel_context_menu(cx);
                    w.begin_folder_inline_create(
                        p,
                        crate::find_state::FolderInlineCreateKind::Folder,
                        window,
                        cx,
                    );
                });
            },
            |_, _| {},
        ));

        // Refresh / Collapse All Below.
        panel = panel.child(separator_row(theme));
        let ws = workspace.clone();
        let p = target.clone();
        panel = panel.child(leaf_row(
            theme,
            "Refresh",
            true,
            false,
            move |_w, cx| {
                let p = p.clone();
                ws.update(cx, |w, cx| {
                    w.hide_folder_panel_context_menu(cx);
                    w.folder_panel.invalidate_subtree(&p);
                    cx.notify();
                });
            },
            |_, _| {},
        ));
        let ws = workspace.clone();
        let p = target.clone();
        panel = panel.child(leaf_row(
            theme,
            "Collapse All Below",
            true,
            false,
            move |_w, cx| {
                let p = p.clone();
                ws.update(cx, |w, cx| {
                    w.hide_folder_panel_context_menu(cx);
                    super::collapse_subtree(&p);
                    w.folder_panel.invalidate_subtree(&p);
                    cx.notify();
                });
            },
            |_, _| {},
        ));
    }

    if matches!(kind, RowKind::Root) {
        // Top-level root-only: Remove from Workspace.
        panel = panel.child(separator_row(theme));
        let ws = workspace.clone();
        let p = target.clone();
        panel = panel.child(leaf_row(
            theme,
            "Remove from Workspace",
            true,
            false,
            move |_w, cx| {
                let p = p.clone();
                ws.update(cx, |w, cx| {
                    w.hide_folder_panel_context_menu(cx);
                    super::remove_root(&p);
                    w.folder_panel.invalidate_subtree(&p);
                    if w.folder_panel.selected.as_ref() == Some(&p) {
                        w.folder_panel.selected = None;
                    }
                    w.set_status(format!(
                        "Removed '{}' from workspace",
                        p.display()
                    ));
                    cx.notify();
                });
            },
            |_, _| {},
        ));
    }

    // Show Hidden Files toggle — reachable from any row, persisted globally.
    panel = panel.child(separator_row(theme));
    let snap = crate::settings::snapshot();
    let label = if snap.folder_panel_show_hidden {
        "Hide Hidden Files"
    } else {
        "Show Hidden Files"
    };
    let ws = workspace.clone();
    panel = panel.child(leaf_row(
        theme,
        label,
        true,
        snap.folder_panel_show_hidden,
        move |_w, cx| {
            ws.update(cx, |w, cx| {
                w.hide_folder_panel_context_menu(cx);
                let snap = crate::settings::snapshot();
                super::set_show_hidden(!snap.folder_panel_show_hidden);
                super::cache_clear();
                cx.notify();
            });
        },
        |_, _| {},
    ));

    // Width fixed across all panels — caller-side anchoring takes care of
    // keeping the panel on-screen via `snap_to_window_with_margin`.
    let _ = rems(MENU_WIDTH_REM);
    panel
}

fn reveal_label() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Open in Finder"
    }
    #[cfg(target_os = "windows")]
    {
        "Open in Explorer"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "Open in File Manager"
    }
}

fn terminal_label() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Open in Terminal"
    }
    #[cfg(target_os = "windows")]
    {
        "Open in cmd"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "Open in Terminal"
    }
}
