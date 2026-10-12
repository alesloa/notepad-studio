use super::*;
use crate::macros;
use crate::{actions::*, tab::Tab, buffer::BlockSelection};
use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance, Action};

fn source(text: &str, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _macros = macros::override_playback_for_test(Vec::new(), Default::default());
    let _execution = macros::override_execution_for_test(false);
    let _settings = crate::settings::override_for_test(Default::default());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty(); tab.buffer.rope = ropey::Rope::from_str(text);
        tab.buffer.mark_saved(); tab.selection.collapse_to(1);
        w.tabs = vec![tab]; w.active = 0; w.word_wrap = false;
        let (sink, _) = std::sync::mpsc::channel(); w.session_snapshot_sink = Some(sink);
        w
    });
    visual.update(|window, cx| {
        crate::app::menus::install_global_actions(cx);
        window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    test(&workspace, visual); cx.quit();
}

fn hold(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, action: Box<dyn Action>) -> futures::channel::oneshot::Sender<()> {
    let (send, receive) = futures::channel::oneshot::channel();
    visual.update_window_entity(workspace, |w, _, _| {
        w.cancel_brace_navigation(); w.brace_navigation.cache = None;
        w.brace_navigation.publish_gate = Some(receive);
    });
    visual.update(|window, cx| window.dispatch_action(action, cx));
    visual.run_until_parked();
    visual.update(|_, cx| assert!(workspace.read(cx).brace_navigation_pending()));
    send
}

#[test]
fn registered_brace_actions_navigate_readonly_unicode_and_include_both_braces() {
    source("(é)", |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].buffer.read_only = true);
        visual.dispatch_action(SearchGoToMatchingBrace); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!((w.selection().anchor, w.selection().head), (3,3));
        });
        visual.dispatch_action(SearchSelectInBetween); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!((w.selection().anchor, w.selection().head), (0,4));
            assert_eq!(w.buffer().rope.to_string(), "(é)");
            assert!(w.tabs[0].undo_stack.is_empty() && w.tabs[0].redo_stack.is_empty());
            assert!(!w.buffer().dirty && w.buffer().read_only && !w.window_closed);
        });
    });
}

#[test]
fn selected_and_rectangular_caret_navigation_preserves_bytes_and_format() {
    for encoding in [crate::text::encoding::Encoding::Utf8, crate::text::encoding::Encoding::Utf8Bom,
        crate::text::encoding::Encoding::Utf16Le, crate::text::encoding::Encoding::Utf16Be,
        crate::text::encoding::Encoding::Latin1] {
        for eol in [crate::text::line_ending::LineEnding::Lf, crate::text::line_ending::LineEnding::Crlf, crate::text::line_ending::LineEnding::Cr] {
            source("(é)\n", |workspace, visual| {
                visual.update_window_entity(workspace, |w, _, _| {
                    let tab = &mut w.tabs[0]; tab.buffer.encoding = encoding; tab.buffer.line_ending = eol; tab.buffer.mark_saved();
                    tab.selection.anchor = 3; tab.selection.head = 1;
                    tab.block_selection = Some(BlockSelection { anchor_line: 0, head_line: 0, anchor_col: 3, head_col: 1, is_dragging: false });
                    tab.desired_col_x = Some(gpui::px(10.));
                });
                visual.dispatch_action(SearchSelectInBetween); visual.run_until_parked();
                visual.update(|_, cx| {
                    let w = workspace.read(cx); let tab = &w.tabs[0];
                    assert_eq!((tab.selection.anchor,tab.selection.head), (0,4));
                    assert!(tab.block_selection.is_none() && tab.desired_col_x.is_none());
                    assert_eq!(tab.buffer.encoding, encoding); assert_eq!(tab.buffer.line_ending,eol);
                    assert_eq!(tab.buffer.rope.to_string(), "(é)\n");
                    assert!(tab.undo_stack.is_empty() && tab.redo_stack.is_empty() && !tab.buffer.dirty);
                });
            });
        }
    }
}

