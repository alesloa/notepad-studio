use super::*;
use crate::{persist::TempDir, rich_doc::ast::Block, settings::Settings, tab::{Tab, TabViewMode}};
use gpui::TestAppContext;
use std::io::{Read, Write};

#[derive(Clone, Copy, Debug)]
enum Barrier { None, Table, Image }
#[derive(Clone, Copy, Debug)]
enum Route { Current, All, Opened }

// Original neutral 1px RGBA fixture generated locally with System.Drawing.
const SAMPLE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
    0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00,
    0x01, 0x73, 0x52, 0x47, 0x42, 0x00, 0xae, 0xce, 0x1c, 0xe9, 0x00, 0x00,
    0x00, 0x04, 0x67, 0x41, 0x4d, 0x41, 0x00, 0x00, 0xb1, 0x8f, 0x0b, 0xfc,
    0x61, 0x05, 0x00, 0x00, 0x00, 0x09, 0x70, 0x48, 0x59, 0x73, 0x00, 0x00,
    0x0e, 0xc3, 0x00, 0x00, 0x0e, 0xc3, 0x01, 0xc7, 0x6f, 0xa8, 0x64, 0x00,
    0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x18, 0x57, 0x63, 0x10, 0x91,
    0xd3, 0xf8, 0x0f, 0x00, 0x01, 0xfe, 0x01, 0x5a, 0x6c, 0x70, 0xa8, 0xe2,
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn native_fixture(path: &std::path::Path, docx: bool, barrier: Barrier) {
    let mut entries: Vec<(&str, Vec<u8>)> = Vec::new();
    if docx {
        let middle = match barrier {
            Barrier::None => "",
            Barrier::Table => r#"<w:tbl><w:tblGrid><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>keep café 中 table</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
            Barrier::Image => r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="914400" cy="914400"/><wp:docPr id="1" name="private sample"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="sample.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImage"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
        };
        let xml = format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body><w:p><w:r><w:t>left</w:t></w:r></w:p>{middle}<w:p><w:r><w:t>right</w:t></w:r></w:p></w:body></w:document>"#);
        entries.extend([
            ("[Content_Types].xml", br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_vec()),
            ("_rels/.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_vec()),
            ("word/document.xml", xml.into_bytes()),
            ("word/_rels/document.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/sample.png"/></Relationships>"#.to_vec()),
        ]);
    } else {
        let middle = match barrier {
            Barrier::None => "",
            Barrier::Table => r#"<table:table><table:table-row><table:table-cell><text:p>keep café 中 table</text:p></table:table-cell></table:table-row></table:table>"#,
            Barrier::Image => r#"<draw:frame svg:width="1in" svg:height="1in"><draw:image xlink:href="Pictures/sample.png"/><svg:desc>keep sample image</svg:desc></draw:frame>"#,
        };
        let xml = format!(r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:body><office:text><text:p>left</text:p>{middle}<text:p>right</text:p></office:text></office:body></office:document-content>"#);
        entries.extend([
            ("mimetype", b"application/vnd.oasis.opendocument.text".to_vec()),
            ("content.xml", xml.into_bytes()),
        ]);
    }
    if matches!(barrier, Barrier::Image) {
        entries.push((if docx { "word/media/sample.png" } else { "Pictures/sample.png" }, SAMPLE_PNG.to_vec()));
    }
    entries.push(("custom/preserved.bin", b"unrelated native container data".to_vec()));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn barrier_present(payload: &crate::rich_doc::RichDocPayload, barrier: Barrier) -> bool {
    payload.ast.blocks.iter().any(|b| match barrier {
        Barrier::None => false, Barrier::Table => matches!(b, Block::Table { .. }),
        Barrier::Image => matches!(b, Block::Image { .. }),
    })
}

fn painted_native_barrier(docx: bool, barrier: Barrier, route: Route, extended: bool) {
    painted_native_case(docx, barrier, route, extended, false, false);
}

fn painted_native_case(docx: bool, barrier: Barrier, route: Route, extended: bool, within: bool, sibling: bool) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        confirm_replace_all_in_all_docs: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default()
    });
    let dir = TempDir::new("rich-replace-barrier");
    let path = dir.join(if docx { "sample.docx" } else { "sample.odt" });
    native_fixture(&path, docx, barrier);
    let original_bytes = std::fs::read(&path).unwrap();
    let buffer = crate::io::read_buffer(&path).unwrap();
    let payload = buffer.rich_doc.as_ref().unwrap();
    assert_eq!(payload.block_text(0), "left");
    assert_eq!(payload.block_text(payload.ast.blocks.len() - 1), "right");
    assert_eq!(payload.ast.blocks.len(), if matches!(barrier, Barrier::None) { 2 } else { 3 }, "real native import shape");
    assert_eq!(barrier_present(payload, barrier), !matches!(barrier, Barrier::None), "fixture must import its actual table/image");
    let revision = payload.revision.clone();
    let raw_before = buffer.rope.to_string();
    let plain_path = dir.join("sibling.txt");
    if sibling { std::fs::write(&plain_path, if within { "left" } else { "left\r\nright" }).unwrap(); }
    let mut cx = TestAppContext::single();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::with_buffer(buffer); tab.view_mode = TabViewMode::RichDocView;
        w.tabs = vec![tab]; w.active = 0;
        if sibling { w.tabs.push(Tab::with_buffer(crate::io::read_buffer(&plain_path).unwrap())); }
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = if within { "left" } else if extended { "left\\nright" } else { "left\nright" }.into();
        w.find_state.extended = extended; w.find_state.replacement = "joined café 中".into();
        w.dialog_tab = DialogTab::Replace; w.find_state.open = true;
        w.refresh_matches(); w
    });
    visual.run_until_parked();
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| crate::ui::find_dialog::FindDialog::new(weak, window, cx));
    dialog.update(|window, cx| { window.activate_window(); window.focus(&entity.read(cx).find_focus.clone(), cx); });
    dialog.run_until_parked();
    let button = dialog.debug_bounds(match route {
        Route::Current => "dialog-button-Replace", Route::All => "dialog-button-Replace All",
        Route::Opened => "dialog-button-Replace All in All Opened Documents",
    }).unwrap();
    dialog.simulate_click(button.center(), gpui::Modifiers::none());
    visual.update(|_, cx| entity.update(cx, |w, _| {
        let retained = barrier_present(w.buffer().rich_doc.as_ref().unwrap(), barrier);
        let blocks = w.buffer().rich_doc.as_ref().unwrap().ast.blocks.len();
        let edits = w.buffer().rich_doc.as_ref().unwrap().edit_log.len();
        let save = w.write_active_to_disk();
        let saved_bytes = std::fs::read(&path).unwrap();
        let reopened = crate::rich_doc::open(&path).unwrap();
        eprintln!("actual native barrier route: docx={docx} barrier={barrier:?} route={route:?} extended={extended}; retained={retained}; blocks={blocks}; edits={edits}; status={:?}; save={save:?}; disk_changed={}; reopened_barrier={}",
            w.find_state.status, original_bytes != saved_bytes, barrier_present(&reopened, barrier));
        assert_eq!(w.buffer().rope.to_string(), raw_before, "Doc View never mutates raw source");
        if within {
            assert!(retained, "within-paragraph edit keeps native barrier");
            assert_eq!(blocks, 3); assert_eq!(edits, 2);
            assert_eq!(w.buffer().rich_doc.as_ref().unwrap().block_text(0), "joined café 中");
            assert!(save.is_err(), "existing native Save refuses edited unsupported content");
            assert_eq!(saved_bytes, original_bytes, "safe native Save refusal keeps original container");
        } else if matches!(barrier, Barrier::None) {
            assert!(save.is_ok()); assert_eq!(blocks, 1); assert!(edits > 0);
            assert_eq!(reopened.block_text(0), "joined café 中");
        } else {
            assert!(retained, "replacement must not delete a skipped native table/image");
            assert_eq!(blocks, 3); assert_eq!(edits, 0);
            assert_eq!(w.buffer().rich_doc.as_ref().unwrap().revision, revision);
            assert_eq!(saved_bytes, original_bytes, "refused/no-match barrier operation preserves native file byte-for-byte");
            assert!(barrier_present(&reopened, barrier));
            let status = w.find_state.status.as_deref().unwrap();
            assert!(status.contains("table or image"), "actionable barrier refusal: {status}");
            if !sibling { assert!(!status.starts_with("Replaced"), "never claim an unsupported hidden-span replacement succeeded"); }
        }
        if sibling {
            assert_eq!(w.active, 0);
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "joined café 中", "valid sibling still replaced");
            assert_eq!(w.tabs[1].undo_stack.len(), 1);
            let expected = if within {
                "Replaced 2 occurrences across 2 documents"
            } else {
                "Replaced 1 occurrence across 1 document; replacement refused in 1 document: A match crosses a table or image; replace within individual paragraphs"
            };
            assert_eq!(w.find_state.status.as_deref(), Some(expected));
            w.active = 1; w.write_active_to_disk().unwrap(); w.active = 0;
            assert_eq!(std::fs::read_to_string(&plain_path).unwrap(), "joined café 中");
        }
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(saved_bytes)).unwrap();
        let mut extra = Vec::new(); archive.by_name("custom/preserved.bin").unwrap().read_to_end(&mut extra).unwrap();
        assert_eq!(extra, b"unrelated native container data");
    }));
    cx.quit();
}

