use super::*;
use crate::{persist::TempDir, settings::Settings, tab::{Tab, TabViewMode}};
use gpui::TestAppContext;

fn write_docx(path: &std::path::Path) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, xml) in [
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#),
        ("_rels/.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
        ("word/document.xml", r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>hit café 中</w:t></w:r></w:p></w:body></w:document>"#),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn painted_rich_opened(extended: bool, raw: bool, read_only: bool, markup: bool, docx: bool) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        confirm_replace_all_in_all_docs: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default()
    });
    let dir = TempDir::new("literal-opened-rich");
    let rich_path = dir.join(if docx { "sample.docx" } else { "sample.rtf" });
    if docx { write_docx(&rich_path); }
    else { std::fs::write(&rich_path, br"{\rtf1\ansi hit caf\u233? \u20013?\par hit}").unwrap(); }
    let plain_path = dir.join("sample.txt");
    let plain_text = "hit café 中\r\nnext";
    let plain_bytes = crate::text::encoding::encode_checked(plain_text,
        crate::text::encoding::Encoding::Utf16Le).unwrap();
    std::fs::write(&plain_path, &plain_bytes).unwrap();
    let original_rich = std::fs::read(&rich_path).unwrap();
    let rich_buffer = crate::io::read_buffer(&rich_path).unwrap();
    let raw_before = rich_buffer.rope.to_string();
    let blocks_before: Vec<_> = (0..if docx { 1 } else { 2 })
        .map(|i| rich_buffer.rich_doc.as_ref().unwrap().block_text(i)).collect();
    assert_eq!(blocks_before[0], "hit café 中", "real imported Unicode fixture");
    if !docx { assert_eq!(blocks_before[1], "hit"); }
    let plain_buffer = crate::io::read_buffer(&plain_path).unwrap();
    let mut cx = TestAppContext::single();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut rich_tab = Tab::with_buffer(rich_buffer);
        rich_tab.view_mode = if raw { TabViewMode::Edit } else { TabViewMode::RichDocView };
        rich_tab.buffer.read_only = read_only;
        w.tabs = vec![rich_tab, Tab::with_buffer(plain_buffer)];
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = if markup { if docx { "w:document" } else { "ansi" } } else { "hit" }.into();
        w.find_state.replacement = if extended { "\\t中" } else { "done" }.into();
        w.find_state.extended = extended;
        w.dialog_tab = DialogTab::Replace;
        w.find_state.open = true;
        w
    });
    visual.run_until_parked();
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| {
        crate::ui::find_dialog::FindDialog::new(weak, window, cx)
    });
    dialog.update(|window, cx| {
        window.activate_window(); window.focus(&entity.read(cx).find_focus.clone(), cx);
    });
    dialog.run_until_parked();
    let button = dialog.debug_bounds("dialog-button-Replace All in All Opened Documents").unwrap();
    dialog.simulate_click(button.center(), gpui::Modifiers::none());
    visual.update(|_, cx| entity.update(cx, |w, _| {
        let blocks_after: Vec<_> = (0..blocks_before.len())
            .map(|i| w.tabs[0].buffer.rich_doc.as_ref().unwrap().block_text(i)).collect();
        let raw_after = w.tabs[0].buffer.rope.to_string();
        let rich_edits = w.tabs[0].buffer.rich_doc.as_ref().unwrap().edit_log.len();
        let raw_undo = w.tabs[0].undo_stack.len();
        let rich_save = if read_only { None } else { Some(w.write_active_to_disk()) };
        w.active = 1;
        let plain_save = w.write_active_to_disk();
        w.active = 0;
        let reopened_blocks = crate::rich_doc::open(&rich_path).map(|p|
            (0..blocks_before.len()).map(|i| p.block_text(i)).collect::<Vec<_>>());
        eprintln!("actual painted all-opened rich: extended={extended} raw={raw} readonly={read_only} markup={markup} docx={docx}; blocks={blocks_after:?}; raw_changed={}; AST_edits={rich_edits}; raw_undo={raw_undo}; status={:?}; rich_save={rich_save:?}; plain_save={plain_save:?}; reopened={reopened_blocks:?}",
            raw_after != raw_before, w.find_state.status);
        assert!(plain_save.is_ok());
        let replacement = if extended { "\t中" } else { "done" };
        let expected_plain = if markup { plain_text.to_string() } else { plain_text.replace("hit", replacement) };
        assert_eq!(std::fs::read(&plain_path).unwrap(),
            crate::text::encoding::encode_checked(&expected_plain, crate::text::encoding::Encoding::Utf16Le).unwrap(),
            "plain sibling save retains Unicode, CRLF and UTF16 BOM");
        if let Some(saved) = rich_save { assert!(saved.is_ok(), "Doc View never creates unsupported raw container edits: {saved:?}"); }
        if markup || read_only {
            assert_eq!(raw_after, raw_before, "no raw mutation for markup-only Doc View query or read-only buffer");
            assert_eq!(blocks_after, blocks_before);
            assert_eq!(rich_edits, 0); assert_eq!(raw_undo, 0);
            assert_eq!(std::fs::read(&rich_path).unwrap(), original_rich);
        } else if raw {
            assert_eq!(blocks_after, blocks_before, "Raw View remains a source edit");
            assert_eq!(raw_after, raw_before.replace("hit", replacement));
            assert_eq!(rich_edits, 0); assert_eq!(raw_undo, 1);
            assert_eq!(std::fs::read_to_string(&rich_path).unwrap(), raw_after);
        } else {
            let expected: Vec<_> = blocks_before.iter().map(|s| s.replace("hit", replacement)).collect();
            assert_eq!(blocks_after, expected, "Doc View replaces the visible AST, not its raw container");
            assert_eq!(raw_after, raw_before); assert!(rich_edits > 0); assert_eq!(raw_undo, 0);
            let reopened = crate::rich_doc::open(&rich_path).unwrap();
            for (i, text) in expected.iter().enumerate() { assert_eq!(&reopened.block_text(i), text); }
        }
    }));
    if !markup && !read_only && !raw {
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            window.activate_window();
            let focus = w.ensure_rich_doc_body_focus(cx);
            window.focus(&focus, cx);
        }));
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::EditUndo);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            for (i, text) in blocks_before.iter().enumerate() {
                assert_eq!(&w.tabs[0].buffer.rich_doc.as_ref().unwrap().block_text(i), text,
                    "one actual Undo reverses every rich replacement");
            }
            assert_eq!(w.tabs[0].buffer.rope.to_string(), raw_before);
            assert!(w.tabs[0].buffer.dirty, "Undo leaves the saved replacement state");
        });
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            w.set_active_tab(1, cx); window.focus(&w.focus_handle.clone(), cx);
        }));
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::EditUndo);
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[1].buffer.rope.to_string(),
            "hit café 中\nnext", "plain sibling retains its own Undo grain"));
    }
    cx.quit();
}