#[test]
fn held_brace_result_rejects_source_caret_style_focus_and_tab_incarnation_changes() {
    for change in 0..7 {
        source("(é)", |workspace, visual| {
            let send = hold(workspace, visual, SearchGoToMatchingBrace.boxed_clone());
            visual.update_window_entity(workspace, |w, window, cx| match change {
                0 => { w.tabs[0].selection.collapse_to(0); w.sync_source_input_caret(); w.tabs[0].selection.collapse_to(1); w.sync_source_input_caret(); },
                1 => w.tabs[0].buffer.highlights = Arc::new(vec![HighlightSpan { range: 0..4, kind: crate::syntax::HighlightKind::String }]),
                2 => { w.tabs[0].instance = Arc::new(()); },
                3 => { window.focus(&w.goto_focus.clone(), cx); window.focus(&w.focus_handle.clone(), cx); },
                4 => { w.tabs[0].content_activity = Default::default(); },
                5 => { w.tabs[0].buffer.language = Some(LanguageId::Rust); },
                _ => { w.tabs[0].buffer.rope = ropey::Rope::from_str("(é)"); },
            });
            let _ = send.send(()); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!((w.selection().anchor,w.selection().head), (1,1));
                assert_eq!(w.buffer().rope.to_string(),"(é)"); assert!(w.tabs[0].undo_stack.is_empty());
            });
        });
    }
}

#[test]
fn escape_cancels_held_navigation_without_selection_or_clipboard_effects() {
    source("(é)", |workspace, visual| {
        visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into())));
        let send = hold(workspace, visual, SearchSelectInBetween.boxed_clone());
        visual.dispatch_action(EditFindClose); let _ = send.send(()); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert!(!w.brace_navigation_pending());
            assert_eq!((w.selection().anchor,w.selection().head),(1,1));
            assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()),Some("sentinel".into()));
        });
    });
}

#[test]
fn saved_brace_macro_resumes_before_its_edit_tail_and_undo_restores_all_text() {
    #[derive(serde::Serialize,serde::Deserialize)] struct Saved { events: Vec<MacroEvent> }
    let encoded = toml::to_string(&Saved { events: vec![MacroEvent::SelectInBetween, MacroEvent::InsertText("中".into())] }).unwrap();
    let events = toml::from_str::<Saved>(&encoded).unwrap().events;
    source("(é)", |workspace, visual| {
        visual.update_window_entity(workspace, |w, window, cx| { w.replay_macro(&events,1,window,cx); });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(),"中");
            assert!(w.regex_replacement.macro_playback.is_none()); assert_eq!(w.tabs[0].undo_stack.len(),1);
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(),"(é)"));
        visual.dispatch_action(EditRedo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(),"中"));
    });
}

#[test]
fn brace_recording_and_readonly_macro_do_not_create_undo_or_recursive_events() {
    source("(é)", |workspace, visual| {
        assert!(macros::start_recording());
        visual.dispatch_action(SearchGoToMatchingBrace); visual.run_until_parked();
        visual.dispatch_action(SearchSelectInBetween); visual.run_until_parked();
        assert!(!macros::stop_recording());
        let events = macros::recorded_events_for_test().unwrap();
        assert!(matches!(events.as_slice(), [MacroEvent::GoToMatchingBrace, MacroEvent::SelectInBetween]));
        visual.update_window_entity(workspace, |w, window, cx| {
            w.tabs[0].buffer.read_only = true; w.tabs[0].selection.collapse_to(1);
            w.replay_macro(&events,1,window,cx);
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!((w.selection().anchor,w.selection().head),(0,4));
            assert!(w.tabs[0].undo_stack.is_empty() && !w.buffer().dirty);
            assert!(w.regex_replacement.macro_playback.is_none());
        });
        assert!(matches!(macros::recorded_events_for_test().unwrap().as_slice(), [MacroEvent::GoToMatchingBrace, MacroEvent::SelectInBetween]));
    });
}

#[test]
fn manual_brace_action_cannot_take_a_waiting_macro_worker() {
    source("(é)", |workspace, visual| {
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(workspace, |w, window, cx| {
            w.cancel_brace_navigation(); w.brace_navigation.cache = None;
            w.brace_navigation.publish_gate = Some(receive);
            w.replay_macro(&[MacroEvent::SelectInBetween, MacroEvent::InsertText("tail".into())], 1, window, cx);
        });
        visual.run_until_parked();
        let (id, started) = visual.update(|_, cx| {
            let w = workspace.read(cx);
            (w.brace_navigation.pending.as_ref().unwrap().id, w.brace_navigation.workers_started)
        });
        visual.dispatch_action(SearchGoToMatchingBrace); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.brace_navigation.pending.as_ref().unwrap().id, id);
            assert_eq!(w.brace_navigation.workers_started, started);
            assert_eq!(w.buffer().rope.to_string(), "(é)");
            assert_eq!((w.selection().anchor,w.selection().head), (1,1));
        });
        send.send(()).unwrap(); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "tail");
            assert!(w.regex_replacement.macro_playback.is_none());
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
        });
    });
}

