// PowerEditor/src/Notepad_plus.rc:483; NppBigSwitch.cpp:2813-2865 — Exit can be cancelled.
use gpui::{App, Global, WindowHandle, WindowId};
use crate::{session, workspace::Workspace};
use std::collections::HashSet;
use uuid::Uuid;

#[path = "quit_state.rs"]
mod state;
use state::WorkspaceStamp;

#[derive(Default)]
struct ExitState {
    pending: Option<Uuid>,
    accepted: Option<ExitCapture>,
}
impl Global for ExitState {}

struct ExitCapture {
    windows: HashSet<WindowId>,
    workspaces: Vec<(WindowHandle<Workspace>, WorkspaceStamp)>,
}

impl ExitCapture {
    fn is_current(&self, cx: &mut App) -> bool {
        if self.windows != cx.windows().iter().map(|handle| handle.window_id()).collect() { return false; }
        self.workspaces.iter().all(|(handle, stamp)| {
            handle.update(cx, |workspace, _, _| WorkspaceStamp::capture(workspace) == *stamp).unwrap_or(false)
        })
    }
}

/// Only an unchanged successful Exit preflight can skip the platform's final
/// fallback. The late app-quit observer cannot cancel shutdown on an error.
fn has_current_exit_recovery(cx: &mut App) -> bool {
    let accepted = cx.default_global::<ExitState>().accepted.take();
    let current = accepted.as_ref().is_some_and(|capture| capture.is_current(cx));
    cx.default_global::<ExitState>().accepted = accepted;
    current
}

fn finish_exit_recovery_if_current(cx: &mut App) -> bool {
    if !has_current_exit_recovery(cx) { return false; }
    #[cfg(test)]
    let barrier = async_tests::final_barrier(cx).unwrap_or_else(session::retire_prepared_writes);
    #[cfg(not(test))]
    let barrier = session::retire_prepared_writes();
    if let Err(err) = barrier {
        eprintln!("[session] could not retire final recovery writes: {err}");
        return false;
    }
    // This runs inside on_app_quit, after its final freshness check. gpui
    // clears the windows in the same call, without accepting another edit.
    for handle in cx.windows() {
        let Some(handle) = handle.downcast::<Workspace>() else { continue; };
        if let Err(err) = handle.update(cx, |workspace, _, _| workspace.release_session_window()) {
            eprintln!("[session] could not release final recovery workspace: {err}");
            return false;
        }
    }
    cx.default_global::<ExitState>().accepted = None;
    true
}

/// The synchronous fallback is still needed for OS quit, stale Exit proof,
/// and a failed final writer barrier. Native window close uses its own save.
pub(crate) fn save_final_recovery(cx: &mut App) {
    if finish_exit_recovery_if_current(cx) { return; }
    for handle in cx.windows() {
        let Some(handle) = handle.downcast::<Workspace>() else { continue; };
        if let Err(err) = handle.update(cx, |workspace, _, _| {
            if let Err(err) = workspace.save_session_now() {
                eprintln!("[session] final recovery write failed: {err}");
            }
        }) {
            eprintln!("[session] could not read final recovery workspace: {err}");
        }
    }
}

fn exit_status(cx: &mut App, message: &str) {
    for handle in cx.windows() {
        let Some(handle) = handle.downcast::<Workspace>() else { continue; };
        if let Err(err) = handle.update(cx, |workspace, _, cx| { workspace.set_status(message); cx.notify(); }) {
            eprintln!("[session] could not show Exit status: {err}");
        }
    }
}

