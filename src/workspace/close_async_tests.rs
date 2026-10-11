use super::{CloseDecision, Workspace};
use super::close_batch_tests::{press, with_window};
use crate::tab::{InputFocus, TabViewMode};
use gpui::{Entity, VisualContext, VisualTestContext};
use std::{cell::RefCell, collections::VecDeque, marker::PhantomData, rc::Rc, sync::{Arc, Mutex}};

pub(super) type PromptSource = Arc<Mutex<VecDeque<futures::channel::oneshot::Receiver<CloseDecision>>>>;
thread_local! { static PROMPTS: RefCell<Option<PromptSource>> = const { RefCell::new(None) }; }
pub(super) fn prompt_source() -> Option<PromptSource> { PROMPTS.with(|slot| slot.borrow().clone()) }

struct PromptOverride { previous: Option<PromptSource>, _local: PhantomData<Rc<()>> }
impl PromptOverride {
    fn new(receiver: futures::channel::oneshot::Receiver<CloseDecision>) -> Self {
        let source = Arc::new(Mutex::new(VecDeque::from([receiver])));
        Self { previous: PROMPTS.with(|slot| slot.replace(Some(source))), _local: PhantomData }
    }
}
impl Drop for PromptOverride {
    fn drop(&mut self) { PROMPTS.with(|slot| slot.replace(self.previous.take())); }
}

fn with_file(extension: &str, source: &[u8], test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext, &std::path::Path, uuid::Uuid)) {
    let directory = crate::persist::TempDir::new("async-close");
    let path = directory.join(&format!("neutral.{extension}"));
    std::fs::write(&path, source).unwrap();
    with_window(3, |entity, visual, ids| {
        visual.update_window_entity(entity, |w, window, cx| {
            w.tabs[2].buffer = crate::io::read_buffer(&path).unwrap();
            w.tabs[2].buffer.dirty = true;
            w.tabs[2].selection.head = w.tabs[2].buffer.rope.len_bytes();
            w.tabs[2].selection.anchor = w.tabs[2].selection.head;
            window.focus(&w.focus_handle, cx);
            cx.notify();
        });
        visual.run_until_parked();
        test(entity, visual, &path, ids[2]);
    });
}

fn begin_close(entity: &Entity<Workspace>, visual: &mut VisualTestContext, batch: bool) {
    press(visual, if batch { "FileCloseAllTabs" } else { "FileCloseTab" });
    visual.run_until_parked();
    visual.update(|_, cx| assert_eq!(entity.read(cx).tabs.len(), 3, "confirmation is still pending"));
}

fn reply(sender: futures::channel::oneshot::Sender<CloseDecision>, save: bool, visual: &mut VisualTestContext) {
    assert!(sender.send(if save { CloseDecision::Save } else { CloseDecision::DontSave }).is_ok());
    visual.run_until_parked();
}

#[test]
fn async_close_preserves_source_typed_after_the_prompt_opened() {
    for batch in [false, true] { for save in [false, true] {
        with_file("txt", "before café\r\n".as_bytes(), |entity, visual, path, id| {
            let (sender, receiver) = futures::channel::oneshot::channel();
            let _prompt = PromptOverride::new(receiver);
            begin_close(entity, visual, batch);
            visual.simulate_input("late 中😀");
            let history = visual.update(|_, cx| entity.read(cx).tabs[2].undo_stack.len());
            visual.update_window_entity(entity, |w, window, cx| w.focus_input(InputFocus::FindQuery, window, cx));
            reply(sender, save, visual);
            visual.update(|window, cx| {
                let w = entity.read(cx);
                assert_eq!(w.tabs.len(), 3);
                assert_eq!(w.tabs[2].session_id, id);
                assert_eq!(w.tabs[2].buffer.rope.to_string(), "before café\nlate 中😀");
                assert_eq!(w.tabs[2].undo_stack.len(), history);
                assert!(w.tabs[2].buffer.dirty);
                assert!(w.closed_tabs.is_empty());
                assert!(w.find_focus.is_focused(window));
                assert!(w.status_message.as_deref().unwrap_or_default().starts_with("Close cancelled:"));
            });
            assert_eq!(std::fs::read(path).unwrap(), "before café\r\n".as_bytes());
        });
    } }
}

