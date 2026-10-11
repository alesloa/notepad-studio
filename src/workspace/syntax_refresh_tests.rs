use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext, WindowAppearance};

use super::Workspace;
use crate::{
    actions::{
        EditRedo, EditUndo, LanguageNormalText, LanguagePython, LanguageRust, TabGoto1, TabGoto2,
    },
    settings::Settings,
    syntax::LanguageId,
    tab::Tab,
};

pub(super) fn with_sources(test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        ..Settings::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
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
            WindowAppearance::Dark,
        );
        w.tabs = ["fn first() {}\n", "fn second() {}\n"]
            .into_iter()
            .map(|text| {
                let mut tab = Tab::empty();
                tab.buffer.rope = ropey::Rope::from_str(text);
                tab.buffer.language = Some(LanguageId::Rust);
                tab
            })
            .collect();
        w.active = 0;
        // Exercise a genuinely open window; synthetic recovery sinks avoid user files.
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.touch_mru();
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
        let close_entity = entity.clone();
        window.on_window_should_close(cx, move |_, cx| {
            close_entity.update(cx, crate::app::quit::save_before_window_close)
        });
    });
    visual.run_until_parked();
    test(&entity, visual);
    cx.quit();
}

fn settle(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual
        .cx
        .executor()
        .advance_clock(Duration::from_millis(130));
    visual.run_until_parked();
}

#[test]
fn syntax_refresh_reparses_the_edited_tab_after_switching_tabs() {
    with_sources(|entity, visual| {
        visual.simulate_input("let edited = 42;\n");
        visual.dispatch_action(TabGoto2);
        settle(visual);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(
                !w.tabs[0].buffer.highlights.is_empty(),
                "the edited inactive tab must be refreshed"
            );
            assert!(
                w.tabs[1].buffer.highlights.is_empty(),
                "the untouched tab must not steal the delayed parse"
            );
        });
    });
}

#[test]
fn syntax_refresh_coalesces_a_typing_burst_before_starting_the_parser() {
    with_sources(|entity, visual| {
        visual.simulate_input("let burst = 42;");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.syntax_refresh.started, 0);
            assert_eq!(w.syntax_pending_for_test(), (1, 0, false));
        });
        settle(visual);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.syntax_refresh.started, 1);
            assert!(!w.tabs[0].buffer.highlights.is_empty());
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
        });
    });
}

#[test]
fn syntax_refresh_keeps_debounced_jobs_for_both_edited_tabs() {
    with_sources(|entity, visual| {
        visual.simulate_input("let first = 1;\n");
        visual.dispatch_action(TabGoto2);
        visual.simulate_input("let second = 2;\n");
        settle(visual);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(
                !w.tabs[0].buffer.highlights.is_empty(),
                "editing another tab must not supersede this job"
            );
            assert!(!w.tabs[1].buffer.highlights.is_empty());
        });
    });
}

pub(super) fn hold_after_input(entity: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|_, cx| entity.update(cx, |w, _| w.hold_syntax_refresh(true)));
    visual.simulate_input("let current = 42;");
    settle(visual);
    visual.update(|_, cx| assert!(entity.read(cx).syntax_pending_for_test().2));
}

fn spans(
    spans: &[crate::syntax::HighlightSpan],
) -> Vec<(std::ops::Range<usize>, crate::syntax::HighlightKind)> {
    spans
        .iter()
        .map(|span| (span.range.clone(), span.kind))
        .collect()
}

#[test]
fn syntax_refresh_rejects_an_older_result_and_starts_only_one_parser() {
    with_sources(|entity, visual| {
        hold_after_input(entity, visual);
        visual.simulate_input("// café 中😀\n");
        settle(visual);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(
                w.syntax_refresh.started, 1,
                "the cancelled parser still occupies the worker slot"
            );
            assert!(w.tabs[0].buffer.highlights.is_empty());
            assert_eq!(w.syntax_pending_for_test(), (1, 1, true));
        });
        visual.update(|_, cx| entity.update(cx, |w, cx| w.release_syntax_refresh(cx)));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.syntax_refresh.started, 2);
            let source = w.tabs[0].buffer.rope.to_string();
            let expected = crate::syntax::highlight(LanguageId::Rust, &source);
            assert_eq!(spans(&w.tabs[0].buffer.highlights), spans(&expected));
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
        });
    });
}

