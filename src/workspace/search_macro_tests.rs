// PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp:4328-4389 records search options.
use super::*;
use crate::{actions::*, settings::Settings, macros::{SearchMacro, saved_round_trip_for_test},
    buffer::Selection, tab::Tab, text::{encoding::Encoding, line_ending::LineEnding}};
use gpui::{Entity, EntityInputHandler, TestAppContext, VisualTestContext};
use std::collections::BTreeMap;

fn fixture<'a>(cx: &'a mut TestAppContext, text: &str) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.active_tab_mut().buffer.rope = ropey::Rope::from_str(text);
        w.active_tab_mut().selection.collapse_to(0);
        w.active_tab_mut().buffer.mark_saved();
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w
    });
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        window.activate_window(); window.focus(&w.focus_handle, cx);
    }));
    visual.run_until_parked();
    (entity, visual)
}

fn search(query: &str, replacement: &str, regex: bool) -> SearchMacro {
    SearchMacro { query: query.into(), replacement: replacement.into(),
        match_case: true, whole_word: false, regex, extended: false, dot_matches_newline: false,
        wrap_around: true, backward: false, in_selection: false, dont_move_to_next: false }
}

fn save_reload(events: Vec<MacroEvent>) -> BTreeMap<String, Vec<MacroEvent>> {
    let dir = crate::persist::TempDir::new("search-macro");
    saved_round_trip_for_test(&dir.path().join("macros.toml"),
        &BTreeMap::from([("search fixture".into(), events)])).unwrap()
}

fn capture(
    visual: &mut VisualTestContext, entity: &Entity<Workspace>,
    configure: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
    action: impl FnOnce(&mut VisualTestContext),
) -> Vec<MacroEvent> {
    visual.update(|window, cx| entity.update(cx, |w, cx| configure(w, window, cx)));
    visual.dispatch_action(MacroStartRecording);
    action(visual);
    visual.dispatch_action(MacroStopRecording);
    crate::macros::current()
}

#[test]
fn search_macro_real_handlers_record_search_and_replace_parameters() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _recording = crate::macros::override_execution_for_test(false);
    let mut cx = TestAppContext::single();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.active_tab_mut().buffer.rope = ropey::Rope::from_str("hit hit hit");
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = "hit".into();
        w.find_state.replacement = "é".into();
        w.refresh_matches();
        w
    });
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        window.activate_window(); window.focus(&w.focus_handle, cx);
        w.macro_start_recording(&MacroStartRecording, window, cx);
        w.find_next(&EditFindNext, window, cx);
        w.find_prev(&EditFindPrev, window, cx);
        w.replace_current_match(&EditReplaceCurrent, window, cx);
        w.replace_all_matches(&EditReplaceAll, window, cx);
        w.macro_stop_recording(&MacroStopRecording, window, cx);
        assert_eq!(crate::macros::current().len(), 4, "search handlers must capture parameters");
    }));
    cx.quit();
}

#[test]
fn search_macro_oversized_query_runs_live_but_refuses_recording() {
    for direction in 0..3 {
        let _settings = crate::settings::override_for_test(Settings::default());
        let _recording = crate::macros::override_execution_for_test(false);
        let mut cx = TestAppContext::single();
        let (entity, visual) = fixture(&mut cx, "hit hit");
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            w.find_state.query = "x".repeat(crate::text::regex_replacement::MAX_TEMPLATE_BYTES + 1);
            w.find_state.backward = true;
            w.macro_start_recording(&MacroStartRecording, window, cx);
            match direction {
                0 => w.find_next(&EditFindNext, window, cx),
                1 => w.find_prev(&EditFindPrev, window, cx),
                _ => w.find_dialog_step(true, cx),
            }
            assert_eq!(w.buffer().rope.to_string(), "hit hit");
            assert!(w.find_state.backward); assert_eq!(w.selection().head, 0);
            let status = w.status_message.clone();
            w.macro_stop_recording(&MacroStopRecording, window, cx);
            assert_eq!(crate::macros::current().len(), 0, "oversized search must not enter saved macros");
            assert!(status.as_deref().is_some_and(|s| s.starts_with("Search macro not recorded:")));
        }));
        cx.quit();
    }
}

