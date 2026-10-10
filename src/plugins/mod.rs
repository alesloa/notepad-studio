// Plugin host.
//
// Loads native cdylib plugins from `<config>/Notepad Studio/plugins/` at startup and
// surfaces their commands in the top-level `Plugins` menu. Each plugin
// exports two C-ABI functions defined in the `notepad-studio-plugin` workspace
// crate: `notepad_studio_plugin_info` (metadata) and `notepad_studio_plugin_run_command`
// (dispatch one menu pick).
//
// Threading: plugin commands run synchronously on the main thread inside the
// `PluginRunCommand` action handler. The host fills a thread-local with raw
// pointers to the active `Workspace` + its `Context` for the duration of the
// call so the C-ABI callbacks in `host_api` can read/write the buffer.
//
// Safety: plugins are native code with full process privileges. v1 ships
// without any sandbox — see the README "Plugins" section warning.

mod host_api;
mod loader;

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use gpui::Context;

pub(crate) use host_api::{end_active_call, start_active_call};
pub(crate) use loader::{LoadedCommand, LoadedPlugin, scan_and_load};

use crate::workspace::Workspace;

/// Process-global registry of loaded plugins. Built once at startup by
/// `init`; subsequent calls reuse the existing list without re-scanning the
/// plugins directory.
static GLOBAL: OnceLock<Mutex<PluginManager>> = OnceLock::new();

pub(crate) struct PluginManager {
    pub(crate) plugins_dir: PathBuf,
    pub(crate) plugins: Vec<LoadedPlugin>,
}

/// Default plugins directory: `<config>/Notepad Studio/plugins`. Falls back to the
/// current working dir's `plugins/` if the platform's config dir is
/// unavailable so a user can still smoke-test by dropping a dylib next to
/// the binary.
fn default_plugins_dir() -> PathBuf {
    if let Some(d) = dirs::config_dir() {
        return d.join(crate::consts::APP_NAME).join("plugins");
    }
    PathBuf::from("plugins")
}

/// Scan the plugins directory and populate the global registry. Best-effort:
/// failures are logged to stderr and the host carries on with whatever
/// plugins did load. Idempotent — calling twice is a no-op.
pub(crate) fn init() {
    GLOBAL.get_or_init(|| {
        let dir = default_plugins_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("[plugins] could not create {}: {e}", dir.display());
        }
        eprintln!("[plugins] scanning {}", dir.display());
        let plugins = scan_and_load(&dir);
        for p in &plugins {
            eprintln!(
                "[plugins] loaded \"{}\" v{} ({} commands)",
                p.name,
                p.version,
                p.commands.len()
            );
        }
        Mutex::new(PluginManager {
            plugins_dir: dir,
            plugins,
        })
    });
}

/// Snapshot for the menu builder. Returns one entry per loaded plugin with
/// its commands in declaration order. Empty `Vec` if no plugins loaded.
pub(crate) fn snapshot() -> Vec<PluginMenuEntry> {
    let Some(g) = GLOBAL.get() else {
        return Vec::new();
    };
    let Ok(guard) = g.lock() else {
        return Vec::new();
    };
    guard
        .plugins
        .iter()
        .map(|p| PluginMenuEntry {
            name: p.name.clone(),
            commands: p
                .commands
                .iter()
                .map(|c| LoadedCommand {
                    id: c.id,
                    name: c.name.clone(),
                })
                .collect(),
        })
        .collect()
}

#[derive(Clone)]
pub(crate) struct PluginMenuEntry {
    pub(crate) name: String,
    pub(crate) commands: Vec<LoadedCommand>,
}

/// Path to the configured plugins directory, for the menu's "Open Plugins
/// Folder" entry and the README documentation pointer.
pub(crate) fn plugins_dir() -> Option<PathBuf> {
    GLOBAL.get().and_then(|g| g.lock().ok().map(|m| m.plugins_dir.clone()))
}

/// Open the plugins directory in the OS file manager. Best-effort — if the
/// platform-specific opener isn't available the call is dropped with a
/// stderr log so the editor never panics on user click.
pub(crate) fn open_plugins_folder() {
    let Some(dir) = plugins_dir() else {
        eprintln!("[plugins] no plugins dir configured");
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[plugins] could not create {}: {e}", dir.display());
        return;
    }
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&dir).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(&dir).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&dir).spawn()
    };
    if let Err(e) = result {
        eprintln!("[plugins] could not open {}: {e}", dir.display());
    }
}

/// Invoke command `command_id` on the loaded plugin named `plugin_name`.
/// Sets the thread-local active-call scope so the C-ABI callbacks in
/// `host_api` can read/write the workspace, dispatches into the plugin's
/// `notepad_studio_plugin_run_command`, and clears the scope on the way out.
pub(crate) fn invoke(
    workspace: &mut Workspace,
    cx: &mut Context<Workspace>,
    plugin_name: &str,
    command_id: u32,
) {
    let Some(g) = GLOBAL.get() else {
        workspace.set_status("Plugins not initialised");
        return;
    };
    let Ok(guard) = g.lock() else {
        workspace.set_status("Plugin registry unavailable");
        return;
    };
    let Some(plugin) = guard.plugins.iter().find(|p| p.name == plugin_name) else {
        workspace.set_status(format!("Plugin '{plugin_name}' not loaded"));
        return;
    };
    let run_fn = plugin.run_command_fn;
    drop(guard);

    if !start_active_call(workspace, cx) {
        workspace.set_status(format!(
            "Plugin '{plugin_name}' not run: another plugin command is still running"
        ));
        return;
    }
    // SAFETY: `run_fn` was read from the plugin's
    // `notepad_studio_plugin_run_command` export with the signature fixed by
    // `PLUGIN_API_VERSION`, which `loader::load_one` checked, and its
    // `Library` stays loaded for the life of the process (`GLOBAL`).
    // `HOST_API` is a `'static` vtable. Plugin code is trusted native code
    // (README "Plugins"); the host-side callbacks validate what it passes.
    let result = unsafe { run_fn(command_id, &host_api::HOST_API as *const _) };
    end_active_call();

    if result != 0 {
        workspace.set_status(format!(
            "Plugin '{plugin_name}' command {command_id} returned status {result}"
        ));
    }
    cx.notify();
}
