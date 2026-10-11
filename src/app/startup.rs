// Startup owns the manifest until recovery identities and readers are ready.
// No editor/save registration is permitted during that ownership interval.
use std::sync::Arc;
use gpui::{App, Global, BackgroundExecutor};
use uuid::Uuid;
use crate::config::WindowEntry;
use crate::session::{SessionManifest, recovery_identity::{self, PreparedRecoveryIds}, startup_restore::RestoredSession};

#[derive(Default)]
struct Startup {
    request: Option<Arc<()>>,
    failure: Option<Arc<()>>,
    blocked: bool,
    published: bool,
    quitting: bool,
    cold: bool,
    pending_new: usize,
}
impl Global for Startup {}

// Cancellation can destroy the foreground receiver after the worker completes.
// In that case even an already-delivered owned manifest is retired off-thread.
struct Delivery<T: Send + 'static> { value: Option<T>, executor: BackgroundExecutor }
impl<T: Send + 'static> Delivery<T> {
    fn new(value: T, executor: BackgroundExecutor) -> Self { Self { value: Some(value), executor } }
    fn take(&mut self) -> Option<T> { self.value.take() }
}
impl<T: Send + 'static> Drop for Delivery<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.executor.spawn(async move { drop(value); }).detach();
        }
    }
}

pub(crate) fn blocks_editor(cx: &App) -> bool {
    cx.try_global::<Startup>().is_some_and(|state| state.quitting
        || ((state.blocked || state.request.is_some()) && !cx.windows().into_iter().any(|handle| handle.downcast::<crate::workspace::Workspace>().is_some())))
}
pub(crate) fn protects_placement(cx: &App) -> bool {
    cx.try_global::<Startup>().is_some_and(|state| !state.published && (state.blocked || state.quitting))
}
pub(crate) fn cancel_for_app_quit(cx: &mut App) {
    let state = cx.default_global::<Startup>();
    state.request = None;
    state.quitting = true;
}
pub(crate) fn cancel_for_quit(cx: &mut App) {
    let has_editor = cx.windows().into_iter().any(|handle| handle.downcast::<crate::workspace::Workspace>().is_some());
    let state = cx.default_global::<Startup>();
    if state.blocked && !has_editor {
        state.request = None;
        state.quitting = true;
    }
}
pub(crate) fn defer_new_window(cx: &mut App) -> bool {
    if !blocks_editor(cx) { return false; }
    let state = cx.default_global::<Startup>();
    if !state.quitting { state.pending_new = state.pending_new.saturating_add(1); }
    true
}

struct Prepared {
    manifest: Option<(SessionManifest, PreparedRecoveryIds)>,
    welcome: Uuid,
    entries: Vec<WindowEntry>,
}

pub(crate) fn begin(cx: &mut App, cold: bool) {
    if cx.windows().into_iter().any(|handle| handle.downcast::<crate::workspace::Workspace>().is_some()) {
        super::window::raise_all_app_windows(cx);
        return;
    }
    let state = cx.default_global::<Startup>();
    state.cold |= cold;
    if state.quitting || state.request.is_some() || state.failure.is_some() { return; }
    let request = Arc::new(());
    state.request = Some(request.clone());
    state.blocked = true;
    state.published = false;
    // These existing loaders can salvage/quarantine corrupt files. Their IO
    // remains foreground; only the owned in-place identity pass moves here.
    #[cfg(test)]
    let (entries, manifest, welcome) = tests::take_inputs(cx).unwrap_or_else(|| (
        crate::config::load_windows(), crate::session::load_manifest(), Uuid::new_v4()));
    #[cfg(not(test))]
    let (entries, manifest, welcome) = (
        crate::config::load_windows(), crate::session::load_manifest(), Uuid::new_v4());
    #[cfg(test)]
    let gate = tests::take_gate(cx);
    #[cfg(test)]
    let phases = tests::phase_sink(cx);
    let executor = cx.background_executor().clone();
    let worker_executor = executor.clone();
    let task = executor.spawn(async move {
        #[cfg(test)]
        if let Some(gate) = gate { let _ = gate.await; }
        #[cfg(test)]
        let started = std::time::Instant::now();
        let result = match manifest {
            Some(mut manifest) => recovery_identity::prepare(&mut manifest, welcome)
                .map(|ids| Prepared { manifest: Some((manifest, ids)), welcome, entries }),
            None => Ok(Prepared { manifest: None, welcome, entries }),
        };
        #[cfg(test)]
        if let Some(phases) = phases { let _ = phases.send(("metadata", started.elapsed())); }
        Delivery::new(result, worker_executor)
    });
    cx.spawn(async move |cx| {
        let mut delivery = task.await;
        cx.update(|cx| {
            if !cx.try_global::<Startup>().is_some_and(|state| state.request.as_ref().is_some_and(|current| Arc::ptr_eq(current, &request)) && !state.quitting) { return; }
            // A separately-created editor owns its new edits. A startup worker
            // must never reopen the old manifest over that ownership.
            if cx.windows().into_iter().any(|handle| handle.downcast::<crate::workspace::Workspace>().is_some()) {
                let state = cx.default_global::<Startup>();
                state.blocked = false; state.published = true;
                if let Some(result) = delivery.take() { retire_before_retry(result, None, cx); }
                return;
            }
            let Some(result) = delivery.take() else { return; };
            match result {
                Ok(prepared) => publish(prepared, cx),
                Err(err) => failed(err.to_string(), cx),
            }
        });
    }).detach();
}