#[test]
fn search_macro_oversized_replacement_runs_live_but_refuses_recording() {
    for all in [false, true] {
        for extra in [1, 0] {
            let _settings = crate::settings::override_for_test(Settings::default());
            let _recording = crate::macros::override_execution_for_test(false);
            let mut cx = TestAppContext::single();
            let (entity, visual) = fixture(&mut cx, "hit");
            visual.update(|window, cx| entity.update(cx, |w, cx| {
                let bytes = crate::text::regex_replacement::MAX_TEMPLATE_BYTES + extra;
                w.find_state.query = "hit".into(); w.find_state.replacement = "x".repeat(bytes);
                w.macro_start_recording(&MacroStartRecording, window, cx);
                if all { w.replace_all_matches(&EditReplaceAll, window, cx); }
                else { w.replace_current_match(&EditReplaceCurrent, window, cx); }
                assert_eq!(w.buffer().rope.len_bytes(), bytes);
                assert!(w.buffer().rope.chunks().all(|chunk| chunk.bytes().all(|b| b == b'x')));
                let status = w.status_message.clone();
                w.macro_stop_recording(&MacroStopRecording, window, cx);
                let captured = crate::macros::current();
                if extra == 0 {
                    assert_eq!(captured.len(), 1, "the recording limit is inclusive");
                    let options = match captured.first() {
                        Some(MacroEvent::ReplaceCurrentWithOptions(options))
                        | Some(MacroEvent::ReplaceAllWithOptions(options)) => options,
                        _ => panic!("expected parameterized replacement"),
                    };
                    assert_eq!(options.query, "hit"); assert_eq!(options.replacement.len(), bytes);
                } else {
                    assert_eq!(captured.len(), 0, "oversized replacement must not enter saved macros");
                    assert!(status.as_deref().is_some_and(|s| s.starts_with("Search macro not recorded:")));
                }
                w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), "hit");
            }));
            cx.quit();
        }
    }
}

#[test]
fn search_macro_saved_find_restores_options_and_explicit_direction_in_fresh_editor() {
    for backward in [false, true] {
        for previous in [false, true] {
            let _settings = crate::settings::override_for_test(Settings::default());
            let _recording = crate::macros::override_execution_for_test(false);
            let mut recorded = TestAppContext::single();
            let (entity, visual) = fixture(&mut recorded, "hit HIT hits hit hit");
            let events = capture(visual, &entity, |w, _, _| {
                w.find_state.query = "hit".into(); w.find_state.match_case = true;
                w.find_state.whole_word = true; w.find_state.backward = backward;
                w.find_state.wrap_around = false; w.refresh_matches();
                w.find_state.current = None; w.active_tab_mut().selection.collapse_to(12);
            }, |visual| {
                if previous { visual.dispatch_action(EditFindPrev); }
                else { visual.dispatch_action(EditFindNext); }
            });
            assert_eq!(events.len(), 1);
            let saved = save_reload(events);
            recorded.quit();
            let _saved = crate::macros::override_playback_for_test(Vec::new(), saved);
            let mut fresh = TestAppContext::single();
            let (entity, visual) = fixture(&mut fresh, "hit HIT hits hit hit");
            visual.update(|_, cx| entity.update(cx, |w, _| {
                w.find_state.query = "HIT".into(); w.find_state.regex = true;
                w.find_state.backward = !backward; w.find_state.wrap_around = true;
                w.active_tab_mut().selection.collapse_to(12);
            }));
            visual.dispatch_action(MacroRunSaved { name: "search fixture".into() });
            visual.update(|_, cx| entity.update(cx, |w, _| {
                assert_eq!(w.selection().ordered(), if previous { 0..3 } else { 13..16 });
                assert_eq!(w.find_state.query, "hit"); assert!(w.find_state.match_case);
                assert!(w.find_state.whole_word); assert!(!w.find_state.regex);
                assert!(!w.find_state.wrap_around); assert_eq!(w.find_state.backward, backward);
                assert!(w.active_tab_mut().undo_stack.is_empty());
            }));
            fresh.quit();
        }
    }
}

#[test]
fn search_macro_dialog_direction_records_shift_reversal_once() {
    for backward in [false, true] {
        for reverse in [false, true] {
            let _settings = crate::settings::override_for_test(Settings::default());
            let _recording = crate::macros::override_execution_for_test(true);
            let mut cx = TestAppContext::single();
            let (entity, visual) = fixture(&mut cx, "hit hit hit");
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.find_state.query = "hit".into(); w.find_state.backward = backward;
                w.active_tab_mut().selection.collapse_to(4);
                w.find_dialog_step(reverse, cx);
                crate::macros::stop_recording();
                let events = crate::macros::current();
                assert!(matches!(events.as_slice(), [MacroEvent::FindNextWithOptions(_)]) == (backward == reverse));
                assert!(matches!(events.as_slice(), [MacroEvent::FindPrevWithOptions(_)]) == (backward != reverse));
            }));
            cx.quit();
        }
    }
}

