// Sample plugin demonstrating the Notepad Studio plugin API.
//
// Two commands:
//   1 — Uppercase Selection: read selection bytes via the host, uppercase
//       them, write the result back through `replace_selection`.
//   2 — Show Hello: posts a one-line status message.
//
// Build with `cargo build -p sample-uppercase-plugin --release`. The
// resulting `libsample_uppercase_plugin.{dylib,so,dll}` drops into the
// configured plugins directory printed at editor startup.

use notepad_studio_plugin::{
    declare_plugin, CommandStatus, Host, Plugin, PluginCommandSpec, PluginMetadata,
};

const COMMANDS: &[PluginCommandSpec] = &[
    PluginCommandSpec {
        id: 1,
        name: "Uppercase Selection",
    },
    PluginCommandSpec {
        id: 2,
        name: "Show Hello",
    },
];

const METADATA: PluginMetadata = PluginMetadata {
    name: "Sample Uppercase",
    version: "0.1.0",
    description: "Demonstrates the plugin API: uppercases a selection, posts a status message.",
    commands: COMMANDS,
};

pub struct SamplePlugin;

impl Plugin for SamplePlugin {
    fn metadata() -> &'static PluginMetadata {
        &METADATA
    }

    fn run(command_id: u32, host: &Host<'_>) -> CommandStatus {
        match command_id {
            1 => {
                let (start, end) = host.selection();
                if end <= start {
                    host.set_status("Sample Uppercase: nothing selected");
                    return CommandStatus::Ok;
                }
                let buffer = host.buffer_text();
                if end > buffer.len() {
                    return CommandStatus::Error;
                }
                let slice = match std::str::from_utf8(&buffer[start..end]) {
                    Ok(s) => s,
                    Err(_) => return CommandStatus::Error,
                };
                let upper = slice.to_uppercase();
                host.replace_selection(&upper);
                host.set_status(format!(
                    "Sample Uppercase: {} bytes -> uppercase",
                    upper.len()
                ).as_str());
                CommandStatus::Ok
            }
            2 => {
                host.set_status("Hello from Sample plugin");
                CommandStatus::Ok
            }
            _ => CommandStatus::Error,
        }
    }
}

declare_plugin!(SamplePlugin);
