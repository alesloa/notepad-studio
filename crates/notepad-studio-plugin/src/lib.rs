// Notepad Studio plugin ABI.
//
// Two surfaces:
//   1. The C-ABI types in `ffi` — the wire format every plugin and the
//      editor host both compile against. Bumped via `PLUGIN_API_VERSION`
//      whenever a struct shape changes.
//   2. Safe Rust wrappers in `wrapper` for plugin authors so they don't
//      hand-roll FFI. Plugin authors implement the `Plugin` trait and call
//      `notepad_studio_plugin::declare_plugin!(MyPlugin)` to emit the required
//      `notepad_studio_plugin_info` / `notepad_studio_plugin_run_command` exports.

#![deny(rust_2018_idioms)]

pub mod ffi;
pub mod wrapper;

pub use ffi::{HostApi, PluginCommand, PluginInfo, PluginRunCommandFn, PLUGIN_API_VERSION};
pub use wrapper::{CommandStatus, Host, Plugin, PluginCommandSpec, PluginMetadata};