#[test]
fn brace_macro_prefix_and_tail_share_one_undo_without_merging_earlier_work() {
    source("(é)", |workspace, visual| {
        visual.update_window_entity(workspace, |w, window, cx| {
            assert!(w.replay_macro(&[MacroEvent::MoveDocEnd, MacroEvent::InsertText("!".into())], 1, window, cx));
            w.tabs[0].selection.collapse_to(1);
            w.replay_macro(&[MacroEvent::InsertText("x".into()), MacroEvent::MoveLeft,
                MacroEvent::GoToMatchingBrace, MacroEvent::InsertText("尾".into())], 1, window, cx);
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "(xé尾)!");
            assert_eq!(w.tabs[0].undo_stack.len(), 2); assert!(w.regex_replacement.macro_playback.is_none());
        });
        for expected in ["(é)!", "(é)"] {
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
        }
        for expected in ["(é)!", "(xé尾)!"] {
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
        }
    });
}

#[test]
fn cancelled_or_stale_brace_macro_keeps_its_completed_prefix_and_stops_the_tail() {
    for change in 0..3 {
        source("(é)", |workspace, visual| {
            let (send, receive) = futures::channel::oneshot::channel();
            visual.update_window_entity(workspace, |w, window, cx| {
                w.cancel_brace_navigation(); w.brace_navigation.cache = None;
                w.brace_navigation.publish_gate = Some(receive);
                w.replay_macro(&[MacroEvent::InsertText("x".into()), MacroEvent::MoveLeft,
                    MacroEvent::GoToMatchingBrace, MacroEvent::InsertText("tail".into())], 1, window, cx);
            });
            visual.run_until_parked();
            visual.update(|_, cx| assert!(workspace.read(cx).brace_navigation_pending()));
            match change {
                0 => visual.dispatch_action(EditFindClose),
                1 => visual.update_window_entity(workspace, |w, window, cx| {
                    window.focus(&w.goto_focus, cx); window.focus(&w.focus_handle, cx);
                }),
                _ => visual.update_window_entity(workspace, |w, _, _| {
                    w.tabs[0].buffer.highlights = Arc::new(vec![HighlightSpan { range: 0..5, kind: crate::syntax::HighlightKind::String }]);
                }),
            }
            let _ = send.send(()); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "(xé)");
                assert!(w.regex_replacement.macro_playback.is_none());
                assert!(w.tabs[0].edit_transaction.is_none()); assert_eq!(w.tabs[0].undo_stack.len(), 1);
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "(é)"));
        });
    }
}

#[test]
fn paint_worker_can_be_promoted_to_navigation_without_starting_another_scan() {
    source("(é)", |workspace, visual| {
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(workspace, |w, window, cx| {
            w.cancel_brace_navigation(); w.brace_navigation.cache = None;
            w.brace_navigation.publish_gate = Some(receive); w.refresh_brace_match(window, cx);
        });
        visual.run_until_parked();
        let (id, started) = visual.update(|_, cx| {
            let w = workspace.read(cx); assert!(!w.brace_navigation_pending());
            (w.brace_navigation.pending.as_ref().unwrap().id, w.brace_navigation.workers_started)
        });
        visual.dispatch_action(SearchSelectInBetween); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert!(w.brace_navigation_pending());
            assert_eq!(w.brace_navigation.pending.as_ref().unwrap().id, id);
            assert_eq!(w.brace_navigation.workers_started, started);
        });
        send.send(()).unwrap(); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!((workspace.read(cx).selection().anchor,workspace.read(cx).selection().head), (0,4)));
    });
}

