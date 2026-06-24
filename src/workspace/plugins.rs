// Plugin command dispatch on the Workspace side. The Plugins menu emits
// `PluginRunCommand { plugin, command_id }` actions; this handler hands
// the call off to `crate::plugins::invoke`, which sets the per-call
// thread-local pointing back at this Workspace and dispatches into the
// loaded plugin's exported `notepad_studio_plugin_run_command`.

use gpui::{Context, Window};

use crate::actions::PluginRunCommand;
use crate::plugins;
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn plugin_run_command(
        &mut self,
        action: &PluginRunCommand,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        plugins::invoke(self, cx, &action.plugin, action.command_id);
    }
}
