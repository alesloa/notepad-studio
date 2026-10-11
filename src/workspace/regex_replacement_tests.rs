// Real FindDialog mouse routing and Workspace action dispatch. Synthetic state only.
use gpui::{Entity, EntityInputHandler, TestAppContext, VisualTestContext, WindowAppearance};

use super::Workspace;
use crate::{actions::*, find_state::DialogTab, settings::Settings, ui::find_dialog::FindDialog};

pub(super) fn fixture<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    query: &str,
    replacement: &str,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (workspace, _) = cx.add_window_view(|_, cx| {
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
        w.active_tab_mut().buffer.rope = ropey::Rope::from_str(text);
        w.window_closed = true;
        w.dialog_tab = DialogTab::Replace;
        w.find_state.open = true;
        w.find_state.query = query.into();
        w.find_state.replacement = replacement.into();
        w.find_state.regex = true;
        w.find_state.match_case = true;
        w.refresh_matches();
        w
    });
    let weak = workspace.downgrade();
    let (_, visual) = cx.add_window_view(|window, cx| FindDialog::new(weak, window, cx));
    visual.update(|window, cx| {
        window.activate_window();
        let focus = workspace.read(cx).find_focus.clone();
        window.focus(&focus, cx);
    });
    visual.run_until_parked();
    (workspace, visual)
}

fn click(visual: &mut VisualTestContext, label: &str) {
    let selector = match label {
        "Replace" => "dialog-button-Replace",
        "Replace All" => "dialog-button-Replace All",
        "All Opened" => "dialog-button-Replace All in All Opened Documents",
        _ => panic!("unknown fixture button"),
    };
    let bounds = visual.debug_bounds(selector).unwrap();
    visual.simulate_click(bounds.center(), gpui::Modifiers::none());
    visual.run_until_parked();
}

fn assert_text(visual: &mut VisualTestContext, w: &Entity<Workspace>, expected: &str) {
    visual.update(|_, cx| assert_eq!(w.read(cx).buffer().rope.to_string(), expected));
}

#[test]
fn dialog_replace_expands_numbered_capture() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "one-12 two-34", r"(\w+)-(\d+)", r"\2:$1");
    click(visual, "Replace");
    assert_text(visual, &w, "12:one two-34");
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            window.focus(&w.focus_handle, cx);
            w.undo(&EditUndo, window, cx);
        })
    });
    assert_text(visual, &w, "one-12 two-34");
    cx.quit();
}

#[test]
fn dialog_replace_all_expands_named_optional_and_literal_tokens() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(
        &mut cx,
        "é1 中",
        r"(?<word>\pL+)(\d)?",
        r"$+{word}/${2}/\$/\\/\($&\)",
    );
    click(visual, "Replace All");
    assert_text(visual, &w, "é/1/$/\\/(é1) 中//$/\\/(中)");
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
            window.focus(&w.focus_handle, cx);
            w.undo(&EditUndo, window, cx);
        })
    });
    assert_text(visual, &w, "é1 中");
    cx.quit();
}

#[test]
fn regex_zero_length_replace_all_inserts_once_per_unicode_boundary() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "é\n中", "^", ">");
    click(visual, "Replace All");
    assert_text(visual, &w, ">é\n>中");
    cx.quit();
}

#[test]
fn unsupported_template_refuses_before_history_or_text_changes() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a a", "(a)", r"\U$1\E");
    click(visual, "Replace All");
    assert_text(visual, &w, "a a");
    visual.update(|_, cx| {
        let w = w.read(cx);
        assert!(w
            .find_state
            .error
            .as_ref()
            .is_some_and(|s| s.contains("unsupported")));
        assert!(w.tabs[w.active].undo_stack.is_empty());
    });
    assert!(crate::settings::snapshot()
        .find_history
        .find_query
        .is_empty());
    cx.quit();
}

#[test]
fn dialog_literal_b_and_numeric_escapes_preserve_document_safety() {
    for value in [r"\0", r"\0101", r"\b$0"] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "a", "(a)", value);
        click(visual, "Replace All");
        assert_text(visual, &w, if value == r"\b$0" { "ba" } else { "a" });
        visual.update(|_, cx| {
            let w = w.read(cx);
            if value != r"\b$0" {
                assert!(w.tabs[0].undo_stack.is_empty());
                assert!(w
                    .find_state
                    .error
                    .as_ref()
                    .is_some_and(|s| s.contains("unsupported")));
            }
        });
        cx.quit();
    }
}