#[test] fn native_docx_table_current_extended_barrier() { painted_native_barrier(true, Barrier::Table, Route::All, true); }
#[test] fn native_docx_table_opened_literal_barrier() { painted_native_barrier(true, Barrier::Table, Route::Opened, false); }
#[test] fn native_odt_table_current_literal_barrier() { painted_native_barrier(false, Barrier::Table, Route::All, false); }
#[test] fn native_odt_table_opened_extended_barrier() { painted_native_barrier(false, Barrier::Table, Route::Opened, true); }
#[test] fn native_docx_image_current_extended_barrier() { painted_native_barrier(true, Barrier::Image, Route::All, true); }
#[test] fn native_docx_image_opened_literal_barrier() { painted_native_barrier(true, Barrier::Image, Route::Opened, false); }
#[test] fn native_odt_image_current_literal_barrier() { painted_native_barrier(false, Barrier::Image, Route::All, false); }
#[test] fn native_odt_image_opened_extended_barrier() { painted_native_barrier(false, Barrier::Image, Route::Opened, true); }
#[test] fn native_docx_table_replace_current_barrier() { painted_native_barrier(true, Barrier::Table, Route::Current, true); }
#[test] fn native_odt_image_replace_current_barrier() { painted_native_barrier(false, Barrier::Image, Route::Current, true); }
#[test] fn native_docx_adjacent_current_positive() { painted_native_barrier(true, Barrier::None, Route::All, true); }
#[test] fn native_docx_adjacent_opened_positive() { painted_native_barrier(true, Barrier::None, Route::Opened, false); }
#[test] fn native_odt_adjacent_current_positive() { painted_native_barrier(false, Barrier::None, Route::All, false); }
#[test] fn native_odt_adjacent_opened_positive() { painted_native_barrier(false, Barrier::None, Route::Opened, true); }
#[test] fn native_docx_table_within_paragraph_current_positive() { painted_native_case(true, Barrier::Table, Route::All, false, true, false); }
#[test] fn native_odt_image_within_paragraph_current_positive() { painted_native_case(false, Barrier::Image, Route::All, true, true, false); }
#[test] fn native_docx_image_within_paragraph_opened_sibling_positive() { painted_native_case(true, Barrier::Image, Route::Opened, false, true, true); }
#[test] fn native_odt_table_within_paragraph_opened_sibling_positive() { painted_native_case(false, Barrier::Table, Route::Opened, true, true, true); }
#[test] fn native_docx_table_opened_refusal_keeps_valid_sibling() { painted_native_case(true, Barrier::Table, Route::Opened, true, false, true); }
#[test] fn native_odt_image_opened_refusal_keeps_valid_sibling() { painted_native_case(false, Barrier::Image, Route::Opened, false, false, true); }

