// Plugin loader. Scans `<plugins_dir>` for shared libraries with the
// platform's native extension, opens each with `libloading`, validates the
// `notepad_studio_plugin_info` metadata, and stashes the `notepad_studio_plugin_run_command`
// fn pointer for later dispatch.
//
// Plugins that fail to open, advertise the wrong API version, or omit a
// required symbol are logged to stderr and skipped — never fatal. The host
// carries on with the plugins that did load.

use core::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};
use notepad_studio_plugin::{HostApi, PLUGIN_API_VERSION, PluginInfo};

/// FFI signature for the per-plugin `notepad_studio_plugin_run_command` export.
/// Stored as a bare `unsafe extern "C" fn` (which is `Copy`) so the manager
/// can keep it alongside the owning `Library` without holding a borrow on
/// the symbol table.
pub(crate) type RunCommandFn =
    unsafe extern "C" fn(command_id: u32, host: *const HostApi) -> u32;

/// One loaded plugin. The `Library` is kept alive in this struct — dropping
/// it unloads the underlying shared library and invalidates `run_command_fn`,
/// so loaded plugins live as long as the editor process (or until the user
/// restarts after dropping a new dylib in).
pub(crate) struct LoadedPlugin {
    pub(crate) name: String,
    #[allow(dead_code)]
    pub(crate) version: String,
    #[allow(dead_code)]
    pub(crate) description: String,
    pub(crate) commands: Vec<LoadedCommand>,
    pub(crate) run_command_fn: RunCommandFn,
    /// Owning handle keeps the dylib resident; never read after construction.
    _library: Library,
}

#[derive(Clone)]
pub(crate) struct LoadedCommand {
    pub(crate) id: u32,
    pub(crate) name: String,
}

/// Walk `dir` and return one `LoadedPlugin` per validated dylib. Non-dylib
/// files, plugins missing the required exports, and plugins reporting a
/// non-matching `api_version` are logged and skipped.
pub(crate) fn scan_and_load(dir: &Path) -> Vec<LoadedPlugin> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_plugin_lib(&path) {
            continue;
        }
        match load_one(&path) {
            Ok(p) => out.push(p),
            Err(e) => eprintln!("[plugins] skip {}: {e}", path.display()),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn is_plugin_lib(path: &PathBuf) -> bool {
    if !path.is_file() {
        return false;
    }
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    if cfg!(target_os = "macos") {
        ext == "dylib"
    } else if cfg!(target_os = "windows") {
        ext == "dll"
    } else {
        ext == "so"
    }
}

fn load_one(path: &Path) -> Result<LoadedPlugin, String> {
    let lib = unsafe { Library::new(path) }.map_err(|e| format!("dlopen failed: {e}"))?;

    type InfoFn = unsafe extern "C" fn() -> *const PluginInfo;
    let info_sym: Symbol<'_, InfoFn> = unsafe { lib.get(b"notepad_studio_plugin_info") }
        .map_err(|e| format!("missing notepad_studio_plugin_info: {e}"))?;
    let info_ptr = unsafe { info_sym() };
    if info_ptr.is_null() {
        return Err("notepad_studio_plugin_info returned null".into());
    }
    let info: &PluginInfo = unsafe { &*info_ptr };
    if info.api_version != PLUGIN_API_VERSION {
        return Err(format!(
            "ABI mismatch: plugin reports v{}, host expects v{}",
            info.api_version, PLUGIN_API_VERSION
        ));
    }

    let name = read_cstr(info.name).ok_or("plugin name not valid UTF-8")?;
    let version = read_cstr(info.version).unwrap_or_else(|| "?".into());
    let description = read_cstr(info.description).unwrap_or_default();

    let mut commands = Vec::with_capacity(info.command_count as usize);
    if !info.commands.is_null() {
        for i in 0..info.command_count {
            let entry = unsafe { &*info.commands.add(i as usize) };
            let cmd_name = read_cstr(entry.name)
                .ok_or_else(|| format!("command {} name not valid UTF-8", entry.id))?;
            commands.push(LoadedCommand {
                id: entry.id,
                name: cmd_name,
            });
        }
    }

    let run_sym: Symbol<'_, RunCommandFn> =
        unsafe { lib.get(b"notepad_studio_plugin_run_command") }
            .map_err(|e| format!("missing notepad_studio_plugin_run_command: {e}"))?;
    let run_command_fn: RunCommandFn = *run_sym;

    Ok(LoadedPlugin {
        name,
        version,
        description,
        commands,
        run_command_fn,
        _library: lib,
    })
}

fn read_cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_str().ok().map(|s| s.to_owned())
}
