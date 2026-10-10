// Safe Rust wrappers for plugin authors.
//
// Plugin authors implement `Plugin` and call `declare_plugin!(MyPlugin)` to
// emit the C-ABI exports. The macro builds the static `PluginInfo` /
// `PluginCommand` table from `PluginMetadata` and dispatches each
// `notepad_studio_plugin_run_command` call into the trait.

use core::slice;

use crate::ffi::{HostApi, PLUGIN_API_VERSION};

/// What plugin authors fill in.
pub struct PluginMetadata {
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub commands: &'static [PluginCommandSpec],
}

/// One menu command. `id` is what gets passed back to `Plugin::run` so the
/// plugin can branch on which menu entry the user picked.
pub struct PluginCommandSpec {
    pub id: u32,
    pub name: &'static str,
}

/// Plugin command return status. Mapped onto the C-ABI's `u32` return.
#[repr(u32)]
pub enum CommandStatus {
    Ok = 0,
    Error = 1,
}

/// Trait implemented by every plugin. The static metadata table is built
/// once at module load (via `declare_plugin!`); `run` is called every time
/// the user picks one of this plugin's commands from the Plugins menu.
pub trait Plugin {
    fn metadata() -> &'static PluginMetadata;
    fn run(command_id: u32, host: &Host<'_>) -> CommandStatus;
}

/// Safe wrapper around the raw `*const HostApi` the host passes in. Pins
/// the pointer to `'a` so plugin code can't accidentally stash it past the
/// `run_command` call boundary.
pub struct Host<'a> {
    api: &'a HostApi,
}

impl<'a> Host<'a> {
    /// Construct from a raw pointer — only the generated `declare_plugin!`
    /// dispatcher should call this. Returns `None` if the host's API
    /// version doesn't match the version this plugin was compiled against,
    /// which protects against host/plugin ABI drift even within a single
    /// `PLUGIN_API_VERSION` (paranoia, since the host already checks the
    /// info-side version).
    ///
    /// # Safety
    /// `ptr` must point to a `HostApi` valid for the duration of the
    /// borrow. Callers (the `declare_plugin!` macro expansion) ensure this
    /// by only constructing `Host` inside `notepad_studio_plugin_run_command` and
    /// not letting the wrapper outlive the call.
    pub unsafe fn from_raw(ptr: *const HostApi) -> Option<Host<'a>> {
        if ptr.is_null() || !ptr.is_aligned() {
            return None;
        }
        // SAFETY: non-null and aligned (checked above); validity for `'a` is
        // this function's contract.
        let api = unsafe { &*ptr };
        if api.api_version != PLUGIN_API_VERSION {
            return None;
        }
        Some(Host { api })
    }

    /// Read the entire active buffer as UTF-8.
    pub fn buffer_text(&self) -> Vec<u8> {
        let mut len = 0usize;
        // SAFETY: `api` is the host's vtable, valid for `'a`; `len` is a
        // live, aligned out-pointer.
        let ptr = unsafe { (self.api.get_buffer_text)(&mut len as *mut usize) };
        if ptr.is_null() {
            return Vec::new();
        }
        let copy = if len == 0 {
            Vec::new()
        } else {
            // SAFETY: the host returned `ptr` with `len` readable bytes,
            // valid until `free_buffer_text` below.
            unsafe { slice::from_raw_parts(ptr, len) }.to_vec()
        };
        // SAFETY: hands back exactly the pointer and length the host gave.
        unsafe { (self.api.free_buffer_text)(ptr, len) };
        copy
    }

    /// Replace the entire buffer with `text`.
    pub fn replace_buffer(&self, text: &str) {
        let bytes = text.as_bytes();
        // SAFETY: `bytes` is valid UTF-8 readable for its length during the
        // call; the host copies it.
        unsafe { (self.api.replace_buffer_text)(bytes.as_ptr(), bytes.len()) };
    }

    /// Active selection's byte range. For block selections this is the
    /// rectangle's `[min_byte, max_byte)` extent.
    pub fn selection(&self) -> (usize, usize) {
        let mut start = 0usize;
        let mut end = 0usize;
        // SAFETY: both out-pointers are live, aligned locals.
        unsafe {
            (self.api.get_selection)(&mut start as *mut usize, &mut end as *mut usize);
        }
        (start, end)
    }

    /// Replace whatever is selected with `text`. With an empty selection,
    /// inserts at the caret.
    pub fn replace_selection(&self, text: &str) {
        let bytes = text.as_bytes();
        // SAFETY: as in `replace_buffer`.
        unsafe { (self.api.replace_selection)(bytes.as_ptr(), bytes.len()) };
    }

    /// Show a one-line message on the editor status bar.
    pub fn set_status(&self, msg: &str) {
        let bytes = msg.as_bytes();
        // SAFETY: as in `replace_buffer`.
        unsafe { (self.api.set_status_message)(bytes.as_ptr(), bytes.len()) };
    }
}

