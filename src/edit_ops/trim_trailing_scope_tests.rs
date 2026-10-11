// NppCommands.cpp:2218–2225; Notepad_plus.cpp:1651–1704 — trailing trim scope.
use crate::{actions::*, workspace::Workspace};
use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

#[test]
fn registered_trim_trailing_preserves_unselected_lines() {
    let _settings = crate::settings::override_for_test(Default::default());
    for reversed in [false, true] {
        let source = "keep  \r\n café  \r\ntail  ";
        let start = source.find("café").unwrap();
        let end = source.find("tail").unwrap();
        let (anchor, head) = if reversed { (end, start) } else { (start, end) };
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source, anchor, head);
        visual.dispatch_action(EditTrimTrailingWs);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "keep  \r\n café\r\ntail  ");
            assert_eq!((w.selection().anchor, w.selection().head), (8, end - 2));
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        });
        visual.dispatch_action(EditUndo);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), source);
            assert_eq!((w.selection().anchor, w.selection().head), (anchor, head));
        });
        visual.dispatch_action(EditRedo);
        visual.update(|_, cx| {
            assert_eq!(
                workspace.read(cx).buffer().rope.to_string(),
                "keep  \r\n café\r\ntail  "
            )
        });
        cx.quit();
    }
}

#[test]
fn registered_trim_trailing_refuses_rectangle() {
    let _settings = crate::settings::override_for_test(Default::default());
    let source = "keep  \n café \t\ntail  ";
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, source, 0, 9);
    visual.update_window_entity(&workspace, |w, _, _| {
        w.active_tab_mut().block_selection = Some(crate::buffer::BlockSelection {
            anchor_line: 0,
            head_line: 1,
            anchor_col: 0,
            head_col: 2,
            is_dragging: false,
        });
    });
    visual.dispatch_action(EditTrimTrailingWs);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), source);
        assert_eq!((w.selection().anchor, w.selection().head), (0, 9));
        assert!(w.tabs[w.active].undo_stack.is_empty());
        assert!(w.tabs[w.active].block_selection.is_some());
    });
    cx.quit();
}

#[test]
fn registered_trim_trailing_handles_each_cr_only_line() {
    let _settings = crate::settings::override_for_test(Default::default());
    let source = " café  \rtail \t\r";
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, source, 0, 0);
    visual.dispatch_action(EditTrimTrailingWs);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), " café\rtail\r");
        assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
    });
    visual.dispatch_action(EditUndo);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
    cx.quit();
}

#[test]
fn registered_trim_trailing_expands_partial_end_and_preserves_file_format() {
    let _settings = crate::settings::override_for_test(Default::default());
    let source = "\u{FEFF}keep  \r\n café \t\r\ntail  ";
    let begin = source.find("café").unwrap() + 4;
    for encoding in [
        crate::text::encoding::Encoding::Utf8Bom,
        crate::text::encoding::Encoding::Utf16Le,
        crate::text::encoding::Encoding::Utf16Be,
    ] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source, begin, begin + 1);
        visual.update_window_entity(&workspace, |w, _, _| {
            w.active_tab_mut().buffer.encoding = encoding;
            w.active_tab_mut().buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
        });
        let before = visual.update(|_, cx| {
            let w = workspace.read(cx);
            w.selection().clamped(&w.buffer().rope)
        });
        visual.dispatch_action(EditTrimTrailingWs);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "\u{FEFF}keep  \r\n café\r\ntail  ");
            let start = "\u{FEFF}keep  \r\n".len();
            assert_eq!((w.selection().anchor, w.selection().head), (start, start + " café".len()));
            assert_eq!(w.buffer().encoding, encoding);
            assert_eq!(w.buffer().line_ending, crate::text::line_ending::LineEnding::Crlf);
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        });
        visual.dispatch_action(EditUndo);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), source);
            assert_eq!((w.selection().anchor, w.selection().head), (before.anchor, before.head));
        });
        cx.quit();
    }
}

