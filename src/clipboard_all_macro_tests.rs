// NppCommands.cpp:4590-4591 — copy-all commands record identity, not clipboard data.
use std::collections::BTreeMap;

use crate::edit_ops::clipboard_all::tests::{
    clipboard, edit_state, first_path, fixture, sample_tabs, second_path,
};
use crate::{actions::*, persist::TempDir};
use gpui::{TestAppContext, VisualTestContext};

fn record_and_replay(paths: bool) {
    let _settings = crate::settings::override_for_test(Default::default());
    let dispatch = |visual: &mut VisualTestContext| {
        if paths {
            visual.dispatch_action(EditCopyAllFilePaths);
        } else {
            visual.dispatch_action(EditCopyAllFilenames);
        }
    };
    let recorded = {
        let _recorder = super::override_execution_for_test(false);
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, sample_tabs());
        let before = visual.update(|_, cx| edit_state(workspace.read(cx)));
        visual.dispatch_action(MacroStartRecording);
        dispatch(visual);
        visual.dispatch_action(MacroStopRecording);
        visual.update(|_, cx| assert_eq!(edit_state(workspace.read(cx)), before));
        let events = super::current();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            (paths, events.as_slice()),
            (true, [super::MacroEvent::CopyAllFilePaths])
                | (false, [super::MacroEvent::CopyAllFilenames])
        ));
        cx.quit();
        events
    };
    let name = "copy all café".to_string();
    let directory = TempDir::new("clipboard-all-saved-macro");
    let path = directory.join("macros.toml");
    let mut saved = BTreeMap::from([(name.clone(), recorded)]);
    // Old persisted event names remain loadable alongside the new commands.
    saved.insert(
        "legacy".into(),
        vec![super::MacroEvent::Copy, super::MacroEvent::MoveRight],
    );
    super::persist_at(&path, &saved).expect("persist recorded macro");
    let serialized = std::fs::read_to_string(&path).unwrap();
    let event_name = if paths {
        "CopyAllFilePaths"
    } else {
        "CopyAllFilenames"
    };
    assert!(serialized.contains(event_name));
    assert!(
        !serialized.contains("new Ω 7"),
        "clipboard payload must not be captured"
    );
    let loaded = super::load_saved_at(&path);
    assert_eq!(loaded.len(), 2);
    assert!(matches!(
        loaded.get("legacy").map(Vec::as_slice),
        Some([super::MacroEvent::Copy, super::MacroEvent::MoveRight])
    ));
    assert!(!directory.join("macros.toml.corrupt").exists());

    let _recorder = super::override_execution_for_test(true);
    let playback = super::override_playback_for_test(Vec::new(), loaded);
    let mut tabs = sample_tabs();
    tabs[1].buffer.new_label = Some("new 中 9".into());
    tabs.rotate_left(2);
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, tabs);
    let before = visual.update(|_, cx| edit_state(workspace.read(cx)));
    visual.dispatch_action(MacroRunSaved { name });
    visual.run_until_parked();
    visual.update(|_, cx| {
        let expected = if paths {
            format!(
                "{}\r\n{}\r\n{}\r\nnew 中 9\r\n",
                second_path().display(),
                first_path().display(),
                first_path().display()
            )
        } else {
            "café.txt\r\ncafé.txt\r\ncafé.txt\r\nnew 中 9\r\n".to_string()
        };
        assert_eq!(clipboard(cx), expected);
        assert_eq!(edit_state(workspace.read(cx)), before);
    });
    drop(playback);
    super::stop_recording();
    assert!(
        super::current().is_empty(),
        "replay must not append recorded events"
    );
    cx.quit();
}

#[test]
fn clipboard_all_filenames_record_persist_and_replay_current_tab_order() {
    record_and_replay(false);
}

#[test]
fn clipboard_all_paths_record_persist_and_replay_current_tab_order() {
    record_and_replay(true);
}