#[test]
fn read_only_replace_does_not_report_success_or_change_history() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a", "(a)", "$1$1");
    visual.update(|_, cx| w.update(cx, |w, _| w.active_tab_mut().buffer.read_only = true));
    click(visual, "Replace");
    assert_text(visual, &w, "a");
    visual.update(|_, cx| {
        let w = w.read(cx);
        assert!(w
            .find_state
            .status
            .as_ref()
            .is_none_or(|s| !s.starts_with("Replaced")));
        assert!(w.tabs[w.active].undo_stack.is_empty());
    });
    assert!(crate::settings::snapshot()
        .find_history
        .find_query
        .is_empty());
    cx.quit();
}

#[test]
fn replacement_history_test_override_is_scoped() {
    use crate::settings::{override_for_test, push_find_history, snapshot, FindHistorySlot};
    let _outer = override_for_test(Settings::default());
    push_find_history(FindHistorySlot::FindQuery, "outer");
    {
        let _inner = override_for_test(Settings::default());
        push_find_history(FindHistorySlot::FindQuery, "inner");
        push_find_history(FindHistorySlot::Replacement, "$1");
        assert_eq!(snapshot().find_history.find_query, ["inner"]);
        assert_eq!(snapshot().find_history.replacement, ["$1"]);
    }
    assert_eq!(snapshot().find_history.find_query, ["outer"]);
    assert!(snapshot().find_history.replacement.is_empty());
}

#[test]
fn same_frame_native_query_and_template_round_trips_cancel_pending_replacement() {
    for replacement_field in [false, true] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "a a", "(a)", "$1$1");
        visual.update(|window, cx| {
            w.update(cx, |w, cx| {
                if replacement_field {
                    window.focus(&w.replace_focus, cx);
                }
                w.replace_all_matches(&EditReplaceAll, window, cx);
                w.replace_text_in_range(None, "x", window, cx);
            });
            window.dispatch_action(Box::new(EditBackspace), cx);
        });
        visual.run_until_parked();
        assert_text(visual, &w, "a a");
        visual.update(|_, cx| {
            let w = w.read(cx);
            assert_eq!(w.find_state.query, "(a)");
            assert_eq!(w.find_state.replacement, "$1$1");
            assert!(w.tabs[w.active].undo_stack.is_empty());
        });
        assert!(crate::settings::snapshot()
            .find_history
            .find_query
            .is_empty());
        cx.quit();
    }
}

#[test]
fn same_frame_view_round_trip_cannot_revive_pending_plan() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a a", "(a)", "$1$1");
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            w.active_tab_mut().buffer.file_path =
                Some(std::env::temp_dir().join("regex-neutral.md"));
            w.replace_all_matches(&EditReplaceAll, window, cx);
            w.set_markdown_view_mode(true, cx);
            w.set_markdown_view_mode(false, cx);
        })
    });
    visual.run_until_parked();
    assert_text(visual, &w, "a a");
    cx.quit();
}

#[test]
fn text_read_only_selection_and_dialog_changes_reject_pending_plan() {
    for mutation in 0..4 {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "a a", "(a)", "$1$1");
        visual.update(|window, cx| {
            w.update(cx, |w, cx| {
                w.replace_all_matches(&EditReplaceAll, window, cx);
                match mutation {
                    0 => w.active_tab_mut().buffer.rope = ropey::Rope::from_str("é 中"),
                    1 => w.active_tab_mut().buffer.read_only = true,
                    2 => w.active_tab_mut().selection.collapse_to(1),
                    _ => crate::ui::find_dialog::close_find_dialog(w, window, cx),
                }
            })
        });
        visual.run_until_parked();
        assert_text(visual, &w, if mutation == 0 { "é 中" } else { "a a" });
        visual.update(|_, cx| assert!(w.read(cx).tabs[0].undo_stack.is_empty()));
        assert!(crate::settings::snapshot()
            .find_history
            .find_query
            .is_empty());
        cx.quit();
    }
}

