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
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::consts::APP_NAME;

#[cfg(test)]
#[path = "macros_playback_fixture.rs"]
mod playback_fixture;
#[cfg(test)]
pub(crate) use playback_fixture::override_for_test as override_playback_for_test;

/// Events captured during recording. Each variant maps onto a single
/// keyboard-driven editor action. Replay dispatches these by calling the
/// matching `Workspace` method directly (not via gpui action dispatch) so
/// re-entry into the recorder is bounded by the explicit `replaying` flag.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum MacroEvent {
    InsertText(String),
    /// Accepted completion replaces a prefix rather than inserting after it.
    /// Existing saved event names remain unchanged.
    CompleteWord { prefix: String, candidate: String },
    Backspace,
    Delete,
    InsertNewline,
    InsertTab,
    DuplicateLine,
    IncreaseLineIndent,
    DecreaseLineIndent,
    DuplicateCurrentLine,
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
    /// Scintilla editor keys (`edit_ops::editor_keys`): Ctrl+Backspace,
    /// Ctrl+Delete, Ctrl+Shift+Backspace / Delete, Ctrl+Up / Down, Insert.
    DeleteWordLeft,
    DeleteWordRight,
    DeleteLineLeft,
    DeleteLineRight,
    ScrollLineUp,
    ScrollLineDown,
    ToggleOvertype,
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

#[cfg(not(test))]
fn load_saved() -> BTreeMap<String, Vec<MacroEvent>> {
    let Some(path) = macros_path() else {
        return BTreeMap::new();
    };
    load_saved_at(&path)
}

/// Saved macros in `path`. A macro this build can't replay (an event name
/// it doesn't know, say from a newer version) is skipped whole instead of
/// losing every macro; the original file is kept as `macros.toml.corrupt`
/// before the next save rewrites it.
fn load_saved_at(path: &Path) -> BTreeMap<String, Vec<MacroEvent>> {
    let parsed = crate::persist::load_toml(
        path,
        |t| {
            let (macros, dropped) =
                crate::persist::salvage_array::<MacroEntry>(t.get("macro_"), &["name", "events"]);
            if dropped > 0 {
                eprintln!("[macros] skipped {dropped} macro(s) this version can't replay");
            }
            MacrosFile { macro_: macros }
        },
        MacrosFile::default,
    )
    .into_option()
    .unwrap_or_default();
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
    if let Err(err) = persist_at(&path, saved) {
        eprintln!("[macros] couldn't save {}: {err}", path.display());
    }
}

fn persist_at(path: &Path, saved: &BTreeMap<String, Vec<MacroEvent>>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
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
    let serialized = toml::to_string_pretty(&file).map_err(std::io::Error::other)?;
    crate::persist::atomic_write(path, serialized.as_bytes())
}

static GLOBAL: OnceLock<Mutex<MacroRecorder>> = OnceLock::new();

fn global() -> &'static Mutex<MacroRecorder> {
    GLOBAL.get_or_init(|| {
        #[cfg(test)]
        let saved = BTreeMap::new();
        #[cfg(not(test))]
        let saved = load_saved();
        Mutex::new(MacroRecorder {
            recording: false,
            replaying: false,
            in_progress: Vec::new(),
            current: Vec::new(),
            saved,
        })
    })
}

/// Push `event` onto the in-progress recording if one is active. Silently
/// dropped during replay or while no recording is in flight. Consecutive
/// `InsertText` events are coalesced into one — typing "hello" arrives as
/// five separate single-char events from the OS input handler, but stores
/// as one `InsertText("hello")` so the recorded macro stays compact.
pub(crate) fn record(event: MacroEvent) {
    let Ok(mut g) = global().lock() else { return; };
    g.record_event(event);
}

impl MacroRecorder {
    pub(crate) fn record_event(&mut self, event: MacroEvent) {
        if !self.recording || self.replaying { return; }
        // PowerEditor/src/WinControls/shortcut/shortcut.cpp:925-1054 excludes history commands.
        if matches!(event, MacroEvent::Undo | MacroEvent::Redo) { return; }
        if let MacroEvent::InsertText(text) = &event {
            if let Some(MacroEvent::InsertText(prev)) = self.in_progress.last_mut() {
                prev.push_str(text);
                return;
            }
        }
        self.in_progress.push(event);
    }

    #[cfg(test)]
    pub(crate) fn recording_for_test() -> Self { Self { recording: true, ..Self::default() } }

