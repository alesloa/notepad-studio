// NppCommands.cpp:4413–4415, 4464–4471, 4607 — Blank Operations record
// command identity and invoke the same command during playback.
use std::collections::BTreeMap;

use crate::{actions::*, persist::TempDir, workspace::Workspace};
use gpui::{Entity, TestAppContext, VisualTestContext, WindowAppearance};

#[derive(Clone, Copy)]
enum Command { Leading, Both, Eol, RemoveBlank, TabToSpace, AllTabs, LeadingTabs, Trailing }

impl Command {
    fn name(self) -> &'static str {
        match self {
            Self::Leading => "TrimLeadingWs",
            Self::Both => "TrimLeadingAndTrailingWs",
            Self::Eol => "EolToSpace",
            Self::RemoveBlank => "RemoveUnnecessaryBlankAndEol",
            Self::TabToSpace => "TabToSpace",
            Self::AllTabs => "SpaceToTabAll",
            Self::LeadingTabs => "SpaceToTabLeading",
            Self::Trailing => "TrimTrailingWs",
        }
    }

    fn sample(self) -> (&'static str, &'static str, bool) {
        match self {
            Self::Leading => ("\u{FEFF}keep \t\r\n \tcafé \t\r\ntail \t", "\u{FEFF}keep \t\r\ncafé \t\r\ntail \t", true),
            Self::Both => ("\u{FEFF}keep \t\r\n \tcafé \t\r\ntail \t", "\u{FEFF}keep \t\r\ncafé\r\ntail \t", true),
            Self::Eol => ("\u{FEFF}café\r\n中\rnext\n", "\u{FEFF}café 中 next ", false),
            Self::RemoveBlank => ("\u{FEFF}café  \r\n\t\r\n  中  \rnext \n", "\u{FEFF}café 中 next ", false),
            Self::TabToSpace => ("\u{FEFF}\tcafé\r\n\tx\t\rnext\t", "\u{FEFF}   café\r\n    x   \rnext    ", false),
            Self::AllTabs => ("    café    中\r\n    x    y\rnext    \u{FEFF}", "\tcafé\t中\r\n\tx\t y\rnext\t\u{FEFF}", false),
            Self::LeadingTabs => ("    café    中\r\n    x    y\rnext    \u{FEFF}", "\tcafé    中\r\n\tx    y\rnext    \u{FEFF}", false),
            Self::Trailing => ("\u{FEFF}keep  \r\n café \t\r\ntail ", "\u{FEFF}keep\r\n café\r\ntail", false),
        }
    }

    fn dispatch(self, visual: &mut VisualTestContext) {
        match self {
            Self::Leading => visual.dispatch_action(EditTrimLeadingWs),
            Self::Both => visual.dispatch_action(EditTrimLeadingAndTrailingWs),
            Self::Eol => visual.dispatch_action(EditEolToSpace),
            Self::RemoveBlank => visual.dispatch_action(EditRemoveUnnecessaryBlankAndEol),
            Self::TabToSpace => visual.dispatch_action(EditTabToSpace),
            Self::AllTabs => visual.dispatch_action(EditSpaceToTabAll),
            Self::LeadingTabs => visual.dispatch_action(EditSpaceToTabLeading),
            Self::Trailing => visual.dispatch_action(EditTrimTrailingWs),
        }
    }
}

fn fixture<'a>(
    cx: &'a mut TestAppContext,
    source: &str,
    anchor: usize,
    head: usize,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let source = source.to_owned();
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            1., WindowAppearance::Dark,
        );
        let tab = w.active_tab_mut();
        tab.buffer.rope = ropey::Rope::from_str(&source);
        tab.buffer.encoding = crate::text::encoding::Encoding::Utf16Le;
        tab.buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
        tab.selection.anchor = anchor;
        tab.selection.head = head;
        w.window_closed = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    (workspace, visual)
}

