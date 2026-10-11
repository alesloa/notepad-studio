// scintilla/src/ScintillaBase.cxx:242, 489 — completion replaces the entered prefix.
use super::super::autocomplete::is_word_char;
use crate::{tab::InputFocus, workspace::Workspace};
use gpui::{Context, Window};
use std::ops::Range;

#[cfg(test)]
#[path = "completion_macro_context_tests.rs"]
mod context_tests;

// Matches the completion UI's identifier limit; config events are hostile input.
const MAX_COMPLETION_BYTES: usize = 256;

fn completion_range(
    rope: &ropey::Rope,
    prefix: &str,
    candidate: &str,
    caret: usize,
) -> Option<Range<usize>> {
    if prefix.len() > MAX_COMPLETION_BYTES
        || candidate.is_empty()
        || candidate.len() > MAX_COMPLETION_BYTES
        || prefix == candidate
        || prefix.chars().any(|c| c.is_control() || c.is_whitespace())
        || candidate.chars().any(char::is_control)
    {
        return None;
    }
    let start = caret.checked_sub(prefix.len())?;
    let slice = rope.get_byte_slice(start..caret)?;
    if !slice.chars().eq(prefix.chars()) {
        return None;
    }
    let before = rope
        .byte_to_char(start)
        .checked_sub(1)
        .and_then(|idx| rope.get_char(idx));
    // API prefixes include configured punctuation; manual Function Completion
    // can choose a non-prefix member. Replay the recorded span without requiring
    // a loaded API or popup, but never replace a suffix of that identifier.
    if before.is_some_and(|c| is_word_char(c) || prefix.contains(c)) {
        return None;
    }
    Some(start..caret)
}

impl Workspace {
    pub(crate) fn replay_completion_word(
        &mut self,
        prefix: &str,
        candidate: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let selection = tab.selection.clamped(&tab.buffer.rope);
        if !window.is_window_active()
            || tab.view_mode != crate::tab::TabViewMode::Edit
            || !self.focus_handle.is_focused(window)
            || self.input_focus(window) != InputFocus::Editor
            || tab.buffer.read_only
            || !selection.is_empty()
            || tab.block_selection.is_some()
            || selection.head != tab.selection.head
            || selection.anchor != tab.selection.anchor
        {
            self.set_status("Macro completion skipped: editor context changed");
            cx.notify();
            return false;
        }
        let Some(range) = completion_range(&tab.buffer.rope, prefix, candidate, selection.head)
        else {
            self.set_status("Macro completion skipped: prefix changed or event is invalid");
            cx.notify();
            return false;
        };
        self.replace_byte_range(range, candidate, cx);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_completion_events_do_not_replace_text() {
        let rope = ropey::Rope::from_str("before café");
        let end = rope.len_bytes();
        for (prefix, candidate, caret) in [
            ("café", "caféine", usize::MAX),
            ("café", "caféine", end - 1),
            ("afé", "aféine", end),
            ("café", "", end),
            ("caff", "zebra", end),
            ("café", "café", end),
            ("", "android", end),
            ("re café", "re caféine", end),
        ] {
            assert_eq!(completion_range(&rope, prefix, candidate, caret), None);
        }
        let oversized = "a".repeat(MAX_COMPLETION_BYTES + 1);
        assert!(completion_range(&rope, &oversized, &oversized, end).is_none());
        assert!(completion_range(&rope, "café", &oversized, end).is_none());
        assert_eq!(
            completion_range(&rope, "café", "caféine", end),
            Some(7..end)
        );
        assert_eq!(completion_range(&rope, "", "android", 0), Some(0..0));
        assert_eq!(completion_range(&rope, "café", "zebra", end), Some(7..end));
    }
}