#[test] fn literal_opened_rtf_doc_view_replaces_visible_text_and_saves() { painted_rich_opened(false, false, false, false, false); }
#[test] fn extended_opened_rtf_doc_view_replaces_visible_text_and_saves() { painted_rich_opened(true, false, false, false, false); }
#[test] fn literal_opened_docx_doc_view_replaces_visible_text_and_saves() { painted_rich_opened(false, false, false, false, true); }
#[test] fn literal_opened_rtf_doc_view_markup_query_is_noop() { painted_rich_opened(false, false, false, true, false); }
#[test] fn literal_opened_docx_doc_view_markup_query_is_noop() { painted_rich_opened(false, false, false, true, true); }
#[test] fn literal_opened_rtf_raw_view_preserves_source_route() { painted_rich_opened(false, true, false, false, false); }
#[test] fn literal_opened_read_only_rich_preserves_writable_plain_sibling() { painted_rich_opened(false, false, true, false, false); }

#[test]
fn literal_opened_alternating_views_restore_plain_active_and_each_rich_map() {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        confirm_replace_all_in_all_docs: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default()
    });
    let dir = TempDir::new("literal-opened-map");
    let paths = [dir.join("first.rtf"), dir.join("second.rtf")];
    std::fs::write(&paths[0], br"{\rtf1\ansi hit\par longer hit}").unwrap();
    std::fs::write(&paths[1], br"{\rtf1\ansi prefix without target\par hit different\par hit}").unwrap();
    let raw_before: Vec<_> = paths.iter().map(|path| crate::io::read_buffer(path).unwrap().rope.to_string()).collect();
    let mut cx = TestAppContext::single();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.tabs.clear();
        for path in &paths {
            let mut plain = Tab::empty();
            plain.buffer.rope = ropey::Rope::from_str("hit");
            w.tabs.push(plain);
            let mut rich = Tab::with_buffer(crate::io::read_buffer(path).unwrap());
            rich.view_mode = TabViewMode::RichDocView;
            w.tabs.push(rich);
        }
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = "hit".into(); w.find_state.replacement = "done".into();
        w.dialog_tab = DialogTab::Replace; w.find_state.open = true;
        w
    });
    visual.run_until_parked();
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| crate::ui::find_dialog::FindDialog::new(weak, window, cx));
    dialog.update(|window, cx| { window.activate_window(); window.focus(&entity.read(cx).find_focus.clone(), cx); });
    dialog.run_until_parked();
    let button = dialog.debug_bounds("dialog-button-Replace All in All Opened Documents").unwrap();
    dialog.simulate_click(button.center(), gpui::Modifiers::none());
    visual.update(|_, cx| entity.update(cx, |w, _| {
        assert_eq!(w.active, 0, "restore original plain active tab");
        assert!(w.find_state.rich_doc_block_offsets.is_none(), "active plain view owns refreshed map");
        assert_eq!(w.find_state.status.as_deref(), Some("Replaced 6 occurrences across 4 documents"));
        for index in [0, 2] { assert_eq!(w.tabs[index].buffer.rope.to_string(), "done"); }
        for (index, expected) in [(1, vec!["done", "longer done"]),
            (3, vec!["prefix without target", "done different", "done"])]
        {
            let payload = w.tabs[index].buffer.rich_doc.as_ref().unwrap();
            for (block, text) in expected.iter().enumerate() { assert_eq!(payload.block_text(block), *text); }
            assert_eq!(w.tabs[index].buffer.rope.to_string(), raw_before[(index - 1) / 2]);
            assert_eq!(w.tabs[index].undo_stack.len(), 0);
            w.active = index; w.write_active_to_disk().unwrap();
            let reopened = crate::rich_doc::open(&paths[(index - 1) / 2]).unwrap();
            for (block, text) in expected.iter().enumerate() { assert_eq!(reopened.block_text(block), *text); }
        }
        w.active = 0;
    }));
    cx.quit();
}
