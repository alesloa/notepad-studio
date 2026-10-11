// Saved legacy operations must not silently change the meaning of a macro.
use super::*;
use crate::buffer::serialize_undo_stack;
use crate::macros::{override_execution_for_test, override_playback_for_test};
use crate::tab::Tab;
use gpui::{TestAppContext, WindowAppearance};
use std::collections::BTreeMap;

fn run_macro(event: MacroEvent, label: &str, refuse: bool) {
    for route in 0..3 {
        let events = vec![MacroEvent::InsertText("before".into()), event.clone(),
            MacroEvent::InsertText("after".into())];
        let _recent = crate::recent::override_snapshot_for_test(Vec::new());
        let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
        let _execution = override_execution_for_test(true);
        let _playback = override_playback_for_test(events.clone(),
            BTreeMap::from([("legacy".into(), events.clone())]));
        let mut cx = TestAppContext::single();
        cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
        let (entity, visual) = cx.add_window_view(|_, cx| {
            let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
                cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
            let mut tab = Tab::empty();
            tab.buffer.rope = ropey::Rope::from_str("caf\u{e9}\n\u{4e2d}\u{1f600}");
            tab.buffer.encoding = crate::text::encoding::Encoding::Utf16Le;
            tab.buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
            tab.buffer.mark_saved();
            w.tabs = vec![tab]; w.active = 0;
            w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
            w.session_final_save_result = Some(true);
            w
        });
        visual.update(|window, cx| {
            window.activate_window(); window.focus(&entity.read(cx).focus_handle.clone(), cx);
        });
        visual.run_until_parked();
        // Seed real Undo/Redo through the painted Source input and action routes.
        let mut input = visual.update(|window, _| window.take_registered_input_handler().unwrap());
        visual.update(|window, cx| input.replace_text_in_range(None, "x", window, cx));
        visual.dispatch_action(EditUndo);
        let (rope, revision, undo, redo, selection) = visual.update(|_, cx| {
            let w = entity.read(cx); let tab = &w.tabs[0];
            assert!(!w.window_closed); assert_eq!(tab.redo_stack.len(), 1);
            (tab.buffer.rope.clone(), tab.buffer.raw_revision.clone(),
                serialize_undo_stack(&tab.undo_stack), serialize_undo_stack(&tab.redo_stack),
                (tab.selection.anchor, tab.selection.head))
        });
        if route == 0 { visual.dispatch_action(MacroPlayback); }
        else if route == 1 { visual.dispatch_action(MacroRunSaved { name: "legacy".into() }); }
        else { visual.update(|window, cx| entity.update(cx, |w, cx| {
            assert_eq!(w.replay_macro(&events, 3, window, cx), !refuse);
        })); }
        visual.update(|_, cx| {
            let w = entity.read(cx); let tab = &w.tabs[0];
            if !refuse {
                let times = if route == 2 { 3 } else { 1 };
                assert_eq!(tab.buffer.rope.to_string(), format!("{}{}", "beforemiddleafter".repeat(times), rope));
                assert_eq!(tab.undo_stack.len(), 1, "supported macro stays one Undo");
                assert!(tab.redo_stack.is_empty()); assert!(tab.buffer.dirty);
                return;
            }
            assert!(tab.buffer.rope.is_instance(&rope), "legacy {label} route {route} must refuse before any edit");
            assert_eq!(tab.buffer.raw_revision, revision);
            assert_eq!(serialize_undo_stack(&tab.undo_stack), undo);
            assert_eq!(serialize_undo_stack(&tab.redo_stack), redo);
            assert_eq!((tab.selection.anchor, tab.selection.head), selection);
            assert!(!tab.buffer.dirty);
            assert_eq!(tab.buffer.encoding, crate::text::encoding::Encoding::Utf16Le);
            assert_eq!(tab.buffer.line_ending, crate::text::line_ending::LineEnding::Crlf);
            assert!(w.status_message.as_deref().is_some_and(|s| s.contains(label) && s.contains("does not support")));
        });
        assert!(crate::macros::is_recording());
        assert!(matches!(crate::macros::current().as_slice(), [MacroEvent::InsertText(_), _, MacroEvent::InsertText(_)]));
        cx.quit();
    }
}

#[test]
fn legacy_find_next_macro_refuses_before_editing() { run_macro(MacroEvent::FindNext, "Find Next", true); }
#[test]
fn legacy_find_previous_macro_refuses_before_editing() { run_macro(MacroEvent::FindPrev, "Find Previous", true); }
#[test]
fn legacy_replace_current_macro_refuses_before_editing() { run_macro(MacroEvent::ReplaceCurrent, "Replace Current", true); }
#[test]
fn legacy_replace_all_macro_refuses_before_editing() { run_macro(MacroEvent::ReplaceAll, "Replace All", true); }
#[test]
fn legacy_save_macro_refuses_before_editing() { run_macro(MacroEvent::Save, "Save", true); }

#[test]
fn legacy_macro_supported_routes_keep_grouping_and_repetitions() {
    run_macro(MacroEvent::InsertText("middle".into()), "supported", false);
}

#[test]
fn legacy_macro_event_names_remain_loadable() {
    #[derive(serde::Deserialize)] struct Stored { events: Vec<MacroEvent> }
    let stored: Stored = toml::from_str("events = ['FindNext', 'FindPrev', 'ReplaceCurrent', 'ReplaceAll', 'Save']").unwrap();
    assert_eq!(stored.events.len(), 5);
    assert!(matches!(stored.events.as_slice(), [MacroEvent::FindNext, MacroEvent::FindPrev,
        MacroEvent::ReplaceCurrent, MacroEvent::ReplaceAll, MacroEvent::Save]));
}
