use super::*;
use gpui::{AppContext, Context, TestAppContext, VisualContext, WindowAppearance};
use crate::{settings::{Settings,override_for_test}, tab::Tab};
#[test]
fn worker_types_are_send() { fn send<T: Send>() {} send::<Request>(); send::<Result<Buffer, Error>>(); }
fn held(action: impl FnOnce(&mut Workspace,&mut Window,&mut Context<Workspace>,Uuid), verify: impl FnOnce(&mut Workspace)) {
    let _settings = override_for_test(Settings::default());
    let fixture = crate::persist::TempDir::new("reload-held"); let path = fixture.join("neutral.txt");
    std::fs::write(&path,"disk replacement").expect("fixture");
    let mut cx = TestAppContext::single();
    let entity = cx.new(|cx| {
        let mut w = Workspace::new(cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),1.,WindowAppearance::Dark);
        w.tabs = vec![Tab::empty(),Tab::empty()]; w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0); w.session_final_save_result = Some(true);
        w.tabs[0].buffer.file_path = Some(path); w.tabs[0].buffer.rope = ropey::Rope::from_str("original"); w.tabs[0].buffer.mark_saved(); w.reload.hold = true; w
    });
    let visual = cx.add_empty_window();
    visual.update(|window,_| window.activate_window()); visual.run_until_parked();
    visual.update_window_entity(&entity, |w,window,cx| { window.focus(&w.focus_handle,cx); w.file_reload_from_disk(&crate::actions::FileReloadFromDisk,window,cx); assert_eq!(w.tabs[0].buffer.rope.to_string(),"original"); });
    visual.run_until_parked();
    visual.update_window_entity(&entity, |w,window,cx| {
        let (id,result) = w.reload.held.take().expect("completed held worker");
        action(w,window,cx,id); w.reload.hold = false; w.finish_reload_worker(id,result,window,cx); verify(w);
    });
    cx.quit();
}
#[test]
fn reload_held_unchanged_publishes_once_and_reorder_tracks_uuid() {
    held(|w,_,_,_| { w.tabs.swap(0,1); w.active = 0; }, |w| { assert_eq!(w.active,0); assert_eq!(w.tabs[1].buffer.rope.to_string(),"disk replacement"); assert!(w.reload.pending.is_none()); });
}
#[test]
fn reload_held_source_edit_cancels_and_preserves_new_history() {
    held(|w,_,cx,_| { w.replace_byte_range(0..0,"typed ",cx); }, |w| { assert_eq!(w.tabs[0].buffer.rope.to_string(),"typed original"); assert_eq!(w.tabs[0].undo_stack.len(),1); assert!(w.tabs[0].buffer.dirty); });
}
#[test]
fn reload_held_cancel_chip_uses_actual_rendered_token() {
    held(|w,_,_,_| { let chip = w.reload_chip().expect("reload chip"); chip.cancel.store(true,Ordering::Relaxed); }, |w| { assert_eq!(w.tabs[0].buffer.rope.to_string(),"original"); assert!(w.reload.pending.is_none()); });
}
#[test]
fn reload_held_window_close_preserves_retained_workspace() {
    held(|w,_,_,_| w.release_session_window(), |w| { assert_eq!(w.tabs[0].buffer.rope.to_string(),"original"); assert!(w.reload.pending.is_none()); });
}
#[test]
fn reload_held_close_reopen_same_uuid_and_rope_refuses_old_reply() {
    held(|w,window,cx,_| { let id = w.tabs[0].session_id; w.close_tab_at(0,window,cx); w.restore_closed_tab(&crate::actions::FileRestoreClosedTab,window,cx); assert!(w.tabs.iter().any(|tab| tab.session_id == id)); }, |w| { assert_eq!(w.tabs[w.active].buffer.rope.to_string(),"original"); });
}
#[test]
fn reload_held_transfer_removal_then_same_object_return_refuses_old_reply() {
    held(|w,window,cx,_| { let tab = w.tabs.remove(0); w.settle_after_tab_removal(0,window,cx); w.tabs.insert(0,tab); w.active = 0; }, |w| assert_eq!(w.tabs[0].buffer.rope.to_string(),"original"));
}
#[test]
fn reload_held_new_status_is_not_replaced_by_old_success() {
    held(|w,_,_,_| w.set_status("newer status"), |w| { assert_eq!(w.tabs[0].buffer.rope.to_string(),"disk replacement"); assert_eq!(w.status_message.as_deref(),Some("newer status")); });
}
#[test]
fn reload_guard_checks_same_length_interior_history_text() {
    let mut tab = Tab::empty(); let op = crate::buffer::EditOp { before_revision: Default::default(), after_revision: Default::default(), byte_start:0,
        removed:"abc".into(), inserted:"def".into(), sel_before_anchor:0, sel_before_head:0 };
    tab.undo_stack.push(crate::buffer::UndoEntry::Group { ops:vec![op.clone(),op.clone(),op],sel_before_anchor:0,sel_before_head:0 });
    let guard = Guard::capture(&tab).expect("guard"); assert!(guard.valid(&tab));
    if let crate::buffer::UndoEntry::Group {ops,..} = &mut tab.undo_stack[0] { ops[1].inserted = "xyz".into(); }
    assert!(!guard.valid(&tab));
}
#[test]
fn reload_guard_refuses_oversized_history_and_open_transactions_before_capture() {
    let mut tab = Tab::empty(); tab.edit_transaction = Some(crate::buffer::EditTransaction {depth:1,ops:Vec::new(),sel_before_anchor:0,sel_before_head:0});
    assert!(Guard::capture(&tab).is_err()); tab.edit_transaction = None;
    tab.buffer.marks = std::iter::repeat_n(0..1,65_537).collect(); assert!(Guard::capture(&tab).is_err());
}

