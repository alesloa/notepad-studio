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
    // SAFETY: loading a library runs its initialisers. Plugins are trusted
    // native code the user installed into the plugins folder (README
    // "Plugins"); there is no sandbox to check them against.
    let lib = unsafe { Library::new(path) }.map_err(|e| format!("dlopen failed: {e}"))?;

    type InfoFn = unsafe extern "C" fn() -> *const PluginInfo;
    // SAFETY: the symbol's type is the one `declare_plugin!` exports for
    // this `PLUGIN_API_VERSION`; the version is checked before any field
    // other than `api_version` is trusted.
    let info_sym: Symbol<'_, InfoFn> = unsafe { lib.get(b"notepad_studio_plugin_info") }
        .map_err(|e| format!("missing notepad_studio_plugin_info: {e}"))?;
    // SAFETY: calls the export resolved above; it takes no arguments.
    let info_ptr = unsafe { info_sym() };
    if info_ptr.is_null() || !info_ptr.is_aligned() {
        return Err("notepad_studio_plugin_info returned a null or misaligned pointer".into());
    }
    // SAFETY: non-null and aligned (checked above); the ABI says the table
    // lives as long as the library, which `LoadedPlugin` keeps loaded.
    let info: &PluginInfo = unsafe { &*info_ptr };
    if info.api_version != PLUGIN_API_VERSION {
        return Err(format!(
            "ABI mismatch: plugin reports v{}, host expects v{}",
            info.api_version, PLUGIN_API_VERSION
        ));
    }

    // SAFETY: the `PluginInfo` string fields are NUL-terminated per the ABI.
    let name = unsafe { read_cstr(info.name) }.ok_or("plugin name missing or not valid UTF-8")?;
    // SAFETY: as above.
    let version = unsafe { read_cstr(info.version) }.unwrap_or_else(|| "?".into());
    // SAFETY: as above.
    let description = unsafe { read_cstr(info.description) }.unwrap_or_default();
    // SAFETY: `info` passed the version check, so `commands` points at
    // `command_count` entries per the ABI.
    let commands = unsafe { read_commands(info) }?;

    // SAFETY: same reasoning as `notepad_studio_plugin_info` above; the
    // pointer is copied out and only called while `lib` stays loaded.
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

/// Copy the plugin's command table. A null table means no commands. The
/// count is the plugin's claim, so it never sizes an allocation up front: a
/// garbage `command_count` used to reserve up to 4 billion entries and abort
/// the editor at startup.
///
/// # Safety
/// When `info.commands` is non-null it must point to `info.command_count`
/// readable `PluginCommand` entries whose names are null or NUL-terminated.
unsafe fn read_commands(info: &PluginInfo) -> Result<Vec<LoadedCommand>, String> {
    let mut commands = Vec::new();
    if info.commands.is_null() {
        return Ok(commands);
    }
    if !info.commands.is_aligned() {
        return Err("command table pointer is misaligned".into());
    }
    for i in 0..info.command_count as usize {
        // SAFETY: `i < command_count`, which the caller guarantees is
        // in bounds of the aligned, non-null table.
        let entry = unsafe { &*info.commands.add(i) };
        // SAFETY: entry names are NUL-terminated per the ABI.
        let cmd_name = unsafe { read_cstr(entry.name) }
            .ok_or_else(|| format!("command {} name missing or not valid UTF-8", entry.id))?;
        commands.push(LoadedCommand {
            id: entry.id,
            name: cmd_name,
        });
    }
    Ok(commands)
}

/// # Safety
/// A non-null `ptr` must point to a NUL-terminated string.
unsafe fn read_cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: non-null; NUL termination is the caller's contract.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_str().ok().map(|s| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(command_count: u32, commands: *const notepad_studio_plugin::PluginCommand) -> PluginInfo {
        PluginInfo {
            api_version: PLUGIN_API_VERSION,
            name: ptr::null(),
            version: ptr::null(),
            description: ptr::null(),
            command_count,
            commands,
        }
    }

    use core::ptr;

    #[test]
    fn huge_command_count_with_null_table_does_not_allocate() {
        // Used to reserve u32::MAX entries (about 137 GB) and abort.
        // SAFETY: null table, nothing is read.
        let cmds = unsafe { read_commands(&info(u32::MAX, ptr::null())) };
        assert_eq!(cmds.map(|c| c.len()), Ok(0));
    }

    #[test]
    fn null_or_non_utf8_command_names_are_errors() {
        let bad = [0xffu8, 0];
        let table = [
            notepad_studio_plugin::PluginCommand { id: 1, name: c"Ok".as_ptr() },
            notepad_studio_plugin::PluginCommand { id: 2, name: ptr::null() },
        ];
        // SAFETY: `table` holds 2 entries with null or NUL-terminated names.
        let one = unsafe { read_commands(&info(1, table.as_ptr())) };
        assert_eq!(one.map(|c| c[0].name.clone()), Ok("Ok".to_string()));
        // SAFETY: as above.
        assert!(unsafe { read_commands(&info(2, table.as_ptr())) }.is_err());
        // SAFETY: `bad` is NUL-terminated.
        assert_eq!(unsafe { read_cstr(bad.as_ptr() as *const c_char) }, None);
    }
}
