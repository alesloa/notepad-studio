// Status-bar fields for the active tab. The text rules live in
// `crate::ui::status_text`; this reads the buffer, caret and settings they
// need.

use crate::ui::chrome::StatusFields;
use crate::ui::status_text::{self, SelectionText};
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn status_fields(&self) -> StatusFields {
        let buffer = self.buffer();
        let rope = &buffer.rope;
        let eol = buffer.line_ending;
        let encoding = buffer.encoding;
        let selection = self.selection();
        let head = selection.head.min(rope.len_bytes());

        let (_, tab_width) =
            crate::settings::indent_for_lang(buffer.language.map(|lang| lang.key()));
        let line = rope.byte_to_line(head).saturating_add(1);
        let col = status_text::caret_column(rope, head, tab_width);

        let sel = match self.block_selection().filter(|b| !b.is_empty()) {
            Some(block) => SelectionText::Rect(status_text::rect_selection(
                rope,
                block.min_line()..=block.max_line(),
                block.min_col()..block.max_col(),
            )),
            None if selection.is_empty() => {
                SelectionText::Pos(status_text::caret_pos(rope, head, eol, encoding))
            }
            None => {
                let range = selection.ordered();
                let (chars, lines) =
                    status_text::stream_selection(rope, range.start, range.end, eol);
                SelectionText::Stream { chars, lines }
            }
        };

        StatusFields {
            doc_type: status_text::doc_type(buffer.language),
            doc_size: status_text::doc_size_text(
                status_text::doc_length(rope, eol, encoding),
                rope.len_lines(),
            ),
            cur_pos: status_text::cur_pos_text(line, col, sel),
            line_ending: buffer.line_ending_status_label(),
            encoding: status_text::encoding_text(encoding),
        }
    }
}