#[test]
fn invalid_template_preserves_existing_redo_and_selection() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a", "(a)", "$2");
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            window.focus(&w.focus_handle, cx);
            w.replace_byte_range(0..1, "aa", cx);
            w.undo(&EditUndo, window, cx);
            assert_eq!(w.tabs[0].redo_stack.len(), 1);
            window.focus(&w.find_focus, cx);
        })
    });
    click(visual, "Replace All");
    assert_text(visual, &w, "a");
    visual.update(|_, cx| assert_eq!(w.read(cx).tabs[0].redo_stack.len(), 1));
    assert!(crate::settings::snapshot()
        .find_history
        .find_query
        .is_empty());
    cx.quit();
}

#[test]
fn regex_noop_replace_preserves_redo() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a", "(a)", "$1");
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            window.focus(&w.focus_handle, cx);
            w.replace_byte_range(0..1, "aa", cx);
            w.undo(&EditUndo, window, cx);
            window.focus(&w.find_focus, cx);
        })
    });
    click(visual, "Replace All");
    visual.update(|_, cx| {
        let w = w.read(cx);
        assert_eq!(w.buffer().rope.to_string(), "a");
        assert!(w.tabs[0].undo_stack.is_empty());
        assert_eq!(w.tabs[0].redo_stack.len(), 1);
    });
    cx.quit();
}

#[test]
fn oversized_or_stale_selection_refuses_atomically() {
    for stale_scope in [false, true] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let original: String = if stale_scope {
            "é".into()
        } else {
            "a".repeat(4097)
        };
        let (w, visual) = fixture(&mut cx, &original, ".", "b");
        if stale_scope {
            visual.update(|_, cx| {
                w.update(cx, |w, _| {
                    w.find_state.in_selection = true;
                    w.find_state.in_selection_scope = Some(1..2);
                })
            });
        }
        click(visual, "Replace All");
        assert_text(visual, &w, &original);
        visual.update(|_, cx| {
            let w = w.read(cx);
            assert!(w.find_state.error.is_some());
            assert!(w.tabs[0].undo_stack.is_empty());
        });
        assert!(crate::settings::snapshot()
            .find_history
            .find_query
            .is_empty());
        cx.quit();
    }
}

#[test]
fn regex_replace_save_and_undo_preserve_unicode_crlf_and_encoding() {
    use crate::text::encoding::{self, Encoding};
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf8Bom,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let directory =
            std::env::temp_dir().join(format!("notepad-studio-regex-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("sample.txt");
        let original = encoding::encode("é1 中2\r\n", encoding);
        std::fs::write(&path, &original).unwrap();
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "", r"(\pL)(\d)", "$2-$1");
        visual.update(|_, cx| {
            w.update(cx, |w, _| {
                w.active_tab_mut().buffer = crate::io::read_buffer(&path).unwrap();
                w.refresh_matches();
            })
        });
        click(visual, "Replace All");
        visual.update(|window, cx| {
            w.update(cx, |w, cx| {
                w.write_active_to_disk().unwrap();
                assert_eq!(
                    std::fs::read(&path).unwrap(),
                    encoding::encode("1-é 2-中\r\n", encoding)
                );
                window.focus(&w.focus_handle, cx);
                w.undo(&EditUndo, window, cx);
                w.write_active_to_disk().unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), original);
            })
        });
        cx.quit();
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn normal_and_extended_replacements_remain_literal() {
    for extended in [false, true] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "a", "a", r"$1\t");
        visual.update(|_, cx| {
            w.update(cx, |w, _| {
                w.find_state.regex = false;
                w.find_state.extended = extended;
            })
        });
        click(visual, "Replace All");
        assert_text(visual, &w, if extended { "$1\t" } else { r"$1\t" });
        cx.quit();
    }
}

#[test]
fn actual_workspace_action_routes_regex_replace_all() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, dialog) = fixture(&mut cx, "é1 中2", r"(\pL)(\d)", "$2:$1");
    let editor = dialog.windows().into_iter().next().unwrap();
    let mut visual = VisualTestContext::from_window(editor, dialog);
    visual.update(|window, cx| {
        window.activate_window();
        let focus = w.read(cx).focus_handle.clone();
        window.focus(&focus, cx);
    });
    visual.run_until_parked();
    visual.dispatch_action(EditReplaceAll);
    visual.run_until_parked();
    assert_text(&mut visual, &w, "1:é 2:中");
    cx.quit();
}