#[test]
fn syntax_refresh_keeps_inactive_readonly_results_and_does_not_require_editor_focus() {
    with_sources(|entity, visual| {
        hold_after_input(entity, visual);
        visual.update(|window, cx| {
            entity.update(cx, |w, _| w.tabs[0].buffer.read_only = true);
            window.blur();
        });
        visual.dispatch_action(TabGoto2);
        // Tab switching itself needs focus; activate the other tab through its real handler.
        visual.update(|_, cx| entity.update(cx, |w, cx| w.set_active_tab(1, cx)));
        visual.update(|_, cx| entity.update(cx, |w, cx| w.release_syntax_refresh(cx)));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(!w.tabs[0].buffer.highlights.is_empty());
            assert!(w.tabs[0].buffer.read_only);
            assert!(w.tabs[1].buffer.highlights.is_empty());
        });
    });
}

#[test]
fn syntax_refresh_language_round_trip_cannot_install_an_old_result() {
    with_sources(|entity, visual| {
        hold_after_input(entity, visual);
        visual.dispatch_action(LanguageNormalText);
        visual.dispatch_action(LanguageRust);
        visual.update(|_, cx| assert_eq!(entity.read(cx).syntax_refresh.started, 1));
        visual.update(|_, cx| entity.update(cx, |w, cx| w.release_syntax_refresh(cx)));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.syntax_refresh.started, 2);
            assert!(!w.tabs[0].buffer.highlights.is_empty());
            assert_eq!(w.tabs[0].buffer.language, Some(LanguageId::Rust));
        });
    });
}

#[test]
fn syntax_refresh_loading_and_policy_guard_changes_retire_held_results() {
    for loading in [false, true] {
        with_sources(|entity, visual| {
            hold_after_input(entity, visual);
            let mut policy = crate::settings::snapshot();
            if !loading {
                policy.large_file_restriction_enabled = true;
                policy.large_file_disable_syntax = true;
                policy.large_file_threshold_bytes = 1;
            }
            let _restricted = crate::settings::override_for_test(policy);
            visual.update(|_, cx| {
                entity.update(cx, |w, cx| {
                    if loading {
                        w.tabs[0].buffer.loading = Some(crate::buffer::LoadProgress {
                            bytes_read: 0,
                            total_bytes: 100,
                            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        });
                    }
                    w.set_status("newer status");
                    w.release_syntax_refresh(cx);
                })
            });
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = entity.read(cx);
                assert!(w.tabs[0].buffer.highlights.is_empty());
                assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
                assert_eq!(w.status_message.as_deref(), Some("newer status"));
            });
        });
    }
}

#[test]
fn syntax_refresh_undo_before_worker_completion_never_installs_the_edited_snapshot() {
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| w.hold_syntax_refresh(true)));
        // simulate_input follows real per-character typing/history. One quote is
        // one Undo entry and changes highlighting across the following source.
        visual.simulate_input("\"");
        settle(visual);
        visual.update(|_, cx| assert!(entity.read(cx).syntax_pending_for_test().2));
        visual.dispatch_action(EditUndo);
        settle(visual);
        visual.update(|_, cx| entity.update(cx, |w, cx| w.release_syntax_refresh(cx)));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[0].buffer.rope.to_string(), "fn first() {}\n");
            assert_eq!(
                spans(&w.tabs[0].buffer.highlights),
                spans(&crate::syntax::highlight(
                    LanguageId::Rust,
                    "fn first() {}\n"
                ))
            );
        });
        // Redo restores op.after_revision after apply_edit too. Its delayed
        // request must pin that final identity before starting the parser.
        visual.update(|_, cx| entity.update(cx, |w, _| w.hold_syntax_refresh(true)));
        visual.dispatch_action(EditRedo);
        settle(visual);
        visual.update(|_, cx| assert!(entity.read(cx).syntax_pending_for_test().2));
        visual.update(|_, cx| entity.update(cx, |w, cx| w.release_syntax_refresh(cx)));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.syntax_refresh.started, 3);
            assert_eq!(
                spans(&w.tabs[0].buffer.highlights),
                spans(&crate::syntax::highlight(
                    LanguageId::Rust,
                    &w.tabs[0].buffer.rope.to_string()
                ))
            );
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
        });
    });
}

