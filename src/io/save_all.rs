// PowerEditor/src/NppIO.cpp:1795–1820, 1907–1990 — Save All visits dirty
// buffers in tab order, including untitled buffers; Cancel skips one buffer.
use std::{path::PathBuf, sync::Arc};

use gpui::{Context, Window};
use uuid::Uuid;

use crate::{consts::APP_NAME, workspace::Workspace};

struct Target {
    id: Uuid,
    instance: Arc<()>,
    path: Option<PathBuf>,
}

impl Target {
    fn index(&self, w: &Workspace) -> Option<usize> {
        w.tabs.iter().position(|tab| tab.session_id == self.id
            && Arc::ptr_eq(&tab.instance, &self.instance)
            && tab.buffer.file_path == self.path)
    }
}

enum Step {
    Stop,
    Skip,
    Saved(Result<(), String>),
    Pick(Option<PathBuf>, String),
}

#[derive(Default)]
struct Progress {
    saved: usize,
    cancelled: usize,
    failures: Vec<String>,
}

pub(super) fn start(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if w.window_closed || !w.commit_csv_drafts_for_save_all(cx) { return; }
    let targets: Vec<_> = w.tabs.iter().filter(|tab| tab.buffer.dirty)
        .map(|tab| Target { id: tab.session_id, instance: tab.instance.clone(), path: tab.buffer.file_path.clone() })
        .collect();
    if targets.is_empty() { return; }
    // Notepad++ bypasses the confirmation for the sole dirty active buffer.
    let sole_active = targets.len() == 1 && targets.first().is_some_and(|target|
        w.tabs.get(w.active).is_some_and(|tab| tab.session_id == target.id && Arc::ptr_eq(&tab.instance, &target.instance)));
    let needs_confirm = crate::settings::snapshot().save_all_confirm && !sole_active;
    if !needs_confirm && targets.iter().all(|target| target.path.is_some()) {
        w.run_save_all(cx);
        return;
    }
    let confirmed = needs_confirm.then(|| confirm(targets.len()));
    cx.spawn_in(window, async move |this, cx| {
        if let Ok(Some(ready)) = this.update(cx, |w, cx| w.wait_for_automatic_titles(cx)) { let _ = ready.await; }
        if let Some(confirmed) = confirmed {
            let decision = confirmed.await;
            let proceed = this.update(cx, |w, cx| {
                if w.window_closed { return false; }
                if decision == Decision::No {
                    w.set_status("Save All cancelled".to_string());
                    cx.notify();
                    return false;
                }
                if decision == Decision::AlwaysYes {
                    let mut prefs = crate::settings::snapshot();
                    prefs.save_all_confirm = false;
                    crate::settings::replace_and_save(prefs);
                }
                true
            }).unwrap_or(false);
            if !proceed { return; }
        }

        let mut progress = Progress::default();
        for target in targets {
            if let Ok(Some(ready)) = this.update(cx, |w, cx| w.wait_for_automatic_titles(cx)) { let _ = ready.await; }
            let step = this.update(cx, |w, cx| prepare(w, &target, cx)).unwrap_or(Step::Stop);
            match step {
                Step::Stop => return,
                Step::Skip => continue,
                Step::Saved(result) => progress.record(result),
                Step::Pick(starting, suggested) => {
                    let path = super::save_as_dialog::pick(starting, suggested).await;
                    if let Ok(Some(ready)) = this.update(cx, |w, cx| w.wait_for_automatic_titles(cx)) { let _ = ready.await; }
                    let result = this.update(cx, |w, cx| {
                        if w.window_closed || !w.commit_csv_drafts_for_save_all(cx) { return Step::Stop; }
                        let Some(index) = target.index(w) else { return Step::Skip; };
                        let Some(tab) = w.tabs.get(index) else { return Step::Skip; };
                        if !tab.buffer.dirty { return Step::Skip; }
                        if let Some(error) = cannot_save(tab) { return Step::Saved(Err(error)); }
                        let Some(path) = path else {
                            progress.cancelled += 1;
                            return Step::Skip;
                        };
                        if w.open_tab_index_for(&path).is_some_and(|other| other != index) {
                            return Step::Saved(Err(crate::workspace::already_open_message()));
                        }
                        let result = w.with_tab_active(target.id, |w| w.save_active_to_new_path(path.clone()));
                        let Some(result) = result else { return Step::Skip; };
                        if result.is_ok() {
                            crate::settings::record_last_used_directory(&path);
                            crate::recent::record_opened(path);
                            crate::app::menus::rebuild_menus(&mut *cx);
                            w.save_session(cx);
                        }
                        Step::Saved(result)
                    }).unwrap_or(Step::Stop);
                    match result {
                        Step::Stop => return,
                        Step::Saved(result) => progress.record(result),
                        _ => {}
                    }
                }
            }
        }
        let _ = this.update(cx, |w, cx| {
            if w.window_closed { return; }
            let mut status = format!("Saved {} file(s)", progress.saved);
            if progress.cancelled > 0 { status.push_str(&format!("; {} cancelled", progress.cancelled)); }
            if !progress.failures.is_empty() {
                status.push_str(&format!("; {} failed: {}", progress.failures.len(), progress.failures.join("; ")));
            }
            w.set_status(status);
            w.save_session(cx);
            cx.notify();
        });
    }).detach();
}