#[test]
fn all_opened_regex_replacements_use_per_tab_captures_and_undo() {
    let _settings = crate::settings::override_for_test(Settings {
        confirm_replace_all_in_all_docs: false,
        ..Settings::default()
    });
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a1", r"(\w)(\d)", "$2-$1");
    visual.update(|_, cx| {
        w.update(cx, |w, _| {
            for (text, read_only) in [("b7 b8", false), ("c9", true)] {
                let mut tab = crate::tab::Tab::empty();
                tab.buffer.rope = ropey::Rope::from_str(text);
                tab.buffer.read_only = read_only;
                w.tabs.push(tab);
            }
        })
    });
    click(visual, "All Opened");
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            assert_eq!(w.active, 0);
            assert_eq!(w.tabs[0].buffer.rope.to_string(), "1-a");
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "7-b 8-b");
            assert_eq!(w.tabs[2].buffer.rope.to_string(), "c9");
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert_eq!(w.tabs[1].undo_stack.len(), 1);
            assert!(w.tabs[2].undo_stack.is_empty());
            window.focus(&w.focus_handle, cx);
            w.undo(&EditUndo, window, cx);
            assert_eq!(w.buffer().rope.to_string(), "a1");
        })
    });
    cx.quit();
}

#[test]
fn rich_regex_replacement_preserves_other_styles_and_has_one_undo() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "", r"(a)(\d)", "$2-$1");
    let path = std::env::temp_dir().join(format!(
        "notepad-studio-regex-rich-{}.rtf",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, br"{\rtf1\ansi \b keep \b0 a1\par a2}").unwrap();
    let mut buffer = crate::io::read_buffer(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    buffer.file_path = None;
    let before = buffer.rich_doc.as_ref().unwrap().ast.clone();
    visual.update(|_, cx| {
        w.update(cx, |w, _| {
            w.active_tab_mut().buffer = buffer;
            w.active_tab_mut().view_mode = crate::tab::TabViewMode::RichDocView;
            w.refresh_matches();
        })
    });
    click(visual, "Replace All");
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            let payload = w.buffer().rich_doc.as_ref().unwrap();
            assert_eq!(payload.flat_search_text().0, "keep 1-a\n2-a");
            assert!(payload.block_runs(0).unwrap()[0].style.bold);
            assert_eq!(payload.undo_stack.len(), 1);
            let focus = w.ensure_rich_doc_body_focus(cx);
            window.focus(&focus, cx);
            w.undo(&EditUndo, window, cx);
            assert_eq!(w.buffer().rich_doc.as_ref().unwrap().ast, before);
        })
    });
    cx.quit();
}

#[test]
#[ignore = "release-only synthetic replacement benchmark; no native paint evidence"]
fn benchmark_large_regex_replacement_pipeline() {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        ..Settings::default()
    });
    let text = format!("{}TARGET\n", "x".repeat(3072)).repeat(4096);
    assert!(text.len() > 10 * 1024 * 1024);
    for (regex, annotations) in [(false, false), (true, false), (true, true)] {
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, &text, "TARGET", "Z");
        visual.update(|_, cx| {
            w.update(cx, |w, _| {
                w.find_state.regex = regex;
                if annotations {
                    w.active_tab_mut().buffer.marks = (0..21).map(|i| i * 2..i * 2 + 1).collect();
                }
            })
        });
        let started = std::time::Instant::now();
        visual.update(|window, cx| {
            w.update(cx, |w, cx| {
                w.replace_all_matches(&EditReplaceAll, window, cx)
            })
        });
        let invocation = started.elapsed();
        visual.run_until_parked();
        let total = started.elapsed();
        visual.update(|_, cx| {
            let w = w.read(cx);
            assert_eq!(w.buffer().rope.len_bytes(), text.len() - 4096 * 5);
            println!("large replacement: regex={regex} annotations={annotations} input={} invocation={invocation:?} total={total:?} capture={:?} prepared-apply={:?}", text.len(), w.regex_replacement.last_capture, w.regex_replacement.last_apply);
        });
        cx.quit();
    }
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "", "TARGET", "Z");
    let rich_text = format!("{}{}", "x".repeat(1024 * 1024 - 64), "TARGET ".repeat(7));
    visual.update(|_, cx| {
        w.update(cx, |w, _| {
            let mut payload =
                crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi fixture}".to_vec()).unwrap();
            payload.ast.blocks = vec![crate::rich_doc::ast::Block::Paragraph {
                runs: vec![crate::rich_doc::ast::InlineRun::plain(&rich_text)],
                props: Default::default(),
            }];
            w.active_tab_mut().buffer.rich_doc = Some(payload);
            w.active_tab_mut().view_mode = crate::tab::TabViewMode::RichDocView;
        })
    });
    let started = std::time::Instant::now();
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            w.replace_all_matches(&EditReplaceAll, window, cx)
        })
    });
    let invocation = started.elapsed();
    visual.run_until_parked();
    let total = started.elapsed();
    visual.update(|_, cx| {
        let w = w.read(cx);
        assert_eq!(w.buffer().rich_doc.as_ref().unwrap().flat_search_text().0, rich_text.replace("TARGET", "Z"));
        println!("rich replacement: input={} edits=7 invocation={invocation:?} total={total:?} capture={:?} prepared-apply={:?}", rich_text.len(), w.regex_replacement.last_capture, w.regex_replacement.last_apply);
    });
    cx.quit();
}

