// PowerEditor/src/NppCommands.cpp:2945-2969 — Run dialog and shortcut validation.
use crate::actions::{RunCommand, RunSavedCommand, RunValidateShortcuts};
use crate::workspace::Workspace;
use gpui::{AppContext, Context, Window};

impl Workspace {
    pub(crate) fn run_saved_command(
        &mut self,
        action: &RunSavedCommand,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(saved) = crate::run_command::saved_snapshot()
            .into_iter()
            .find(|saved| saved.id == action.id)
        else {
            self.set_status(
                "This saved Run command was removed. Open Run... to choose or save a command.",
            );
            cx.notify();
            return;
        };
        let context = self
            .tabs
            .get(self.active)
            .ok_or_else(|| "Open a document before running a command.".to_string())
            .and_then(|tab| {
                crate::run_command::RunContext::from_buffer(&tab.buffer, tab.selection)
            });
        let context = match context {
            Ok(context) => context,
            Err(err) => {
                self.set_status(err);
                cx.notify();
                return;
            }
        };
        let task = cx.background_spawn(async move {
            let prepared = crate::run_command::prepare(
                &saved.command,
                &context,
                crate::run_command::Platform::current(),
                |name| std::env::var(name).ok(),
            )?;
            crate::run_command::launch(prepared)?;
            let warning = crate::run_command::update_config(|history| {
                history.record(&saved.command);
                Ok(())
            })
            .err();
            Ok::<_, String>((saved.name, warning))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.set_status(match result {
                    Ok((name, None)) => format!("Launched {name}."),
                    Ok((name, Some(err))) => {
                        format!("Launched {name}; Run history could not be saved: {err}")
                    }
                    Err(err) => err,
                });
                cx.notify();
            });
        })
        .detach();
        self.set_status("Launching saved command…");
        cx.notify();
    }

    pub(crate) fn open_run(&mut self, _: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.entity().downgrade();
        let errors = workspace.clone();
        let mode = self.theme_mode;
        let appearance = window.appearance();
        let zoom = self.zoom_level;
        cx.defer(move |cx| {
            if let Err(err) =
                crate::ui::run_dialog::open_or_focus(workspace, mode, appearance, zoom, cx)
            {
                let _ = errors.update(cx, |workspace, cx| {
                    workspace.set_status(err);
                    cx.notify();
                });
            }
        });
    }

    pub(crate) fn validate_run_shortcuts(
        &mut self,
        _: &RunValidateShortcuts,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_status("Notepad Studio uses shortcuts.toml, loaded with validation by the Shortcut Mapper. Notepad++'s shortcuts.xml machine authentication does not apply.");
        cx.notify();
    }
}
