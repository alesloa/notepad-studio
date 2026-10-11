// Notepad_plus.cpp:1651–1704 — selected trim expands physical lines, not document.
use crate::{actions::*, workspace::Workspace};
use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

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

#[test]
fn registered_trim_leading_limits_partial_selection_to_its_lines() {
    let _settings = crate::settings::override_for_test(Default::default());
    for reversed in [false, true] {
        let source = "  keep\r\n \tcafé  \r\n  tail";
        let start = source.find("café").unwrap();
        let end = source.find("  tail").unwrap();
        let (anchor, head) = if reversed { (end, start) } else { (start, end) };
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source, anchor, head);
        visual.dispatch_action(EditTrimLeadingWs);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "  keep\r\ncafé  \r\n  tail");
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
            assert_eq!((w.selection().anchor, w.selection().head), (8, end - 2));
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
                "  keep\r\ncafé  \r\n  tail"
            )
        });
        cx.quit();
    }
}

#[test]
fn registered_trim_leading_refuses_rectangular_selection() {
    let _settings = crate::settings::override_for_test(Default::default());
    let source = "  keep\n  café\n  tail";
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
    visual.dispatch_action(EditTrimLeadingWs);
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
fn registered_trim_leading_expands_partial_end_and_preserves_file_format() {
    let _settings = crate::settings::override_for_test(Default::default());
    let source = "\u{FEFF}  keep\r\n  café  \r\n  tail";
    let begin = source.find("café").unwrap() + 1;
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, source, begin, begin + 1);
    visual.update_window_entity(&workspace, |w, _, _| {
        w.active_tab_mut().buffer.encoding = crate::text::encoding::Encoding::Utf16Le;
        w.active_tab_mut().buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
    });
    visual.dispatch_action(EditTrimLeadingWs);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(
            w.buffer().rope.to_string(),
            "\u{FEFF}  keep\r\ncafé  \r\n  tail"
        );
        let start = "\u{FEFF}  keep\r\n".len();
        assert_eq!(
            (w.selection().anchor, w.selection().head),
            (start, start + "café  ".len())
        );
        assert_eq!(
            w.buffer().encoding,
            crate::text::encoding::Encoding::Utf16Le
        );
        assert_eq!(
            w.buffer().line_ending,
            crate::text::line_ending::LineEnding::Crlf
        );
    });
    visual.dispatch_action(EditUndo);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), source);
        assert_eq!(
            (w.selection().anchor, w.selection().head),
            (begin, begin + 1)
        );
    });
    cx.quit();
}

#[test]
fn registered_trim_leading_noop_keeps_reversed_selection_and_history() {
    let _settings = crate::settings::override_for_test(Default::default());
    let source = "  keep\ncafé\n  tail";
    let start = source.find("café").unwrap();
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, source, start + 3, start + 1);
    visual.dispatch_action(EditTrimLeadingWs);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), source);
        assert_eq!(
            (w.selection().anchor, w.selection().head),
            (start + 3, start + 1)
        );
        assert!(w.tabs[w.active].undo_stack.is_empty());
    });
    cx.quit();
}

#[test]
fn registered_trim_leading_refuses_readonly_previews_and_stale_empty_scope() {
    let _settings = crate::settings::override_for_test(Default::default());
    for refusal in 0..5 {
        let source = "  keep\n  café\n  tail";
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
        visual.dispatch_action(EditTrimLeadingWs);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), source, "refusal {refusal}");
            assert_eq!(
                (w.selection().anchor, w.selection().head),
                (before.anchor, before.head)
            );
            assert!(w.tabs[w.active].undo_stack.is_empty());
            assert!(w.tabs[w.active].redo_stack.is_empty());
        });
        cx.quit();
    }
}

#[test]
fn selected_trim_span_uses_crlf_physical_lines_and_clamps_hostile_positions() {
    let source = "a\u{85}  b\r\n  café\r\nlast";
    let rope = ropey::Rope::from_str(source);
    let b = source.find('b').unwrap();
    assert_eq!(
        super::trim_selected_line_range(&rope, b..b + 1),
        0..source.find('\r').unwrap()
    );
    let lf = source.find('\n').unwrap();
    assert_eq!(
        super::trim_selected_line_range(&rope, lf..lf + 1),
        0..lf + 1
    );
    let cafe = source.find("café").unwrap();
    let next_line = source.find("last").unwrap();
    assert_eq!(
        super::trim_selected_line_range(&rope, next_line..cafe),
        lf + 1..next_line
    );
    let span = super::trim_selected_line_range(&rope, cafe + 4..usize::MAX);
    assert_eq!(span, lf + 1..source.len());
    assert!(rope.get_byte_slice(span).is_some());
}
