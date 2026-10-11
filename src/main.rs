// Notepad Studio — lightweight cross-platform text editor.

// On Windows, the default Rust binary uses the "console" subsystem, so Windows
// spawns a terminal window alongside the GUI and stdout (e.g. the plugin
// scanner's logs) prints into it. Build release binaries with the "windows"
// (GUI) subsystem so no console appears. Debug builds keep the console so
// `println!`/`eprintln!` diagnostics stay visible during development. No-op on
// non-Windows targets.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui::{App, Application, QuitMode};

mod actions;
mod api_registry;
mod app;
mod assets;
mod buffer;
mod call_context;
mod changelog;
mod clipboard_history;
mod clipboard_rich;
mod config;
mod consts;
mod edit_ops;
mod editor_element;
mod find_block_map;
mod find_in_files;
mod find_state;
mod fonts;
mod gutter;
mod input_field;
mod io;
mod io_stream;
mod io_reload;
mod locale_date;
mod macros;
mod panic_log;
mod persist;
mod plugins;
mod print;
mod recent;
mod rich_doc;
mod run_command;
mod scrollbar;
mod session;
mod settings;
mod shell;
mod shortcuts;
mod single_instance;
mod style_model;
mod syntax;
mod tab;
mod text;
mod theme;
mod ui;
mod workspace;

use crate::app::keymap::bind_app_keys;
use crate::app::menus::{install_global_actions, install_menus};
use crate::app::window::{raise_all_app_windows, restore_windows_on_boot};
use crate::config::flush_window_settings_for_alive;
use crate::panic_log::install_panic_hook;

fn main() {
    install_panic_hook();
    // Single-instance gate. If another live Notepad Studio is already running, this
    // call drops a sentinel the running instance polls for, then exits the
    // current process with code 0. Has to land before the gpui Application
    // is built so a duplicate launch doesn't briefly flash a window.
    single_instance::acquire_or_raise();
    // Queue any positional file path arguments so the activation poller opens
    // them once the workspace is up. Covers cold-start `Exec=notepad-studio
    // %F` (Linux), Explorer's "Open With → Notepad Studio" (Windows), and
    // shell launches like `notepad-studio README.md`. macOS Finder uses the
    // `application:openURLs:` delegate instead — see `app.on_open_urls`
    // below.
    single_instance::queue_argv_files();
    let app = Application::new().with_assets(crate::assets::NotepadStudioAssets);

    // macOS: Finder's `Open With → Notepad Studio`, drop-on-dock-icon, and
    // double-click on an associated file all route through the
    // `application:openURLs:` NSApplication delegate. gpui exposes this as
    // `on_open_urls`. The callback fires from the Cocoa main thread without a
    // `cx`, so we just translate URLs to `PathBuf`s and queue them — the
    // activation poller drains the queue on its next tick and opens each
    // file via `open_path_in_active_or_new_window`.
    app.on_open_urls(|urls| {
        for url in urls {
            if let Some(p) = single_instance::parse_file_url(&url) {
                single_instance::pending_open_paths_push(p);
            }
        }
    });

    // macOS: re-open windows when the dock icon is clicked while no windows
    // are visible. NSApplication's applicationShouldHandleReopen: usually
    // only fires with no visible windows, but check anyway — hidden /
    // minimized windows can drive it too. With a Notepad Studio window already
    // alive, bring the whole app window group forward. With none alive, run
    // the cold-start restore so tabs + placement come back.
    app.on_reopen(|cx| {
        let has_window = cx
            .windows()
            .into_iter()
            .any(|h| h.downcast::<workspace::Workspace>().is_some());
        if has_window {
            raise_all_app_windows(cx);
        } else {
            restore_windows_on_boot(cx);
        }
    });

    app.run(|cx: &mut App| {
        cx.activate(true);
        recent::init(cx.background_executor().clone());
        // Resolve the platform monospace face before any window renders.
        fonts::init(cx);
        // QuitMode::Default keeps macOS apps alive after the last window closes
        // (use the dock icon to reopen) and exits Linux/Windows when the last
        // window closes. Set explicitly so the choice is visible.
        cx.set_quit_mode(QuitMode::Default);
        // Each window close prunes the in-memory cache to the still-open
        // set and rewrites the placement file, so the layout on disk
        // tracks exactly what the user has on screen.
        cx.on_window_closed(|cx| {
            let alive: Vec<_> = cx.windows().into_iter().map(|h| h.window_id()).collect();
            if !app::startup::protects_placement(cx) { flush_window_settings_for_alive(&alive); }
            // Also force a session snapshot on every window close — the
            // 2s debounced autosave covers steady-state typing, but a
            // user who hits Cmd+W immediately after Cmd+N would otherwise
            // close before the timer fires and lose the new tab.
            for handle in cx.windows() {
                if let Some(ws) = handle.downcast::<workspace::Workspace>() {
                    let _ = ws.update(cx, |w, _window, _cx| {
                        if let Err(err) = w.save_session_now() {
                            eprintln!("[session] final recovery write failed: {err}");
                        }
                    });
                }
            }
        })
        .detach();
        // App-level quit (macOS Cmd+Q, Linux/Windows last-window close) — save
        // every live workspace's session before processes start unwinding.
        // on_window_closed runs per-window; on_app_quit catches the case where
        // the OS kills the run loop without firing per-window close handlers.
        cx.on_app_quit(|cx: &mut App| {
            app::startup::cancel_for_app_quit(cx);
            app::quit::save_final_recovery(cx);
            let alive: Vec<_> = cx.windows().into_iter().map(|h| h.window_id()).collect();
            if !app::startup::protects_placement(cx) { flush_window_settings_for_alive(&alive); }
            // Release after window teardown has flushed its recovery state,
            // so the next instance cannot race the final writes.
            // A crash leaves the lock behind; the next launch's stale-PID
            // branch will clear it.
            recent::finish_shutdown();
            async { single_instance::release(); }
        })
        .detach();
        // Scan + load plugins before menu install so the Plugins menu
        // reflects whatever's in `<config>/Notepad Studio/plugins/` on this run.
        plugins::init();
        install_global_actions(cx);
        bind_app_keys(cx);
        install_menus(cx);
        run_command::init_saved_commands(cx);
        // Watch for a sentinel file from a second-launch attempt so we can
        // raise this instance to the foreground instead of letting the new
        // process spawn its own window.
        single_instance::start_activation_poller(cx);
        settings::start_settings_refresh_poller(cx);
        app::startup::begin(cx, true);
    });
}
