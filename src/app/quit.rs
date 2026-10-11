// PowerEditor/src/Notepad_plus.rc:483; NppBigSwitch.cpp:2813-2865 — Exit can be cancelled.
use gpui::App;

pub(super) fn quit_with_recovery(cx: &mut App) {
    for handle in cx.windows() {
        let Some(handle) = handle.downcast::<crate::workspace::Workspace>() else { continue; };
        match handle.update(cx, |workspace, _window, cx| {
            match workspace.save_session_now() {
                Ok(()) => true,
                Err(err) => {
                    workspace.set_status(format!("Exit cancelled: session recovery could not be saved: {err}; save your documents or retry Exit"));
                    cx.notify();
                    false
                }
            }
        }) {
            Ok(true) => {},
            Ok(false) => return,
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
    finish_quit(cx);
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
    cx.quit();
}

#[cfg(test)]
#[path = "quit_tests.rs"]
mod tests;