#[test]
fn syntax_refresh_running_result_cannot_retarget_a_changed_revision() {
    with_sources(|entity, visual| {
        hold_after_input(entity, visual);
        visual.update(|_, cx| {
            entity.update(cx, |w, cx| {
                w.tabs[0].buffer.raw_revision = crate::buffer::ContentRevision::default();
                w.set_status("newer revision status");
                w.release_syntax_refresh(cx);
            })
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.tabs[0].buffer.highlights.is_empty());
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
            assert_eq!(w.status_message.as_deref(), Some("newer revision status"));
        });
    });
}

#[test]
fn syntax_refresh_queue_refusal_is_bounded_and_same_language_can_retry() {
    with_sources(|entity, visual| {
        hold_after_input(entity, visual);
        visual.update(|_, cx| {
            entity.update(cx, |w, cx| {
                while w.tabs.len() < 257 {
                    let mut tab = Tab::empty();
                    tab.buffer.rope = ropey::Rope::from_str("fn queued() {}\n");
                    tab.buffer.language = Some(LanguageId::Rust);
                    w.tabs.push(tab);
                }
                for index in 1..257 {
                    w.active = index;
                    w.request_syntax_refresh(Duration::ZERO, cx);
                }
                assert_eq!(w.syntax_pending_for_test(), (256, 255, true));
                assert_eq!(w.syntax_refresh.started, 1);
                assert!(w
                    .status_message
                    .as_deref()
                    .unwrap()
                    .contains("select this language again"));
                w.release_syntax_refresh(cx);
            })
        });
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).syntax_pending_for_test(), (0, 0, false)));
        visual.dispatch_action(LanguageRust);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.syntax_refresh.started, 257);
            assert!(!w.tabs[256].buffer.highlights.is_empty());
        });
    });
}

#[test]
fn syntax_refresh_recovery_is_independent_of_disabled_syntax_and_held_parsing() {
    for disabled in [false, true] {
        with_sources(|entity, visual| {
            let (sender, receiver) = std::sync::mpsc::channel();
            visual.update(|_, cx| {
                entity.update(cx, |w, _| {
                    w.session_snapshot_sink = Some(sender);
                    w.hold_syntax_refresh(true);
                    if disabled {
                        w.tabs[0].buffer.language = None;
                    }
                })
            });
            visual.simulate_input("// recovered café 中😀\n");
            settle(visual);
            std::thread::sleep(Duration::from_millis(130));
            visual.run_until_parked();
            let manifest = receiver
                .try_recv()
                .expect("eager recovery must not wait for syntax");
            assert_eq!(
                manifest.tabs.len(),
                1,
                "the untouched scratch tab is not a recovery candidate"
            );
            visual.update(|_, cx| {
                let w = entity.read(cx);
                assert_eq!(w.syntax_refresh.started, usize::from(!disabled));
                assert_eq!(w.syntax_pending_for_test().2, !disabled);
            });
        });
    }
}

#[test]
fn syntax_refresh_manual_language_requests_share_the_same_parser_slot() {
    with_sources(|entity, visual| {
        hold_after_input(entity, visual);
        visual.dispatch_action(TabGoto2);
        visual.dispatch_action(LanguagePython);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.syntax_refresh.started, 1);
            assert_eq!(w.syntax_pending_for_test(), (2, 1, true));
        });
        visual.update(|_, cx| entity.update(cx, |w, cx| w.release_syntax_refresh(cx)));
        visual.run_until_parked();
        visual.dispatch_action(TabGoto1);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.syntax_refresh.started, 2);
            assert!(!w.tabs[0].buffer.highlights.is_empty());
            assert!(!w.tabs[1].buffer.highlights.is_empty());
        });
    });
}

#[test]
fn syntax_refresh_native_close_retires_completed_and_queued_work_with_retained_workspace() {
    with_sources(|entity, visual| {
        hold_after_input(entity, visual);
        visual.dispatch_action(TabGoto2);
        visual.simulate_input("// queued");
        settle(visual);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(!w.window_closed);
            assert_eq!(w.syntax_pending_for_test(), (2, 1, true));
            assert_eq!(w.syntax_refresh.started, 1);
        });
        assert!(
            visual.simulate_close(),
            "the actual should-close callback succeeds"
        );
        // The headless helper asks permission; the platform's following close
        // stage removes the window. Exercise that stage through Window as well.
        visual.update(|window, _| window.remove_window());
        visual.cx.update(|cx| {
            assert!(cx.windows().is_empty());
            entity.update(cx, |w, cx| {
                assert!(w.window_closed);
                assert_eq!(
                    w.tabs.len(),
                    2,
                    "native close retains the original buffers while references exist"
                );
                w.release_syntax_refresh(cx);
                w.request_syntax_refresh(Duration::ZERO, cx);
                w.schedule_reparse(cx);
            });
        });
        visual.run_until_parked();
        visual.cx.update(|cx| {
            let w = entity.read(cx);
            assert_eq!(w.syntax_refresh.started, 1);
            assert!(w.tabs.iter().all(|tab| tab.buffer.highlights.is_empty()));
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
        });
    });
}