struct PaintedStatus { workspace: gpui::Entity<Workspace> }
impl gpui::Render for PaintedStatus {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        let theme = crate::theme::Theme::for_mode(crate::theme::ThemeMode::Dark);
        let fields = crate::ui::chrome::StatusFields {doc_type:"Normal Text",doc_size:"12 bytes".into(),cur_pos:"Ln1 Col1".into(),line_ending:"LF",encoding:"UTF-8"};
        let chip = self.workspace.read(cx).reload_chip();
        gpui::div().size_full().child(crate::ui::chrome::status_bar(&theme,fields,false,None,chip,self.workspace.clone()))
    }
}
#[test]
fn reload_painted_cancel_button_cancels_the_held_worker_and_preserves_source() {
    let _settings = override_for_test(Settings::default());
    let fixture = crate::persist::TempDir::new("reload-painted-cancel"); let path = fixture.join("neutral.txt"); std::fs::write(&path,"disk replacement").expect("fixture");
    let mut cx = TestAppContext::single();
    let workspace = cx.new(|cx| {
        let mut w = Workspace::new(cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),1.,WindowAppearance::Dark);
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0); w.session_final_save_result = Some(true);
        w.tabs[0].buffer.file_path = Some(path); w.tabs[0].buffer.rope = ropey::Rope::from_str("original"); w.tabs[0].buffer.mark_saved(); w.reload.hold = true; w
    });
    let (painted,visual) = cx.add_window_view(|_,_| PaintedStatus {workspace:workspace.clone()});
    visual.update(|window,_| window.activate_window()); visual.run_until_parked();
    visual.update_window_entity(&workspace, |w,window,cx| w.file_reload_from_disk(&crate::actions::FileReloadFromDisk,window,cx));
    visual.run_until_parked();
    painted.update(visual, |_,cx| cx.notify()); visual.run_until_parked();
    let bounds = visual.debug_bounds("loading-cancel").expect("painted actual cancel control");
    visual.simulate_click(bounds.origin + gpui::point(gpui::px(3.),gpui::px(3.)),gpui::Modifiers::none());
    visual.update_window_entity(&workspace, |w,window,cx| {
        assert!(w.reload.pending.as_ref().expect("pending").request.progress.cancel.load(Ordering::Relaxed));
        let (id,result) = w.reload.held.take().expect("held"); w.reload.hold = false;
        w.finish_reload_worker(id,result,window,cx); assert_eq!(w.tabs[0].buffer.rope.to_string(),"original");
    });
    cx.quit();
}