#[test]
fn changed_bytes_and_annotation_work_limits_refuse_all_edits() {
    for annotation_limit in [false, true] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let text = if annotation_limit {
            "a".repeat(128)
        } else {
            "a".repeat(4 * 1024 * 1024 + 1)
        };
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(
            &mut cx,
            &text,
            if annotation_limit { "a" } else { "a+" },
            "",
        );
        if annotation_limit {
            visual.update(|_, cx| {
                w.update(cx, |w, _| {
                    w.active_tab_mut().buffer.marks = vec![0..1; 1000]
                })
            });
        }
        click(visual, "Replace All");
        assert_text(visual, &w, &text);
        visual.update(|_, cx| {
            let w = w.read(cx);
            assert!(w.find_state.error.is_some());
            assert!(w.tabs[0].undo_stack.is_empty());
            if annotation_limit {
                assert_eq!(w.buffer().marks.len(), 1000);
            }
        });
        cx.quit();
    }
}

#[test]
fn refused_newer_invocation_cancels_an_earlier_valid_plan() {
    let _settings = crate::settings::override_for_test(Settings {
        confirm_replace_all_in_all_docs: false,
        ..Settings::default()
    });
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a a", "(a)", "$1$1");
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            let mut tab = crate::tab::Tab::empty();
            tab.view_mode = crate::tab::TabViewMode::CsvPreview;
            w.tabs.push(tab);
            w.replace_current_match(&EditReplaceCurrent, window, cx);
            w.replace_all_in_all_opened(&EditReplaceAllOpened, window, cx);
        })
    });
    visual.run_until_parked();
    assert_text(visual, &w, "a a");
    visual.update(|_, cx| assert!(w.read(cx).tabs[0].undo_stack.is_empty()));
    cx.quit();
}

#[test]
fn all_opened_updates_empty_active_cache_and_keeps_read_only_cache_owner() {
    for read_only in [false, true] {
        let _settings = crate::settings::override_for_test(Settings {
            confirm_replace_all_in_all_docs: false,
            ..Settings::default()
        });
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, if read_only { "a" } else { "x" }, "(a)", "$1$1");
        visual.update(|_, cx| {
            w.update(cx, |w, _| {
                w.active_tab_mut().buffer.read_only = read_only;
                if !read_only {
                    w.find_state.matches = std::iter::once(100..200).collect();
                }
                let mut tab = crate::tab::Tab::empty();
                tab.buffer.rope = ropey::Rope::from_str("a");
                tab.buffer.rich_doc =
                    Some(crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi a}".to_vec()).unwrap());
                tab.view_mode = crate::tab::TabViewMode::RichDocView;
                w.tabs.push(tab);
            })
        });
        click(visual, "All Opened");
        visual.update(|_, cx| {
            let w = w.read(cx);
            assert_eq!(
                w.tabs[1]
                    .buffer
                    .rich_doc
                    .as_ref()
                    .unwrap()
                    .flat_search_text()
                    .0,
                "aa"
            );
            assert!(w.find_state.rich_doc_block_offsets.is_none());
            assert_eq!(
                w.find_state.matches,
                if read_only {
                    std::iter::once(0..1).collect::<Vec<_>>()
                } else {
                    vec![]
                }
            );
        });
        cx.quit();
    }
}

#[test]
fn current_regex_replace_respects_dont_move_preference() {
    for dont_move in [false, true] {
        let _settings = crate::settings::override_for_test(Settings {
            replace_dont_move_to_next: dont_move,
            ..Settings::default()
        });
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "a a", "(a)", "${1}X");
        click(visual, "Replace");
        assert_text(visual, &w, "aX a");
        visual.update(|_, cx| {
            assert_eq!(
                w.read(cx).find_state.current,
                Some(if dont_move { 0 } else { 1 })
            )
        });
        cx.quit();
    }
}

