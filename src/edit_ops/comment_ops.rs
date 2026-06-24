// Comment/Uncomment ▶ submenu — directional (set / uncomment) variants. The
// toggle versions (Toggle Block / Toggle Stream) live in
// `workspace::edits` as `block_comment` and `stream_comment` and share the
// `apply_line_comment` / `apply_stream_comment` helpers via `CommentDir`.
//
// Notepad++ source:
//   PowerEditor/src/NppCommands.cpp:2149 — IDM_EDIT_BLOCK_COMMENT (toggle)
//   PowerEditor/src/NppCommands.cpp:2153 — IDM_EDIT_BLOCK_COMMENT_SET
//   PowerEditor/src/NppCommands.cpp:2157 — IDM_EDIT_BLOCK_UNCOMMENT
//   PowerEditor/src/NppCommands.cpp:2161 — IDM_EDIT_STREAM_COMMENT (toggle)
//   PowerEditor/src/NppCommands.cpp:2165 — IDM_EDIT_STREAM_UNCOMMENT

use gpui::{Context, Window};

use crate::actions::*;
use crate::tab::InputFocus;
use crate::workspace::Workspace;
use crate::workspace::edits::CommentDir;

impl Workspace {
    pub(crate) fn block_comment_set(
        &mut self,
        _: &EditBlockCommentSet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.apply_line_comment(CommentDir::ForceSet, cx);
    }

    pub(crate) fn block_uncomment(
        &mut self,
        _: &EditBlockUncomment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.apply_line_comment(CommentDir::ForceUncomment, cx);
    }

    pub(crate) fn stream_uncomment(
        &mut self,
        _: &EditStreamUncomment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.apply_stream_comment(CommentDir::ForceUncomment, cx);
    }
}
