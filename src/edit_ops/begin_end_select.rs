// Begin/End Select markers — NPP's "drop a marker, move the caret, then
// trigger Begin/End Select again to extend the selection from the marker
// to the new caret position".
//
// Notepad++ source:
//   PowerEditor/src/NppCommands.cpp:877 — IDM_EDIT_BEGINENDSELECT /
//                                         IDM_EDIT_BEGINENDSELECT_COLUMNMODE
//   ScintillaComponent/ScintillaEditView.cpp:3012 ::beginOrEndSelect
//
// Behaviour mirrors `_beginSelectPosition` (Option<usize> here vs. -1 sentinel
// in NPP):
//   - First press: stash the current caret offset.
//   - Second press: select from stash to current caret, drop the marker.
//
// Column-mode flag (Alt+Shift+B) takes the same two-press path; without
// rectangular-selection extensions on top of the linear path, it falls back
// to a linear stream selection and emits a runtime log so the limitation is
// visible to the user.

use gpui::{Context, Window};

use crate::actions::*;
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn begin_end_select(
        &mut self,
        _: &EditBeginEndSelect,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_begin_end_select(false, cx);
    }

    pub(crate) fn begin_end_select_column_mode(
        &mut self,
        _: &EditBeginEndSelectColumnMode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_begin_end_select(true, cx);
    }

    fn toggle_begin_end_select(&mut self, column_mode: bool, cx: &mut Context<Self>) {
        let head = self.tabs[self.active].selection.head;
        let total = self.tabs[self.active].buffer.rope.len_bytes();
        let head = head.min(total);

        let tab = &mut self.tabs[self.active];
        match tab.begin_select_marker.take() {
            None => {
                tab.begin_select_marker = Some(head);
                let line = tab.buffer.rope.byte_to_line(head) + 1;
                self.set_status(format!(
                    "Begin/End Select: marker set at line {line} \
                     (gutter bar shows anchor; press again to extend selection)"
                ));
            }
            Some(marker) => {
                if column_mode {
                    eprintln!(
                        "begin_end_select: column-mode selection requires multi-cursor infra — \
                         falling back to linear selection"
                    );
                }
                let tab = &mut self.tabs[self.active];
                let anchor = marker.min(total);
                tab.selection.anchor = anchor;
                tab.selection.head = head;
                tab.selection.is_dragging = false;
                tab.desired_col_x = None;
                let mode_label = if column_mode {
                    "linear (column-mode unsupported)"
                } else {
                    "linear"
                };
                self.set_status(format!("Begin/End Select: extended selection ({mode_label})"));
                self.ensure_cursor_visible(cx);
            }
        }
        cx.notify();
    }
}
