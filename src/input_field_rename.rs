// A platform may retain a field's input protocol after its prompt is replaced.
use std::ops::Range;

use gpui::{
    App, Bounds, ElementInputHandler, Entity, FocusHandle, InputHandler, Pixels, Point,
    UTF16Selection, Window,
};
use uuid::Uuid;

use crate::workspace::Workspace;

pub(crate) struct RenameInputHandler {
    inner: ElementInputHandler<Workspace>,
    workspace: Entity<Workspace>,
    prompt_id: Uuid,
    focus: FocusHandle,
}

impl RenameInputHandler {
    pub(crate) fn new(
        bounds: Bounds<Pixels>,
        workspace: Entity<Workspace>,
        prompt_id: Uuid,
        focus: FocusHandle,
    ) -> Self {
        Self {
            inner: ElementInputHandler::new(bounds, workspace.clone()),
            workspace,
            prompt_id,
            focus,
        }
    }

    fn is_current(&self, window: &Window, cx: &App) -> bool {
        let workspace = self.workspace.read(cx);
        window.is_window_active()
            && self.focus.is_focused(window)
            && workspace.tab_rename_focus.as_ref() == Some(&self.focus)
            && super::rename_prompt_matches(
                workspace,
                crate::tab::InputFocus::TabRename,
                Some(self.prompt_id),
            )
    }
}

impl InputHandler for RenameInputHandler {
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        if !self.is_current(window, cx) {
            return None;
        }
        self.inner
            .selected_text_range(ignore_disabled_input, window, cx)
    }

    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        if !self.is_current(window, cx) {
            return None;
        }
        self.inner.marked_text_range(window, cx)
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        if !self.is_current(window, cx) {
            return None;
        }
        self.inner
            .text_for_range(range_utf16, adjusted_range, window, cx)
    }

    fn replace_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.is_current(window, cx) {
            return;
        }
        self.inner
            .replace_text_in_range(replacement_range, text, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.is_current(window, cx) {
            return;
        }
        self.inner.replace_and_mark_text_in_range(
            range_utf16,
            new_text,
            new_selected_range,
            window,
            cx,
        );
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        if !self.is_current(window, cx) {
            return;
        }
        self.inner.unmark_text(window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        if !self.is_current(window, cx) {
            return None;
        }
        self.inner.bounds_for_range(range_utf16, window, cx)
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        if !self.is_current(window, cx) {
            return None;
        }
        self.inner.character_index_for_point(point, window, cx)
    }

    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.is_current(window, cx) && self.inner.accepts_text_input(window, cx)
    }
}