#[test]
fn syntax_refresh_failed_native_close_keeps_the_window_and_parser_usable() {
    with_sources(|entity, visual| {
        hold_after_input(entity, visual);
        visual.update(|_, cx| entity.update(cx, |w, _| w.session_final_save_result = Some(false)));
        assert!(!visual.simulate_close());
        visual.update(|_, cx| {
            entity.update(cx, |w, cx| {
                assert!(!w.window_closed);
                assert_eq!(w.syntax_pending_for_test(), (1, 0, true));
                assert!(w
                    .status_message
                    .as_deref()
                    .unwrap()
                    .contains("recovery could not be saved"));
                w.release_syntax_refresh(cx);
            })
        });
        visual.run_until_parked();
        visual.simulate_input("\"");
        settle(visual);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(!w.window_closed);
            assert_eq!(w.syntax_refresh.started, 2);
            assert!(!w.tabs[0].buffer.highlights.is_empty());
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
        });
    });
}

#[test]
fn syntax_refresh_close_cancels_an_unfinished_parser_and_delayed_tab() {
    with_sources(|entity, visual| {
        visual.update(|window, cx| {
            entity.update(cx, |w, cx| {
                w.hold_syntax_refresh(true);
                w.language_rust(&LanguageRust, window, cx);
                w.set_active_tab(1, cx);
                w.schedule_reparse(cx);
                assert_eq!(w.syntax_refresh.started, 1);
                assert_eq!(
                    w.syntax_pending_for_test(),
                    (2, 0, false),
                    "the background result has not been polled yet"
                );
            })
        });
        assert!(visual.simulate_close());
        visual.update(|window, _| window.remove_window());
        visual
            .cx
            .executor()
            .advance_clock(Duration::from_millis(130));
        visual.run_until_parked();
        visual.cx.update(|cx| {
            let w = entity.read(cx);
            assert!(w.window_closed);
            assert_eq!(w.syntax_refresh.started, 1);
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
            assert!(w.tabs.iter().all(|tab| tab.buffer.highlights.is_empty()));
        });
    });
}

#[test]
#[ignore = "release syntax invocation/capture/background/install timing; no native paint claim"]
fn syntax_refresh_release_timings_above_streaming_threshold() {
    with_sources(|entity, visual| {
        let mut text = String::from("fn main() {}\n");
        let line = format!("// café 中😀 {}\n", "x".repeat(2048));
        while text.len() < 12 * 1024 * 1024 {
            text.push_str(&line);
        }
        let bytes = text.len();
        visual.update(|_, cx| {
            entity.update(cx, |w, _| {
                w.tabs[0].buffer.rope = ropey::Rope::from_str(&text);
                w.tabs[0].buffer.highlights.clear();
            })
        });
        for sample in 0..3 {
            let total = std::time::Instant::now();
            visual.update(|_, cx| {
                entity.update(cx, |w, cx| {
                    let invocation = std::time::Instant::now();
                    w.schedule_reparse(cx);
                    eprintln!(
                        "SYNTAX_NEW bytes={bytes} sample={sample} invocation={:?} capture={:?}",
                        invocation.elapsed(),
                        w.syntax_refresh.capture_time
                    );
                })
            });
            settle(visual);
            visual.update(|_, cx| {
                let w = entity.read(cx);
                assert!(!w.tabs[0].buffer.highlights.is_empty());
                assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
                eprintln!(
                    "SYNTAX_NEW sample={sample} worker={:?} install={:?} settle={:?} spans={}",
                    w.syntax_refresh.worker_time,
                    w.syntax_refresh.install_time,
                    total.elapsed(),
                    w.tabs[0].buffer.highlights.len()
                );
            });
        }
    });
}