#[test]
fn search_macro_record_save_reload_replace_all_preserves_encoding_and_one_undo() {
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        for extended in [false, true] {
            let _settings = crate::settings::override_for_test(Settings::default());
            let _recording = crate::macros::override_execution_for_test(false);
            let original = "\u{feff}hit café\r\nhit 中 hit";
            let mut recorded = TestAppContext::single();
            let (entity, visual) = fixture(&mut recorded, original);
            let events = capture(visual, &entity, |w, _, _| {
                w.find_state.query = if extended { r"hit\x20" } else { "hit " }.into();
                w.find_state.replacement = if extended { r"中\t" } else { "中\t" }.into();
                w.find_state.match_case = true; w.find_state.extended = extended;
                w.find_state.whole_word = true; w.find_state.wrap_around = false;
            }, |visual| visual.dispatch_action(EditReplaceAll));
            assert!(matches!(events.as_slice(), [MacroEvent::ReplaceAllWithOptions(_)]));
            let saved = save_reload(events);
            recorded.quit();
            let _saved = crate::macros::override_playback_for_test(Vec::new(), saved);
            let mut fresh = TestAppContext::single();
            let (entity, visual) = fixture(&mut fresh, original);
            visual.update(|_, cx| entity.update(cx, |w, _| {
                w.active_tab_mut().buffer.encoding = encoding;
                w.active_tab_mut().buffer.line_ending = LineEnding::Crlf;
                w.active_tab_mut().buffer.mark_saved();
            }));
            visual.dispatch_action(MacroRunSaved { name: "search fixture".into() });
            visual.update(|window, cx| entity.update(cx, |w, cx| {
                assert_eq!(w.buffer().rope.to_string(), "\u{feff}中\tcafé\r\n中\t中 hit");
                assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
                w.undo(&EditUndo, window, cx);
                assert_eq!(w.buffer().rope.to_string(), original);
                assert!(!w.buffer().dirty); assert_eq!(w.buffer().encoding, encoding);
                assert_eq!(w.buffer().line_ending, LineEnding::Crlf);
                w.redo(&EditRedo, window, cx);
                assert_eq!(w.buffer().rope.to_string(), "\u{feff}中\tcafé\r\n中\t中 hit");
            }));
            fresh.quit();
        }
    }
}

#[test]
fn search_macro_record_save_reload_regex_capture_replacement_sequences_and_repetitions() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _recording = crate::macros::override_execution_for_test(false);
    let original = "é-1 中-2\r\n";
    let mut recorded = TestAppContext::single();
    let (entity, visual) = fixture(&mut recorded, original);
    let events = capture(visual, &entity, |w, _, _| {
        w.find_state.query = r"(\pL+)-(\d)".into();
        w.find_state.replacement = "$2:$1".into();
        w.find_state.regex = true; w.find_state.match_case = true;
    }, |visual| { visual.dispatch_action(EditReplaceCurrent); visual.run_until_parked(); });
    assert!(matches!(events.as_slice(), [MacroEvent::ReplaceCurrentWithOptions(_)]));
    let mut events = events;
    events.insert(0, MacroEvent::MoveDocStart);
    events.push(MacroEvent::ReplaceAllWithOptions(search(r"(\pL+)-(\d)", "$2:$1", true)));
    events.push(MacroEvent::MoveDocEnd);
    events.push(MacroEvent::InsertText("done\r\n".into()));
    let saved = save_reload(events);
    recorded.quit();
    let _saved = crate::macros::override_playback_for_test(Vec::new(), saved);
    let mut fresh = TestAppContext::single();
    let (entity, visual) = fixture(&mut fresh, original);
    visual.dispatch_action(MacroRunSaved { name: "search fixture".into() });
    visual.run_until_parked();
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        assert_eq!(w.buffer().rope.to_string(), "1:é 2:中\r\ndone\r\n");
        assert!(w.regex_replacement.macro_playback.is_none());
        assert!(w.active_tab_mut().edit_transaction.is_none());
        assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
        assert!(w.status_message.as_deref().is_some_and(|s| s.starts_with("Played macro")));
        w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), original);
        assert_eq!(w.selection().head, 0); assert!(!w.buffer().dirty);
    }));
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        let events = crate::macros::saved_events("search fixture").unwrap();
        w.replay_macro(&events, 3, window, cx);
    }));
    visual.run_until_parked();
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        assert_eq!(w.buffer().rope.to_string(), "1:é 2:中\r\ndone\r\ndone\r\ndone\r\n");
        assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
        w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), original);
    }));
    fresh.quit();
}

