// Actual menu action → recorder → production macro writer/loader → fresh-editor replay.
use super::*;
use crate::{actions::*, edit_ops::bookmark_clipboard::tests::fixture, settings::Settings};
use gpui::{ClipboardItem, TestAppContext};

#[test]
fn bookmark_clipboard_record_save_reload_and_fresh_editor_replay_keep_legacy_names() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let recorded = {
        let _recorder = override_execution_for_test(true);
        let mut cx = TestAppContext::single();
        let (_, visual) = fixture(&mut cx, "café😀\nignored\n中", &[0, "café😀\nignored\n".len()]);
        visual.dispatch_action(CopyBookmarkedLines);
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("café😀\n中"));
        assert!(!stop_recording());
        let events = current();
        assert!(matches!(events.as_slice(), [MacroEvent::CopyBookmarkedLines]));
        cx.quit();
        events
    };
    let dir = crate::persist::TempDir::new("bookmark-copy-macro");
    let path = dir.join("macros.toml");
    let name = "copy café bookmarks".to_owned();
    let saved = BTreeMap::from([(name.clone(), recorded), ("legacy".into(), vec![MacroEvent::Copy, MacroEvent::Paste, MacroEvent::FindNext])]);
    persist_at(&path, &saved).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("\"CopyBookmarkedLines\""));
    let loaded = load_saved_at(&path);
    assert_eq!(loaded.len(), 2);
    assert!(matches!(loaded.get("legacy").map(Vec::as_slice), Some([MacroEvent::Copy, MacroEvent::Paste, MacroEvent::FindNext])));
    assert!(!dir.join("macros.toml.corrupt").exists());
    let _recorder = override_execution_for_test(true);
    let _playback = override_playback_for_test(Vec::new(), loaded);
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "fresh 😀\nignored\n新", &[0, "fresh 😀\nignored\n".len()]);
    visual.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
    visual.dispatch_action(MacroRunSaved { name });
    assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("fresh 😀\n新"));
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), "fresh 😀\nignored\n新");
        assert!(w.tabs[0].undo_stack.is_empty() && w.tabs[0].redo_stack.is_empty() && w.tabs[0].edit_transaction.is_none());
        assert_eq!((w.selection().anchor, w.selection().head), ("fresh 😀\nignored\n新".len(), 0));
        assert!(!w.buffer().dirty);
    });
    assert!(!stop_recording());
    assert!(current().is_empty(), "Replay must not record itself");
    cx.quit();
}