fn record_and_replay(command: Command) {
    let _settings = crate::settings::override_for_test(Default::default());
    let (source, expected, scoped) = command.sample();
    let (anchor, head) = if scoped {
        (source.find("tail").unwrap(), source.find("café").unwrap())
    } else { (0, 0) };
    let recorded = {
        let _recorder = super::override_execution_for_test(false);
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source, anchor, head);
        visual.dispatch_action(MacroStartRecording);
        command.dispatch(visual);
        visual.dispatch_action(MacroStopRecording);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), expected, "live {}", command.name());
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        });
        let recorded = super::current();
        assert_eq!(recorded.len(), 1, "{} must record one command", command.name());
        cx.quit();
        recorded
    };
    let name = format!("blank {} café", command.name());
    let dir = TempDir::new("blank-command-record-replay");
    let path = dir.join("macros.toml");
    let saved = BTreeMap::from([(name.clone(), recorded)]);
    assert!(super::persist_at(&path, &saved).is_ok());
    let serialized = std::fs::read_to_string(&path).unwrap();
    assert!(serialized.contains(&format!("\"{}\"", command.name())), "{serialized}");
    let loaded = super::load_saved_at(&path);
    assert_eq!(loaded.len(), 1);
    assert!(!dir.join("macros.toml.corrupt").exists());

    // Destroy the original editor and recorder before using only persisted data
    // in a fresh editor. Replay runs while recording to check recursion suppression.
    let _recorder = super::override_execution_for_test(true);
    let playback = super::override_playback_for_test(Vec::new(), loaded);
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, source, anchor, head);
    assert!(super::current().is_empty());
    visual.dispatch_action(MacroRunSaved { name });
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), expected, "cold {}", command.name());
        assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        assert!(w.tabs[w.active].edit_transaction.is_none());
        assert_eq!(w.buffer().encoding, crate::text::encoding::Encoding::Utf16Le);
        assert_eq!(w.buffer().line_ending, crate::text::line_ending::LineEnding::Crlf);
    });
    visual.dispatch_action(EditUndo);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), source);
        assert_eq!((w.selection().anchor, w.selection().head), (anchor, head));
        assert!(w.tabs[w.active].undo_stack.is_empty());
        assert_eq!(w.tabs[w.active].redo_stack.len(), 1);
        assert_eq!(w.buffer().encoding, crate::text::encoding::Encoding::Utf16Le);
        assert_eq!(w.buffer().line_ending, crate::text::line_ending::LineEnding::Crlf);
    });
    drop(playback);
    super::stop_recording();
    assert!(super::current().is_empty(), "playback must not record itself");
    cx.quit();
}

#[test]
fn registered_blank_leading_record_persist_cold_replay_and_undo() { record_and_replay(Command::Leading); }
#[test]
fn registered_blank_both_record_persist_cold_replay_and_undo() { record_and_replay(Command::Both); }
#[test]
fn registered_blank_eol_record_persist_cold_replay_and_undo() { record_and_replay(Command::Eol); }
#[test]
fn registered_blank_remove_record_persist_cold_replay_and_undo() { record_and_replay(Command::RemoveBlank); }
#[test]
fn registered_blank_tab_expand_record_persist_cold_replay_and_undo() { record_and_replay(Command::TabToSpace); }
#[test]
fn registered_blank_all_tabs_record_persist_cold_replay_and_undo() { record_and_replay(Command::AllTabs); }
#[test]
fn registered_blank_leading_tabs_record_persist_cold_replay_and_undo() { record_and_replay(Command::LeadingTabs); }
#[test]
fn registered_blank_existing_trailing_record_persist_cold_replay_and_undo() { record_and_replay(Command::Trailing); }

#[test]
fn blank_command_names_round_trip_with_existing_macro_events() {
    use super::MacroEvent::*;
    let dir = TempDir::new("blank-macro-names");
    let path = dir.join("macros.toml");
    let legacy = "[[macro_]]\nname = \"existing\"\nevents = [\"TrimTrailingWs\", { InsertText = \"café\\r\\n\" }, \"MoveRight\"]\n";
    std::fs::write(&path, legacy).unwrap();
    let mut saved = super::load_saved_at(&path);
    assert!(matches!(saved.get("existing").map(Vec::as_slice), Some([
        TrimTrailingWs, InsertText(text), MoveRight,
    ]) if text == "café\r\n"));
    saved.insert("blank".into(), vec![TrimLeadingWs, TrimLeadingAndTrailingWs,
        EolToSpace, RemoveUnnecessaryBlankAndEol, TabToSpace, SpaceToTabAll, SpaceToTabLeading]);
    assert!(super::persist_at(&path, &saved).is_ok());
    let loaded = super::load_saved_at(&path);
    assert!(matches!(loaded.get("blank").map(Vec::as_slice), Some([
        TrimLeadingWs, TrimLeadingAndTrailingWs, EolToSpace,
        RemoveUnnecessaryBlankAndEol, TabToSpace, SpaceToTabAll, SpaceToTabLeading,
    ])));
    assert!(matches!(loaded.get("existing").map(Vec::as_slice), Some([
        TrimTrailingWs, InsertText(text), MoveRight,
    ]) if text == "café\r\n"));
    assert!(!dir.join("macros.toml.corrupt").exists());
}

