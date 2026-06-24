// Macro menu actions. Notepad++'s `Macro` top-level menu — start/stop record,
// playback, run multiple times, save current, and a dynamically-built sub-list
// of saved macros (each `MacroRunSaved { name }`).

gpui::actions!(
    notepad_studio,
    [
        MacroStartStopRecording,
        MacroStartRecording,
        MacroStopRecording,
        MacroPlayback,
        MacroRunMultipleTimes,
        MacroSaveCurrent,
    ]
);

/// Run a saved macro by name. Built dynamically into the Macro menu — the
/// menu rebuild iterates `macros::saved_snapshot()` and emits one of these
/// per saved macro. `no_json` because we never construct it from a keymap.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = notepad_studio, no_json)]
pub struct MacroRunSaved {
    pub name: String,
}
