// Held integration tests: root owns the two tab-removal hooks.
use super::syntax_refresh_tests::{hold_after_input, with_sources};
use crate::actions::{FileCloseAllButActive, FileCloseTab, FileRestoreClosedTab, TabGoto2};

fn close_and_restore(batch: bool) {
    with_sources(|entity, visual| {
        hold_after_input(entity, visual);
        visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[0].buffer.mark_saved()));
        if batch {
            visual.dispatch_action(TabGoto2);
            visual.dispatch_action(FileCloseAllButActive);
        } else {
            visual.dispatch_action(FileCloseTab);
        }
        visual.dispatch_action(FileRestoreClosedTab);
        visual.update(|_, cx| {
            entity.update(cx, |w, _| {
                assert_eq!(w.tabs.len(), 2);
                assert_eq!(
                    w.syntax_pending_for_test().0,
                    0,
                    "close must retire the invocation before the same UUID returns"
                );
                // Restoring independently reparses today. Mark its freshly installed vector
                // so an older painted/worker callback cannot silently replace its ownership.
                w.tabs[w.active].buffer.highlights.clear();
                w.set_status("newer restored-tab status");
            })
        });
        visual.update(|_, cx| entity.update(cx, |w, cx| w.release_syntax_refresh(cx)));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.tabs[w.active].buffer.highlights.is_empty());
            assert_eq!(
                w.status_message.as_deref(),
                Some("newer restored-tab status")
            );
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
        });
    });
}

#[test]
fn syntax_refresh_close_and_restore_cannot_revive_a_held_result() {
    close_and_restore(false);
}

#[test]
fn syntax_refresh_batch_close_and_restore_cannot_revive_a_held_result() {
    close_and_restore(true);
}