/// Emit the C-ABI exports for a plugin type implementing `Plugin`.
///
/// Usage:
/// ```ignore
/// struct MyPlugin;
/// impl notepad_studio_plugin::Plugin for MyPlugin { ... }
/// notepad_studio_plugin::declare_plugin!(MyPlugin);
/// ```
///
/// The macro builds a `'static` `PluginInfo` / `PluginCommand` table from
/// the trait's `metadata()` return and exports the two `extern "C"`
/// functions the host expects.
#[macro_export]
macro_rules! declare_plugin {
    ($plugin_ty:ty) => {
        const _NOTEPAD_STUDIO_PLUGIN_ASSERT: fn() = || {
            // Compile-time check that the type implements the trait.
            fn _assert<P: $crate::Plugin>() {}
            _assert::<$plugin_ty>();
        };

        // Holds the built table for `PluginInfo` to point at. Lives as long
        // as the loaded library so the host's borrowed pointers stay valid.
        // The fields are never read: they only own what `info` points at.
        #[allow(dead_code)]
        struct __NotepadStudioPluginCells {
            commands: Vec<$crate::PluginCommand>,
            // Each name needs a NUL-terminated `Vec<u8>` so the `*const c_char`
            // in `PluginCommand` has a stable backing store.
            command_names: Vec<Vec<u8>>,
            name: Vec<u8>,
            version: Vec<u8>,
            description: Vec<u8>,
            info: $crate::PluginInfo,
        }

        // Safety: `__NotepadStudioPluginCells` is only constructed once via the
        // `OnceLock`, and after construction nothing mutates it. The raw
        // pointers inside `info` reference cells stored in the same struct,
        // which lives in static storage for the lifetime of the loaded
        // library. The only consumer is the host on the same thread, so
        // Send/Sync is fine.
        unsafe impl Send for __NotepadStudioPluginCells {}
        unsafe impl Sync for __NotepadStudioPluginCells {}

        static __NOTEPAD_STUDIO_PLUGIN_CELLS: ::std::sync::OnceLock<__NotepadStudioPluginCells> =
            ::std::sync::OnceLock::new();

        fn __notepad_studio_plugin_cells() -> &'static __NotepadStudioPluginCells {
            __NOTEPAD_STUDIO_PLUGIN_CELLS.get_or_init(|| {
                let meta: &'static $crate::PluginMetadata =
                    <$plugin_ty as $crate::Plugin>::metadata();
                let mut command_names: Vec<Vec<u8>> =
                    Vec::with_capacity(meta.commands.len());
                let mut commands: Vec<$crate::PluginCommand> =
                    Vec::with_capacity(meta.commands.len());
                for spec in meta.commands {
                    let mut name_bytes = spec.name.as_bytes().to_vec();
                    name_bytes.push(0);
                    command_names.push(name_bytes);
                }
                for (spec, name) in meta.commands.iter().zip(command_names.iter()) {
                    commands.push($crate::PluginCommand {
                        id: spec.id,
                        name: name.as_ptr() as *const ::core::ffi::c_char,
                    });
                }
                let mut name = meta.name.as_bytes().to_vec();
                name.push(0);
                let mut version = meta.version.as_bytes().to_vec();
                version.push(0);
                let mut description = meta.description.as_bytes().to_vec();
                description.push(0);
                let info = $crate::PluginInfo {
                    api_version: $crate::PLUGIN_API_VERSION,
                    name: name.as_ptr() as *const ::core::ffi::c_char,
                    version: version.as_ptr() as *const ::core::ffi::c_char,
                    description: description.as_ptr() as *const ::core::ffi::c_char,
                    command_count: commands.len() as u32,
                    commands: commands.as_ptr(),
                };
                __NotepadStudioPluginCells {
                    commands,
                    command_names,
                    name,
                    version,
                    description,
                    info,
                }
            })
        }

        // A panic must not unwind across `extern "C"`: Rust aborts the whole
        // process there, closing every editor window with unsaved work.
        // Both exports catch it and report failure through the ABI instead
        // (a null info pointer, which the host skips, or status 1). Plugins
        // built with `panic = "abort"` still abort; build them with unwind.
        #[no_mangle]
        pub unsafe extern "C" fn notepad_studio_plugin_info() -> *const $crate::PluginInfo {
            ::std::panic::catch_unwind(|| &__notepad_studio_plugin_cells().info as *const _)
                .unwrap_or(::core::ptr::null())
        }

        #[no_mangle]
        pub unsafe extern "C" fn notepad_studio_plugin_run_command(
            command_id: u32,
            host: *const $crate::HostApi,
        ) -> u32 {
            // SAFETY: the host passes a pointer to its `'static` vtable for the
            // duration of this call; `from_raw` checks null and alignment.
            let Some(host_wrap) = (unsafe { $crate::Host::from_raw(host) }) else {
                return $crate::CommandStatus::Error as u32;
            };
            ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                <$plugin_ty as $crate::Plugin>::run(command_id, &host_wrap) as u32
            }))
            .unwrap_or($crate::CommandStatus::Error as u32)
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::{CommandStatus, Host, Plugin, PluginCommandSpec, PluginMetadata};

    struct Panicky;

    const COMMANDS: &[PluginCommandSpec] = &[PluginCommandSpec { id: 1, name: "Boom" }];
    const META: PluginMetadata = PluginMetadata {
        name: "Panicky",
        version: "0",
        description: "",
        commands: COMMANDS,
    };

    impl Plugin for Panicky {
        fn metadata() -> &'static PluginMetadata {
            &META
        }
        fn run(_command_id: u32, _host: &Host<'_>) -> CommandStatus {
            panic!("plugin bug");
        }
    }

    crate::declare_plugin!(Panicky);

    unsafe extern "C" fn get_text(out_len: *mut usize) -> *const u8 {
        // SAFETY: test caller passes a valid out-pointer.
        unsafe { *out_len = 0 };
        core::ptr::null()
    }
    unsafe extern "C" fn free_text(_: *const u8, _: usize) {}
    unsafe extern "C" fn take_text(_: *const u8, _: usize) {}
    unsafe extern "C" fn get_sel(_: *mut usize, _: *mut usize) {}

    static HOST: crate::HostApi = crate::HostApi {
        api_version: crate::PLUGIN_API_VERSION,
        get_buffer_text: get_text,
        free_buffer_text: free_text,
        replace_buffer_text: take_text,
        get_selection: get_sel,
        replace_selection: take_text,
        set_status_message: take_text,
    };

    #[test]
    fn a_panicking_command_returns_an_error_status() {
        // Without the catch this unwound out of an `extern "C"` fn, which
        // aborts the process.
        // SAFETY: `HOST` is a valid `'static` vtable.
        let status = unsafe { notepad_studio_plugin_run_command(1, &HOST) };
        assert_eq!(status, CommandStatus::Error as u32);
    }

    #[test]
    fn null_or_misaligned_host_is_rejected() {
        // SAFETY: the export must not dereference either pointer.
        unsafe {
            assert_eq!(notepad_studio_plugin_run_command(1, core::ptr::null()), 1);
            let misaligned = (&HOST as *const crate::HostApi as *const u8).add(1)
                as *const crate::HostApi;
            assert_eq!(notepad_studio_plugin_run_command(1, misaligned), 1);
        }
    }

    #[test]
    fn info_table_lists_the_commands() {
        // SAFETY: plain call into the export generated above.
        let info = unsafe { &*notepad_studio_plugin_info() };
        assert_eq!(info.command_count, 1);
        assert_eq!(info.api_version, crate::PLUGIN_API_VERSION);
    }
}
