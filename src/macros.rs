// Macro record / replay.
//
// Notepad++'s Macro feature: record a sequence of editor actions, stop, then
// play them back once or N times. v1 captures keyboard-driven editor actions
// only — mouse events, tab switches, and window-level operations are not
// recorded (NPP behaves the same for the same reasons; mouse coordinates
// don't replay meaningfully across different document state).
//
// Storage: an in-memory "current" macro (the last recorded sequence) plus a
// `BTreeMap<String, Vec<MacroEvent>>` of named, persisted macros loaded from
// `<config>/Notepad Studio/macros.toml` on startup.
//
// Re-entrancy: the recorder funnel checks a `replaying` flag; events fired
// while replay is in flight are skipped so a macro can't infinitely append
// itself during playback.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::consts::APP_NAME;

/// Events captured during recording. Each variant maps onto a single
/// keyboard-driven editor action. Replay dispatches these by calling the
/// matching `Workspace` method directly (not via gpui action dispatch) so
/// re-entry into the recorder is bounded by the explicit `replaying` flag.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum MacroEvent {
    InsertText(String),
    Backspace,
    Delete,
    InsertNewline,
    InsertTab,
    DuplicateLine,
    MoveLineUp,
    MoveLineDown,
    DeleteLine,
    Uppercase,
    Lowercase,
    BlockComment,
    StreamComment,
    InsertBlankLineAbove,
    InsertBlankLineBelow,
    TrimTrailingWs,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Undo,
    Redo,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    PageUp,
    PageDown,
    SelectPageUp,
    SelectPageDown,
    MoveLineStart,
    MoveLineEnd,
    SelectLineStart,
    SelectLineEnd,
    MoveDocStart,
    MoveDocEnd,
    SelectDocStart,
    SelectDocEnd,
    MoveWordLeft,
    MoveWordRight,
    SelectWordLeft,
    SelectWordRight,
    FindNext,
    FindPrev,
    ReplaceCurrent,
    ReplaceAll,
    Save,
    /// Edit ▸ Insert ▸ Date/Time Short. Replay re-resolves the OS locale at
    /// playback time, matching NPP: the recorded macro inserts whatever the
    /// short date/time is *when replayed*, not when recorded.
    InsertDateTimeShort,
    /// Edit ▸ Insert ▸ Date/Time Long. Same replay-time resolution policy.
    InsertDateTimeLong,
    /// Edit ▸ Insert ▸ Date/Time Customized. Replay reads the *current*
    /// `Settings::datetime_custom_format` and applies it to `now()`. If the
    /// user changed the format between recording and replay, the new format
    /// is used — same as NPP, where the menu item itself reads the current
    /// preference each invocation.
    InsertDateTimeCustomized,
}

#[derive(Default)]
pub(crate) struct MacroRecorder {
    /// True from `start_recording` until `stop_recording`. Events fired while
    /// false are dropped before they reach `in_progress`.
    pub(crate) recording: bool,
    /// True while a `replay` is actively dispatching events. Suppresses the
    /// recorder so the replayed actions don't append back into a recording
    /// (or worse, into themselves on a multi-run replay).
    pub(crate) replaying: bool,
    /// Events being captured for the in-flight recording. Moved into
    /// `current` when `stop_recording` fires.
    in_progress: Vec<MacroEvent>,
    /// The most-recently-stopped macro. `Playback` runs this; `SaveCurrent`
    /// names + persists this.
    pub(crate) current: Vec<MacroEvent>,
    /// Saved, named macros — loaded from disk on first global access and
    /// persisted on every `save_macro` / `delete_saved`.
    pub(crate) saved: BTreeMap<String, Vec<MacroEvent>>,
}

#[derive(Default, Serialize, Deserialize)]
struct MacrosFile {
    #[serde(default)]
    macro_: Vec<MacroEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct MacroEntry {
    name: String,
    events: Vec<MacroEvent>,
}

fn macros_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join("macros.toml"))
}

fn load_saved() -> BTreeMap<String, Vec<MacroEvent>> {
    let Some(path) = macros_path() else {
        return BTreeMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    let parsed: MacrosFile = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return BTreeMap::new(),
    };
    parsed
        .macro_
        .into_iter()
        .map(|m| (m.name, m.events))
        .collect()
}

