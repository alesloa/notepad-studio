// Plugin menu actions. The Plugins menu spawns one `PluginRunCommand` per
// command exported by every loaded plugin, plus a global
// `PluginsOpenFolder` that reveals the plugins directory in the OS file
// manager. `no_json` because both actions carry runtime data (plugin name
// or process side effects) and aren't user-bindable.

gpui::actions!(notepad_studio, [PluginsOpenFolder]);

/// Run command `command_id` on the loaded plugin named `plugin`. The menu
/// builder emits one of these per per-plugin command. Both fields together
/// uniquely identify which native callback the host should fire.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = notepad_studio, no_json)]
pub struct PluginRunCommand {
    pub plugin: String,
    pub command_id: u32,
}