#[test]
fn registered_trim_trailing_noop_preserves_reversed_selection_and_redo() {
    let _settings = crate::settings::override_for_test(Default::default());
    let source = "keep  \ncafé\ntail  ";
    let begin = source.find("café").unwrap();
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, source, begin + 3, begin + 1);
    visual.update_window_entity(&workspace, |w, _, cx| w.replace_byte_range(0..0, "x", cx));
    visual.dispatch_action(EditUndo);
    visual.update_window_entity(&workspace, |w, _, _| {
        w.active_tab_mut().selection.anchor = begin + 3;
        w.active_tab_mut().selection.head = begin + 1;
    });
    visual.dispatch_action(EditTrimTrailingWs);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), source);
        assert_eq!((w.selection().anchor, w.selection().head), (begin + 3, begin + 1));
        assert!(w.tabs[w.active].undo_stack.is_empty());
        assert_eq!(w.tabs[w.active].redo_stack.len(), 1);
    });
    visual.dispatch_action(EditRedo);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), format!("x{source}")));
    cx.quit();
}

#[test]
fn registered_trim_trailing_refuses_readonly_previews_and_stale_empty_scope() {
    let _settings = crate::settings::override_for_test(Default::default());
    for refusal in 0..5 {
        let source = "keep  \n café \t\ntail  ";
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source, 9, 12);
        visual.update_window_entity(&workspace, |w, _, _| match refusal {
            0 => w.active_tab_mut().buffer.read_only = true,
            1 => w.active_tab_mut().view_mode = crate::tab::TabViewMode::MarkdownPreview,
            2 => w.active_tab_mut().view_mode = crate::tab::TabViewMode::RichDocView,
            3 => w.active_tab_mut().view_mode = crate::tab::TabViewMode::CsvPreview,
            _ => {
                w.active_tab_mut().selection.anchor = usize::MAX - 1;
                w.active_tab_mut().selection.head = usize::MAX;
            }
        });
        let before = visual.update(|_, cx| workspace.read(cx).selection());
        visual.dispatch_action(EditTrimTrailingWs);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), source, "refusal {refusal}");
            assert_eq!((w.selection().anchor, w.selection().head), (before.anchor, before.head));
            assert!(w.tabs[w.active].undo_stack.is_empty());
            assert!(w.tabs[w.active].redo_stack.is_empty());
        });
        cx.quit();
    }
}

#[test]
fn registered_trim_trailing_collapsed_caret_snaps_to_new_utf8_boundary() {
    let _settings = crate::settings::override_for_test(Default::default());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "a \né", 3, 3);
    visual.dispatch_action(EditTrimTrailingWs);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), "a\né");
        assert_eq!((w.selection().anchor, w.selection().head), (2, 2));
        assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
    });
    visual.dispatch_action(EditUndo);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), "a \né");
        assert_eq!((w.selection().anchor, w.selection().head), (3, 3));
    });
    cx.quit();
}

#[test]
fn registered_trim_trailing_records_one_event_and_replays_the_selected_scope() {
    let _settings = crate::settings::override_for_test(Default::default());
    let _recorder = crate::macros::override_execution_for_test(true);
    let source = "keep  \r\n café  \r\ntail  ";
    let start = source.find("café").unwrap();
    let end = source.find("tail").unwrap();
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, source, end, start);
    visual.dispatch_action(EditTrimTrailingWs);
    crate::macros::stop_recording();
    let recorded = crate::macros::current();
    assert!(matches!(recorded.as_slice(), [crate::macros::MacroEvent::TrimTrailingWs]));
    visual.dispatch_action(EditUndo);
    visual.dispatch_action(MacroPlayback);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), "keep  \r\n café\r\ntail  ");
        assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        assert!(w.tabs[w.active].edit_transaction.is_none());
    });
    visual.dispatch_action(EditUndo);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), source);
        assert_eq!((w.selection().anchor, w.selection().head), (end, start));
    });
    assert!(matches!(crate::macros::current().as_slice(), [crate::macros::MacroEvent::TrimTrailingWs]));
    cx.quit();
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
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            1.,
            WindowAppearance::Dark,
        );
        w.active_tab_mut().buffer.rope = ropey::Rope::from_str(&source);
        w.active_tab_mut().selection.anchor = anchor;
        w.active_tab_mut().selection.head = head;
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