// One admitted request spans preparation, reader ownership, native publication
// and physical refusal retirement. Retry cannot enqueue another large drop.
fn retire_before_retry<T: Send + 'static>(retained: T, failure: Option<String>, cx: &mut App) {
    let Some(request) = cx.default_global::<Startup>().request.clone() else {
        // Actual app quit is terminal, so this fallback cannot admit retries.
        cx.background_executor().spawn(async move { drop(retained); }).detach();
        return;
    };
    #[cfg(test)]
    let gate = tests::take_retirement_gate(cx);
    let task = cx.background_executor().spawn(async move {
        #[cfg(test)]
        if let Some(gate) = gate { let _ = gate.await; }
        drop(retained);
    });
    cx.spawn(async move |cx| {
        task.await;
        cx.update(|cx| {
            if !cx.try_global::<Startup>().is_some_and(|state| state.request.as_ref().is_some_and(|current| Arc::ptr_eq(current, &request)) && !state.quitting) { return; }
            if let Some(message) = failure { failed(message, cx); }
            else { cx.default_global::<Startup>().request = None; }
        });
    }).detach();
}

fn failed(message: String, cx: &mut App) {
    let state = cx.default_global::<Startup>();
    state.request = None;
    let failure = Arc::new(());
    state.failure = Some(failure.clone());
    eprintln!("[session] startup recovery failed: {message}");
    #[cfg(test)]
    if tests::capture_failure(cx, &message) { return; }
    cx.spawn(async move |cx| {
        let answer = rfd::AsyncMessageDialog::new().set_title(crate::consts::APP_NAME)
            .set_level(rfd::MessageLevel::Error).set_buttons(rfd::MessageButtons::YesNo)
            .set_description(format!("Session recovery failed: {message}. Recovery backups have been retained. Retry recovery? Choosing No exits without changing recovery backups."))
            .show().await;
        cx.update(|cx| answer_failure(&failure, answer == rfd::MessageDialogResult::Yes, cx));
    }).detach();
}

fn answer_failure(failure: &Arc<()>, retry: bool, cx: &mut App) {
    if !cx.try_global::<Startup>().is_some_and(|state| state.blocked && !state.quitting
        && state.request.is_none() && state.failure.as_ref().is_some_and(|current| Arc::ptr_eq(current, failure))) { return; }
    cx.default_global::<Startup>().failure = None;
    if retry { begin(cx, false); } else { super::quit::quit_with_recovery(cx); }
}

fn publish(prepared: Prepared, cx: &mut App) {
    let Prepared { manifest, welcome, entries } = prepared;
    #[cfg(test)]
    let started = std::time::Instant::now();
    let restored = match manifest {
        Some((manifest, ids)) => match RestoredSession::read(manifest, ids, welcome) {
            Ok(restored) => Some(restored),
            Err(failure) => {
                let message = failure.error.to_string();
                retire_before_retry((failure.retained, entries), Some(message), cx);
                return;
            }
        },
        None => None,
    };
    #[cfg(test)]
    tests::record_phase(cx, "readers", started.elapsed());
    let welcome = restored.as_ref().map(|session| session.welcome).unwrap_or(welcome);
    let executor = cx.background_executor().clone();
    let mut restored = Delivery::new(restored, executor);
    let mut entries = entries.into_iter();
    let first = entries.next();
    let Some(payload) = restored.value.as_mut() else { return; };
    #[cfg(test)]
    let started = std::time::Instant::now();
    if let Err(err) = super::window::spawn_prepared_window(cx, first, welcome, payload) {
        #[cfg(test)]
        tests::capture_retained(payload.as_ref(), cx);
        if let Some(retained) = restored.take() { retire_before_retry((retained, entries), Some(err.to_string()), cx); }
        return;
    }
    #[cfg(test)]
    tests::record_phase(cx, "test_window_publication", started.elapsed());
    let state = cx.default_global::<Startup>();
    state.request = None;
    state.blocked = false;
    state.published = true;
    let cold = std::mem::take(&mut state.cold);
    let pending_new = std::mem::take(&mut state.pending_new);
    if cold { crate::changelog::maybe_open_changelog(cx); }
    // Existing extra-placement behavior is retained in this first caller phase.
    // A bounded refill pump is a separate reviewed extension.
    for entry in entries { super::window::spawn_fresh_window(cx, Some(entry)); }
    for _ in 0..pending_new { super::window::spawn_fresh_window(cx, None); }
}

#[cfg(test)]
#[path = "startup_tests.rs"]
pub(super) mod tests;