#[test]
fn blank_noop_commands_record_without_creating_undo_entries() {
    let _settings = crate::settings::override_for_test(Default::default());
    for command in [Command::Leading, Command::Both, Command::Eol,
        Command::RemoveBlank, Command::TabToSpace, Command::AllTabs, Command::LeadingTabs] {
        let _recorder = super::override_execution_for_test(true);
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "café", 0, 0);
        command.dispatch(visual);
        super::stop_recording();
        assert_eq!(super::current().len(), 1, "noop {}", command.name());
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "café");
            assert_eq!((w.selection().anchor, w.selection().head), (0, 0));
            assert!(w.tabs[w.active].undo_stack.is_empty());
        });
        cx.quit();
    }
}

#[test]
fn blank_conversion_replay_reads_current_tab_width() {
    for command in [Command::TabToSpace, Command::AllTabs, Command::LeadingTabs] {
        let (source, expected) = if matches!(command, Command::TabToSpace) {
            ("\tcafé", "  café")
        } else { ("    café", "\t\tcafé") };
        let recorded = {
            let settings = crate::settings::Settings { tab_width: 4, ..Default::default() };
            let _settings = crate::settings::override_for_test(settings);
            let _recorder = super::override_execution_for_test(true);
            let mut cx = TestAppContext::single();
            let (_, visual) = fixture(&mut cx, source, 0, 0);
            command.dispatch(visual);
            super::stop_recording();
            let recorded = super::current();
            assert_eq!(recorded.len(), 1);
            cx.quit();
            recorded
        };
        let settings = crate::settings::Settings { tab_width: 2, ..Default::default() };
        let _settings = crate::settings::override_for_test(settings);
        let _recorder = super::override_execution_for_test(false);
        let _playback = super::override_playback_for_test(recorded, BTreeMap::new());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source, 0, 0);
        visual.dispatch_action(MacroPlayback);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
        visual.dispatch_action(EditUndo);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
        cx.quit();
    }
}

#[test]
fn registered_blank_command_sequence_cold_replay_is_one_undo() {
    let _settings = crate::settings::override_for_test(Default::default());
    let source = "\t café  \r\n\t中 \t\r\n";
    let recorded = {
        let _recorder = super::override_execution_for_test(false);
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source, 0, 0);
        visual.dispatch_action(MacroStartRecording);
        for command in [Command::Leading, Command::Both, Command::Eol,
            Command::RemoveBlank, Command::TabToSpace, Command::AllTabs, Command::LeadingTabs] {
            command.dispatch(visual);
        }
        visual.dispatch_action(MacroStopRecording);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "café 中"));
        let recorded = super::current();
        assert_eq!(recorded.len(), 7);
        cx.quit();
        recorded
    };
    let dir = TempDir::new("blank-macro-sequence");
    let path = dir.join("macros.toml");
    let name = "blank sequence".to_owned();
    assert!(super::persist_at(&path, &BTreeMap::from([(name.clone(), recorded)])).is_ok());
    let _recorder = super::override_execution_for_test(false);
    let _playback = super::override_playback_for_test(Vec::new(), super::load_saved_at(&path));
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, source, 0, 0);
    visual.dispatch_action(MacroRunSaved { name });
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), "café 中");
        assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        assert!(w.tabs[w.active].edit_transaction.is_none());
    });
    visual.dispatch_action(EditUndo);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), source);
        assert_eq!((w.selection().anchor, w.selection().head), (0, 0));
        assert!(w.tabs[w.active].undo_stack.is_empty());
    });
    cx.quit();
}
