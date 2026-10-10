use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{Entity, IntoElement, MouseButton, ParentElement, div, prelude::*, rems, rgb};

use crate::actions::{EditEolCycle, EditGotoLine, EncodingCycle};
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

/// Text of the five data fields, from `crate::ui::status_text`.
pub(crate) struct StatusFields {
    pub(crate) doc_type: &'static str,
    pub(crate) doc_size: String,
    pub(crate) cur_pos: String,
    pub(crate) line_ending: &'static str,
    pub(crate) encoding: &'static str,
}

/// Notepad++'s six status-bar parts, left to right: document type, length
/// and lines, caret position, EOL, encoding, typing mode
/// (Notepad_plus.cpp:447 sets the widths; the first part takes the rest).
/// A status message or a streaming load shows in the document-type part,
/// so the other fields stay readable.
pub(crate) fn status_bar(
    theme: &Theme,
    fields: StatusFields,
    overtype: bool,
    message: Option<&str>,
    loading: Option<LoadingChip>,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let border = theme.border;
    // Notepad++'s part widths in pixels at 96 DPI. A floor, not a fixed
    // width: the monospace font here runs wider than Notepad++'s UI font.
    let part = move |width_px: f32| {
        div()
            .flex_none()
            .min_w(rems(width_px / 16.))
            .h_full()
            .px(rems(6. / 16.))
            .flex()
            .items_center()
            .whitespace_nowrap()
            .border_l_1()
            .border_color(rgb(border))
    };

    let first = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap();
    // Streaming load wins over `message`: the Cancel chip must stay
    // reachable.
    let first = if let Some(chip) = loading {
        let cancel = chip.cancel.clone();
        first
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
            )
    } else {
        first.child(div().truncate().child(message.unwrap_or(fields.doc_type).to_string()))
    };

    let goto_handle = workspace.clone();
    let eol_handle = workspace.clone();
    let encoding_handle = workspace.clone();
    let typing_mode_handle = workspace;

    div()
        .h(rems(24. / 16.))
        .pl(rems(8. / 16.))
        .flex()
        .items_center()
        .bg(rgb(theme.chrome))
        .border_t_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(11. / 16.))
        .child(first)
        .child(part(220.).child(fields.doc_size))
        // Double-click opens Go to Line (NppNotification.cpp:981).
        .child(part(260.).child(fields.cur_pos).on_mouse_down(
            MouseButton::Left,
            move |e, window, cx| {
                if e.click_count == 2 {
                    goto_handle.update(cx, |w, cx| w.open_goto_line(&EditGotoLine, window, cx));
                }
            },
        ))
        // A click cycles the line ending and the encoding.
        .child(part(110.).child(fields.line_ending).on_mouse_down(
            MouseButton::Left,
            move |_e, window, cx| {
                eol_handle.update(cx, |w, cx| w.eol_cycle(&EditEolCycle, window, cx));
            },
        ))
        .child(part(120.).child(fields.encoding).on_mouse_down(
            MouseButton::Left,
            move |_e, window, cx| {
                encoding_handle.update(cx, |w, cx| w.encoding_cycle(&EncodingCycle, window, cx));
            },
        ))
        // Notepad++'s typing-mode field (Notepad_plus.cpp:4615); a click
        // flips it, as NppNotification.cpp:941 does.
        .child(part(45.).child(typing_mode_label(overtype)).on_mouse_down(
            MouseButton::Left,
            move |_e, _window, cx| {
                typing_mode_handle.update(cx, |w, cx| w.set_overtype(!w.overtype, cx));
            },
        ))
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
