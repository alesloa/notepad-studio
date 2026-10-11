use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, Focusable, Pixels, Size, TitlebarOptions, WindowBounds, WindowOptions,
    point, prelude::*, px, size,
};

use crate::config::{WindowEntry, load_windows, record_window_bounds, record_zoom_level};
use crate::consts::{APP_NAME, ZOOM_DEFAULT, ZOOM_REM_BASE};
use crate::session::load_manifest;
use crate::tab::Tab;
use crate::workspace::Workspace;

/// Place a modal dialog centered over the active Workspace window. Returns
/// `WindowBounds::Windowed(...)` ready to drop into `WindowOptions`.
///
/// Notepad++ and every other native editor opens dialogs over the editor
/// they were summoned from. `Bounds::centered(None, ..., cx)` always centers
/// on the primary display, which on a multi-monitor or moved-window setup
/// puts the dialog far from where the user is looking. Anchoring to
/// `window.bounds()` (current visible bounds, including maximized state)
/// keeps the dialog where the user expects it.
pub(crate) fn modal_bounds_centered_on_workspace(
    cx: &mut App,
    modal_size: Size<Pixels>,
) -> WindowBounds {
    let any_handle = cx
        .active_window()
        .filter(|h| h.downcast::<Workspace>().is_some())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find(|h| h.downcast::<Workspace>().is_some())
        });

    if let Some(h) = any_handle {
        if let Ok(parent) = cx.update_window(h, |_, window, _| window.bounds()) {
            let origin = point(
                parent.origin.x + (parent.size.width - modal_size.width) / 2.0,
                parent.origin.y + (parent.size.height - modal_size.height) / 2.0,
            );
            return WindowBounds::Windowed(Bounds::new(origin, modal_size));
        }
    }
    WindowBounds::Windowed(Bounds::centered(None, modal_size, cx))
}