#[test]
fn refused_brace_contexts_preserve_document_selection_focus_and_recording() {
    for change in 0..8 {
        source("(é)", |workspace, visual| {
            let directory = crate::persist::TempDir::new("brace-preview-context");
            if change < 3 {
                let (name, bytes) = match change {
                    0 => ("fixture.md", "(é)"),
                    1 => ("fixture.csv", "(é)"),
                    _ => ("fixture.rtf", "{\\rtf1\\ansi (\\u233?)}"),
                };
                let path = directory.join(name); std::fs::write(&path, bytes).unwrap();
                let buffer = crate::io::read_buffer(&path).unwrap();
                visual.update_window_entity(workspace, |w, window, cx| {
                    w.tabs[0] = Tab::with_buffer(buffer);
                    w.view_preview(&ViewPreview, window, cx); cx.notify();
                });
                visual.run_until_parked();
                if change == 1 {
                    visual.update_window_entity(workspace, |w, window, cx| {
                        let model = w.csv_models.get(&w.tabs[0].session_id).unwrap();
                        window.focus(model.grid_focus.as_ref().unwrap(), cx);
                    });
                    visual.run_until_parked();
                }
                visual.update(|_, cx| {
                    let w = workspace.read(cx);
                    assert_eq!(w.tabs[0].view_mode, match change {
                        0 => TabViewMode::MarkdownPreview, 1 => TabViewMode::CsvPreview, _ => TabViewMode::RichDocView,
                    });
                    if change == 2 { assert!(w.buffer().rich_doc.is_some()); }
                });
            }
            visual.update_window_entity(workspace, |w, window, cx| match change {
                3 => w.tabs[0].buffer.loading = Some(crate::buffer::LoadProgress {
                    bytes_read: 0, total_bytes: 4, cancel: Default::default() }),
                4 => w.begin_edit_group(),
                5 => { w.goto_state.open = true; window.focus(&w.goto_focus, cx); },
                _ => {},
            });
            if change == 6 { visual.deactivate_window(); }
            if change == 7 {
                visual.update(|window, cx| {
                    use gpui::{ElementInputHandler, InputHandler};
                    let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
                    handler.replace_and_mark_text_in_range(None, "日", Some(1..1), window, cx);
                    assert!(workspace.read(cx).is_composing_text(window));
                });
            }
            let before = visual.update(|window, cx| {
                let w = workspace.read(cx);
                (w.buffer().rope.to_string(), w.selection(), window.input_context_token(), w.tabs[0].undo_stack.len())
            });
            assert!(macros::start_recording());
            visual.dispatch_action(SearchSelectInBetween); visual.run_until_parked();
            assert!(!macros::stop_recording()); assert!(macros::recorded_events_for_test().unwrap().is_empty());
            visual.update(|window, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), before.0);
                assert_eq!((w.selection().anchor,w.selection().head),(before.1.anchor,before.1.head));
                assert!(window.input_context_token() == before.2); assert_eq!(w.tabs[0].undo_stack.len(),before.3);
                assert!(!w.brace_navigation_pending());
                if change == 7 { assert!(w.is_composing_text(window)); }
            });
        });
    }
}

#[test]
fn accepted_brace_misses_preserve_stream_and_rectangle_geometry() {
    for text in ["(unmatched", "xé", ")(x)"] {
        source(text, |workspace, visual| {
            let block = BlockSelection { anchor_line: 9, head_line: 2, anchor_col: 7, head_col: 3, is_dragging: true };
            visual.update_window_entity(workspace, |w, _, _| {
                w.tabs[0].selection.anchor = text.len(); w.tabs[0].selection.head = 1;
                w.tabs[0].block_selection = Some(block); w.tabs[0].desired_col_x = Some(gpui::px(13.));
            });
            for select in [false, true] {
                if select { visual.dispatch_action(SearchSelectInBetween); } else { visual.dispatch_action(SearchGoToMatchingBrace); }
                visual.run_until_parked();
                visual.update(|_, cx| {
                    let w = workspace.read(cx); assert_eq!((w.selection().anchor,w.selection().head),(text.len(),1));
                    let actual = w.tabs[0].block_selection.unwrap();
                    assert_eq!((actual.anchor_line,actual.head_line,actual.anchor_col,actual.head_col,actual.is_dragging),(9,2,7,3,true));
                    assert_eq!(w.tabs[0].desired_col_x,Some(gpui::px(13.))); assert_eq!(w.buffer().rope.to_string(),text);
                    assert!(w.tabs[0].undo_stack.is_empty() && !w.buffer().dirty);
                });
            }
        });
    }
}

#[test]
fn unchanged_large_paint_reads_cached_pairs_and_misses_without_foreground_scans() {
    for matched in [false, true] {
        let text = format!("({}{}","x\n".repeat(6*1024*1024), if matched { ")" } else { "" });
        source(&text, |workspace, visual| {
            visual.run_until_parked();
            let started = visual.update(|_, cx| workspace.read(cx).brace_navigation.workers_started);
            super::super::brace_match::tests::take_visited();
            for _ in 0..2 {
                visual.draw(gpui::point(gpui::px(80.),gpui::px(110.)),gpui::size(gpui::px(400.),gpui::px(150.)), |_,_| {
                    use gpui::IntoElement;
                    crate::editor_element::EditorElement { workspace: workspace.clone() }.into_any_element()
                });
                assert_eq!(super::super::brace_match::tests::take_visited(),0);
            }
            visual.update(|window,cx| {
                let w = workspace.read(cx); assert_eq!(w.brace_navigation.workers_started,started);
                assert_eq!(w.cached_brace_pair(window),matched.then_some((0,text.len()-1)));
                assert!(w.tabs[0].undo_stack.is_empty());
            });
        });
    }
}