#[test]
fn search_macro_in_selection_retargets_playback_selection_without_recorded_offsets() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _recording = crate::macros::override_execution_for_test(false);
    let mut recorded = TestAppContext::single();
    let (entity, visual) = fixture(&mut recorded, "hit hit hit");
    let events = capture(visual, &entity, |w, _, _| {
        w.find_state.query = "hit".into(); w.find_state.replacement = "中".into();
        w.find_state.in_selection = true;
        w.find_state.in_selection_scope = Some(0..3);
    }, |visual| visual.dispatch_action(EditReplaceAll));
    let saved = save_reload(events);
    recorded.quit();
    let _saved = crate::macros::override_playback_for_test(Vec::new(), saved);
    let mut fresh = TestAppContext::single();
    let (entity, visual) = fixture(&mut fresh, "hit xx hit hit");
    visual.update(|_, cx| entity.update(cx, |w, _| {
        w.active_tab_mut().selection = Selection { anchor: 7, head: 10, is_dragging: false };
    }));
    visual.dispatch_action(MacroRunSaved { name: "search fixture".into() });
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        assert_eq!(w.buffer().rope.to_string(), "hit xx 中 hit");
        w.undo(&EditUndo, window, cx); assert_eq!(w.selection().ordered(), 7..10);
    }));
    fresh.quit();
}

#[test]
fn search_macro_legacy_names_load_and_refuse_before_any_prefix_edit() {
    for event in [MacroEvent::FindNext, MacroEvent::FindPrev, MacroEvent::ReplaceCurrent,
        MacroEvent::ReplaceAll, MacroEvent::Save] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let saved = save_reload(vec![MacroEvent::InsertText("bad".into()), event]);
        let _saved = crate::macros::override_playback_for_test(Vec::new(), saved);
        let mut cx = TestAppContext::single();
        let (entity, visual) = fixture(&mut cx, "hit");
        visual.dispatch_action(MacroRunSaved { name: "search fixture".into() });
        visual.update(|_, cx| entity.update(cx, |w, _| {
            assert_eq!(w.buffer().rope.to_string(), "hit");
            assert!(w.status_message.as_deref().is_some_and(|s| s.contains("does not support")));
            assert!(w.active_tab_mut().undo_stack.is_empty());
        }));
        cx.quit();
    }
}