pub(crate) fn spawn_caret_blink(cx: &mut Context<Workspace>) {
    cx.spawn(async move |this, cx| {
        loop {
            // Re-read each tick so a Preferences edit lands on the next blink
            // without needing to restart this task.
            let interval = crate::settings::caret_blink_interval_ms();
            gpui::Timer::after(Duration::from_millis(interval as u64)).await;
            if this
                .update(cx, |w, cx| {
                    w.caret_visible = !w.caret_visible;
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}

/// NPP `Backup ▸ Snapshot ▸ Enable session snapshot and periodic backup` —
/// every `backup_periodic_interval_ms`, snapshots the workspace to the
/// session manifest (which writes `.bak`s for dirty/untitled tabs). Re-reads
/// the interval each tick so a Preferences edit takes effect on the next
/// snapshot without restarting the task. When `backup_periodic_enabled`
/// is false the tick is a no-op — keeps the loop alive so toggling the
/// master switch back on resumes snapshotting without restarting Notepad Studio.
pub(crate) fn spawn_periodic_backup(cx: &mut Context<Workspace>) {
    cx.spawn(async move |this, cx| {
        loop {
            let interval = crate::settings::backup_periodic_interval_ms();
            gpui::Timer::after(Duration::from_millis(interval as u64)).await;
            let snap = crate::settings::snapshot();
            if !snap.backup_periodic_enabled {
                continue;
            }
            if this.update(cx, |w, cx| w.save_session(cx)).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Boot-time entry point — restores every previously persisted window in
/// slot order. Falls back to a single fresh window if the placement file
/// is missing or empty (first launch, or all windows were closed before
/// any layout was saved).
///
/// The session manifest (open tabs, theme, etc.) is single-instance and
/// only applied to the first restored window — matches Notepad++ where
/// the session belongs to the "main" instance.
pub(crate) fn restore_windows_on_boot(cx: &mut App) {
    let entries = load_windows();
    if entries.is_empty() {
        spawn_window(cx, None, true);
        return;
    }
    for (idx, entry) in entries.into_iter().enumerate() {
        // Only the first restored window inherits the session manifest;
        // additional windows boot empty so we don't double-restore tabs.
        spawn_window(cx, Some(entry), idx == 0);
    }
}

/// Open a fresh editor window with default placement and no session
/// restore. Used by `File ▸ New Window`.
pub(crate) fn open_editor_window(cx: &mut App) {
    spawn_window(cx, None, false);
}

/// Bring every app window forward as a group.
///
/// GPUI floating windows are separate top-level windows on Windows/Linux, so
/// raising the app alone can leave modeless dialogs like Find buried behind
/// another application. Use this for explicit app-level activation paths such
/// as dock/taskbar reopen and single-instance raise requests.
pub(crate) fn raise_all_app_windows(cx: &mut App) {
    cx.activate(true);
    raise_windows_in_front_order(cx);
}

fn raise_windows_in_front_order(cx: &mut App) {
    let mut workspace_windows = Vec::new();
    let mut auxiliary_windows = Vec::new();

    for handle in cx.windows() {
        if handle.downcast::<Workspace>().is_some() {
            workspace_windows.push(handle);
        } else {
            auxiliary_windows.push(handle);
        }
    }

    // Auxiliary first, workspaces last — the last-raised window ends up on
    // top, so a Find Files / Character Panel / etc. floater doesn't cover
    // the editor after a dock reopen or external activation request.
    for handle in auxiliary_windows {
        let _ = handle.update(cx, |_, window, _| {
            window.activate_window();
        });
    }
    for handle in workspace_windows {
        let _ = handle.update(cx, |_, window, _| {
            window.activate_window();
        });
    }
}

/// Lower-level spawn. `entry` carries the persisted bounds + zoom for a
/// restored window; `None` means "use platform defaults". `restore_session`
/// gates whether the on-disk session manifest is applied to the new
/// workspace — only the first window on boot should pick it up.
fn spawn_window(cx: &mut App, entry: Option<WindowEntry>, restore_session: bool) {
    let loaded_zoom = entry
        .and_then(|e| e.zoom_level)
        .unwrap_or(ZOOM_DEFAULT)
        .clamp(crate::consts::ZOOM_MIN, crate::consts::ZOOM_MAX);

    let bounds = entry
        .and_then(WindowEntry::into_bounds)
        .unwrap_or_else(|| Bounds::centered(None, size(px(1100.), px(720.)), cx));

    let window = match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
                appears_transparent: false,
                traffic_light_position: Some(point(px(10.), px(10.))),
            }),
            // Wayland / X11 WM class — pairs window with the .desktop file
            // so GNOME / KDE pick up the icon and proper app name in the
            // taskbar / Alt-Tab. No-op on macOS (`set_app_id` is a noop on
            // mac platform). Match the StartupWMClass in
            // assets/notepad-studio.desktop.
            app_id: Some("notepad-studio".into()),
            ..Default::default()
        },
        |window, cx| {
            // Apply persisted zoom before the first paint so chrome shapes
            // straight into the saved size — no flash of unzoomed UI.
            window.set_rem_size(px(ZOOM_REM_BASE * loaded_zoom));
            // Seed the cache with the window's initial bounds + zoom so
            // a session that never resizes / never zooms still saves
            // something useful on exit.
            record_zoom_level(window, loaded_zoom);
            record_window_bounds(window);
            let initial_appearance = window.appearance();
            let editor_focus = cx.focus_handle();
            let find_focus = cx.focus_handle();
            let replace_focus = cx.focus_handle();
            let goto_focus = cx.focus_handle();
            let fif_query_focus = cx.focus_handle();
            let fif_filter_focus = cx.focus_handle();
            let workspace_entity = cx.new(|cx| {
                cx.observe_window_bounds(window, |_workspace, window, _cx| {
                    record_window_bounds(window);
                })
                .detach();
                // Mirror the OS appearance so `ThemeMode::FollowSystem` flips
                // live when the user toggles macOS Light/Dark mode.
                cx.observe_window_appearance(
                    window,
                    |workspace: &mut Workspace, win, cx| {
                        let next = win.appearance();
                        if workspace.system_appearance != next {
                            workspace.system_appearance = next;
                            cx.notify();
                        }
                    },
                )
                .detach();
                // Failsafe: snapshot the session manifest at entity release.
                // The macOS red ⏺ destroys the NSWindow without routing
                // through `finish_close`, so without this the workspace
                // dies with its tabs unsaved and a dock-icon reopen comes
                // back blank. `on_window_should_close` below is the early
                // hook (fires before the window actually closes); this is
                // the late hook (fires when gpui drops the entity) — either
                // is sufficient, both is belt-and-suspenders.
                cx.on_release(|workspace: &mut Workspace, _app: &mut App| {
                    workspace.save_session_now();
                    workspace.release_session_window();
                })
                .detach();
                let mut workspace = Workspace::new(
                    editor_focus,
                    find_focus,
                    replace_focus,
                    goto_focus,
                    fif_query_focus,
                    fif_filter_focus,
                    loaded_zoom,
                    initial_appearance,
                );
                if restore_session {
                    if let Some(manifest) = load_manifest() {
                        workspace.apply_session(manifest);
                    }
                }
                workspace.register_session_window();
                spawn_caret_blink(cx);
                spawn_periodic_backup(cx);
                workspace
            });

            // Earliest save hook — fires when AppKit asks the window
            // whether it should close, before any teardown begins. Pairs
            // with the `on_release` registration above.
            let entity_for_close = workspace_entity.clone();
            window.on_window_should_close(cx, move |_window, app_cx| {
                entity_for_close.update(app_cx, |w, _cx| {
                    if !w.save_session_now() { return false; }
                    w.release_session_window();
                    true
                })
            });

            workspace_entity
        },
    ) {
        Ok(window) => window,
        Err(err) => {
            eprintln!("failed to open Notepad Studio window: {err}");
            return;
        }
    };

    window
        .update(cx, |workspace, window, cx| {
            window.focus(&workspace.focus_handle(cx), cx);
        })
        .ok();
}

/// Open `path` in whatever editor window is currently active. Falls through
/// to spawning a new window when no Workspace window exists (e.g. macOS
/// after the last window was closed) — matches Notepad++ behaviour where
/// File ▸ Open Recent always lands on a viewable window.
///
/// On a successful in-place open, the existing window's `open_path` does
/// the recents-push + menu rebuild itself (same code path as File ▸ Open).
/// On the new-window fallback we hand the path to a one-shot post-mount
/// hook so the open happens after `Workspace::new` finishes hydrating.
pub(crate) fn open_path_in_active_or_new_window(path: PathBuf, cx: &mut App) {
    // Look for any existing Notepad Studio window — `active_window` returns the OS-
    // focused one when there is one, but a Recent click that comes from a
    // dock-icon menu (macOS, no window focused) should still land on the
    // first available window if one is around.
    let workspace_window = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find_map(|h| h.downcast::<Workspace>())
        });

    if let Some(handle) = workspace_window {
        let path_for_update = path.clone();
        if handle
            .update(cx, |workspace, _window, cx| {
                workspace.open_path(path_for_update, cx);
            })
            .is_ok()
        {
            // External open path (Finder "Open With", drop-on-dock-icon,
            // Explorer file association, cold-start argv via the single-
            // instance forwarder). The app may be background / on another
            // macOS Space / behind an aux floater — bring it forward and
            // focus the target editor window so the user lands on the file
            // they just opened.
            cx.activate(true);
            let mut aux_windows = Vec::new();
            for h in cx.windows() {
                if h.downcast::<Workspace>().is_none() {
                    aux_windows.push(h);
                }
            }
            for h in aux_windows {
                let _ = h.update(cx, |_, window, _| {
                    window.activate_window();
                });
            }
            let _ = handle.update(cx, |_, window, _| {
                window.activate_window();
            });
            return;
        }
        // The window was closed between `windows()` and `update()` — fall
        // through to spawning a new one.
    }

    open_editor_window_with_initial_path(path, cx);
}

/// Spawn a fresh editor window and adopt a tab handed over from another
/// window. Used by the tab right-click `Move to New Instance` /
/// `Open in New Instance` flow — the source has already removed (Move) or
/// cloned (Open) the tab; here it just lands in a fresh window. Always
/// spawns a new window — never folds into the active one, even if one
/// exists — which is the explicit user intent behind the menu rows.
pub(crate) fn open_tab_in_new_window(tab: Tab, cx: &mut App) {
    open_editor_window(cx);
    let Some(handle) = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find_map(|h| h.downcast::<Workspace>())
        })
    else {
        return;
    };
    handle
        .update(cx, |workspace, _window, cx| {
            workspace.adopt_transferred_tab(tab, cx);
        })
        .ok();
}

/// Spawn a fresh editor window and load `path` once the Workspace has
/// finished hydrating from the session manifest. The session restore happens
/// inside the `cx.new` closure that builds the Workspace, so the open call
/// has to land *after* that body returns — we re-enter the window via its
/// handle from outside.
pub(crate) fn open_editor_window_with_initial_path(path: PathBuf, cx: &mut App) {
    open_editor_window(cx);
    // `cx.active_window()` after `open_editor_window` returns picks up the
    // newly-spawned one — `cx.activate(true)` + the open_window call drive
    // the platform's active-window state synchronously.
    let Some(handle) = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find_map(|h| h.downcast::<Workspace>())
        })
    else {
        return;
    };
    handle
        .update(cx, |workspace, window, cx| {
            workspace.open_path(path, cx);
            // Same activation reasoning as the in-place branch in
            // `open_path_in_active_or_new_window` — the trigger here is
            // always external (Finder / argv / dock-drop), so bring the
            // new window forward across Spaces and above aux floaters.
            window.activate_window();
        })
        .ok();
    cx.activate(true);
}