#[test]
fn reload_held_metadata_and_view_changes_refuse_without_losing_original() {
    for change in 0..9 {
        held(|w,_,_,_| { let t = &mut w.tabs[0]; match change {
            0 => t.buffer.file_path = Some(std::path::PathBuf::from("changed.txt")),
            1 => t.buffer.encoding = crate::text::encoding::Encoding::Utf16Le,
            2 => t.buffer.line_ending = if t.buffer.line_ending == crate::text::line_ending::LineEnding::Crlf { crate::text::line_ending::LineEnding::Lf } else { crate::text::line_ending::LineEnding::Crlf },
            3 => t.buffer.read_only = true,
            4 => t.buffer.tab_color = Some(2),
            5 => t.buffer.language = Some(crate::syntax::LanguageId::Rust),
            6 => t.view_mode = crate::tab::TabViewMode::MarkdownPreview,
            7 => { t.buffer.bookmarks.insert(0); },
            _ => t.begin_select_marker = Some(2),
        } }, |w| assert_eq!(w.tabs[0].buffer.rope.to_string(),"original"));
    }
}
#[test]
fn reload_held_new_request_owns_pending_state_after_old_success_or_error() {
    for error in [false,true] {
        held(|w,window,cx,id| {
            let tab = w.tabs[0].session_id; w.begin_reload(tab,false,None,window,cx);
            let current = w.reload.pending.as_ref().expect("new pending").id; assert_ne!(current,id);
            if error { w.finish_reload_worker(id,Err(Error::Failed("old failure".into())),window,cx); }
        }, |w| {
            assert!(w.reload.pending.is_some()); assert_eq!(w.tabs[0].buffer.rope.to_string(),"original");
            assert_eq!(w.status_message.as_deref(),Some("Reloading document…"));
        });
    }
}
#[test]
fn reload_guard_rich_history_same_revision_and_same_length_mutation_is_detected() {
    let fixture = crate::persist::TempDir::new("reload-rich-history"); let path = fixture.join("neutral.rtf");
    std::fs::write(&path,br"{\rtf1 original}").expect("fixture"); let mut tab = Tab::empty(); tab.buffer = crate::io::read_buffer(&path).expect("rich fixture");
    let payload = tab.buffer.rich_doc.as_mut().expect("payload");
    payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockBold {block:0,on:true});
    let guard = Guard::capture(&tab).expect("guard"); assert!(guard.valid(&tab));
    let payload = tab.buffer.rich_doc.as_mut().expect("payload");
    if let crate::rich_doc::ast::Block::Paragraph {runs,..} = &mut payload.undo_stack[0].ast.blocks[0] { runs[0].text = "modified".into(); }
    assert!(!guard.valid(&tab));
}
#[test]
fn reload_csv_guard_checks_same_length_snapshot_mutation() {
    let mut model = crate::ui::csv_doc::CsvModel::empty(b','); model.rows = vec![vec!["abc".into()]]; model.has_header = false;
    model.undo_stack.push(model.snapshot());
    let guard = super::super::reload_csv::Guard::capture(&model,65_536,8*1024*1024).expect("guard"); assert!(guard.valid(&model));
    model.undo_stack[0].rows[0][0] = "xyz".into(); assert!(!guard.valid(&model));
}
#[test]
fn reload_csv_guard_refuses_excessive_rows_before_copying() {
    let mut model = crate::ui::csv_doc::CsvModel::empty(b','); model.rows = vec![Vec::new();65_537];
    assert!(super::super::reload_csv::Guard::capture(&model,65_536,8*1024*1024).is_err());
}
#[test]
fn reload_capture_writer_never_requests_capacity_past_its_limit() {
    use std::io::Write;
    let mut writer = super::super::reload_guard::Capped {bytes:Vec::new(),maximum:1001};
    for _ in 0..1001 { writer.write_all(b"x").expect("bounded byte"); assert!(writer.bytes.capacity() <= 1001); }
    assert!(writer.write_all(b"x").is_err()); assert_eq!(writer.bytes.len(),1001);
}
#[test]
fn reload_csv_guard_preserves_float_bits_filters_and_search_but_allows_cell_navigation() {
    let mut model = crate::ui::csv_doc::CsvModel::empty(b','); model.rows = vec![vec!["abc".into()]];
    model.col_widths = vec![gpui::px(f32::from_bits(0x7fc00001))];
    let guard = super::super::reload_csv::Guard::capture(&model,65_536,8*1024*1024).expect("guard");
    model.active_cell = Some((0,0)); assert!(guard.valid(&model));
    model.col_widths[0] = gpui::px(f32::from_bits(0x7fc00002)); assert!(!guard.valid(&model));
    model.col_widths[0] = gpui::px(f32::from_bits(0x7fc00001)); assert!(guard.valid(&model));
    model.col_filters.entry(0).or_default().insert("abc".into()); assert!(!guard.valid(&model));
    model.col_filters.clear(); model.find_query = "abc".into(); assert!(!guard.valid(&model));
}
#[test]
fn reload_huge_retry_checks_cancel_and_document_guard_before_starting_worker() {
    for changed in [false,true] {
        held(|w,window,cx,_| {
            w.reload.running = None;
            if changed { w.tabs[0].buffer.read_only = true; }
            else { w.reload.pending.as_ref().expect("pending").request.progress.stop(); }
            w.reload.pending.as_mut().expect("pending").request.allow_huge = true;
            w.start_reload_worker(window,cx);
            assert!(w.reload.running.is_none()); assert!(w.reload.pending.is_none());
        }, |w| assert_eq!(w.tabs[0].buffer.rope.to_string(),"original"));
    }
}
#[test]
fn reload_held_prepared_undo_redo_does_not_discard_intervening_edits() {
    let _execution = crate::macros::override_execution_for_test(false);
    use crate::buffer::{ContentRevision,Selection,UndoEntry,prepared::{PreparedGroup,PrefixEdit}};
    let _settings = override_for_test(Settings::default());
    let fixture = crate::persist::TempDir::new("reload-prepared-aba"); let path = fixture.join("neutral.txt"); std::fs::write(&path,"disk replacement").expect("fixture");
    let mut cx = TestAppContext::single();
    let entity = cx.new(|cx| {
        let mut w = Workspace::new(cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),1.,WindowAppearance::Dark);
        let group = Arc::new(PreparedGroup::build(ropey::Rope::from_str("original"),ContentRevision::default(),Selection::default(),Selection::default(),
            vec![PrefixEdit {range:0..0,text:"    ".into()}],&std::sync::atomic::AtomicBool::new(false)).expect("prepared fixture"));
        let (rope,revision,selection) = group.target(false);
        let mut tab = Tab::empty(); tab.buffer.rope = rope; tab.buffer.raw_revision = revision; tab.selection = selection;
        tab.undo_stack = vec![UndoEntry::Prepared(group)]; tab.buffer.file_path = Some(path); tab.buffer.mark_saved();
        w.tabs = vec![tab]; w.active = 0; w.reload.hold = true;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0); w.session_final_save_result = Some(true); w
    });
    let visual = cx.add_empty_window();
    visual.update(|window,_| window.activate_window()); visual.run_until_parked();
    visual.update_window_entity(&entity,|w,window,cx| { window.focus(&w.focus_handle,cx); w.file_reload_from_disk(&crate::actions::FileReloadFromDisk,window,cx); });
    visual.run_until_parked();
    visual.update_window_entity(&entity,|w,window,cx| w.undo(&crate::actions::EditUndo,window,cx)); visual.run_until_parked();
    visual.update_window_entity(&entity,|w,window,cx| { assert_eq!(w.buffer().rope.to_string(),"original"); w.redo(&crate::actions::EditRedo,window,cx); }); visual.run_until_parked();
    visual.update_window_entity(&entity,|w,window,cx| {
        assert_eq!(w.buffer().rope.to_string(),"    original");
        let (id,result) = w.reload.held.take().expect("held reload"); w.reload.hold = false; w.finish_reload_worker(id,result,window,cx);
        assert_eq!(w.buffer().rope.to_string(),"    original");
    }); cx.quit();
}
#[test]
fn reload_held_noop_flush_boundary_undo_caret_and_unrelated_edits_stay_valid() {
    held(|w,window,cx,_| {
        let activity = w.tabs[0].content_activity.clone();
        let _ = w.apply_edit(0..0,"",cx);
        w.after_edit_flush(cx); w.undo(&crate::actions::EditUndo,window,cx);
        w.tabs[0].selection.collapse_to(2); assert!(w.tabs[0].content_activity == activity);
        w.active = 1; w.replace_byte_range(0..0,"other tab edit",cx); w.active = 0;
        assert!(w.tabs[0].content_activity == activity);
    }, |w| {assert_eq!(w.tabs[0].buffer.rope.to_string(),"disk replacement"); assert_eq!(w.tabs[1].buffer.rope.to_string(),"other tab edit");});
}
#[test]
fn reload_held_supported_macro_undo_redo_preserves_intervening_activity() {
    held(|w,window,cx,_| {
        let tab = &mut w.tabs[0]; let revision = tab.buffer.raw_revision.clone();
        tab.undo_stack.push(crate::buffer::UndoEntry::Single(crate::buffer::EditOp {byte_start:0,removed:String::new(),inserted:"original".into(),
            before_revision:Default::default(),after_revision:revision,sel_before_anchor:0,sel_before_head:0}));
        assert!(w.replay_macro(&[crate::macros::MacroEvent::Undo,crate::macros::MacroEvent::Redo],1,window,cx));
        assert_eq!(w.buffer().rope.to_string(),"original"); assert_eq!(w.tabs[0].undo_stack.len(),1); assert!(w.tabs[0].redo_stack.is_empty());
    }, |w| assert_eq!(w.buffer().rope.to_string(),"original"));
}
#[test]
fn reload_held_relevant_syntax_policy_change_refuses_old_output() {
    held(|w,window,cx,id| {
        let _changed = override_for_test(Settings {large_file_disable_syntax:false,..Default::default()});
        let buffer = Buffer::empty(); w.finish_reload_worker(id,Ok(buffer),window,cx);
        assert_eq!(w.buffer().rope.to_string(),"original"); assert!(w.reload.pending.is_none());
    }, |w| assert_eq!(w.buffer().rope.to_string(),"original"));
}