#[test]
fn search_macro_cancelled_or_stale_worker_stops_before_suffix_and_keeps_user_history() {
    for case in 0..12 {
        let _settings = crate::settings::override_for_test(Settings::default());
        let _execution = crate::macros::override_execution_for_test(false);
        let mut cx = TestAppContext::single();
        let (entity, visual) = fixture(&mut cx, "hit hit");
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            w.hold_macro_replacement_for_test();
            assert!(!w.replay_macro(&[MacroEvent::InsertText("prefix ".into()),
                MacroEvent::ReplaceAllWithOptions(search("(hit)", "$1$1", true)),
                MacroEvent::InsertText("suffix".into())], 1, window, cx));
            assert_eq!(w.buffer().rope.to_string(), "prefix hit hit");
            assert!(w.active_tab_mut().edit_transaction.is_none());
            assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
            match case {
                0 => w.close_find(&EditFindClose, window, cx),
                1 => w.replace_byte_range(0..0, "user ", cx),
                2 => {
                    w.move_right(&MoveRight, window, cx); w.move_left(&MoveLeft, window, cx);
                }
                3 => w.active_tab_mut().buffer.read_only = true,
                4 => {
                    window.focus(&w.find_focus, cx); window.focus(&w.focus_handle, cx);
                }
                5 => {
                    let id = w.active_tab_mut().session_id;
                    let text = w.buffer().rope.to_string();
                    let rope = w.buffer().rope.clone();
                    let revision = w.buffer().raw_revision.clone();
                    let undo = w.active_tab_mut().undo_stack.clone();
                    let redo = w.active_tab_mut().redo_stack.clone();
                    let mut restored = Tab::empty(); restored.session_id = id;
                    restored.buffer.rope = rope; restored.buffer.raw_revision = revision;
                    restored.undo_stack = undo; restored.redo_stack = redo;
                    restored.selection.collapse_to("prefix ".len());
                    w.tabs[0] = restored;
                    assert_eq!(w.buffer().rope.to_string(), text);
                }
                6 => w.window_closed = true,
                7 => { w.undo(&EditUndo, window, cx); w.redo(&EditRedo, window, cx); }
                8 => { w.find_state.replacement = "different".into(); }
                9 => { w.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx); }
                10 => {
                    let mut tab = Tab::empty(); tab.buffer.rope = ropey::Rope::from_str("other");
                    w.tabs.push(tab); w.active = 1;
                }
                11 => { w.tabs.clear(); }
                _ => {}
            }
            let before_reply = w.tabs.get(w.active).map(|tab| tab.buffer.rope.to_string());
            w.finish_held_macro_replacement_for_test(window, cx);
            assert_eq!(w.tabs.get(w.active).map(|tab| tab.buffer.rope.to_string()), before_reply, "stale case {case}");
            assert!(w.regex_replacement.macro_playback.is_none(), "case {case}");
            assert!(w.tabs.iter().all(|tab| tab.edit_transaction.is_none()));
            if case == 1 {
                assert_eq!(w.active_tab_mut().undo_stack.len(), 2, "real input owns a separate Undo");
                w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), "prefix hit hit");
                w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), "hit hit");
            }
            // Real closure removes its window before an empty tab list can
            // render. Keep this guard-only fixture renderable after the reply.
            if w.tabs.is_empty() { w.tabs.push(Tab::empty()); w.active = 0; }
            w.window_closed = false;
        }));
        cx.quit();
    }
}

#[test]
fn search_macro_invalid_regex_or_template_preserves_prefix_with_one_undo() {
    for (query, template) in [("(", "a"), ("(hit)", r"\U$1\E")] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (entity, visual) = fixture(&mut cx, "hit");
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            w.replay_macro(&[MacroEvent::InsertText("prefix ".into()),
                MacroEvent::ReplaceAllWithOptions(search(query, template, true)),
                MacroEvent::InsertText("suffix".into())], 1, window, cx);
        }));
        visual.run_until_parked();
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            assert_eq!(w.buffer().rope.to_string(), "prefix hit");
            assert!(w.regex_replacement.macro_playback.is_none());
            assert!(w.status_message.as_deref().is_some_and(|s| s.starts_with("Macro stopped after 1 event:")));
            assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
            w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), "hit");
        }));
        cx.quit();
    }
}

#[test]
fn search_macro_nonsearch_playback_keeps_open_selection_scope() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (entity, visual) = fixture(&mut cx, "hit hit hit");
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        w.find_state.open = true; w.find_state.query = "hit".into();
        w.find_state.in_selection = true; w.find_state.in_selection_scope = Some(4..7);
        w.capture_find_scope_owner(); w.refresh_matches();
        assert!(w.replay_macro(&[MacroEvent::MoveDocStart, MacroEvent::InsertText("!".into())], 1, window, cx));
        assert!(w.find_state.in_selection); assert_eq!(w.find_state.in_selection_scope, Some(4..7));
        assert!(w.find_state.in_selection_scope_owner.is_some());
        w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), "hit hit hit");
    }));
    cx.quit();
}

