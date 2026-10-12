use super::*;
use crate::app::menus::hash_tools_tests::{with_editor, draft};
use gpui::{Action, ClipboardItem, VisualContext, VisualTestContext};
#[derive(Default)]
pub(super) struct Picker(pub(super) Option<Vec<PathBuf>>);
impl Global for Picker {}
fn dialog(source: &mut VisualTestContext, action: impl gpui::Action) -> (Entity<Dialog>, VisualTestContext) {
    source.dispatch_action(action); source.run_until_parked();
    let handle = source.cx.windows().into_iter().find_map(|h| h.downcast::<Dialog>()).unwrap();
    let entity = source.update(|_, cx| handle.entity(cx).unwrap());
    let mut visual = VisualTestContext::from_window(handle.into(), &source.cx);
    visual.update(|window, cx| { window.activate_window(); window.refresh(); let _ = window.draw(cx); }); visual.run_until_parked();
    (entity, visual)
}
fn commit(visual: &mut VisualTestContext, text: &str) {
    visual.update(|window, cx| {
        window.refresh(); let _ = window.draw(cx);
        window.take_registered_input_handler().unwrap().replace_text_in_range(None, text, window, cx);
    }); visual.run_until_parked();
}
fn click(visual: &mut VisualTestContext, selector: &'static str) {
    visual.update(|window, cx| { window.refresh(); let _ = window.draw(cx); });
    let bounds = visual.debug_bounds(selector).unwrap_or_else(|| panic!("missing painted {selector}"));
    visual.simulate_click(bounds.center(), gpui::Modifiers::none()); visual.run_until_parked();
}
fn gate(source: &mut VisualTestContext) -> futures::channel::oneshot::Sender<()> {
    let (tx, rx) = futures::channel::oneshot::channel(); source.update(|_, cx| cx.default_global::<jobs::TestGate>().0 = Some(rx)); tx
}
fn clipboard(visual: &VisualTestContext) -> Option<String> { visual.read_from_clipboard().and_then(|item| item.text()) }
#[test]
fn hash_tools_dialog_actual_input_copy_cut_undo_output_copy_and_close_reopen_preserve_editor() {
    with_editor(|workspace, source| {
        let before = draft(workspace, source); let (entity, mut visual) = dialog(source, ToolsMd5Generate);
        commit(&mut visual, "abc");
        visual.update(|_, cx| { let d = entity.read(cx); assert!(d.ready); assert_eq!(d.fields[1].data.rope.to_string(), "900150983cd24fb0d6963f7d28e17f72"); });
        click(&mut visual, "hash-copy"); assert_eq!(clipboard(&visual).as_deref(), Some("900150983cd24fb0d6963f7d28e17f72"));
        click(&mut visual, "hash-field-0");
        visual.dispatch_action(EditSelectAll); visual.dispatch_action(EditCopy); visual.run_until_parked(); assert_eq!(clipboard(&visual).as_deref(), Some("abc"));
        visual.dispatch_action(EditCut); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).fields[0].data.rope.len_bytes(), 0));
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).fields[0].data.rope.to_string(), "abc"));
        click(&mut visual, "hash-close");
        source.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
        let (reopened, mut visual) = dialog(source, ToolsMd5Generate); assert_eq!(reopened, entity);
        visual.update(|_, cx| assert_eq!(entity.read(cx).fields[0].data.rope.to_string(), "abc")); click(&mut visual, "hash-copy");
        assert_eq!(clipboard(&visual).as_deref(), Some("900150983cd24fb0d6963f7d28e17f72")); assert!(draft(workspace, source) == before);
    });
}
#[test]
fn hash_tools_dialog_per_line_button_copy_nul_and_readonly_output_are_real_callers() {
    with_editor(|workspace, source| {
        let before = draft(workspace, source); let (_, mut visual) = dialog(source, ToolsMd5Generate);
        commit(&mut visual, "abc\r\n\r\nxyz\0ignored"); click(&mut visual, "hash-per-line"); click(&mut visual, "hash-copy");
        assert_eq!(clipboard(&visual).as_deref(), Some("900150983cd24fb0d6963f7d28e17f72\r\n\r\nd16fb36f0911f878998c136191af705e\r\n"));
        click(&mut visual, "hash-field-1");
        commit(&mut visual, "overwrite");
        visual.write_to_clipboard(ClipboardItem::new_string("fresh-output-sentinel".into()));
        visual.simulate_keystrokes("ctrl-a ctrl-c"); visual.run_until_parked();
        assert_eq!(clipboard(&visual).as_deref(), Some("900150983cd24fb0d6963f7d28e17f72\r\n\r\nd16fb36f0911f878998c136191af705e\r\n"));
        assert!(draft(workspace, source) == before);
    });
}
#[test]
fn hash_tools_held_selection_is_retired_by_registered_copy_and_empty_paste_does_not_cancel_it() {
    for copy in [true, false] {
        with_editor(|workspace, source| {
            let before = draft(workspace, source); let tx = gate(source);
            source.dispatch_action(ToolsMd5GenerateSelection); source.run_until_parked();
            source.update(|_, cx| assert!(workspace.read(cx).pending_hash_selection.is_some()));
            if copy { source.dispatch_action(EditCopy); } else { source.write_to_clipboard(ClipboardItem::new_string(String::new())); source.dispatch_action(EditPaste); }
            source.run_until_parked(); tx.send(()).unwrap(); source.run_until_parked();
            assert_eq!(clipboard(source).as_deref(), Some(if copy { "abc" } else { "900150983cd24fb0d6963f7d28e17f72" }));
            assert!(draft(workspace, source) == before);
        });
    }
}
#[test]
fn hash_tools_held_selection_cancel_source_selection_aba_and_tab_incarnation_preserve_clipboard() {
    for mode in 0..4 {
        with_editor(|workspace, source| {
            let tx = gate(source); source.dispatch_action(ToolsMd5GenerateSelection); source.run_until_parked();
            match mode {
                0 => source.simulate_keystrokes("escape"),
                1 => { source.dispatch_action(MoveRight); source.dispatch_action(SelectDocStart); },
                2 => source.update_window_entity(workspace, |w, _, _| { w.active_tab_mut().instance = Arc::new(()); }),
                _ => source.update_window_entity(workspace, |w, window, cx| { window.focus(&w.find_focus, cx); window.focus(&w.focus_handle, cx); }),
            }
            let changed = draft(workspace, source); tx.send(()).unwrap(); source.run_until_parked();
            assert_eq!(clipboard(source).as_deref(), Some("sentinel"), "source lifetime mode {mode}"); assert!(draft(workspace, source) == changed);
        });
    }
}
#[test]
fn hash_tools_empty_refused_intent_does_not_retire_an_existing_eligible_copy() {
    with_editor(|workspace, source| {
        let cancel = Arc::new(AtomicBool::new(false)); let owner = source.update(|_, _| clipboard_ownership::claim(cancel.clone()));
        source.update_window_entity(workspace, |w, _, _| { w.active_tab_mut().selection.collapse_to(0); });
        source.dispatch_action(ToolsMd5GenerateSelection); source.run_until_parked();
        assert!(!cancel.load(Ordering::Relaxed)); source.update(|_, _| assert!(clipboard_ownership::is_current(&owner)));
        source.update_window_entity(workspace, |w, _, _| { w.active_tab_mut().selection.anchor = usize::MAX; });
        let before = draft(workspace, source); source.dispatch_action(ToolsMd5GenerateSelection); source.run_until_parked();
        assert!(!cancel.load(Ordering::Relaxed)); assert!(draft(workspace, source) == before); assert_eq!(clipboard(source).as_deref(), Some("sentinel"));
    });
}
#[test]
fn hash_tools_dialog_held_paste_selection_change_cancel_and_newer_text_never_publish_old_input() {
    for mode in 0..3 {
        with_editor(|workspace, source| {
            let before = draft(workspace, source); let (entity, mut visual) = dialog(source, ToolsMd5Generate); commit(&mut visual, "abc");
            let tx = gate(&mut visual); visual.write_to_clipboard(ClipboardItem::new_string("OLD\nPaste".into())); visual.dispatch_action(EditPaste); visual.run_until_parked();
            match mode { 0 => { click(&mut visual, "hash-cancel"); }, 1 => visual.simulate_keystrokes("left right"), _ => commit(&mut visual, "NEW") }
            let expected = visual.update(|_, cx| entity.read(cx).fields[0].data.rope.to_string());
            tx.send(()).unwrap(); visual.run_until_parked(); visual.update(|_, cx| assert_eq!(entity.read(cx).fields[0].data.rope.to_string(), expected));
            assert!(draft(workspace, source) == before); assert_eq!(clipboard(&visual).as_deref(), Some("OLD\nPaste"));
        });
    }
}
#[test]
fn hash_tools_dialog_budget_and_native_commit_refusals_preserve_input_geometry_and_editor() {
    with_editor(|workspace, source| {
        let before = draft(workspace, source); let (entity, mut visual) = dialog(source, ToolsMd5Generate); commit(&mut visual, "é😀");
        visual.dispatch_action(EditSelectAll); visual.run_until_parked();
        let previous = visual.update(|_, cx| { let d = entity.read(cx); (d.fields[0].data.clone(), d.fields[0].head, d.fields[0].anchor) });
        commit(&mut visual, &"x".repeat(hash_tools::CHUNK + 1));
        let permit = visual.update(|_, cx| cx.default_global::<Budget>().reserve(jobs::MAX_MEMORY - 16 * 1024).unwrap());
        commit(&mut visual, &"x".repeat(16 * 1024));
        visual.update(|_, cx| { let d = entity.read(cx); assert!(Arc::ptr_eq(&d.fields[0].data, &previous.0)); assert_eq!((d.fields[0].head,d.fields[0].anchor), (previous.1,previous.2)); });
        drop(permit); assert!(draft(workspace, source) == before);
    });
}
struct Temp(std::path::PathBuf);
impl Temp { fn new() -> Self { let path = std::env::temp_dir().join(format!("notepad-studio-hash-{}", uuid::Uuid::new_v4())); std::fs::create_dir(&path).unwrap(); Self(path) } }
impl Drop for Temp { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
#[test]
fn hash_tools_file_dialog_choose_copy_raw_bytes_order_and_error_prefix_preserve_source() {
    for (action, algorithm) in [(ToolsMd5GenerateFromFiles.boxed_clone(),Algorithm::Md5), (ToolsSha1GenerateFromFiles.boxed_clone(),Algorithm::Sha1),
        (ToolsSha256GenerateFromFiles.boxed_clone(),Algorithm::Sha256),(ToolsSha512GenerateFromFiles.boxed_clone(),Algorithm::Sha512)] {
        with_editor(|workspace, source| {
            let before = draft(workspace, source); source.update(|window,cx| window.dispatch_action(action,cx)); source.run_until_parked();
            let handle = source.cx.windows().into_iter().find_map(|h| h.downcast::<Dialog>()).unwrap(); let entity = source.update(|_, cx| handle.entity(cx).unwrap());
            let mut visual = VisualTestContext::from_window(handle.into(), &source.cx); visual.update(|window,cx| {window.activate_window();window.refresh();let _=window.draw(cx);});
            let temp = Temp::new(); let first = temp.0.join("one.bin"); let second = temp.0.join("é中.bin"); let missing = temp.0.join("missing.bin");
            let first_bytes = b"\xEF\xBB\xBFa\0b\r\n"; let second_bytes = b"\xFF\xFEa\0\r\0\n\0";
            std::fs::write(&first,first_bytes).unwrap(); std::fs::write(&second,second_bytes).unwrap();
            visual.update(|_,cx| cx.default_global::<Picker>().0=Some(vec![second.clone(),first.clone(),missing])); click(&mut visual,"hash-choose");
            let hash = |bytes: &[u8]| hash_tools::digest_reader(algorithm,&mut std::io::Cursor::new(bytes), &AtomicBool::new(false), &AtomicU64::new(0)).unwrap();
            let expected = format!("{}  é中.bin\r\n{}  one.bin\r\n", hash(second_bytes),hash(first_bytes));
            visual.update(|_,cx| {let d=entity.read(cx); assert!(d.ready);assert!(d.status.contains("missing.bin"));assert_eq!(d.fields[1].data.rope.to_string(),format!("{}\r\n{}\r\n",second.display(),first.display()));});
            click(&mut visual,"hash-copy"); assert_eq!(clipboard(&visual),Some(expected)); assert!(draft(workspace,source)==before);
        });
    }
}
#[test]
fn hash_tools_file_dialog_cancel_and_metadata_limit_preserve_prior_panels_and_clipboard() {
    with_editor(|workspace,source| {
        let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5GenerateFromFiles);
        let tx=gate(&mut visual); let temp=Temp::new();let path=temp.0.join("file.bin");std::fs::write(&path,b"abc").unwrap();
        visual.update(|_,cx|cx.default_global::<Picker>().0=Some(vec![path]));click(&mut visual,"hash-choose");click(&mut visual,"hash-cancel");tx.send(()).unwrap();visual.run_until_parked();
        visual.update(|_,cx|{let d=entity.read(cx);assert!(!d.ready);assert_eq!(d.fields[1].data.rope.len_bytes(),0);assert_eq!(d.fields[2].data.rope.len_bytes(),0);});
        visual.update(|_,cx|cx.default_global::<Picker>().0=Some(vec![PathBuf::new();files::MAX_FILES+1]));click(&mut visual,"hash-choose");
        visual.update(|_,cx| assert!(entity.read(cx).status.contains("4,096"))); assert!(draft(workspace,source)==before);assert_eq!(clipboard(&visual).as_deref(),Some("sentinel"));
    });
}
#[test]
fn hash_tools_registered_selection_and_dialog_input_do_not_record_editor_macro_events() {
    with_editor(|workspace, source| {
        let before=draft(workspace,source);assert!(crate::macros::start_recording());source.dispatch_action(ToolsMd5GenerateSelection);source.run_until_parked();
        let(_,mut visual)=dialog(source,ToolsSha256Generate);commit(&mut visual,"abc");click(&mut visual,"hash-copy");
        crate::macros::stop_recording();assert!(crate::macros::current().is_empty());assert!(draft(workspace,source)==before);
    });
}
#[test]
fn hash_tools_actual_separate_find_registered_dispatch_preserves_focus_query_and_source() {
    with_editor(|workspace, source| {
        source.update_window_entity(workspace, |w, _, _| { w.find_state.query = "find field".into(); });
        source.dispatch_action(EditFind); source.run_until_parked();
        source.update_window_entity(workspace, |w, _, cx| { w.find_state.query = "find field".into(); cx.notify(); });
        let handle = source.cx.windows().into_iter().find_map(|h| h.downcast::<crate::ui::find_dialog::FindDialog>()).unwrap();
        let mut find = VisualTestContext::from_window(handle.into(), &source.cx);
        find.update(|window, cx| { window.activate_window(); window.refresh(); let _ = window.draw(cx); }); find.run_until_parked();
        let before = draft(workspace, source); let focus = find.update(|window, _| window.input_context_token());
        find.dispatch_action(ToolsMd5GenerateSelection); find.run_until_parked();
        assert_eq!(clipboard(&find).as_deref(), Some("900150983cd24fb0d6963f7d28e17f72")); assert!(draft(workspace,source)==before);
        find.update(|window,cx| { assert_eq!(window.input_context_token(),focus); assert!(workspace.read(cx).find_focus.is_focused(window));assert_eq!(workspace.read(cx).find_state.query,"find field"); });
    });
}
#[test]
fn hash_tools_held_separate_find_close_foreground_focus_aba_owner_and_incarnation_refuse() {
    for mode in 0..7 {
        with_editor(|workspace,source| {
            source.dispatch_action(EditFind);source.run_until_parked();
            let handle=source.cx.windows().into_iter().find_map(|h|h.downcast::<crate::ui::find_dialog::FindDialog>()).unwrap();
            let mut find=VisualTestContext::from_window(handle.into(),&source.cx);find.update(|window,cx|{window.activate_window();window.refresh();let _=window.draw(cx);});find.run_until_parked();
            let root=source.update(|_,cx|handle.entity(cx).unwrap());
            let tx=gate(&mut find);find.dispatch_action(ToolsMd5GenerateSelection);find.run_until_parked();
            source.update(|_,cx|assert!(workspace.read(cx).pending_hash_selection.is_some()));
            match mode {
                0=>find.update(|window,_|window.remove_window()),
                1=>source.update(|window,_|window.activate_window()),
                2=>find.update(|window,cx|{let (replace,find)={let w=workspace.read(cx);(w.replace_focus.clone(),w.find_focus.clone())};window.focus(&replace,cx);window.focus(&find,cx);}),
                3=>source.update_window_entity(workspace,|w,_,_|w.window_closed=true),
                4=>source.update_window_entity(workspace,|w,_,_|w.active_tab_mut().instance=Arc::new(())),
                5=>find.update(|window,cx|{let owner=workspace.downgrade();window.replace_root(cx,|window,cx|crate::ui::find_dialog::FindDialog::new(owner,window,cx));}),
                _=>find.update(|_,cx|{
                    let replacement=cx.new(|cx| Workspace::new(cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),cx.focus_handle(),1.,gpui::WindowAppearance::Dark));
                    root.update(cx,|dialog,_|dialog.workspace=replacement.downgrade());
                }),
            }
            let before=draft(workspace,source);tx.send(()).unwrap();source.run_until_parked();
            assert_eq!(clipboard(source).as_deref(),Some("sentinel"),"held Find context mode {mode}");assert!(draft(workspace,source)==before);
        });
    }
}
#[test]
fn hash_tools_unrelated_dialog_selection_action_does_not_claim_source_clipboard() {
    with_editor(|workspace,source|{
        let before=draft(workspace,source);let(_,mut visual)=dialog(source,ToolsMd5Generate);
        let cancel=Arc::new(AtomicBool::new(false));let owner=visual.update(|_,_|clipboard_ownership::claim(cancel.clone()));
        visual.dispatch_action(ToolsMd5GenerateSelection);visual.run_until_parked();assert!(!cancel.load(Ordering::Relaxed));
        visual.update(|_,_|assert!(clipboard_ownership::is_current(&owner)));assert!(draft(workspace,source)==before);assert_eq!(clipboard(&visual).as_deref(),Some("sentinel"));
    });
}
#[test]
fn hash_tools_new_dialog_copy_retires_completed_cut_before_any_editor_history_or_text_effect() {
    with_editor(|workspace,source|{
        let(entity,mut visual)=dialog(source,ToolsMd5Generate);commit(&mut visual,"abc");
        source.update_window_entity(workspace,|w,window,cx|{
            let tab=w.active_tab_mut();tab.buffer.rope=Rope::from_str(&format!("abc\n{}","x".repeat(300_000)));tab.buffer.read_only=false;
            tab.selection.anchor=3;tab.selection.head=0;tab.buffer.bookmarks.clear();tab.buffer.bookmarks.insert(0);
            window.activate_window();window.focus(&w.focus_handle,cx);
        });source.run_until_parked();
        let before=draft(workspace,source);let(tx,rx)=futures::channel::oneshot::channel();
        source.update_window_entity(workspace,|w,_,_|w.prepared_test_publish_gate=Some(rx));source.dispatch_action(CutBookmarkedLines);source.run_until_parked();
        source.update(|_,cx|assert!(workspace.read(cx).pending_prepared_edit.is_some()));
        // Invoke the same Copy button handler without activating the dialog.
        // This synthetic caller isolates ownership from window-focus refusal;
        // the separately clicked Copy control above proves the button route.
        source.update(|_,cx|entity.update(cx,|dialog,cx|dialog.copy_field(true,false,cx)));source.run_until_parked();
        source.update(|window,cx|{assert!(window.is_window_active());assert!(workspace.read(cx).pending_prepared_edit.is_some());});
        assert_eq!(clipboard(source).as_deref(),Some("900150983cd24fb0d6963f7d28e17f72"));
        tx.send(()).unwrap();source.run_until_parked();assert!(draft(workspace,source)==before);
        assert_eq!(clipboard(source).as_deref(),Some("900150983cd24fb0d6963f7d28e17f72"));
    });
}
#[test]
fn hash_tools_new_selection_retires_held_bookmark_copy_and_only_publishes_once() {
    with_editor(|workspace,source|{
        source.update_window_entity(workspace,|w,_,_|{
            w.active_tab_mut().buffer.rope=Rope::from_str(&format!("abc\n{}","x".repeat(300_000)));
        });
        let(tx,rx)=futures::channel::oneshot::channel();source.update_window_entity(workspace,|w,_,_|w.bookmark_copy_test_gate=Some(rx));
        source.dispatch_action(CopyBookmarkedLines);source.run_until_parked();let before=draft(workspace,source);
        source.dispatch_action(ToolsMd5GenerateSelection);source.run_until_parked();tx.send(()).unwrap();source.run_until_parked();
        assert_eq!(clipboard(source).as_deref(),Some("900150983cd24fb0d6963f7d28e17f72"));assert!(draft(workspace,source)==before);
        // One start/finish on the registered editor route: a duplicated global
        // fallback would rotate this token and invalidate the first job.
        source.update(|_,cx|assert!(workspace.read(cx).pending_hash_selection.is_none()));
        source.update(|_,cx|assert_eq!(cx.default_global::<jobs::TestGate>().1,1));
    });
}
#[test]
fn hash_tools_completed_dialog_copy_close_reopen_cancel_input_and_newer_copy_never_publish_old_reply() {
    for mode in 0..4 {
        with_editor(|workspace,source| {
            let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5Generate);commit(&mut visual,"abc");
            let used=visual.update(|_,cx|cx.default_global::<Budget>().used());
            let(tx,rx)=futures::channel::oneshot::channel();visual.update(|_,cx|cx.default_global::<jobs::TestGate>().2=Some(rx));
            click(&mut visual,"hash-copy");
            visual.update(|_,cx|{
                let d=entity.read(cx);assert!(d.copy_cancel.is_some());assert_eq!(d.copy_progress.as_ref().unwrap().load(Ordering::Relaxed),32);
                assert!(cx.global::<Budget>().used()>=used+64,"both completed owned clipboard payloads remain charged until publication/disposal");
            });
            assert_eq!(clipboard(&visual).as_deref(),Some("sentinel"));
            match mode {
                0=>{click(&mut visual,"hash-close");source.update(|window,cx|{window.activate_window();window.focus(&workspace.read(cx).focus_handle.clone(),cx);});let(_,reopened)=dialog(source,ToolsMd5Generate);visual=reopened;},
                1=>click(&mut visual,"hash-cancel"),
                2=>{click(&mut visual,"hash-field-0");commit(&mut visual,"NEW");},
                _=>{source.update(|window,cx|{window.activate_window();window.focus(&workspace.read(cx).focus_handle.clone(),cx);});source.dispatch_action(EditCopy);source.run_until_parked();},
            }
            tx.send(()).unwrap();source.run_until_parked();visual.run_until_parked();
            assert_eq!(clipboard(source).as_deref(),Some(if mode==3 {"abc"} else {"sentinel"}));assert!(draft(workspace,source)==before);
            visual.update(|_,cx|assert!(entity.read(cx).copy_progress.is_none()));
        });
    }
}
#[test]
fn hash_tools_actual_all_four_text_algorithms_copy_independent_abc_vectors() {
    for (action,expected) in [
        (ToolsMd5Generate.boxed_clone(),"900150983cd24fb0d6963f7d28e17f72"),
        (ToolsSha1Generate.boxed_clone(),"a9993e364706816aba3e25717850c26c9cd0d89d"),
        (ToolsSha256Generate.boxed_clone(),"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        (ToolsSha512Generate.boxed_clone(),"ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f")
    ] {
        with_editor(|workspace,source| {
            let before=draft(workspace,source);source.update(|window,cx|window.dispatch_action(action,cx));source.run_until_parked();
            let handle=source.cx.windows().into_iter().find_map(|h|h.downcast::<Dialog>()).unwrap();let mut visual=VisualTestContext::from_window(handle.into(),&source.cx);
            visual.update(|window,cx|{window.activate_window();window.refresh();let _=window.draw(cx);});visual.run_until_parked();commit(&mut visual,"abc");click(&mut visual,"hash-copy");
            assert_eq!(clipboard(&visual).as_deref(),Some(expected));assert!(draft(workspace,source)==before);
        });
    }
}
#[test]
fn hash_tools_selection_worker_strict_ansi_error_retires_prior_intent_without_substitution() {
    with_editor(|workspace,source|{
        source.update_window_entity(workspace,|w,_,_|{let tab=w.active_tab_mut();tab.buffer.rope=Rope::from_str("😀");tab.buffer.encoding=crate::text::encoding::Encoding::Latin1;tab.selection.anchor=4;tab.selection.head=0;});
        let before=draft(workspace,source);let cancel=Arc::new(AtomicBool::new(false));source.update(|_,_|clipboard_ownership::claim(cancel.clone()));
        source.dispatch_action(ToolsMd5GenerateSelection);source.run_until_parked();assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(clipboard(source).as_deref(),Some("sentinel"));assert!(draft(workspace,source)==before);
        source.update(|_,cx|assert!(workspace.read(cx).status_message.as_deref().unwrap().contains("U+1F600")));
    });
}
#[test]
fn hash_tools_dialog_keyboard_crlf_navigation_deletion_and_ime_undo_stay_local() {
    use gpui::{ElementInputHandler,InputHandler};
    with_editor(|workspace,source| {
        let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5Generate);commit(&mut visual,"é😀\r\nx");
        visual.simulate_keystrokes("left left");visual.update(|_,cx|assert_eq!(entity.read(cx).fields[0].head,2));
        visual.simulate_keystrokes("right backspace");visual.run_until_parked();visual.update(|_,cx|assert_eq!(entity.read(cx).fields[0].data.rope.to_string(),"é😀x"));
        visual.dispatch_action(EditUndo);visual.run_until_parked();visual.update(|_,cx|assert_eq!(entity.read(cx).fields[0].data.rope.to_string(),"é😀\r\nx"));
        visual.dispatch_action(EditSelectAll);visual.update(|window,cx| {
            let mut input=ElementInputHandler::new(window.bounds(),entity.clone());input.replace_and_mark_text_in_range(None,"日本😀",Some(2..4),window,cx);
            assert_eq!(input.marked_text_range(window,cx),Some(0..4));assert_eq!(input.selected_text_range(false,window,cx).unwrap().range,2..4);
            input.replace_text_in_range(None,"日本語",window,cx);assert_eq!(input.marked_text_range(window,cx),None);
        });visual.run_until_parked();visual.dispatch_action(EditUndo);visual.run_until_parked();
        visual.update(|_,cx|assert_eq!(entity.read(cx).fields[0].data.rope.to_string(),"é😀\r\nx"));assert!(draft(workspace,source)==before);
    });
}
#[test]
fn hash_tools_registered_loaded_disk_selection_uses_editor_bytes_without_unicode_boms() {
    use crate::text::encoding::{self,Encoding};
    for encoding in [Encoding::Utf8,Encoding::Utf8Bom,Encoding::Utf16Le,Encoding::Utf16Be,Encoding::Latin1] {
        with_editor(|workspace,source| {
            let temp=Temp::new();let path=temp.0.join("source.txt");let text=if encoding==Encoding::Latin1 {"é€\u{81}ÿ\r\n"} else {"é中\r\nnext\r\n"};
            let disk=encoding::encode_checked(text,encoding).unwrap();std::fs::write(&path,&disk).unwrap();let mut buffer=crate::io::read_buffer(&path).unwrap();buffer.read_only=true;
            source.update_window_entity(workspace,|w,_,_|{let tab=w.active_tab_mut();tab.buffer=buffer;tab.selection.anchor=tab.buffer.rope.len_bytes();tab.selection.head=0;});
            source.update(|window,cx| { window.refresh(); let _=window.draw(cx); }); source.run_until_parked();
            let before=draft(workspace,source);source.dispatch_action(ToolsMd5GenerateSelection);source.run_until_parked();
            let expected_bytes=if encoding==Encoding::Latin1 {disk} else {text.as_bytes().to_vec()};
            let expected=hash_tools::digest_reader(Algorithm::Md5,&mut std::io::Cursor::new(expected_bytes),&AtomicBool::new(false),&AtomicU64::new(0)).unwrap();
            assert_eq!(clipboard(source),Some(expected));assert!(draft(workspace,source)==before);
        });
    }
}
#[test]
fn hash_tools_registered_preview_loading_and_source_ime_refuse_without_retiring_prior_intent() {
    use gpui::{ElementInputHandler,InputHandler};
    for mode in 0..3 {
        with_editor(|workspace,source| {
            source.update_window_entity(workspace,|w,_,_|match mode {
                0=>w.active_tab_mut().view_mode=crate::tab::TabViewMode::MarkdownPreview,
                1=>w.active_tab_mut().buffer.loading=Some(crate::buffer::LoadProgress{bytes_read:3,total_bytes:6,cancel:Arc::new(AtomicBool::new(false))}),
                _=>w.active_tab_mut().buffer.read_only=false,
            });
            if mode==2 { source.update(|window,cx|ElementInputHandler::new(window.bounds(),workspace.clone()).replace_and_mark_text_in_range(None,"日本",Some(0..2),window,cx)); }
            source.run_until_parked();let before=draft(workspace,source);let cancel=Arc::new(AtomicBool::new(false));source.update(|_,_|clipboard_ownership::claim(cancel.clone()));
            source.dispatch_action(ToolsMd5GenerateSelection);source.run_until_parked();assert!(!cancel.load(Ordering::Relaxed));assert_eq!(clipboard(source).as_deref(),Some("sentinel"));assert!(draft(workspace,source)==before);
            if mode==2 { source.update(|window,cx|assert!(workspace.read(cx).is_composing_text(window))); }
        });
    }
}
#[test]
fn hash_tools_dialog_tab_and_enter_activate_controls_without_editing_the_last_field() {
    with_editor(|workspace,source| {
        let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5Generate);commit(&mut visual,"abc");
        visual.simulate_keystrokes("tab space");visual.run_until_parked();
        visual.update(|window,cx|{let d=entity.read(cx);assert!(d.per_line);assert!(d.buttons[0].is_focused(window));});
        visual.simulate_keystrokes("tab ctrl-a");visual.write_to_clipboard(ClipboardItem::new_string("output-tab-sentinel".into()));
        visual.simulate_keystrokes("ctrl-c");visual.run_until_parked();assert_eq!(clipboard(&visual).as_deref(),Some("900150983cd24fb0d6963f7d28e17f72\r\n"));
        visual.simulate_keystrokes("tab");visual.update(|window,cx|assert!(entity.read(cx).buttons[1].is_focused(window)));
        visual.write_to_clipboard(ClipboardItem::new_string("copy-button-sentinel".into()));visual.simulate_keystrokes("enter");visual.run_until_parked();
        assert_eq!(clipboard(&visual).as_deref(),Some("900150983cd24fb0d6963f7d28e17f72\r\n"));
        visual.simulate_keystrokes("ctrl-x ctrl-v");visual.run_until_parked();
        visual.update(|_,cx|assert_eq!(entity.read(cx).fields[0].data.rope.to_string(),"abc"));assert!(draft(workspace,source)==before);
    });
}
#[test]
fn hash_tools_retained_native_callbacks_refuse_focus_selection_and_window_lifetime_changes() {
    for mode in 0..5 {
        with_editor(|workspace,source| {
            let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5Generate);commit(&mut visual,"abc");
            let mut handler=visual.update(|window,cx|{window.refresh();let _=window.draw(cx);window.take_registered_input_handler().unwrap()});
            match mode {
                0=>{click(&mut visual,"hash-field-1");click(&mut visual,"hash-field-0");},
                1=>click(&mut visual,"hash-field-1"),
                2=>visual.simulate_keystrokes("left right"),
                3=>{click(&mut visual,"hash-close");source.update(|window,cx|{window.activate_window();window.focus(&workspace.read(cx).focus_handle.clone(),cx);});let(reopened,new_visual)=dialog(source,ToolsMd5Generate);assert_eq!(reopened,entity);visual=new_visual;},
                _=>source.update(|window,cx|{window.activate_window();window.focus(&workspace.read(cx).focus_handle.clone(),cx);}),
            }
            source.run_until_parked();
            let old=visual.update(|_,cx|{let d=entity.read(cx);(d.fields[0].data.clone(),d.fields[0].head,d.fields[0].anchor,d.fields[0].marked.clone())});
            visual.update(|window,cx|{
                assert!(!handler.accepts_text_input(window,cx),"stale callback mode {mode}");assert!(handler.selected_text_range(false,window,cx).is_none());
                handler.replace_text_in_range(None,"late",window,cx);handler.replace_and_mark_text_in_range(None,"遅",Some(0..1),window,cx);handler.unmark_text(window,cx);
                let d=entity.read(cx);assert!(Arc::ptr_eq(&old.0,&d.fields[0].data));assert_eq!((d.fields[0].head,d.fields[0].anchor,d.fields[0].marked.clone()),(old.1,old.2,old.3));
            });
            assert!(draft(workspace,source)==before);assert_eq!(clipboard(&visual).as_deref(),Some("sentinel"));
        });
    }
}
#[test]
fn hash_tools_registered_native_ime_callbacks_refresh_after_their_own_live_commits() {
    with_editor(|workspace,source| {
        let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5Generate);
        visual.update(|window,cx|{
            window.refresh();let _=window.draw(cx);let mut handler=window.take_registered_input_handler().unwrap();assert!(handler.accepts_text_input(window,cx));
            handler.replace_and_mark_text_in_range(None,"日本",Some(0..2),window,cx);assert_eq!(handler.marked_text_range(window,cx),Some(0..2));
            handler.replace_text_in_range(None,"日本語",window,cx);assert!(handler.marked_text_range(window,cx).is_none());handler.unmark_text(window,cx);
            assert_eq!(handler.selected_text_range(false,window,cx).unwrap().range,3..3);
        });visual.run_until_parked();visual.update(|_,cx|assert_eq!(entity.read(cx).fields[0].data.rope.to_string(),"日本語"));
        assert!(draft(workspace,source)==before);assert_eq!(clipboard(&visual).as_deref(),Some("sentinel"));
    });
}
#[test]
fn hash_tools_sha512_file_caption_algorithm_and_golden_digest_agree() {
    with_editor(|workspace,source| {
        let before=draft(workspace,source);let temp=Temp::new();let path=temp.0.join("abc.txt");std::fs::write(&path,b"abc").unwrap();
        let(entity,mut visual)=dialog(source,ToolsSha512GenerateFromFiles);
        assert_eq!(visual.window_title().as_deref(),Some("Generate SHA-512 digest from files"));
        visual.update(|_,cx|cx.default_global::<Picker>().0=Some(vec![path]));click(&mut visual,"hash-choose");
        visual.update(|_,cx|{let d=entity.read(cx);assert!(d.algorithm==Algorithm::Sha512);assert_eq!(d.fields[2].data.rope.to_string(),"ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f  abc.txt\r\n");});
        assert!(draft(workspace,source)==before);
    });
}
#[test]
fn hash_tools_completed_file_results_keep_their_budget_until_cancelled_delivery_is_disposed() {
    for close in [false,true] {
        with_editor(|workspace,source| {
            let before=draft(workspace,source);let temp=Temp::new();let path=temp.0.join("abc.txt");std::fs::write(&path,b"abc").unwrap();
            let(entity,mut visual)=dialog(source,ToolsSha256GenerateFromFiles);let baseline=visual.update(|_,cx|cx.default_global::<Budget>().used());
            let(tx,rx)=futures::channel::oneshot::channel();visual.update(|_,cx|{cx.default_global::<jobs::TestGate>().2=Some(rx);cx.default_global::<Picker>().0=Some(vec![path]);});
            click(&mut visual,"hash-choose");let held=visual.update(|_,cx|cx.default_global::<Budget>().used());assert!(held>baseline);
            click(&mut visual,if close {"hash-close"}else{"hash-cancel"});source.run_until_parked();
            source.update(|_,cx|{assert_eq!(cx.default_global::<Budget>().used(),held);assert_eq!(entity.read(cx).fields[2].data.rope.len_bytes(),0);});
            tx.send(()).unwrap();source.run_until_parked();source.update(|_,cx|assert_eq!(cx.default_global::<Budget>().used(),baseline));
            assert!(draft(workspace,source)==before);assert_eq!(clipboard(source).as_deref(),Some("sentinel"));
        });
    }
}
#[test]
fn hash_tools_reopening_after_output_focus_accepts_live_input_without_losing_retained_text() {
    with_editor(|workspace,source| {
        let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5Generate);commit(&mut visual,"abc");
        click(&mut visual,"hash-field-1");click(&mut visual,"hash-close");
        source.update(|window,cx|{window.activate_window();window.focus(&workspace.read(cx).focus_handle.clone(),cx);});
        let(reopened,mut visual)=dialog(source,ToolsMd5Generate);assert_eq!(reopened,entity);commit(&mut visual,"X");
        visual.update(|_,cx|assert_eq!(entity.read(cx).fields[0].data.rope.to_string(),"abcX"));
        assert!(draft(workspace,source)==before);assert_eq!(clipboard(&visual).as_deref(),Some("sentinel"));
    });
}
#[test]
fn hash_tools_live_native_composition_survives_real_output_publication_before_next_paint() {
    with_editor(|workspace,source| {
        let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5Generate);
        let mut handler=visual.update(|window,cx|{window.refresh();let _=window.draw(cx);window.take_registered_input_handler().unwrap()});
        visual.update(|window,cx|handler.replace_and_mark_text_in_range(None,"日本",Some(2..2),window,cx));
        visual.run_until_parked();
        visual.update(|window,cx|{
            let d=entity.read(cx);assert!(d.ready);assert_eq!(d.fields[1].data.rope.len_bytes(),32);assert_eq!(d.fields[0].data.rope.to_string(),"日本");assert_eq!(d.fields[0].marked,Some(0..2));
            assert!(handler.accepts_text_input(window,cx),"output publication changed neither the live input nor its context");
            handler.replace_and_mark_text_in_range(None,"日本語",Some(3..3),window,cx);
            assert_eq!(handler.marked_text_range(window,cx),Some(0..3));handler.unmark_text(window,cx);
        });visual.run_until_parked();visual.update(|_,cx|assert_eq!(entity.read(cx).fields[0].data.rope.to_string(),"日本語"));
        assert!(draft(workspace,source)==before);assert_eq!(clipboard(&visual).as_deref(),Some("sentinel"));
    });
}
#[test]
fn hash_tools_retained_native_input_refuses_edit_undo_aba_with_original_rope_and_selection() {
    with_editor(|workspace,source| {
        let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5Generate);commit(&mut visual,"abc");
        let old=visual.update(|_,cx|{let field=&entity.read(cx).fields[0];(field.data.clone(),field.head,field.anchor,field.marked.clone())});
        let mut handler=visual.update(|window,cx|{window.refresh();let _=window.draw(cx);window.take_registered_input_handler().unwrap()});
        commit(&mut visual,"X");visual.dispatch_action(EditUndo);visual.run_until_parked();
        visual.update(|window,cx| {
            let d=entity.read(cx);assert!(Arc::ptr_eq(&old.0,&d.fields[0].data));
            assert_eq!((d.fields[0].head,d.fields[0].anchor,d.fields[0].marked.clone()),(old.1,old.2,old.3.clone()));
            assert!(!handler.accepts_text_input(window,cx));assert!(handler.selected_text_range(false,window,cx).is_none());
            handler.replace_text_in_range(None,"late",window,cx);handler.replace_and_mark_text_in_range(None,"遅",Some(0..1),window,cx);
            let d=entity.read(cx);assert!(Arc::ptr_eq(&old.0,&d.fields[0].data));
            assert_eq!((d.fields[0].head,d.fields[0].anchor,d.fields[0].marked.clone()),(old.1,old.2,old.3));
        });
        assert!(draft(workspace,source)==before);assert_eq!(clipboard(&visual).as_deref(),Some("sentinel"));
    });
}
#[test]
fn hash_tools_retained_native_output_refuses_real_result_replacement_before_next_paint() {
    with_editor(|workspace,source| {
        let before=draft(workspace,source);let(entity,mut visual)=dialog(source,ToolsMd5Generate);commit(&mut visual,"abc");
        let(tx,rx)=futures::channel::oneshot::channel();visual.update(|_,cx|cx.default_global::<jobs::TestGate>().2=Some(rx));
        commit(&mut visual,"X");click(&mut visual,"hash-field-1");
        let mut handler=visual.update(|window,cx|{window.refresh();let _=window.draw(cx);window.take_registered_input_handler().unwrap()});
        visual.update(|window,cx| {
            assert!(handler.selected_text_range(false,window,cx).is_some());let mut adjusted=None;
            assert_eq!(handler.text_for_range(0..32,&mut adjusted,window,cx).as_deref(),Some("900150983cd24fb0d6963f7d28e17f72"));
        });
        tx.send(()).unwrap();visual.run_until_parked();
        visual.update(|window,cx| {
            let d=entity.read(cx);assert!(d.ready);assert_eq!(d.fields[0].data.rope.to_string(),"abcX");
            assert_ne!(d.fields[1].data.rope.to_string(),"900150983cd24fb0d6963f7d28e17f72");assert!(d.fields[1].focus.is_focused(window));
            assert!(handler.selected_text_range(false,window,cx).is_none());let mut adjusted=None;
            assert!(handler.text_for_range(0..32,&mut adjusted,window,cx).is_none());
            handler.replace_text_in_range(None,"late",window,cx);assert_eq!(entity.read(cx).fields[0].data.rope.to_string(),"abcX");
        });
        assert!(draft(workspace,source)==before);assert_eq!(clipboard(&visual).as_deref(),Some("sentinel"));
    });
}
