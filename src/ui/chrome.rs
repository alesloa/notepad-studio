use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{Entity, IntoElement, MouseButton, ParentElement, SharedString, div, prelude::*, rems, rgb};

use crate::actions::{EditEolCycle, EncodingCycle};
use crate::theme::Theme;
use crate::workspace::Workspace;

/// Snapshot of an in-flight streaming load handed to `status_bar`. The
/// progress text is pre-formatted ("Loading 12.4 MB / 215.0 MB (5.7%)") and
/// `cancel` is a clone of the same `Arc<AtomicBool>` that the streaming
/// task checks between chunks — flipping it here aborts the read.
pub(crate) struct LoadingChip {
    pub(crate) progress_text: String,
    pub(crate) cancel: Arc<AtomicBool>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn status_bar(
    theme: &Theme,
    line: usize,
    col: usize,
    encoding: &str,
    line_ending: &str,
    syntax: &str,
    overtype: bool,
    message: Option<&str>,
    loading: Option<LoadingChip>,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let mut bar = div()
        .h(rems(24. / 16.))
        .px(rems(14. / 16.))
        .flex()
        .items_center()
        .justify_between()
        .bg(rgb(theme.chrome))
        .border_t_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(11. / 16.))
        .child(format!("Ln {line}, Col {col}"));

    // Streaming load takes over the right side of the status bar — progress
    // text plus a clickable Cancel chip. Wins over `message` because the
    // user needs the Cancel button to be reachable.
    if let Some(chip) = loading {
        let cancel = chip.cancel.clone();
        let right = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(rems(8. / 16.))
            .child(div().child(chip.progress_text))
            .child(
                div()
                    .px(rems(6. / 16.))
                    .text_color(rgb(theme.toggle_active))
                    .child("Cancel")
                    .on_mouse_down(MouseButton::Left, move |_e, _window, _cx| {
                        cancel.store(true, Ordering::Relaxed);
                    }),
            );
        bar = bar.child(right);
        return bar;
    }

    if let Some(m) = message {
        bar = bar.child(m.to_string());
        return bar;
    }

    // Right-side segment row. Encoding and line-ending segments are
    // clickable: a left-click cycles each through its mode list, mirroring
    // Notepad++'s status-bar behaviour.
    let encoding_handle = workspace.clone();
    let eol_handle = workspace.clone();
    let typing_mode_handle = workspace;

    let segment = |label: SharedString| {
        div()
            .px(rems(8. / 16.))
            .child(label)
    };

    let right = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(4. / 16.))
        .child(
            segment(encoding.to_string().into())
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    encoding_handle.update(cx, |w, cx| w.encoding_cycle(&EncodingCycle, window, cx));
                }),
        )
        .child(div().px(rems(4. / 16.)).child("·"))
        .child(
            segment(line_ending.to_string().into())
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    eol_handle.update(cx, |w, cx| w.eol_cycle(&EditEolCycle, window, cx));
                }),
        )
        .child(div().px(rems(4. / 16.)).child("·"))
        .child(segment(syntax.to_string().into()))
        .child(div().px(rems(4. / 16.)).child("·"))
        // Notepad++'s typing-mode field (Notepad_plus.cpp:4615); a click
        // flips it, as NppNotification.cpp:941 does.
        .child(
            segment(typing_mode_label(overtype).into())
                .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                    typing_mode_handle.update(cx, |w, cx| w.set_overtype(!w.overtype, cx));
                }),
        );

    bar.child(right)
}

/// Notepad++'s status-bar text for the typing mode.
fn typing_mode_label(overtype: bool) -> &'static str {
    if overtype { "OVR" } else { "INS" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_mode_label_matches_notepad_plus_plus() {
        assert_eq!(typing_mode_label(false), "INS");
        assert_eq!(typing_mode_label(true), "OVR");
    }
}