#[test]
fn search_macro_recorded_current_replace_restores_normal_extended_and_regex_options() {
    for mode in 0..3 {
        let _settings = crate::settings::override_for_test(Settings {
            replace_dont_move_to_next: true, ..Default::default()
        });
        let _recording = crate::macros::override_execution_for_test(false);
        let original = "hit café\r\nhit 中";
        let mut recorded = TestAppContext::single();
        let (entity, visual) = fixture(&mut recorded, original);
        let events = capture(visual, &entity, |w, _, _| {
            w.find_state.query = match mode { 0 => "hit", 1 => r"h\x69t", _ => "(hit)" }.into();
            w.find_state.replacement = match mode { 0 => "中", 1 => r"中\t", _ => "$1中" }.into();
            w.find_state.match_case = true; w.find_state.extended = mode == 1;
            w.find_state.regex = mode == 2; w.find_state.dot_matches_newline = true;
            w.find_state.wrap_around = false; w.find_state.backward = true;
        }, |visual| { visual.dispatch_action(EditReplaceCurrent); visual.run_until_parked(); });
        assert!(matches!(events.as_slice(), [MacroEvent::ReplaceCurrentWithOptions(options)]
            if options.dont_move_to_next && options.dot_matches_newline && options.backward));
        let saved = save_reload(events); recorded.quit();
        let _fresh_settings = crate::settings::override_for_test(Settings::default());
        let _saved = crate::macros::override_playback_for_test(Vec::new(), saved);
        let mut fresh = TestAppContext::single();
        let (entity, visual) = fixture(&mut fresh, original);
        visual.dispatch_action(MacroRunSaved { name: "search fixture".into() });
        visual.run_until_parked();
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            let expected = match mode { 0 => "中 café\r\nhit 中", 1 => "中\t café\r\nhit 中", _ => "hit中 café\r\nhit 中" };
            assert_eq!(w.buffer().rope.to_string(), expected);
            assert_eq!(w.find_state.current, Some(0), "recorded Don't move option");
            assert!(!crate::settings::snapshot().replace_dont_move_to_next, "playback cannot rewrite preferences");
            assert!(w.find_state.dot_matches_newline); assert!(w.find_state.backward);
            assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
            w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), original);
        }));
        fresh.quit();
    }
}

#[test]
fn search_macro_recorded_regex_all_handles_dot_newline_named_groups_and_zero_length() {
    for (text, query, template, expected) in [
        ("hit\r\n中", "(?<part>hit.*中)", "$+{part}!", "hit\r\n中!"),
        ("é\r\n中", "^", ">", ">é\r\n>中"),
    ] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let _recording = crate::macros::override_execution_for_test(false);
        let mut recorded = TestAppContext::single();
        let (entity, visual) = fixture(&mut recorded, text);
        let events = capture(visual, &entity, |w, _, _| {
            w.find_state.query = query.into(); w.find_state.replacement = template.into();
            w.find_state.regex = true; w.find_state.dot_matches_newline = true;
        }, |visual| { visual.dispatch_action(EditReplaceAll); visual.run_until_parked(); });
        assert!(matches!(events.as_slice(), [MacroEvent::ReplaceAllWithOptions(_)]));
        let saved = save_reload(events); recorded.quit();
        let _saved = crate::macros::override_playback_for_test(Vec::new(), saved);
        let mut fresh = TestAppContext::single();
        let (entity, visual) = fixture(&mut fresh, text);
        visual.dispatch_action(MacroRunSaved { name: "search fixture".into() });
        visual.run_until_parked();
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            assert_eq!(w.buffer().rope.to_string(), expected);
            assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
            w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), text);
        }));
        fresh.quit();
    }
}

#[test]
fn search_macro_read_only_composition_and_rectangular_selection_refuse_without_changes() {
    for case in 0..3 {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (entity, visual) = fixture(&mut cx, "hit hit");
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            let mut options = search("(hit)", "中", true);
            match case {
                0 => w.active_tab_mut().buffer.read_only = true,
                1 => w.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx),
                2 => {
                    options.in_selection = true;
                    w.active_tab_mut().block_selection = Some(crate::buffer::BlockSelection {
                        anchor_line: 0, head_line: 0, anchor_col: 0, head_col: 3, is_dragging: false,
                    });
                }
                _ => {}
            }
            let before = w.buffer().rope.to_string(); let history = w.active_tab_mut().undo_stack.len();
            assert!(!w.replay_macro(&[MacroEvent::ReplaceAllWithOptions(options)], 1, window, cx));
            assert_eq!(w.buffer().rope.to_string(), before); assert_eq!(w.active_tab_mut().undo_stack.len(), history);
            assert!(w.regex_replacement.macro_playback.is_none());
        }));
        cx.quit();
    }
}