impl Progress {
    fn record(&mut self, result: Result<(), String>) {
        match result { Ok(()) => self.saved += 1, Err(error) => self.failures.push(error) }
    }
}

fn cannot_save(tab: &crate::tab::Tab) -> Option<String> {
    if tab.buffer.read_only {
        Some(format!("{} is read-only in Notepad Studio; clear Read-Only or use Save As", tab.buffer.display_short_name()))
    } else if tab.buffer.loading.is_some() {
        Some(format!("{} is still loading", tab.buffer.display_short_name()))
    } else { None }
}

fn prepare(w: &mut Workspace, target: &Target, cx: &mut Context<Workspace>) -> Step {
    if w.window_closed || !w.commit_csv_drafts_for_save_all(cx) { return Step::Stop; }
    let Some(index) = target.index(w) else { return Step::Skip; };
    let Some(tab) = w.tabs.get(index) else { return Step::Skip; };
    if !tab.buffer.dirty { return Step::Skip; }
    if let Some(error) = cannot_save(tab) { return Step::Saved(Err(error)); }
    if target.path.is_some() {
        let result = w.with_tab_active(target.id, Workspace::write_active_to_disk);
        if matches!(result, Some(Ok(()))) { w.save_session(cx); }
        return result.map(Step::Saved).unwrap_or(Step::Skip);
    }
    let starting = crate::settings::resolve_starting_dir(None)
        .or_else(dirs::document_dir).or_else(dirs::home_dir);
    Step::Pick(starting, super::suggested_save_name(&tab.buffer))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision { Yes, No, AlwaysYes }

// Capture a caller-scoped test reply before the asynchronous dialog starts.
#[cfg_attr(not(test), allow(clippy::manual_async_fn))]
fn confirm(count: usize) -> impl std::future::Future<Output = Decision> {
    #[cfg(test)]
    let reply = REPLY.with(|slot| slot.borrow_mut().take());
    async move {
        #[cfg(test)]
        if let Some(reply) = reply {
            reply.count.set(Some(count));
            return reply.receiver.await.unwrap_or(Decision::No);
        }
        let result = rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Info).set_title(APP_NAME)
            .set_description(format!("Save {count} modified document(s)?\n\nChoose Always Yes to disable this confirmation. You can enable it again in Preferences."))
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom("Yes".to_string(), "No".to_string(), "Always Yes".to_string()))
            .show().await;
        decision(result)
    }
}

pub(super) fn decision(result: rfd::MessageDialogResult) -> Decision {
    match result {
        rfd::MessageDialogResult::Yes => Decision::Yes,
        rfd::MessageDialogResult::Custom(label) if label == "Yes" => Decision::Yes,
        rfd::MessageDialogResult::Custom(label) if label == "Always Yes" => Decision::AlwaysYes,
        _ => Decision::No,
    }
}

#[cfg(test)]
struct Reply {
    receiver: futures::channel::oneshot::Receiver<Decision>,
    count: std::rc::Rc<std::cell::Cell<Option<usize>>>,
}

#[cfg(test)]
thread_local! {
    static REPLY: std::cell::RefCell<Option<Reply>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) struct Override {
    previous: Option<Reply>,
    count: std::rc::Rc<std::cell::Cell<Option<usize>>>,
    _local: std::marker::PhantomData<std::rc::Rc<()>>,
}
#[cfg(test)]
impl Override {
    pub(super) fn new(reply: futures::channel::oneshot::Receiver<Decision>) -> Self {
        let count = std::rc::Rc::new(std::cell::Cell::new(None));
        let previous = REPLY.with(|slot| slot.replace(Some(Reply { receiver: reply, count: count.clone() })));
        Self { previous, count, _local: std::marker::PhantomData }
    }
    pub(super) fn count(&self) -> Option<usize> { self.count.get() }
}
#[cfg(test)]
impl Drop for Override {
    fn drop(&mut self) { REPLY.with(|slot| slot.replace(self.previous.take())); }
}
