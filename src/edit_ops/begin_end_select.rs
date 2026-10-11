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
use ropey::Rope;

use crate::actions::*;
use crate::edit_ops::util::floor_char_boundary;
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
        let head = snap(&self.tabs[self.active].buffer.rope, head);

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
                self.settle_source_input_edit_caret();
                if column_mode {
                    eprintln!(
                        "begin_end_select: column-mode selection requires multi-cursor infra — \
                         falling back to linear selection"
                    );
                }
                let tab = &mut self.tabs[self.active];
                let anchor = snap(&tab.buffer.rope, marker);
                tab.selection.anchor = anchor;
                tab.selection.head = head;
                tab.selection.is_dragging = false;
                tab.desired_col_x = None;
                self.sync_source_input_caret();
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

/// Clamp a stored marker or caret offset to the rope and floor it to a char
/// boundary. Edits keep the marker on its char (`Workspace::apply_edit`),
/// but File ▸ Reload swaps the rope and leaves the marker as it was.
fn snap(rope: &Rope, byte: usize) -> usize {
    floor_char_boundary(rope, byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_marker_lands_on_a_char_boundary() {
        // A reload replaces the rope but keeps the marker, so byte 2 can end
        // up inside `中` (0..3); a selection anchored there panics on copy.
        let rope = Rope::from_str("中\r\né");
        assert_eq!(rope.byte_slice(snap(&rope, 2)..snap(&rope, 6)).to_string(), "中\r\n");
        assert_eq!(snap(&rope, 2), 0);
        assert_eq!(snap(&rope, 3), 3);
        assert_eq!(snap(&rope, 6), 5);
        assert_eq!(snap(&rope, 99), 7);
        assert_eq!(snap(&Rope::new(), 4), 0);
    }
}
