// C-ABI surface. Both the editor host and every plugin compile against this
// module — the layout of every struct here is part of the wire format and
// must not change without bumping `PLUGIN_API_VERSION`.
//
// The host loads plugin shared libraries with `libloading`, looks up
// `notepad_studio_plugin_info` for metadata, and dispatches commands by calling
// `notepad_studio_plugin_run_command(command_id, &host_api)`. The plugin uses the
// function pointers in the supplied `HostApi` table to query / mutate the
// active buffer or surface a status message.

use core::ffi::c_char;

/// Bumped on any breaking change to the structs / function signatures below.
/// The host refuses to load plugins whose `PluginInfo::api_version` differs
/// from this constant — they may have been built against a different
/// `notepad-studio-plugin` revision and the layouts won't match.
pub const PLUGIN_API_VERSION: u32 = 1;

/// Plugin metadata returned by `notepad_studio_plugin_info`. All `*const c_char`
/// fields must point to NUL-terminated UTF-8 with `'static` lifetime — the
/// host borrows these for the lifetime of the loaded library.
#[repr(C)]
pub struct PluginInfo {
    pub api_version: u32,
    pub name: *const c_char,
    pub version: *const c_char,
    pub description: *const c_char,
    pub command_count: u32,
    pub commands: *const PluginCommand,
}

/// One command exposed by a plugin. `id` is plugin-private and round-trips
/// through `notepad_studio_plugin_run_command(id, host)` when the user picks the
/// matching menu entry. `name` is shown in the Plugins ▸ <plugin> submenu.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginCommand {
    pub id: u32,
    pub name: *const c_char,
}

/// Host -> plugin: invoke command `command_id`. Plugins use the function
/// pointers in `host` to act on the editor. Return value is reserved — `0`
/// signals success, any non-zero value is logged by the host.
pub type PluginRunCommandFn =
    unsafe extern "C" fn(command_id: u32, host: *const HostApi) -> u32;

/// Plugin -> host: callbacks the plugin can use during a `run_command`.
/// All callbacks borrow from the live editor on the main thread; calls
/// that originate from background threads are undefined behavior. v1 ships
/// with no async — plugin commands are called synchronously and must
/// return promptly.
#[repr(C)]
pub struct HostApi {
    pub api_version: u32,

    // --- Buffer access ---------------------------------------------------------
    /// Returns the entire active-buffer text as UTF-8 bytes. Caller must
    /// release with `free_buffer_text`. Sets `*out_len` to the byte length.
    /// Returns null on error (in which case `*out_len` is unchanged).
    pub get_buffer_text: unsafe extern "C" fn(out_len: *mut usize) -> *const u8,
    /// Free a buffer previously returned by `get_buffer_text`. The host
    /// allocates and frees on the same side of the FFI boundary — plugins
    /// must never call `free` themselves on these pointers.
    pub free_buffer_text: unsafe extern "C" fn(ptr: *const u8, len: usize),
    /// Replace the entire active buffer with `len` bytes of UTF-8 starting
    /// at `ptr`. Bytes are copied; the plugin retains ownership of `ptr`.
    pub replace_buffer_text: unsafe extern "C" fn(ptr: *const u8, len: usize),

    // --- Selection -------------------------------------------------------------
    /// Reports the active linear selection's byte range as `[*start, *end)`.
    /// When no selection is active, both equal the caret offset. For a
    /// block (column-mode) selection v1 returns `[min_byte, max_byte)`
    /// covering the rectangle's extent — the plugin sees one slice, not a
    /// per-line set.
    pub get_selection: unsafe extern "C" fn(start: *mut usize, end: *mut usize),
    /// Replace the active selection with `len` UTF-8 bytes starting at
    /// `ptr`. Like `replace_buffer_text` this copies; plugin retains
    /// ownership.
    pub replace_selection: unsafe extern "C" fn(ptr: *const u8, len: usize),

    // --- Status bar ------------------------------------------------------------
    /// Show `len` UTF-8 bytes from `ptr` on the editor's status bar. Same
    /// surface used by file ops; replaced by the next status update.
    pub set_status_message: unsafe extern "C" fn(ptr: *const u8, len: usize),
}

impl HostApi {
    /// Convenience for plugins compiled with the same crate version as the
    /// host: confirms the supplied API version matches before any callback
    /// is invoked.
    pub fn supports(&self) -> bool {
        self.api_version == PLUGIN_API_VERSION
    }
}