fn persist(saved: &BTreeMap<String, Vec<MacroEvent>>) {
    let Some(path) = macros_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = MacrosFile {
        macro_: saved
            .iter()
            .map(|(name, events)| MacroEntry {
                name: name.clone(),
                events: events.clone(),
            })
            .collect(),
    };
    if let Ok(serialized) = toml::to_string_pretty(&file) {
        let _ = std::fs::write(&path, serialized);
    }
}

static GLOBAL: OnceLock<Mutex<MacroRecorder>> = OnceLock::new();

fn global() -> &'static Mutex<MacroRecorder> {
    GLOBAL.get_or_init(|| {
        Mutex::new(MacroRecorder {
            recording: false,
            replaying: false,
            in_progress: Vec::new(),
            current: Vec::new(),
            saved: load_saved(),
        })
    })
}

/// Push `event` onto the in-progress recording if one is active. Silently
/// dropped during replay or while no recording is in flight. Consecutive
/// `InsertText` events are coalesced into one — typing "hello" arrives as
/// five separate single-char events from the OS input handler, but stores
/// as one `InsertText("hello")` so the recorded macro stays compact.
pub(crate) fn record(event: MacroEvent) {
    let Ok(mut g) = global().lock() else {
        return;
    };
    if !g.recording || g.replaying {
        return;
    }
    if let MacroEvent::InsertText(text) = &event {
        if let Some(MacroEvent::InsertText(prev)) = g.in_progress.last_mut() {
            prev.push_str(text);
            return;
        }
    }
    g.in_progress.push(event);
}

/// Set the replaying flag and return its previous value. Used by the replay
/// engine to suspend recording while it dispatches events; pair every call
/// with a restoring call using the returned previous value.
pub(crate) fn set_replaying(value: bool) -> bool {
    let Ok(mut g) = global().lock() else {
        return false;
    };
    let prev = g.replaying;
    g.replaying = value;
    prev
}

/// True iff a recording is currently in progress and not paused by a replay.
#[allow(dead_code)]
pub(crate) fn is_recording() -> bool {
    global().lock().map(|g| g.recording).unwrap_or(false)
}

/// Toggle the recording flag. Starting a fresh recording clears the previous
/// in-progress buffer; stopping flushes the in-progress buffer into `current`
/// so the next Playback runs the just-recorded macro.
pub(crate) fn toggle_recording() -> bool {
    let Ok(mut g) = global().lock() else {
        return false;
    };
    if g.recording {
        let captured = std::mem::take(&mut g.in_progress);
        g.current = captured;
        g.recording = false;
    } else {
        g.in_progress.clear();
        g.recording = true;
    }
    g.recording
}

/// Start a fresh recording. No-op if already recording (mirrors NPP's
/// separate Start Recording toolbar button, which is disabled while
/// recording is in flight but stays a safe call).
pub(crate) fn start_recording() -> bool {
    let Ok(mut g) = global().lock() else {
        return false;
    };
    if !g.recording {
        g.in_progress.clear();
        g.recording = true;
    }
    g.recording
}

/// Stop an in-progress recording and flush its captured events into
/// `current`. No-op if not recording.
pub(crate) fn stop_recording() -> bool {
    let Ok(mut g) = global().lock() else {
        return false;
    };
    if g.recording {
        let captured = std::mem::take(&mut g.in_progress);
        g.current = captured;
        g.recording = false;
    }
    g.recording
}

/// Snapshot of the most-recently-stopped macro for replay.
pub(crate) fn current() -> Vec<MacroEvent> {
    global().lock().map(|g| g.current.clone()).unwrap_or_default()
}

/// Snapshot of saved macro names + event counts for the menu builder.
pub(crate) fn saved_snapshot() -> Vec<(String, usize)> {
    global()
        .lock()
        .map(|g| {
            g.saved
                .iter()
                .map(|(n, ev)| (n.clone(), ev.len()))
                .collect()
        })
        .unwrap_or_default()
}

/// Look up a saved macro by name for replay.
pub(crate) fn saved_events(name: &str) -> Option<Vec<MacroEvent>> {
    global().lock().ok().and_then(|g| g.saved.get(name).cloned())
}

/// Persist the current in-memory macro under `name`. If `name` is already
/// taken the existing entry is overwritten.
pub(crate) fn save_macro(name: String) -> bool {
    let Ok(mut g) = global().lock() else {
        return false;
    };
    if g.current.is_empty() {
        return false;
    }
    let events = g.current.clone();
    g.saved.insert(name, events);
    persist(&g.saved);
    true
}

