// Real Workspace action routing from one painted Source frame. Synthetic state only.
use super::*;
use crate::actions::{EditReplaceAll, EditReplaceCurrent, MoveLeft, MoveRight};
use crate::{settings::Settings, tab::Tab};
use gpui::{Entity, TestAppContext, VisualTestContext};

fn with_source(test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default()
    });
    let mut cx = TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            1.,
            gpui::WindowAppearance::Dark,
        );
        w.tabs = ["first 中😀\n", "second café\n"]
            .into_iter()
            .map(|text| {
                let mut tab = Tab::empty();
                tab.buffer.rope = ropey::Rope::from_str(text);
                tab
            })
            .collect();
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = "(first)".into();
        w.find_state.replacement = "$1$1".into();
        w.find_state.regex = true;
        w.find_state.match_case = true;
        w.regex_replacement.hold_worker = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    test(&entity, visual);
    cx.quit();
}

fn invoke(visual: &mut VisualTestContext, mode: Mode) {
    if mode == Mode::Current {
        visual.dispatch_action(EditReplaceCurrent);
    } else {
        visual.dispatch_action(EditReplaceAll);
    }
}

fn source_caret_worker(mode: Mode, movement: u8) {
    with_source(|entity, visual| {
        invoke(visual, mode);
        let (request, job, epoch) = visual.update(|_, cx| {
            entity.update(cx, |w, _| {
                assert!(!w.window_closed, "exercise a genuinely open editor");
                let (request, job) = w.regex_replacement.held.take().unwrap();
                assert!(request.mode == mode);
                (request, job, w.source_input_token())
            })
        });
        if movement == 1 {
            // Registered handlers run before a repaint can replace the Source node.
            visual.update(|window, cx| {
                window.dispatch_action(Box::new(MoveRight), cx);
                window.dispatch_action(Box::new(MoveLeft), cx);
            });
        } else if movement == 2 {
            visual.dispatch_action(MoveLeft);
        }
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let snapshot = request.tabs.first().unwrap();
            assert_eq!(w.selection().anchor, snapshot.selection.anchor);
            assert_eq!(w.selection().head, snapshot.selection.head);
            assert_eq!(w.buffer().raw_revision, snapshot.revision);
            assert!(w.buffer().rope.is_instance(&snapshot.rope));
            assert_eq!(w.source_input_token() == epoch, movement != 1,
                "only actual caret movement retires Source identity");
            eprintln!("held replacement: mode={} movement={movement} live-proof={} source-epoch-equal={}",
                if mode == Mode::Current { "Current" } else { "All" },
                w.request_valid(&request), w.source_input_token() == epoch);
            w.finish_regex_replacement(request, prepare(job), cx);
            let expected = if movement == 1 { "first 中😀\n" } else { "firstfirst 中😀\n" };
            assert_eq!(w.buffer().rope.to_string(), expected);
            assert_eq!(w.tabs[0].undo_stack.len(), usize::from(movement != 1));
            assert_eq!(w.buffer().dirty, movement != 1);
            assert!(w.regex_replacement.pending.is_none());
            assert_ne!(w.find_state.status.as_deref(), Some(PREPARING));
        }));
    });
}

#[test]
fn source_caret_regex_current_worker_rejects_real_round_trip() {
    source_caret_worker(Mode::Current, 1);
}

#[test]
fn source_caret_regex_all_worker_rejects_real_round_trip() {
    source_caret_worker(Mode::All, 1);
}

#[test]
fn source_caret_regex_current_worker_accepts_unchanged_control() {
    source_caret_worker(Mode::Current, 0);
}

#[test]
fn source_caret_regex_all_worker_accepts_unchanged_control() {
    source_caret_worker(Mode::All, 0);
}

#[test]
fn source_caret_regex_current_worker_accepts_boundary_noop_control() {
    source_caret_worker(Mode::Current, 2);
}

#[test]
fn source_caret_regex_all_worker_accepts_boundary_noop_control() {
    source_caret_worker(Mode::All, 2);
}

#[test]
fn regex_source_guard_keeps_both_worker_requests_send() {
    fn require_send<T: Send>() {}
    require_send::<Request>();
}