// The later adjacent pair is visited first by reverse-order Replace All.
// A per-match guard would edit it before discovering the earlier table span.
fn painted_native_mixed_ranges(route: Route, outer_group: bool) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        confirm_replace_all_in_all_docs: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default()
    });
    let dir = TempDir::new("rich-replace-mixed-ranges");
    let path = dir.join("sample.docx");
    native_fixture(&path, true, Barrier::Table);
    let original = std::fs::read(&path).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(original)).unwrap();
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().to_owned();
        let mut bytes = Vec::new(); entry.read_to_end(&mut bytes).unwrap();
        if name == "word/document.xml" {
            let xml = String::from_utf8(bytes).unwrap();
            bytes = xml.replace("</w:body>",
                "<w:p><w:r><w:t>left</w:t></w:r></w:p><w:p><w:r><w:t>right</w:t></w:r></w:p></w:body>").into_bytes();
        }
        entries.push((name, bytes));
    }
    drop(archive);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
    let original_bytes = std::fs::read(&path).unwrap();
    let mut buffer = crate::io::read_buffer(&path).unwrap();
    let payload = buffer.rich_doc.as_mut().unwrap();
    assert_eq!(payload.ast.blocks.len(), 5, "real native five-block import");
    assert!(matches!(payload.ast.blocks[1], Block::Table { .. }));
    assert_eq!(payload.flat_search_text().0, "left\nright\nleft\nright");
    // Seed both histories through production edit/undo operations, not fake stacks.
    payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockBold { block: 0, on: true });
    payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockItalic { block: 0, on: true });
    assert!(payload.undo());
    assert_eq!((payload.undo_stack.len(), payload.redo_stack.len()), (1, 1));
    if outer_group { assert_eq!(payload.begin_edit_group(), None); }
    let before_ast = payload.ast.clone();
    let before_revision = payload.revision.clone();
    let before_log = payload.edit_log.clone();
    let before_undo = format!("{:?}", payload.undo_stack);
    let before_redo = format!("{:?}", payload.redo_stack);
    let before_group = payload.edit_group_start;
    let before_selection = payload.selection;
    let before_active = payload.active_block;
    let raw_before = buffer.rope.to_string();
    let mut cx = TestAppContext::single();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::with_buffer(buffer); tab.view_mode = TabViewMode::RichDocView;
        w.tabs = vec![tab]; w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = "left\\nright".into(); w.find_state.extended = true;
        w.find_state.replacement = "joined café 中".into();
        w.dialog_tab = DialogTab::Replace; w.find_state.open = true;
        w.refresh_matches(); assert_eq!(w.find_state.matches.len(), 2); w
    });
    visual.run_until_parked();
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| crate::ui::find_dialog::FindDialog::new(weak, window, cx));
    dialog.update(|window, cx| { window.activate_window(); window.focus(&entity.read(cx).find_focus.clone(), cx); });
    dialog.run_until_parked();
    let button = dialog.debug_bounds(match route {
        Route::All => "dialog-button-Replace All",
        Route::Opened => "dialog-button-Replace All in All Opened Documents",
        Route::Current => unreachable!(),
    }).unwrap();
    dialog.simulate_click(button.center(), gpui::Modifiers::none());
    visual.update(|_, cx| entity.update(cx, |w, _| {
        assert_eq!(w.buffer().rope.to_string(), raw_before);
        let payload = w.active_tab_mut().buffer.rich_doc.as_mut().unwrap();
        assert_eq!(payload.ast, before_ast, "preflight refuses safe and blocked matches together");
        assert_eq!(payload.revision, before_revision);
        assert_eq!(payload.edit_log, before_log);
        assert_eq!(format!("{:?}", payload.undo_stack), before_undo);
        assert_eq!(format!("{:?}", payload.redo_stack), before_redo);
        assert_eq!(payload.edit_group_start, before_group, "no group was opened or closed");
        assert_eq!(payload.selection, before_selection);
        assert_eq!(payload.active_block, before_active);
        assert!(payload.redo(), "accepted Redo remains usable after refusal");
        assert!(payload.undo()); assert_eq!(payload.ast, before_ast);
        assert_eq!(payload.edit_log, before_log);
        let status = w.find_state.status.as_deref().unwrap();
        assert!(status.contains("table or image"), "actionable refusal: {status}");
        assert!(!status.starts_with("Replaced"), "safe pair must not be partially replaced: {status}");
        assert!(w.write_active_to_disk().is_err(), "existing style edit with table retains safe Save refusal");
        assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
        let reopened = crate::rich_doc::open(&path).unwrap();
        assert!(barrier_present(&reopened, Barrier::Table));
        assert_eq!(reopened.ast.blocks.len(), 5);
        assert_eq!(reopened.flat_search_text().0, "left\nright\nleft\nright");
    }));
    cx.quit();
}

#[test] fn native_docx_mixed_ranges_all_preflight_preserves_history() { painted_native_mixed_ranges(Route::All, false); }
#[test] fn native_docx_mixed_ranges_opened_preflight_preserves_outer_group() { painted_native_mixed_ranges(Route::Opened, true); }
