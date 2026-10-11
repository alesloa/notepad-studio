// PowerEditor/src/Notepad_plus.rc:1318-1323 — Run menu commands.
gpui::actions!(notepad_studio, [RunCommand, RunValidateShortcuts]);

/// Dynamic Run menu entry. Definitions are looked up by stable id at launch,
/// so an old menu cannot execute a command that has since been removed.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = notepad_studio, no_json)]
pub struct RunSavedCommand {
    pub id: String,
}