#[test]
fn async_close_preserves_a_cell_editor_started_after_confirmation() {
    for batch in [false, true] { for save in [false, true] {
        with_file("csv", b"label,value\r\nseed,1\r\n", |entity, visual, path, id| {
            let (sender, receiver) = futures::channel::oneshot::channel();
            let _prompt = PromptOverride::new(receiver);
            begin_close(entity, visual, batch);
            visual.update_window_entity(entity, |w, window, cx| {
                w.tabs[2].view_mode = TabViewMode::CsvPreview;
                crate::ui::csv_doc::actions::ensure_model(w, window, cx);
                crate::ui::csv_doc::actions::open_cell_editor(w, 0, 0, window, cx);
            });
            visual.run_until_parked();
            visual.simulate_input(" late 中😀");
            reply(sender, save, visual);
            visual.update(|window, cx| {
                let w = entity.read(cx);
                assert_eq!(w.tabs.len(), 3);
                assert_eq!(w.tabs[2].session_id, id);
                assert_eq!(w.tabs[2].buffer.rope.to_string(), "label,value\nseed,1\n");
                let input = w.csv_models.get(&id).unwrap().cell_input.as_ref().unwrap();
                assert_eq!(input.buffer, "seed late 中😀");
                assert!(input.focus_handle.is_focused(window));
                assert!(w.closed_tabs.is_empty());
            });
            assert_eq!(std::fs::read(path).unwrap(), b"label,value\r\nseed,1\r\n");
        });
    } }
}

#[test]
fn async_close_preserves_lazy_and_detached_drafts_created_during_confirmation() {
    for batch in [false, true] { for detached in [false, true] {
        with_file("csv", b"label,value\nseed,1\n", |entity, visual, _, id| {
            let (sender, receiver) = futures::channel::oneshot::channel();
            let _prompt = PromptOverride::new(receiver);
            begin_close(entity, visual, batch);
            visual.update_window_entity(entity, |w, window, cx| {
                let draft = crate::session::CsvDraft { value: "late recovered 中😀".into(), cursor: 5, anchor: 0,
                    refusal: Some("keep this text".into()), ..Default::default() };
                if detached {
                    crate::ui::csv_doc::actions::ensure_model(w, window, cx);
                    w.csv_models.get_mut(&id).unwrap().detached_draft = Some(draft);
                } else { w.tabs[2].csv_draft = Some(draft); }
                w.focus_input(InputFocus::FindQuery, window, cx);
            });
            reply(sender, false, visual);
            visual.update(|window, cx| {
                let w = entity.read(cx);
                assert_eq!(w.tabs.len(), 3);
                let draft = if detached { w.csv_models.get(&id).unwrap().detached_draft.as_ref() }
                    else { w.tabs[2].csv_draft.as_ref() }.unwrap();
                assert_eq!(draft.value, "late recovered 中😀");
                assert_eq!((draft.cursor, draft.anchor), (5, 0));
                assert_eq!(draft.refusal.as_deref(), Some("keep this text"));
                assert!(w.find_focus.is_focused(window));
            });
        });
    } }
}

#[test]
fn async_close_preserves_rich_edits_when_raw_source_is_unchanged() {
    for batch in [false, true] {
        with_file("rtf", br"{\rtf1\ansi seed}", |entity, visual, path, id| {
            let (sender, receiver) = futures::channel::oneshot::channel();
            let _prompt = PromptOverride::new(receiver);
            visual.update_window_entity(entity, |w, _, cx| {
                w.tabs[2].view_mode = TabViewMode::RichDocView;
                let payload = w.tabs[2].buffer.rich_doc.as_mut().unwrap();
                payload.active_block = Some(0);
                let pos = crate::rich_doc::TextPos { block: 0, run: 0, byte: 0 };
                payload.selection = Some(crate::rich_doc::TextSelection { anchor: pos, cursor: pos });
                cx.notify();
            });
            visual.run_until_parked();
            visual.update_window_entity(entity, |w, window, cx| {
                let focus = w.rich_doc_body_focus.clone().unwrap();
                window.focus(&focus, cx);
            });
            let raw = visual.update(|_, cx| entity.read(cx).tabs[2].buffer.rope.to_string());
            begin_close(entity, visual, batch);
            visual.simulate_input("late 中😀 ");
            let history = visual.update(|_, cx| entity.read(cx).tabs[2].buffer.rich_doc.as_ref().unwrap().undo_stack.len());
            reply(sender, false, visual);
            visual.update(|window, cx| {
                let w = entity.read(cx);
                assert_eq!(w.tabs.len(), 3);
                assert_eq!(w.tabs[2].session_id, id);
                assert_eq!(w.tabs[2].buffer.rope.to_string(), raw);
                let payload = w.tabs[2].buffer.rich_doc.as_ref().unwrap();
                assert_eq!(payload.flat_search_text().0, "late 中😀 seed");
                assert_eq!(payload.undo_stack.len(), history);
                assert!(w.rich_doc_body_focus.as_ref().unwrap().is_focused(window));
            });
            assert_eq!(std::fs::read(path).unwrap(), br"{\rtf1\ansi seed}");
        });
    }
}

