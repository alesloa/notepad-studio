// Synthetic GPUI handler tests; no native visual verification or user-state writes.
use gpui::{
    AppContext, Context, EntityInputHandler, TestAppContext, VisualContext, Window,
    WindowAppearance,
};

use crate::{
    actions::edit::{EditInsertNewline, EditInsertTab, EditRedo, EditUndo},
    rich_doc::{
        RichDocPayload, TextPos, TextSelection,
        ast::{Alignment, Block, BlockProps, InlineRun, InlineStyle},
        edits::Edit,
    },
    tab::{Tab, TabViewMode},
    workspace::Workspace,
};

fn with_workspace(test: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)) {
    let path = std::env::temp_dir().join(format!(
        "notepad-studio-rich-input-{}.rtf",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, br"{\rtf1\ansi seed}").expect("write neutral RTF fixture");
    let mut buffer = crate::io::read_buffer(&path).expect("read neutral RTF fixture");
    std::fs::remove_file(path).expect("remove neutral RTF fixture");
    buffer.file_path = None;
    let payload = buffer.rich_doc.as_mut().expect("rich fixture");
    payload.ast.blocks = vec![
        Block::Paragraph {
            runs: vec![
                InlineRun::plain("\u{feff}café "),
                InlineRun {
                    text: "bold".into(),
                    style: InlineStyle {
                        bold: true,
                        ..Default::default()
                    },
                },
            ],
            props: BlockProps {
                alignment: Alignment::Right,
                indent_level: 2,
                ..Default::default()
            },
        },
        Block::Heading {
            level: 2,
            runs: vec![InlineRun {
                text: "middle".into(),
                style: InlineStyle {
                    italic: true,
                    ..Default::default()
                },
            }],
            props: BlockProps {
                alignment: Alignment::Center,
                ..Default::default()
            },
        },
        Block::Paragraph {
            runs: vec![
                InlineRun::plain("tail "),
                InlineRun {
                    text: "末尾".into(),
                    style: InlineStyle {
                        underline: true,
                        ..Default::default()
                    },
                },
            ],
            props: BlockProps::default(),
        },
    ];
    // Seed prior history so a new action must preserve an existing Undo entry.
    payload.apply_edit(Edit::SetBlockColor {
        block: 0,
        color: Some(0x123456),
    });
    let mut cx = TestAppContext::single();
    let entity = cx.new(move |cx| {
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
        let mut tab = Tab::empty();
        tab.buffer = buffer;
        tab.view_mode = TabViewMode::RichDocView;
        w.tabs = vec![tab];
        w.active = 0;
        // Disable recovery writes before assertions, including failing regressions.
        w.window_closed = true;
        w
    });
    {
        let cx = cx.add_empty_window();
        cx.update_window_entity(&entity, |w, window, cx| {
            let focus = w.ensure_rich_doc_body_focus(cx);
            window.focus(&focus, cx);
            test(w, window, cx);
        });
    }
    cx.quit();
}

fn select(w: &mut Workspace, across_blocks: bool, reversed: bool) {
    let payload = w
        .active_tab_mut()
        .buffer
        .rich_doc
        .as_mut()
        .expect("rich fixture");
    let lo = TextPos {
        block: 0,
        run: 1,
        byte: 1,
    };
    let hi = if across_blocks {
        TextPos {
            block: 2,
            run: 0,
            byte: 2,
        }
    } else {
        TextPos {
            block: 0,
            run: 1,
            byte: 3,
        }
    };
    let (anchor, cursor) = if reversed { (hi, lo) } else { (lo, hi) };
    payload.selection = Some(TextSelection { anchor, cursor });
    payload.active_block = Some(cursor.block);
}

fn assert_state(actual: &RichDocPayload, expected: &RichDocPayload) {
    assert_eq!(
        actual.ast, expected.ast,
        "text, inline styles, and paragraph properties"
    );
    assert_eq!(actual.selection, expected.selection);
    assert_eq!(actual.active_block, expected.active_block);
    assert_eq!(actual.edit_log, expected.edit_log);
    assert!(
        actual.edit_group_start.is_none(),
        "completed handler must end its edit group"
    );
}

fn assert_one_action(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
    action: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
) {
    let before = w.buffer().rich_doc.as_ref().expect("rich fixture").clone();
    action(w, window, cx);
    let after = w.buffer().rich_doc.as_ref().expect("rich fixture").clone();
    assert_ne!(after.ast, before.ast);
    assert_eq!(
        after.undo_stack.len(),
        before.undo_stack.len() + 1,
        "one user action needs one Undo"
    );
    assert!(after.edit_group_start.is_none());
    assert!(after.selection.expect("caret after action").is_collapsed());
    w.undo(&EditUndo, window, cx);
    assert_state(w.buffer().rich_doc.as_ref().expect("rich fixture"), &before);
    w.redo(&EditRedo, window, cx);
    assert_state(w.buffer().rich_doc.as_ref().expect("rich fixture"), &after);
}

#[test]
fn selected_typing_restores_text_styles_and_selection_with_one_undo() {
    for across in [false, true] {
        for reversed in [false, true] {
            for text in ["新é", "新é\r\nnext\rlast"] {
                with_workspace(|w, window, cx| {
                    select(w, across, reversed);
                    assert_one_action(w, window, cx, |w, window, cx| {
                        w.replace_text_in_range(None, text, window, cx);
                        let inserted = text.replace("\r\n", "\n").replace('\r', "\n");
                        let expected = if across {
                            format!("\u{feff}café b{inserted}il 末尾")
                        } else {
                            format!("\u{feff}café b{inserted}d\nmiddle\ntail 末尾")
                        };
                        assert_eq!(
                            w.buffer()
                                .rich_doc
                                .as_ref()
                                .expect("rich fixture")
                                .flat_search_text()
                                .0,
                            expected
                        );
                    });
                });
            }
        }
    }
}

#[test]
fn enter_replaces_selection_and_restores_it_with_one_undo() {
    for across in [false, true] {
        for reversed in [false, true] {
            with_workspace(|w, window, cx| {
                select(w, across, reversed);
                assert_one_action(w, window, cx, |w, window, cx| {
                    w.insert_newline(&EditInsertNewline, window, cx);
                    let expected = if across {
                        "\u{feff}café b\nil 末尾"
                    } else {
                        "\u{feff}café b\nd\nmiddle\ntail 末尾"
                    };
                    assert_eq!(
                        w.buffer()
                            .rich_doc
                            .as_ref()
                            .expect("rich fixture")
                            .flat_search_text()
                            .0,
                        expected
                    );
                });
            });
        }
    }
}

#[test]
fn rich_os_input_preserves_unicode_and_strips_real_c0_controls() {
    with_workspace(|w, window, cx| {
        select(w, true, true);
        assert_one_action(w, window, cx, |w, window, cx| {
            w.replace_text_in_range(None, "\0Ā😀\u{1}中\u{7f}\r\n漢\rtail", window, cx);
            assert_eq!(
                w.buffer()
                    .rich_doc
                    .as_ref()
                    .expect("rich fixture")
                    .flat_search_text()
                    .0,
                "\u{feff}café bĀ😀中\n漢\ntailil 末尾"
            );
        });
    });
}

#[test]
fn tab_replaces_selection_and_restores_it_with_one_undo() {
    for across in [false, true] {
        for reversed in [false, true] {
            with_workspace(|w, window, cx| {
                select(w, across, reversed);
                assert_one_action(w, window, cx, |w, window, cx| {
                    w.insert_tab(&EditInsertTab, window, cx);
                    let expected = if across {
                        "\u{feff}café b\til 末尾"
                    } else {
                        "\u{feff}café b\td\nmiddle\ntail 末尾"
                    };
                    assert_eq!(
                        w.buffer()
                            .rich_doc
                            .as_ref()
                            .expect("rich fixture")
                            .flat_search_text()
                            .0,
                        expected
                    );
                });
            });
        }
    }
}

#[test]
fn no_op_and_read_only_input_preserve_redo() {
    for missing_caret in [false, true] {
        with_workspace(|w, window, cx| {
            select(w, true, false);
            w.rich_doc_insert_text("prior", cx);
            let redo_target = w.buffer().rich_doc.as_ref().expect("rich fixture").clone();
            w.undo(&EditUndo, window, cx);
            if missing_caret {
                let payload = w
                    .active_tab_mut()
                    .buffer
                    .rich_doc
                    .as_mut()
                    .expect("rich fixture");
                payload.selection = None;
                payload.active_block = None;
            }
            let before = w.buffer().rich_doc.as_ref().expect("rich fixture").clone();
            assert!(!before.redo_stack.is_empty());
            w.active_tab_mut().buffer.dirty = false;
            let generation = w.autosave_generation;
            w.replace_text_in_range(None, "", window, cx);
            w.replace_text_in_range(None, "\0\u{1}\u{7f}", window, cx);
            if missing_caret {
                w.replace_text_in_range(None, "ignored\ntext", window, cx);
                w.insert_newline(&EditInsertNewline, window, cx);
                w.insert_tab(&EditInsertTab, window, cx);
            }
            w.active_tab_mut().buffer.read_only = true;
            w.replace_text_in_range(None, "blocked\r\ntext", window, cx);
            w.insert_newline(&EditInsertNewline, window, cx);
            w.insert_tab(&EditInsertTab, window, cx);
            let payload = w.buffer().rich_doc.as_ref().expect("rich fixture");
            assert_state(payload, &before);
            assert_eq!(payload.undo_stack.len(), before.undo_stack.len());
            assert_eq!(payload.redo_stack.len(), before.redo_stack.len());
            assert!(!w.buffer().dirty);
            assert_eq!(w.autosave_generation, generation);
            w.active_tab_mut().buffer.read_only = false;
            w.redo(&EditRedo, window, cx);
            assert_state(
                w.buffer().rich_doc.as_ref().expect("rich fixture"),
                &redo_target,
            );
            assert_eq!(
                w.buffer()
                    .rich_doc
                    .as_ref()
                    .expect("rich fixture")
                    .flat_search_text()
                    .0,
                "\u{feff}café bprioril 末尾"
            );
        });
    }
}

#[test]
fn typing_enter_and_tab_keep_an_outer_edit_group_open() {
    with_workspace(|w, window, cx| {
        select(w, true, false);
        assert_one_action(w, window, cx, |w, window, cx| {
            w.with_rich_doc_edit_group(cx, |w, cx| {
                w.replace_text_in_range(None, "新\r\nnext", window, cx);
                assert!(
                    w.buffer()
                        .rich_doc
                        .as_ref()
                        .expect("rich fixture")
                        .edit_group_start
                        .is_some()
                );
                w.insert_newline(&EditInsertNewline, window, cx);
                assert!(
                    w.buffer()
                        .rich_doc
                        .as_ref()
                        .expect("rich fixture")
                        .edit_group_start
                        .is_some()
                );
                w.insert_tab(&EditInsertTab, window, cx);
                assert!(
                    w.buffer()
                        .rich_doc
                        .as_ref()
                        .expect("rich fixture")
                        .edit_group_start
                        .is_some()
                );
            });
        });
    });
}