#[test]
#[ignore = "optimized 12 MiB Reload handler/worker/publication timing"]
fn reload_release_completion_sample() {
    let _settings = override_for_test(Settings::default());
    let fixture = crate::persist::TempDir::new("reload-release"); let path = fixture.join("neutral.txt");
    let disk = "new document\n".repeat(1_048_576); std::fs::write(&path,&disk).expect("fixture");
    for sample in 0..3 {
        let mut cx = TestAppContext::single();
        let entity = cx.new(|cx| {
            let mut w = Workspace::new(cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),1.,WindowAppearance::Dark);
            w.session_snapshot_sink = Some(std::sync::mpsc::channel().0); w.session_final_save_result = Some(true);
            w.tabs[0].buffer.file_path = Some(path.clone()); w.tabs[0].buffer.rope = ropey::Rope::from_str(&"old document\n".repeat(1_048_576));
            w.tabs[0].buffer.mark_saved(); w.reload.hold = true; w
        });
        let visual = cx.add_empty_window(); visual.update(|window,_| window.activate_window()); visual.run_until_parked();
        let handler = visual.update_window_entity(&entity,|w,window,cx| {
            let rope = w.buffer().rope.clone(); let start = std::time::Instant::now();
            w.file_reload_from_disk(&crate::actions::FileReloadFromDisk,window,cx);
            let elapsed = start.elapsed(); assert!(w.buffer().rope.is_instance(&rope)); elapsed
        });
        visual.run_until_parked();
        visual.update_window_entity(&entity,|w,window,cx| {
            let worker = w.reload.pending.as_ref().expect("pending").request.progress.elapsed_nanos.load(Ordering::Relaxed);
            let (id,result) = w.reload.held.take().expect("held"); w.reload.hold = false;
            let start = std::time::Instant::now(); w.finish_reload_worker(id,result,window,cx); let publication = start.elapsed();
            assert_eq!(w.buffer().rope.len_bytes(),disk.len()); assert_eq!(w.buffer().rope.to_string(),disk);
            eprintln!("reload_release sample={sample} bytes={} handler_us={:.3} worker_ms={:.3} publication_ms={:.3}",disk.len(),handler.as_secs_f64()*1e6,worker as f64/1e6,publication.as_secs_f64()*1e3);
        }); cx.quit();
    }
}