#[test]
fn regex_source_guard_accepts_input_corrected_caret_after_the_next_paint() {
    for mode in [Mode::Current, Mode::All] {
        with_source(|entity, visual| {
            let mut handler =
                visual.update(|window, _| window.take_registered_input_handler().unwrap());
            let before = visual.update(|_, cx| entity.read(cx).source_input_token());
            visual.update(|window, cx| {
                handler.replace_text_in_range(None, "x", window, cx);
                if mode == Mode::Current {
                    window.dispatch_action(Box::new(EditReplaceCurrent), cx);
                } else {
                    window.dispatch_action(Box::new(EditReplaceAll), cx);
                }
            });
            let (request, job, captured) = visual.update(|_, cx| {
                entity.update(cx, |w, _| {
                    let (request, job) = w.regex_replacement.held.take().unwrap();
                    assert_eq!(w.selection().head, 1);
                    assert_eq!(w.buffer().rope.to_string(), "xfirst 中😀\n");
                    let captured = w.source_input_token();
                    assert_ne!(
                        captured, before,
                        "committed input retires the old Source frame"
                    );
                    assert_eq!(
                        w.regex_replacement.source,
                        Some((request.id, captured.clone()))
                    );
                    (request, job, captured)
                })
            });
            visual.run_until_parked();
            visual.update(|_, cx| {
                entity.update(cx, |w, cx| {
                    assert_eq!(
                        w.source_input_token(),
                        captured,
                        "settled caret stays valid after paint"
                    );
                    assert!(w.request_valid(&request));
                    w.finish_regex_replacement(request, prepare(job), cx);
                    assert_eq!(w.buffer().rope.to_string(), "xfirstfirst 中😀\n");
                    assert_eq!(w.tabs[0].undo_stack.len(), 2);
                    assert!(w.regex_replacement.source.is_none());
                    assert!(w.regex_replacement.pending.is_none());
                })
            });
        });
    }
}

#[test]
fn regex_source_guard_old_reply_preserves_the_new_pin_and_status() {
    with_source(|entity, visual| {
        invoke(visual, Mode::Current);
        let (old, old_job) = visual
            .update(|_, cx| entity.update(cx, |w, _| w.regex_replacement.held.take().unwrap()));
        visual.update(|window, cx| {
            window.dispatch_action(Box::new(MoveRight), cx);
            window.dispatch_action(Box::new(MoveLeft), cx);
        });
        invoke(visual, Mode::All);
        visual.update(|_, cx| {
            entity.update(cx, |w, cx| {
                let (new, new_job) = w.regex_replacement.held.take().unwrap();
                let current_pin = w.regex_replacement.source.clone();
                assert_eq!(current_pin, Some((new.id, w.source_input_token())));
                w.set_status("newer user-visible status");
                w.finish_regex_replacement(old, prepare(old_job), cx);
                assert_eq!(w.regex_replacement.pending, Some(new.id));
                assert_eq!(w.regex_replacement.source, current_pin);
                assert_eq!(w.find_state.status.as_deref(), Some(PREPARING));
                assert_eq!(
                    w.status_message.as_deref(),
                    Some("newer user-visible status")
                );
                assert!(w.request_valid(&new));
                w.finish_regex_replacement(new, prepare(new_job), cx);
                assert_eq!(w.buffer().rope.to_string(), "firstfirst 中😀\n");
                assert_eq!(
                    w.status_message.as_deref(),
                    Some("newer user-visible status")
                );
                assert!(w.regex_replacement.pending.is_none());
                assert!(w.regex_replacement.source.is_none());
            })
        });
    });
}

#[test]
fn regex_source_guard_cancel_clears_only_its_pending_context() {
    with_source(|entity, visual| {
        invoke(visual, Mode::All);
        visual.update(|_, cx| {
            entity.update(cx, |w, cx| {
                let (request, job) = w.regex_replacement.held.take().unwrap();
                assert!(w.regex_replacement.source.is_some());
                w.set_status("newer cancellation status");
                w.cancel_regex_replacement();
                assert!(w.regex_replacement.pending.is_none());
                assert!(w.regex_replacement.source.is_none());
                assert!(w.find_state.status.is_none());
                w.finish_regex_replacement(request, prepare(job), cx);
                assert_eq!(w.buffer().rope.to_string(), "first 中😀\n");
                assert!(w.tabs[0].undo_stack.is_empty());
                assert_eq!(
                    w.status_message.as_deref(),
                    Some("newer cancellation status")
                );
            })
        });
    });
}

#[test]
fn regex_source_guard_does_not_pin_unrelated_source_context_for_rich_view() {
    with_source(|entity, visual| {
        visual.update(|_, cx| {
            entity.update(cx, |w, cx| {
                w.active_tab_mut().buffer.rich_doc = Some(
                    crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi first}".to_vec()).unwrap(),
                );
                w.active_tab_mut().view_mode = TabViewMode::RichDocView;
                w.start_regex_replacement(Mode::All, cx);
                let (request, job) = w.regex_replacement.held.take().unwrap();
                assert!(w.regex_replacement.source.is_none());
                let old = w.source_input_token();
                w.retire_source_input();
                assert_ne!(w.source_input_token(), old);
                assert!(
                    w.request_valid(&request),
                    "Rich still uses its AST/selection guards"
                );
                w.finish_regex_replacement(request, prepare(job), cx);
                let payload = w.buffer().rich_doc.as_ref().unwrap();
                assert_eq!(payload.flat_search_text().0, "firstfirst");
                assert_eq!(payload.undo_stack.len(), 1);
                assert!(w.regex_replacement.pending.is_none());
                assert!(w.regex_replacement.source.is_none());
            })
        });
    });
}