#[test]
fn variable_capture_replacements_update_the_validated_selection_scope() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "xxé1 中2yy", r"(\pL)(\d)", "$2");
    visual.update(|_, cx| {
        w.update(cx, |w, _| {
            w.find_state.in_selection = true;
            w.find_state.in_selection_scope = Some(2..10);
        })
    });
    click(visual, "Replace All");
    assert_text(visual, &w, "xx1 2yy");
    visual.update(|_, cx| assert_eq!(w.read(cx).find_state.in_selection_scope, Some(2..5)));
    cx.quit();
}

#[test]
fn all_opened_confirmation_cannot_apply_newer_fields_or_documents() {
    for (mode, mutation) in (0..3).flat_map(|mode| (0..6).map(move |mutation| (mode, mutation))) {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "a a", "a", "z");
        visual.update(|window, cx| {
            w.update(cx, |w, cx| {
                w.find_state.regex = mode == 2;
                w.find_state.extended = mode == 1;
                let confirmation = w.begin_replace_all_confirmation(cx).unwrap();
                match mutation {
                    0 => {
                        w.find_state.replacement = "other".into();
                    }
                    1 => {
                        w.active_tab_mut().buffer.rope = ropey::Rope::from_str("newer text");
                    }
                    2 => {
                        w.replace_text_in_range(None, "x", window, cx);
                        w.input_backspace(window);
                        assert_eq!(w.find_state.query, "a");
                    }
                    3 => {
                        window.focus(&w.replace_focus, cx);
                        w.replace_text_in_range(None, "x", window, cx);
                        w.input_backspace(window);
                        assert_eq!(w.find_state.replacement, "z");
                    }
                    4 => {
                        w.toggle_find_case(&EditFindToggleCase, window, cx);
                        w.toggle_find_case(&EditFindToggleCase, window, cx);
                    }
                    _ => {
                        window.focus(&w.focus_handle, cx);
                        w.replace_byte_range(0..1, "aa", cx);
                        w.undo(&EditUndo, window, cx);
                        assert_eq!(w.buffer().rope.to_string(), "a a");
                    }
                }
                w.resume_replace_all_confirmation(confirmation, cx);
            })
        });
        visual.run_until_parked();
        assert_text(visual, &w, if mutation == 1 { "newer text" } else { "a a" });
        visual.update(|_, cx| assert!(w.read(cx).tabs[0].undo_stack.is_empty()));
        cx.quit();
    }
}

#[test]
fn valid_confirmed_normal_extended_and_regex_modes_keep_their_replacement_rules() {
    for mode in 0..3 {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "a", "a", "$0\\t");
        visual.update(|_, cx| {
            w.update(cx, |w, cx| {
                w.find_state.regex = mode == 2;
                w.find_state.extended = mode == 1;
                let confirmation = w.begin_replace_all_confirmation(cx).unwrap();
                w.resume_replace_all_confirmation(confirmation, cx);
            })
        });
        visual.run_until_parked();
        assert_text(
            visual,
            &w,
            match mode {
                0 => "$0\\t",
                1 => "$0\t",
                _ => "a\t",
            },
        );
        cx.quit();
    }
}

#[test]
fn rich_regex_cross_block_and_zero_length_endpoints_are_prevalidated() {
    use crate::rich_doc::ast::{Block, BlockProps, InlineRun};
    for (text, pattern, replacement, expected) in [
        ("é\n中", "(é)\\n(中)", "$2-$1", "中-é"),
        ("é\n中", "(?m)^|$", ">", ">é>\n>中>"),
        ("", "^", ">", ">"),
    ] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "", pattern, replacement);
        visual.update(|_, cx| {
            w.update(cx, |w, _| {
                let mut payload =
                    crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi fixture}".to_vec()).unwrap();
                payload.ast.blocks = text
                    .split('\n')
                    .map(|text| Block::Paragraph {
                        runs: if text.is_empty() {
                            vec![]
                        } else {
                            vec![InlineRun::plain(text)]
                        },
                        props: BlockProps::default(),
                    })
                    .collect();
                w.active_tab_mut().buffer.rich_doc = Some(payload);
                w.active_tab_mut().view_mode = crate::tab::TabViewMode::RichDocView;
                w.refresh_matches();
            })
        });
        click(visual, "Replace All");
        visual.update(|window, cx| {
            w.update(cx, |w, cx| {
                assert_eq!(
                    w.buffer().rich_doc.as_ref().unwrap().flat_search_text().0,
                    expected
                );
                assert_eq!(w.buffer().rich_doc.as_ref().unwrap().undo_stack.len(), 1);
                let focus = w.ensure_rich_doc_body_focus(cx);
                window.focus(&focus, cx);
                w.undo(&EditUndo, window, cx);
                assert_eq!(
                    w.buffer().rich_doc.as_ref().unwrap().flat_search_text().0,
                    text
                );
            })
        });
        cx.quit();
    }
}