    #[cfg(test)]
    pub(crate) fn captured_events(&self) -> &[MacroEvent] { &self.in_progress }
}

pub(crate) fn completion_event(prefix: &str, candidate: &str) -> MacroEvent {
    MacroEvent::CompleteWord { prefix: prefix.to_string(), candidate: candidate.to_string() }
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
    #[cfg(test)]
    if let Some(events) = playback_fixture::current() {
        return events;
    }
    global().lock().map(|g| g.current.clone()).unwrap_or_default()
}

/// Snapshot of saved macro names + event counts for the menu builder.
pub(crate) fn saved_snapshot() -> Vec<(String, usize)> {
    #[cfg(test)]
    if let Some(saved) = playback_fixture::saved_snapshot() {
        return saved;
    }
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
    #[cfg(test)]
    if let Some(events) = playback_fixture::saved_events(name) {
        return events;
    }
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::TempDir;

    fn names(m: &BTreeMap<String, Vec<MacroEvent>>) -> Vec<&str> {
        m.keys().map(String::as_str).collect()
    }

    #[test]
    fn a_macro_with_an_unknown_event_does_not_lose_the_others() {
        let dir = TempDir::new("macros-load");
        let p = dir.join("macros.toml");
        let src = "[[macro_]]\nname = \"keep\"\nevents = [{ InsertText = \"hi\" }, \"Backspace\"]\n\
                   [[macro_]]\nname = \"newer\"\nevents = [\"Backspace\", \"Teleport\"]\n\
                   [[macro_]]\nevents = [\"Delete\"]\n";
        assert!(std::fs::write(&p, src).is_ok());
        let got = load_saved_at(&p);
        assert_eq!(names(&got), vec!["keep"]);
        assert!(matches!(
            got.get("keep").map(Vec::as_slice),
            Some([MacroEvent::InsertText(t), MacroEvent::Backspace]) if t == "hi"
        ));
        assert_eq!(
            std::fs::read_to_string(dir.join("macros.toml.corrupt")).ok().as_deref(),
            Some(src),
            "the skipped macros stay recoverable"
        );
    }

    #[test]
    fn saved_macros_round_trip() {
        let dir = TempDir::new("macros-save");
        let p = dir.join("macros.toml");
        let mut saved = BTreeMap::new();
        saved.insert(
            "crlf ñ".to_string(),
            vec![
                MacroEvent::InsertText("a\r\nb\u{1F600}\"\\".into()),
                MacroEvent::InsertDateTimeCustomized,
                MacroEvent::SelectWordLeft,
            ],
        );
        assert!(persist_at(&p, &saved).is_ok());
        let got = load_saved_at(&p);
        assert_eq!(names(&got), vec!["crlf ñ"]);
        assert!(matches!(
            got.get("crlf ñ").map(Vec::as_slice),
            Some([
                MacroEvent::InsertText(t),
                MacroEvent::InsertDateTimeCustomized,
                MacroEvent::SelectWordLeft,
            ]) if t == "a\r\nb\u{1F600}\"\\"
        ));
        assert!(!dir.join("macros.toml.tmp").exists());
    }

    #[test]
    fn completion_events_round_trip_alongside_existing_macro_names() {
        let dir = TempDir::new("macro-completion-save");
        let path = dir.join("macros.toml");
        let mut saved = BTreeMap::new();
        saved.insert("completion".into(), vec![
            MacroEvent::InsertText("café".into()), completion_event("café", "caféine"), MacroEvent::MoveRight,
        ]);
        assert!(persist_at(&path, &saved).is_ok());
        let loaded = load_saved_at(&path);
        assert!(matches!(loaded.get("completion").map(Vec::as_slice), Some([
            MacroEvent::InsertText(text), MacroEvent::CompleteWord { prefix, candidate }, MacroEvent::MoveRight,
        ]) if text == "café" && prefix == "café" && candidate == "caféine"));
        assert!(!dir.join("macros.toml.corrupt").exists());
    }

    #[test]
    fn unreadable_macro_file_is_kept_aside() {
        let dir = TempDir::new("macros-junk");
        let p = dir.join("macros.toml");
        assert!(load_saved_at(&p).is_empty());
        assert!(std::fs::write(&p, "[[macro_]\nname = ").is_ok());
        assert!(load_saved_at(&p).is_empty());
        assert!(dir.join("macros.toml.corrupt").exists());
    }
}