fn all_opened_worker_after_confirmation(movement: u8) {
    with_source(|entity, visual| {
        if movement == 3 {
            let mut handler = visual.update(|window, _| window.take_registered_input_handler().unwrap());
            visual.update(|window, cx| handler.replace_text_in_range(None, "x", window, cx));
        }
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.tabs[1].buffer.rope = ropey::Rope::from_str("first café\n");
            w.regex_replacement.hold_confirmation = true;
        }));
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.dialog_tab = crate::find_state::DialogTab::Replace;
            w.find_state.open = true;
        }));
        let weak = entity.downgrade();
        let (_, dialog) = visual.cx.add_window_view(|window, cx| {
            crate::ui::find_dialog::FindDialog::new(weak, window, cx)
        });
        dialog.update(|window, cx| {
            window.activate_window();
            window.focus(&entity.read(cx).find_focus.clone(), cx);
        });
        dialog.run_until_parked();
        let button = dialog.debug_bounds("dialog-button-Replace All in All Opened Documents").unwrap();
        dialog.simulate_click(button.center(), gpui::Modifiers::none());
        // Return to the visible Source before the confirmation's Yes starts its worker.
        visual.update(|window, cx| {
            window.activate_window();
            window.focus(&entity.read(cx).focus_handle.clone(), cx);
        });
        visual.run_until_parked();
        let (request, job, epoch) = visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert!(!w.window_closed);
            let confirmation = w.regex_replacement.held_confirmation.take().unwrap();
            assert_eq!(w.regex_replacement.pending, Some(confirmation.id));
            assert!(w.regex_replacement.held.is_none(), "actual action entered confirmation first");
            w.resume_replace_all_confirmation(confirmation, cx);
            let (request, job) = w.regex_replacement.held.take().unwrap();
            assert!(request.mode == Mode::AllOpened);
            assert_eq!(request.tabs.len(), 2);
            (request, job, w.source_input_token())
        }));
        if movement == 3 {
            visual.run_until_parked();
        }
        if movement == 1 {
            visual.update(|window, cx| {
                window.dispatch_action(Box::new(MoveRight), cx);
                window.dispatch_action(Box::new(MoveLeft), cx);
            });
        } else if movement == 2 {
            visual.dispatch_action(MoveLeft);
        }
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            for snapshot in &request.tabs {
                let tab = w.tabs.iter().find(|tab| tab.session_id == snapshot.tab).unwrap();
                assert_eq!(tab.selection.anchor, snapshot.selection.anchor);
                assert_eq!(tab.selection.head, snapshot.selection.head);
                assert_eq!(tab.buffer.raw_revision, snapshot.revision);
                assert!(tab.buffer.rope.is_instance(&snapshot.rope));
            }
            assert_eq!(w.source_input_token() == epoch, movement != 1);
            eprintln!("AllOpened after actual action/confirmation: movement={movement} live-proof={} source-epoch-equal={}",
                w.request_valid(&request), w.source_input_token() == epoch);
            w.finish_regex_replacement(request, prepare(job), cx);
            let changed = movement != 1;
            assert_eq!(w.tabs[0].buffer.rope.to_string(), if movement == 3 { "xfirstfirst \u{4e2d}\u{1f600}\n" } else if changed { "firstfirst 中😀\n" } else { "first 中😀\n" });
            assert_eq!(w.tabs[1].buffer.rope.to_string(), if changed { "firstfirst café\n" } else { "first café\n" });
            assert_eq!(w.active, 0);
            for (index, tab) in w.tabs.iter().enumerate() {
                let input_edit = usize::from(movement == 3 && index == 0);
                assert_eq!(tab.undo_stack.len(), usize::from(changed) + input_edit);
                assert_eq!(tab.buffer.dirty, changed);
            }
            assert!(w.regex_replacement.pending.is_none());
            assert_ne!(w.find_state.status.as_deref(), Some(PREPARING));
        }));
    });
}

#[test]
fn source_caret_regex_all_opened_worker_rejects_real_round_trip_after_confirmation() {
    all_opened_worker_after_confirmation(1);
}

#[test]
fn source_caret_regex_all_opened_worker_accepts_unchanged_after_confirmation() {
    all_opened_worker_after_confirmation(0);
}

#[test]
fn source_caret_regex_all_opened_worker_accepts_boundary_noop_after_confirmation() {
    all_opened_worker_after_confirmation(2);
}

#[test]
fn source_caret_regex_all_opened_worker_accepts_corrected_input_after_confirmation() {
    all_opened_worker_after_confirmation(3);
}