#[test]
fn async_close_rechecks_path_metadata_loading_and_replaced_document() {
    for change in 0..5 { for batch in [false, true] {
        with_file("txt", b"before\n", |entity, visual, path, id| {
            let (sender, receiver) = futures::channel::oneshot::channel();
            let _prompt = PromptOverride::new(receiver);
            begin_close(entity, visual, batch);
            visual.update_window_entity(entity, |w, window, cx| {
                match change {
                    0 => w.tabs[2].buffer.file_path = Some(path.with_file_name("new-path.txt")),
                    1 => w.tabs[2].buffer.encoding = crate::text::encoding::Encoding::Utf16Le,
                    2 => w.tabs[2].buffer.line_ending = crate::text::line_ending::LineEnding::Cr,
                    3 => w.tabs[2].buffer.loading = Some(crate::buffer::LoadProgress { bytes_read: 1, total_bytes: 2,
                        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)) }),
                    _ => {
                        w.tabs[2] = crate::tab::Tab::empty(); w.tabs[2].session_id = id;
                        w.tabs[2].buffer.rope = ropey::Rope::from_str("restored contents");
                    }
                }
                w.focus_input(InputFocus::FindQuery, window, cx);
            });
            reply(sender, false, visual);
            visual.update(|window, cx| {
                let w = entity.read(cx);
                assert_eq!(w.tabs.len(), 3);
                assert_eq!(w.tabs[2].session_id, id);
                assert_eq!(w.tabs[2].buffer.rope.to_string(), if change == 4 { "restored contents" } else { "before\n" });
                if change == 0 { assert_eq!(w.tabs[2].buffer.file_path.as_deref(), Some(path.with_file_name("new-path.txt").as_path())); }
                if change == 1 { assert_eq!(w.tabs[2].buffer.encoding, crate::text::encoding::Encoding::Utf16Le); }
                if change == 2 { assert_eq!(w.tabs[2].buffer.line_ending, crate::text::line_ending::LineEnding::Cr); }
                assert_eq!(w.tabs[2].buffer.loading.is_some(), change == 3);
                assert!(w.find_focus.is_focused(window));
            });
        });
    } }
}

#[test]
fn async_close_unchanged_discard_and_save_still_complete() {
    for batch in [false, true] { for save in [false, true] {
        let source = crate::text::encoding::encode("café 中😀\r\n", crate::text::encoding::Encoding::Utf16Le);
        with_file("txt", &source, |entity, visual, path, id| {
            let (sender, receiver) = futures::channel::oneshot::channel();
            let _prompt = PromptOverride::new(receiver);
            begin_close(entity, visual, batch);
            reply(sender, save, visual);
            visual.update(|_, cx| {
                let w = entity.read(cx);
                assert!(w.tabs.iter().all(|tab| tab.session_id != id));
                assert_eq!(w.tabs.len(), if batch { 1 } else { 2 });
                let closed = w.closed_tabs.iter().find(|tab| tab.session_id == id).unwrap();
                assert_eq!(closed.buffer.rope.to_string(), "café 中😀\n");
                assert_eq!(closed.buffer.dirty, !save);
            });
            assert_eq!(std::fs::read(path).unwrap(), source);
        });
    } }
}