#[test]
fn unmappable_rich_match_and_excessive_formatting_work_refuse_atomically() {
    use crate::rich_doc::ast::{Block, BlockProps, InlineRun};
    for excessive_work in [false, true] {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(
            &mut cx,
            "",
            if excessive_work { "TARGET" } else { "^" },
            ">",
        );
        visual.update(|_, cx| {
            w.update(cx, |w, _| {
                let mut payload =
                    crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi fixture}".to_vec()).unwrap();
                let mut runs = vec![];
                if excessive_work {
                    runs.push(InlineRun::plain("TARGET ".repeat(128)));
                    runs.extend((0..1499).map(|_| InlineRun::plain("")));
                }
                payload.ast.blocks = if excessive_work {
                    vec![Block::Paragraph {
                        runs,
                        props: BlockProps::default(),
                    }]
                } else {
                    vec![]
                };
                w.active_tab_mut().buffer.rich_doc = Some(payload);
                w.active_tab_mut().view_mode = crate::tab::TabViewMode::RichDocView;
                w.refresh_matches();
            })
        });
        let before =
            visual.update(|_, cx| w.read(cx).buffer().rich_doc.as_ref().unwrap().ast.clone());
        click(visual, "Replace All");
        visual.update(|_, cx| {
            let w = w.read(cx);
            let payload = w.buffer().rich_doc.as_ref().unwrap();
            assert_eq!(payload.ast, before);
            assert!(payload.undo_stack.is_empty());
            assert!(payload.edit_log.is_empty());
            assert!(w.find_state.error.is_some());
        });
        cx.quit();
    }
}

#[test]
fn multiple_rich_documents_refuse_capture_before_any_candidate_clone() {
    let _settings = crate::settings::override_for_test(Settings {
        confirm_replace_all_in_all_docs: false,
        ..Settings::default()
    });
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a", "(a)", "$1$1");
    visual.update(|_, cx| {
        w.update(cx, |w, _| {
            for _ in 0..3 {
                let mut tab = crate::tab::Tab::empty();
                let mut payload =
                    crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi fixture}".to_vec()).unwrap();
                payload.ast.blocks = vec![crate::rich_doc::ast::Block::Paragraph {
                    runs: vec![crate::rich_doc::ast::InlineRun::plain(
                        "a".repeat(1_000_000),
                    )],
                    props: Default::default(),
                }];
                tab.buffer.rich_doc = Some(payload);
                tab.view_mode = crate::tab::TabViewMode::RichDocView;
                w.tabs.push(tab);
            }
        })
    });
    let before = super::rich_replacement::captures_for_test();
    click(visual, "All Opened");
    assert_eq!(super::rich_replacement::captures_for_test(), before);
    assert_text(visual, &w, "a");
    visual.update(|_, cx| {
        let w = w.read(cx);
        assert!(w
            .find_state
            .error
            .as_ref()
            .is_some_and(|s| s.contains("capture")));
        assert!(w.tabs.iter().all(|tab| tab.undo_stack.is_empty()
            && tab
                .buffer
                .rich_doc
                .as_ref()
                .is_none_or(|p| p.edit_log.is_empty())));
    });
    cx.quit();
}

