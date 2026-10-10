// Macro menu — Notepad++'s top-level "Macro" menu. Surfaces start/stop
// recording, playback, run-N-times, save, and a list of saved-macro entries
// pulled from the persistent `<config>/Notepad Studio/macros.toml`. The saved-macro
// list is rebuilt on every menu refresh; saving a new macro calls
// `rebuild_menus(cx)` so the new entry appears immediately.

use gpui::{Menu, MenuItem};

use crate::actions::*;
use super::MenuState;
use crate::macros;
use crate::ui::menu_bar::mnemonic;

pub(crate) fn build(_state: &MenuState) -> Menu {
    let saved = macros::saved_snapshot();
    let mut items: Vec<MenuItem> = vec![
        MenuItem::action("Start/Stop Recording", MacroStartStopRecording),
        MenuItem::action("Playback", MacroPlayback),
        MenuItem::action("Run a Macro Multiple Times…", MacroRunMultipleTimes),
        MenuItem::action("Save Currently Recorded Macro…", MacroSaveCurrent),
    ];
    if !saved.is_empty() {
        items.push(MenuItem::separator());
        for (name, _count) in saved {
            let label = mnemonic::escape(&name);
            items.push(MenuItem::action(
                label,
                MacroRunSaved { name },
            ));
        }
    }
    Menu {
        name: "&Macro".into(),
        items,
    }
}
