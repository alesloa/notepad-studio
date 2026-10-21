use super::*;
use crate::tab::{Tab, TabViewMode};
use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext};

thread_local! { static GATE: std::cell::RefCell<Option<futures::channel::oneshot::Receiver<()>>> = const { std::cell::RefCell::new(None) }; }
pub(super) fn take_gate() -> Option<futures::channel::oneshot::Receiver<()>> {
    GATE.with(|slot| slot.borrow_mut().take())
}

fn with_pending(
    run: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext, futures::channel::oneshot::Sender<()>),
) {
    with_source(format!("h\n{}\n", "é".repeat(40_000)), run);
}

fn with_source(
    source: String,
    run: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext, futures::channel::oneshot::Sender<()>),
) {
    let _settings = crate::settings::override_for_test(crate::settings::Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        ..Default::default()
    });
    let (release, gate) = futures::channel::oneshot::channel();
    let _ = GATE.with(|slot| slot.replace(Some(gate)));
    let mut app = TestAppContext::single();
    let (entity, visual) = app.add_window_view(|_, cx| {
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
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(&source);
        tab.buffer.file_path = Some("neutral.csv".into());
        tab.view_mode = TabViewMode::CsvPreview;
        w.tabs = vec![tab];
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w
    });
    visual.update_window_entity(&entity, |w, window, cx| {
        window.activate_window();
        super::super::actions::ensure_model(w, window, cx);
    });
    visual.run_until_parked();
    visual.update(|_, cx| {
        assert!(
            entity
                .read(cx)
                .csv_models
                .values()
                .next()
                .unwrap()
                .parse_job
                .is_some()
        )
    });
    run(&entity, visual, release);
    app.quit();
}

#[test]
fn large_grid_prepares_off_thread_without_changing_source_or_dirty_state() {
    with_pending(|entity, visual, release| {
        release.send(()).unwrap();
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            let model = w.csv_models.values().next().unwrap();
            assert!(model.parse_job.is_none() && model.parse_error.is_none());
            assert_eq!(model.cell(0, 0), "é".repeat(40_000));
            assert_eq!(
                w.buffer().rope.to_string(),
                format!("h\n{}\n", "é".repeat(40_000))
            );
            assert!(!w.buffer().dirty);
        });
    });
}

#[test]
fn cancelled_or_replaced_csv_cannot_publish_old_worker_result() {
    for mode in 0..3 {
        with_pending(|entity, visual, release| {
            visual.update_window_entity(entity, |w, window, cx| {
                if mode == 1 {
                    w.tabs[0].buffer.rope = ropey::Rope::from_str("new\nfresh\n");
                    w.tabs[0].buffer.raw_revision = ContentRevision::default();
                    super::super::actions::ensure_model(w, window, cx);
                } else if mode == 0 {
                    w.view_csv_source(&crate::actions::ViewCsvSource, window, cx);
                } else {
                    w.view_source(&crate::actions::ViewSource, window, cx);
                }
            });
            release.send(()).unwrap();
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = entity.read(cx);
                let model = w.csv_models.values().next().unwrap();
                assert!(model.parse_job.is_none());
                if mode == 1 {
                    assert_eq!(model.cell(0, 0), "fresh");
                } else {
                    assert_eq!(w.tabs[0].view_mode, TabViewMode::Edit);
                    assert_eq!(model.row_count(), 0);
                }
            });
        });
    }
}

#[test]
fn unfinished_stream_defers_grid_until_the_source_is_complete() {
    with_pending(|entity, visual, release| {
        visual.update_window_entity(entity, |w, window, cx| {
            let id = w.tabs[0].session_id;
            cancel(w.csv_models.get_mut(&id).unwrap());
            w.tabs[0].buffer.loading = Some(crate::buffer::LoadProgress {
                bytes_read: 80_000,
                total_bytes: 12_000_000,
                cancel: Arc::new(AtomicBool::new(false)),
            });
            super::super::actions::ensure_model(w, window, cx);
            let model = w.csv_models.get(&id).unwrap();
            assert_eq!(model.parse_error, Some("CSV source is still loading"));
            assert!(model.parse_job.is_none());
            w.tabs[0].buffer.loading = None;
            super::super::actions::ensure_model(w, window, cx);
        });
        release.send(()).unwrap();
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            let model = w.csv_models.values().next().unwrap();
            assert!(model.parse_error.is_none());
            assert_eq!(model.row_count(), 1);
        });
    });
}

#[test]
fn delimiter_change_restarts_pending_parse_and_rejects_old_configuration() {
    with_source(
        format!("h,v\n{},2\n", "é".repeat(40_000)),
        |entity, visual, release| {
            visual.update_window_entity(entity, |w, window, cx| {
                let id = w.tabs[0].session_id;
                w.csv_models.get_mut(&id).unwrap().set_delimiter(b';');
                super::super::actions::ensure_model(w, window, cx);
            });
            release.send(()).unwrap();
            visual.run_until_parked();
            visual.update(|_, cx| {
                let model = entity.read(cx).csv_models.values().next().unwrap();
                assert!(model.parse_job.is_none());
                assert_eq!(model.delimiter, b';');
                assert_eq!(model.col_count(), 1);
                assert_eq!(model.header(0), "h,v");
                assert_eq!(model.cell(0, 0), format!("{},2", "é".repeat(40_000)));
            });
        },
    );
}

#[test]
fn long_first_field_uses_full_source_delimiter_inference_off_thread() {
    with_source(
        format!("{};v\n{}", "x".repeat(2000), "x;2\n".repeat(20_000)),
        |entity, visual, release| {
            release.send(()).unwrap();
            visual.run_until_parked();
            visual.update(|_, cx| {
                let model = entity.read(cx).csv_models.values().next().unwrap();
                assert_eq!(model.delimiter, b';');
                assert_eq!(model.col_count(), 2);
                assert_eq!(model.cell(0, 1), "2");
                assert_eq!(model.row_count(), 20_000);
            });
        },
    );
}

#[test]
fn pending_grid_keeps_keyboard_routing_and_source_view_cancels_it() {
    with_pending(|entity, visual, release| {
        visual.update_window_entity(entity, |w, window, cx| {
            let focus = w
                .csv_models
                .values()
                .next()
                .unwrap()
                .grid_focus
                .clone()
                .unwrap();
            window.focus(&focus, cx);
            cx.notify();
        });
        visual.run_until_parked();
        visual.simulate_input("must not edit hidden source");
        visual.update(|_, cx| {
            assert_eq!(
                entity.read(cx).buffer().rope.to_string(),
                format!("h\n{}\n", "é".repeat(40_000))
            )
        });
        visual.dispatch_action(crate::actions::ViewCsvSource);
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[0].view_mode, TabViewMode::Edit);
            assert!(w.focus_handle.is_focused(window));
        });
        release.send(()).unwrap();
        visual.run_until_parked();
    });
}