#[test]
fn async_close_cancel_preserves_newer_field_and_current_document() {
    for batch in [false, true] {
        with_file("txt", b"before\n", |entity, visual, _, _| {
            let (sender, receiver) = futures::channel::oneshot::channel();
            let _prompt = PromptOverride::new(receiver);
            begin_close(entity, visual, batch);
            visual.simulate_input("late 中😀");
            visual.update_window_entity(entity, |w, window, cx| w.focus_input(InputFocus::FindQuery, window, cx));
            assert!(sender.send(CloseDecision::Cancel).is_ok());
            visual.run_until_parked();
            visual.update(|window, cx| {
                let w = entity.read(cx);
                assert_eq!(w.tabs.len(), 3);
                assert_eq!(w.tabs[2].buffer.rope.to_string(), "before\nlate 中😀");
                assert!(w.find_focus.is_focused(window));
                assert!(w.closed_tabs.is_empty());
            });
        });
    }
}

#[test]
fn async_close_and_save_refuse_uninitialized_recovered_drafts() {
    for operation in 0..3 {
        with_file("csv", b"label,value\nseed,1\n", |entity, visual, path, id| {
            let (sender, receiver) = futures::channel::oneshot::channel();
            let _prompt = PromptOverride::new(receiver);
            assert!(sender.send(CloseDecision::DontSave).is_ok());
            visual.update_window_entity(entity, |w, window, cx| {
                w.tabs[2].csv_draft = Some(crate::session::CsvDraft { value: "recover 中😀".into(),
                    row: Some(0), col: Some(0), ..Default::default() });
                match operation {
                    0 => w.close_tab_at(2, window, cx),
                    1 => w.close_all_tabs(&crate::actions::FileCloseAllTabs, window, cx),
                    _ => w.save_active(&crate::actions::FileSave, window, cx),
                }
            });
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = entity.read(cx);
                assert_eq!(w.tabs.len(), 3);
                assert_eq!(w.tabs[2].session_id, id);
                assert_eq!(w.tabs[2].csv_draft.as_ref().unwrap().value, "recover 中😀");
                assert!(w.closed_tabs.is_empty());
                assert!(w.status_message.as_deref().unwrap_or_default().starts_with("Cell edit not committed:"));
            });
            assert_eq!(std::fs::read(path).unwrap(), b"label,value\nseed,1\n");
        });
    }
}

#[test]
fn recovered_close_refusal_still_allows_opening_the_csv_grid() {
    with_file("csv", b"label,value\nseed,1\n", |entity, visual, _, id| {
        visual.update_window_entity(entity, |w, window, cx| {
            w.tabs[2].csv_draft = Some(crate::session::CsvDraft { value: "recover \u{4E2D}\u{1F600}".into(),
                row: Some(0), col: Some(0), ..Default::default() });
            w.close_tab_at(2, window, cx);
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.dispatch_action(Box::new(crate::actions::ViewCsvPreview), cx));
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs.len(), 3);
            assert_eq!(w.tabs[2].view_mode, TabViewMode::CsvPreview);
            let model = w.csv_models.get(&id).unwrap();
            let input = model.cell_input.as_ref().unwrap();
            assert_eq!(input.buffer, "recover \u{4E2D}\u{1F600}");
            assert!(input.focus_handle.is_focused(window));
            assert_eq!(w.tabs[2].buffer.rope.to_string(), "label,value\nseed,1\n");
        });
    });
}

#[test]
fn close_after_save_preserves_further_unsaved_input() {
    with_file("txt", "café\r\n".as_bytes(), |entity, visual, path, id| {
        let snapshot = visual.update_window_entity(entity, |w, _, _| {
            let snapshot = super::DocumentSnapshot::capture(&w.tabs[2]);
            w.write_active_to_disk().unwrap();
            snapshot
        });
        visual.simulate_input("late \u{4E2D}\u{1F600}");
        visual.update_window_entity(entity, |w, window, cx| {
            w.focus_input(InputFocus::FindQuery, window, cx);
            assert_eq!(w.close_confirmation_index(id, &snapshot, true, cx), Err(()));
            assert_eq!(w.tabs.len(), 3);
            assert!(w.tabs[2].buffer.dirty);
            assert_eq!(w.tabs[2].buffer.rope.to_string(), "café\nlate \u{4E2D}\u{1F600}");
            assert!(w.find_focus.is_focused(window));
            assert!(w.closed_tabs.is_empty());
        });
        assert_eq!(std::fs::read(path).unwrap(), "café\r\n".as_bytes());
    });
}