pub(super) fn quit_with_recovery(cx: &mut App) {
    if cx.default_global::<ExitState>().pending.is_some() { return; }
    cx.default_global::<ExitState>().accepted = None;
    let windows = cx.windows();
    let mut capture = ExitCapture { windows: windows.iter().map(|handle| handle.window_id()).collect(), workspaces: Vec::new() };
    let mut snapshots = Vec::new();
    #[cfg(test)]
    let mut fixture_results = Vec::new();
    for handle in windows {
        let Some(handle) = handle.downcast::<Workspace>() else { continue; };
        match handle.update(cx, |workspace, _, _| {
            if workspace.tabs.iter().any(|tab| tab.buffer.loading.is_some())
                || workspace.closed_tabs.iter().any(|tab| tab.buffer.loading.is_some()) {
                return Err("a document is still loading; wait for it to finish or cancel its load before Exit".to_owned());
            }
            let stamp = WorkspaceStamp::capture(workspace);
            #[cfg(test)]
            if let Some(result) = workspace.session_final_save_result {
                let snapshot = workspace.session_snapshot(session::BakPolicy::Full);
                if let Some(sink) = &workspace.session_snapshot_sink { let _ = sink.send(snapshot.clone()); }
                fixture_results.push(result);
                return Ok((stamp, Some(snapshot)));
            }
            Ok((stamp, (!workspace.window_closed).then(|| workspace.session_snapshot(session::BakPolicy::Full))))
        }) {
            Ok(Ok((stamp, snapshot))) => {
                if let Some(snapshot) = snapshot { snapshots.push((stamp.id, snapshot)); }
                capture.workspaces.push((handle, stamp));
            }
            Ok(Err(message)) => {
                exit_status(cx, &format!("Exit cancelled: {message}"));
                return;
            }
            Err(err) => {
                cx.spawn(async move |_| {
                    rfd::AsyncMessageDialog::new().set_level(rfd::MessageLevel::Error)
                        .set_title(crate::consts::APP_NAME)
                        .set_description(format!("Exit cancelled: a workspace could not be checked for session recovery: {err}. Retry Exit."))
                        .show().await;
                }).detach();
                return;
            }
        }
    }
    if snapshots.is_empty() {
        cx.default_global::<ExitState>().accepted = Some(capture);
        finish_quit(cx);
        return;
    }
    let mut prepared = None;
    #[cfg(test)]
    let fixture = (!fixture_results.is_empty()).then(|| fixture_results.iter().all(|success| *success));
    #[cfg(test)]
    if fixture.is_some() && fixture_results.len() != snapshots.len() {
        exit_status(cx, "Exit cancelled: mixed recovery fixture and real workspaces");
        return;
    }
    #[cfg(test)]
    let gate = async_tests::take_gate(cx, snapshots.iter().map(|(_, snapshot)| snapshot.clone()).collect());
    for (id, snapshot) in snapshots {
        #[cfg(test)]
        if fixture.is_some() { continue; }
        match session::prepare_save(id, snapshot, session::BakPolicy::Full) {
            Ok(pending) => prepared = Some(pending),
            Err(err) => {
                exit_status(cx, &format!("Exit cancelled: session recovery could not be saved: {err}; save your documents or retry Exit"));
                return;
            }
        }
    }
    let request = Uuid::new_v4();
    cx.default_global::<ExitState>().pending = Some(request);
    exit_status(cx, "Saving session recovery before Exit…");
    let task = cx.background_executor().spawn(async move {
        #[cfg(test)]
        if let Some(gate) = gate { return gate.await.unwrap_or_else(|_| Err(std::io::Error::other("injected recovery worker cancelled"))); }
        #[cfg(test)]
        if let Some(success) = fixture {
            return if success { Ok(()) } else { Err(std::io::Error::other("injected final recovery failure")) };
        }
        let pending = prepared.ok_or_else(|| std::io::Error::other("no captured recovery job"))?;
        session::write_prepared(pending)
    });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            if cx.default_global::<ExitState>().pending != Some(request) { return; }
            cx.default_global::<ExitState>().pending = None;
            if let Err(err) = result {
                exit_status(cx, &format!("Exit cancelled: session recovery could not be saved: {err}; save your documents or retry Exit"));
            } else if !capture.is_current(cx) {
                exit_status(cx, "Exit cancelled: documents or windows changed while saving recovery; retry Exit");
            } else {
                cx.default_global::<ExitState>().accepted = Some(capture);
                finish_quit(cx);
            }
        });
    }).detach();
}

pub(super) fn save_before_window_close(
    workspace: &mut crate::workspace::Workspace, cx: &mut gpui::Context<crate::workspace::Workspace>,
) -> bool {
    if let Err(err) = workspace.save_session_now() {
        workspace.set_status(format!("Window close cancelled: session recovery could not be saved: {err}; save your documents or retry closing"));
        cx.notify();
        return false;
    }
    workspace.release_session_window();
    true
}

pub(super) fn finish_quit(cx: &mut App) {
    #[cfg(test)]
    if tests::capture_quit() { return; }
    #[cfg(test)]
    if async_tests::capture_quit() { return; }
    cx.quit();
}

#[cfg(test)]
#[path = "quit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "quit_async_tests.rs"]
mod async_tests;