#[test]
fn read_only_rich_document_needs_no_candidate_or_text_capture() {
    let _settings = crate::settings::override_for_test(Settings {
        confirm_replace_all_in_all_docs: false,
        ..Settings::default()
    });
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a", "(a)", "$1$1");
    visual.update(|_, cx| {
        w.update(cx, |w, _| {
            let mut tab = crate::tab::Tab::empty();
            let mut payload =
                crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi fixture}".to_vec()).unwrap();
            payload.ast.blocks = vec![crate::rich_doc::ast::Block::Table { rows: vec![] }];
            tab.buffer.rich_doc = Some(payload);
            tab.buffer.read_only = true;
            tab.view_mode = crate::tab::TabViewMode::RichDocView;
            w.tabs.push(tab);
        })
    });
    let before = super::rich_replacement::captures_for_test();
    click(visual, "All Opened");
    assert_eq!(super::rich_replacement::captures_for_test(), before);
    assert_text(visual, &w, "aa");
    visual.update(|_, cx| {
        assert!(w.read(cx).tabs[1]
            .buffer
            .rich_doc
            .as_ref()
            .unwrap()
            .edit_log
            .is_empty())
    });
    cx.quit();
}

#[test]
fn skipped_read_only_loading_csv_tab_preserves_its_draft() {
    use crate::ui::csv_doc::{model::CsvCellInput, CsvModel};
    let _settings = crate::settings::override_for_test(Settings {
        confirm_replace_all_in_all_docs: false,
        ..Settings::default()
    });
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a", "(a)", "$1$1");
    visual.update(|_, cx| {
        w.update(cx, |w, cx| {
            let mut tab = crate::tab::Tab::empty();
            tab.buffer.rope = ropey::Rope::from_str("cell\n");
            tab.buffer.read_only = true;
            tab.buffer.loading = Some(crate::buffer::LoadProgress {
                bytes_read: 1,
                total_bytes: 2,
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            });
            tab.view_mode = crate::tab::TabViewMode::CsvPreview;
            let mut model = CsvModel::empty(b',');
            model.has_header = false;
            model.rows = vec![vec!["cell".into()]];
            model.row_order = vec![0];
            model.col_order = vec![0];
            model.bind_source(&tab.buffer.rope, &tab.buffer.raw_revision);
            model.grid_focus = Some(cx.focus_handle());
            model.editing_cell = Some((0, 0));
            model.cell_input = Some(CsvCellInput::new(
                cx.focus_handle(),
                "draft café\n中".into(),
            ));
            w.csv_models.insert(tab.session_id, model);
            w.tabs.push(tab);
        })
    });
    click(visual, "All Opened");
    assert_text(visual, &w, "aa");
    visual.update(|_, cx| {
        let w = w.read(cx);
        let tab = &w.tabs[1];
        assert_eq!(tab.buffer.rope.to_string(), "cell\n");
        assert!(tab.buffer.loading.is_some());
        assert!(tab.undo_stack.is_empty());
        let model = &w.csv_models[&tab.session_id];
        assert_eq!(model.cell_input.as_ref().unwrap().buffer, "draft café\n中");
        assert_eq!(model.editing_cell, Some((0, 0)));
    });
    cx.quit();
}


#[test]
fn same_frame_tab_round_trip_cannot_revive_pending_regex_replacement() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (w, visual) = fixture(&mut cx, "a a", "(a)", "$1$1");
    visual.update(|window, cx| {
        w.update(cx, |w, cx| {
            w.tabs.push(crate::tab::Tab::empty());
            w.replace_all_matches(&EditReplaceAll, window, cx);
            w.set_active_tab(1, cx);
            w.set_active_tab(0, cx);
        })
    });
    visual.run_until_parked();
    assert_text(visual, &w, "a a");
    visual.update(|_, cx| assert!(w.read(cx).tabs[0].undo_stack.is_empty()));
    cx.quit();
}

#[test]
fn same_frame_tab_round_trip_invalidates_confirmations_in_every_search_mode() {
    for mode in 0..3 {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = fixture(&mut cx, "a a", "a", "z");
        visual.update(|_, cx| {
            w.update(cx, |w, cx| {
                w.find_state.regex = mode == 2;
                w.find_state.extended = mode == 1;
                w.tabs.push(crate::tab::Tab::empty());
                let confirmation = w.begin_replace_all_confirmation(cx).unwrap();
                w.set_active_tab(1, cx);
                w.set_active_tab(0, cx);
                assert!(!w.replacement_confirmation_valid(&confirmation));
                w.resume_replace_all_confirmation(confirmation, cx);
            })
        });
        visual.run_until_parked();
        assert_text(visual, &w, "a a");
        visual.update(|_, cx| assert!(w.read(cx).tabs[0].undo_stack.is_empty()));
        cx.quit();
    }
}