#[test]
fn reload_huge_prompt_requires_a_current_request() {
    for change in 0..7 {
        held(|w,window,cx,id| {
            let (sender,receiver) = std::sync::mpsc::channel();
            w.reload.huge_prompt_sink = Some(sender);
            let _policy = if change == 3 {
                let mut settings = crate::settings::snapshot();
                settings.large_file_disable_syntax = !settings.large_file_disable_syntax;
                Some(override_for_test(settings))
            } else { None };
            match change {
                1 => { w.replace_byte_range(0..0,"typed ",cx); }
                2 => { w.reload.pending.as_ref().expect("pending").request.progress.stop(); }
                4 => { w.tabs[0].buffer.read_only = true; }
                5 => { w.csv_models.insert(w.tabs[0].session_id,crate::ui::csv_doc::CsvModel::empty(b',')); }
                6 => { w.release_session_window(); }
                _ => {}
            }
            w.finish_reload_worker(id,Err(Error::Huge(3*1024*1024*1024)),window,cx);
            assert_eq!(receiver.try_recv().ok(),if change == 0 {Some(3*1024*1024*1024)} else {None},"change={change}");
            assert_eq!(w.reload.pending.is_some(),change == 0,"change={change}");
        }, |w| assert_eq!(w.tabs[0].buffer.rope.to_string(),if change == 1 {"typed original"} else {"original"}));
    }
}

#[test]
fn reload_csv_guard_distinguishes_adjacent_width_bit_patterns() {
    for history in [false,true] {
        let mut model = crate::ui::csv_doc::CsvModel::empty(b',');
        model.col_widths = vec![gpui::px(f32::from_bits(1)),gpui::px(f32::from_bits(23))];
        if history { model.undo_stack.push(model.snapshot()); }
        let guard = super::super::reload_csv::Guard::capture(&model,65_536,8*1024*1024).expect("guard");
        assert!(guard.valid(&model));
        let widths = if history { &mut model.undo_stack[0].col_widths } else { &mut model.col_widths };
        *widths = vec![gpui::px(f32::from_bits(12)),gpui::px(f32::from_bits(3))];
        assert!(!guard.valid(&model),"history={history}");
    }
}