#[test]
fn search_macro_empty_in_selection_matches_live_source_fallback() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (entity, visual) = fixture(&mut cx, "hit hit");
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        w.find_state.query = "hit".into(); w.find_state.replacement = "中".into();
        w.find_state.in_selection = true;
        w.replace_all_matches(&EditReplaceAll, window, cx);
        let live = w.buffer().rope.to_string();
        w.undo(&EditUndo, window, cx);
        let mut options = search("hit", "中", false); options.in_selection = true;
        assert!(w.replay_macro(&[MacroEvent::ReplaceAllWithOptions(options)], 1, window, cx));
        assert_eq!(w.buffer().rope.to_string(), live); assert_eq!(live, "中 中");
        assert!(w.find_state.in_selection_scope.is_none());
    }));
    cx.quit();
}

#[test]
fn search_macro_failure_after_successful_background_edit_remains_one_undo() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (entity, visual) = fixture(&mut cx, "hit hit");
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        w.replay_macro(&[MacroEvent::InsertText("prefix ".into()),
            MacroEvent::ReplaceAllWithOptions(search("(hit)", "$1中", true)),
            MacroEvent::CompleteWord { prefix: "missing".into(), candidate: "missingWord".into() },
            MacroEvent::InsertText("suffix".into())], 1, window, cx);
    }));
    visual.run_until_parked();
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        assert_eq!(w.buffer().rope.to_string(), "prefix hit中 hit中");
        assert!(w.status_message.as_deref().is_some_and(|s| s.starts_with("Macro stopped after 2 events:")));
        assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
        w.undo(&EditUndo, window, cx); assert_eq!(w.buffer().rope.to_string(), "hit hit");
        assert_eq!(w.selection().head, 0); assert!(!w.buffer().dirty);
    }));
    cx.quit();
}

#[test]
#[ignore = "manual optimized synthetic source timing"]
fn search_macro_release_large_source_timing_and_cancellation() {
    let _settings = crate::settings::override_for_test(Settings {
        autocompletion_mode: crate::settings::AutocompletionMode::None, ..Default::default()
    });
    let text = format!("{}hit-1 hit-2\r\n", format!("{}\r\n", "x".repeat(78)).repeat(157_287));
    assert!(text.len() > 10 * 1024 * 1024);
    for direct in [true, false] {
        let mut cx = TestAppContext::single();
        let (entity, visual) = fixture(&mut cx, &text);
        let started = std::time::Instant::now();
        let mut dispatch = std::time::Duration::ZERO;
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            w.find_state.query = "(hit)-(\\d)".into(); w.find_state.replacement = "$2:$1".into();
            w.find_state.regex = true; w.find_state.match_case = true;
            let caller = std::time::Instant::now();
            if direct { w.replace_all_matches(&EditReplaceAll, window, cx); }
            else { w.replay_macro(&[MacroEvent::ReplaceAllWithOptions(search("(hit)-(\\d)", "$2:$1", true))], 1, window, cx); }
            dispatch = caller.elapsed();
            assert!(w.active_tab_mut().edit_transaction.is_none());
        }));
        visual.run_until_parked();
        let total = started.elapsed();
        visual.update(|_, cx| entity.update(cx, |w, _| {
            assert!(w.buffer().rope.to_string().ends_with("1:hit 2:hit\r\n"));
            assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
            eprintln!("source_bytes={} route={} caller_ms={:.3} capture_ms={:.3} apply_ms={:.3} wall_ms={:.3}",
                text.len(), if direct { "direct" } else { "macro" }, dispatch.as_secs_f64() * 1000.,
                w.regex_replacement.last_capture.unwrap().as_secs_f64() * 1000.,
                w.regex_replacement.last_apply.unwrap().as_secs_f64() * 1000., total.as_secs_f64() * 1000.);
        }));
        cx.quit();
    }
    let mut cx = TestAppContext::single();
    let (entity, visual) = fixture(&mut cx, &text);
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        w.replay_macro(&[MacroEvent::ReplaceAllWithOptions(search("(hit)-(\\d)", "$2:$1", true))], 1, window, cx);
        let cancel = std::time::Instant::now();
        w.close_find(&EditFindClose, window, cx);
        eprintln!("source_bytes={} cancel_ms={:.3} no_open_transaction={}", text.len(), cancel.elapsed().as_secs_f64() * 1000., w.active_tab_mut().edit_transaction.is_none());
        assert!(w.regex_replacement.macro_playback.is_none());
    }));
    visual.run_until_parked();
    visual.update(|_, cx| entity.update(cx, |w, _| {
        assert_eq!(w.buffer().rope.to_string(), text); assert!(w.active_tab_mut().undo_stack.is_empty());
    }));
    cx.quit();
}
