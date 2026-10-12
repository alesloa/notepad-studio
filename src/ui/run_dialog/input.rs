use std::ops::Range;

use gpui::{
    App, Bounds, ElementInputHandler, Entity, FocusHandle, IntoElement, Pixels, ShapedLine,
    SharedString, TextRun, Window, fill, point, px, rgb, rgba, size,
};

use crate::text::utf16::{next_char_byte_in_str, prev_char_byte_in_str, replace_input_utf16};

use super::RunDialog;

#[derive(Clone)]
struct InputSnapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

pub(super) struct CommandInput {
    pub(super) text: String,
    pub(super) cursor: usize,
    pub(super) anchor: usize,
    pub(super) marked: Option<Range<usize>>,
    pub(super) dragging: bool,
    pub(super) touched: bool,
    max_bytes: usize,
    undo: Vec<InputSnapshot>,
    redo: Vec<InputSnapshot>,
}

impl Default for CommandInput {
    fn default() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            anchor: 0,
            marked: None,
            dragging: false,
            touched: false,
            max_bytes: crate::run_command::MAX_COMMAND_BYTES,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}

fn push_snapshot(stack: &mut Vec<InputSnapshot>, snapshot: InputSnapshot) {
    stack.push(snapshot);
    let mut bytes = stack.iter().map(|state| state.text.len()).sum::<usize>();
    while stack.len() > 64 || bytes > 1024 * 1024 {
        if stack.is_empty() {
            break;
        }
        bytes = bytes.saturating_sub(stack.remove(0).text.len());
    }
}

impl CommandInput {
    pub(super) fn for_name() -> Self {
        Self {
            max_bytes: crate::run_command::MAX_NAME_BYTES,
            ..Self::default()
        }
    }

    fn snapshot(&self) -> InputSnapshot {
        InputSnapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn restore(&mut self, snapshot: InputSnapshot) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor;
        self.anchor = snapshot.anchor;
        self.marked = None;
        self.dragging = false;
        self.touched = true;
    }

    pub(super) fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        let current = self.snapshot();
        push_snapshot(&mut self.redo, current);
        self.restore(snapshot);
        true
    }

    pub(super) fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        let current = self.snapshot();
        push_snapshot(&mut self.undo, current);
        self.restore(snapshot);
        true
    }

    fn limit_error(&self) -> String {
        if self.max_bytes == crate::run_command::MAX_NAME_BYTES {
            "Command names must be at most 256 bytes.".into()
        } else {
            "Run commands must be a single line of at most 32 KB.".into()
        }
    }

    pub(super) fn edit_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        cx: &mut App,
    ) -> Result<bool, String> {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        let command = mods.control || mods.platform;
        match key {
            "z" if command && mods.shift => {
                self.redo();
            }
            "z" if command => {
                self.undo();
            }
            "y" if command => {
                self.redo();
            }
            "backspace" if mods.alt => {
                self.undo();
            }
            "left" => self.move_to(prev_char_byte_in_str(&self.text, self.cursor), mods.shift),
            "right" => self.move_to(next_char_byte_in_str(&self.text, self.cursor), mods.shift),
            "home" => self.move_to(0, mods.shift),
            "end" => self.move_to(self.text.len(), mods.shift),
            "backspace" => self.delete(true),
            "delete" => self.delete(false),
            "a" if command => {
                self.anchor = 0;
                self.cursor = self.text.len();
            }
            "c" | "x" if command => {
                let text = self.selected().to_string();
                if !text.is_empty() {
                    crate::clipboard_ownership::write_now(cx, gpui::ClipboardItem::new_string(text));
                }
                if key == "x" {
                    self.replace(None, "")?;
                }
            }
            "v" if command => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(None, &text)?;
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(super) fn set(&mut self, text: String) -> Result<(), String> {
        if text.len() > self.max_bytes || text.contains(['\0', '\r', '\n']) {
            return Err(self.limit_error());
        }
        self.text = text;
        self.cursor = self.text.len();
        self.anchor = 0;
        self.marked = None;
        self.undo.clear();
        self.redo.clear();
        Ok(())
    }

    pub(super) fn replace(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
    ) -> Result<(), String> {
        if text.len() > self.max_bytes {
            return Err(self.limit_error());
        }
        let text: String = text
            .chars()
            .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
            .collect();
        let range = range.or_else(|| self.marked.clone());
        let mut candidate = self.text.clone();
        let mut cursor = self.cursor;
        let mut anchor = self.anchor;
        replace_input_utf16(
            &mut candidate,
            &mut cursor,
            &mut anchor,
            self.cursor,
            self.anchor,
            range,
            &text,
        );
        if candidate.len() > self.max_bytes {
            return Err(self.limit_error());
        }
        if candidate != self.text {
            // Marked-text updates replace one composition, not separate edits.
            if self.marked.is_none() {
                let current = self.snapshot();
                push_snapshot(&mut self.undo, current);
            }
            self.redo.clear();
        }
        self.text = candidate;
        self.cursor = cursor;
        self.anchor = anchor;
        self.touched = true;
        self.marked = None;
        Ok(())
    }

    pub(super) fn replace_composition(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
    ) -> Result<(), String> {
        let initial = self.marked.is_none().then(|| self.snapshot());
        self.replace(range, text)?;
        // A composition can start by echoing the selected text unchanged.
        // Keep its original selection so later marked updates still undo once.
        if let Some(initial) = initial.filter(|initial| initial.text == self.text) {
            push_snapshot(&mut self.undo, initial);
        }
        Ok(())
    }

    pub(super) fn move_to(&mut self, cursor: usize, extend: bool) {
        self.cursor = crate::edit_ops::util::floor_str_boundary(&self.text, cursor);
        if !extend {
            self.anchor = self.cursor;
        }
        self.marked = None;
    }

    pub(super) fn delete(&mut self, backward: bool) {
        if self.cursor == self.anchor {
            self.anchor = if backward {
                prev_char_byte_in_str(&self.text, self.cursor)
            } else {
                next_char_byte_in_str(&self.text, self.cursor)
            };
        }
        let _ = self.replace(None, "");
    }

    pub(super) fn selected(&self) -> &str {
        self.text
            .get(self.cursor.min(self.anchor)..self.cursor.max(self.anchor))
            .unwrap_or_default()
    }
}

pub(super) struct InputElement(pub(super) Entity<RunDialog>);

impl IntoElement for InputElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

fn shaped(text: &str, theme: &crate::theme::Theme, window: &mut Window) -> ShapedLine {
    let style = window.text_style();
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color: rgb(theme.text).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(
        SharedString::from(text.to_string()),
        style.font_size.to_pixels(window.rem_size()),
        &[run],
        None,
    )
}

impl gpui::Element for InputElement {
    type RequestLayoutState = ();
    type PrepaintState = (FocusHandle, ShapedLine, Pixels, usize, usize);

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let dialog = self.0.read(cx);
        let input = dialog.active_input();
        let line = shaped(&input.text, &dialog.theme(), window);
        let cursor = input.cursor;
        let offset = (line.x_for_index(cursor) - bounds.size.width + px(4.)).max(px(0.));
        (dialog.focus.clone(), line, offset, cursor, input.anchor)
    }
    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus, line, offset, cursor, anchor) = state;
        let theme = self.0.read(cx).theme();
        let height = window.line_height();
        let origin = point(
            bounds.left() - *offset,
            bounds.top() + (bounds.size.height - height).max(px(0.)) / 2.,
        );
        if cursor != anchor {
            let left = line.x_for_index((*cursor).min(*anchor));
            let right = line.x_for_index((*cursor).max(*anchor));
            window.paint_quad(fill(
                Bounds::new(point(origin.x + left, origin.y), size(right - left, height)),
                rgba(theme.selection),
            ));
        }
        let _ = line.paint(origin, height, gpui::TextAlign::Left, None, window, cx);
        if focus.is_focused(window) {
            window.paint_quad(fill(
                Bounds::new(
                    point(origin.x + line.x_for_index(*cursor), origin.y),
                    size(px(crate::consts::CARET_WIDTH_PX), height),
                ),
                rgb(theme.text),
            ));
        }
        window.handle_input(focus, ElementInputHandler::new(bounds, self.0.clone()), cx);
        let entity = self.0.clone();
        let offset = *offset;
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || event.button != gpui::MouseButton::Left
                || !bounds.contains(&event.position)
            {
                return;
            }
            cx.stop_propagation();
            entity.update(cx, |dialog, cx| {
                window.focus(&dialog.focus, cx);
                if event.click_count >= 2 {
                    let input = dialog.active_input_mut();
                    input.anchor = 0;
                    input.cursor = input.text.len();
                } else {
                    let line = shaped(&dialog.active_input().text, &dialog.theme(), window);
                    let byte = line.closest_index_for_x(event.position.x - bounds.left() + offset);
                    dialog
                        .active_input_mut()
                        .move_to(byte, event.modifiers.shift);
                }
                dialog.active_input_mut().dragging = true;
                cx.notify();
            });
        });
        let entity = self.0.clone();
        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble {
                return;
            }
            entity.update(cx, |dialog, cx| {
                if !dialog.active_input().dragging {
                    return;
                }
                let line = shaped(&dialog.active_input().text, &dialog.theme(), window);
                let byte = line.closest_index_for_x(event.position.x - bounds.left() + offset);
                dialog.active_input_mut().move_to(byte, true);
                cx.notify();
            });
        });
        let entity = self.0.clone();
        window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _, cx| {
            if phase == gpui::DispatchPhase::Bubble && event.button == gpui::MouseButton::Left {
                entity.update(cx, |dialog, _| dialog.active_input_mut().dragging = false);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::utf16::{str_byte_to_utf16, str_utf16_to_byte};
    #[test]
    fn command_and_name_undo_redo_restore_only_their_own_unicode_selection() {
        let mut command = CommandInput::default();
        command.set("tool café 中".into()).unwrap();
        command.replace(Some(5..11), "😀").unwrap();
        let edited = command.text.clone();
        let mut name = CommandInput::for_name();
        name.replace(None, "Name 中").unwrap();
        assert!(name.undo());
        assert!(name.text.is_empty());
        assert_eq!(command.text, edited);
        assert!(command.undo());
        assert_eq!(command.text, "tool café 中");
        assert_eq!((command.anchor, command.cursor), (0, command.text.len()));
        assert!(command.redo());
        assert_eq!(command.text, edited);
        assert!(name.redo());
        assert_eq!(name.text, "Name 中");
        assert!(command.undo());
        command.replace(None, "new").unwrap();
        assert!(!command.redo());
        assert!(
            name.set("x".repeat(crate::run_command::MAX_NAME_BYTES + 1))
                .is_err()
        );
        assert!(
            name.replace(None, &"x".repeat(crate::run_command::MAX_NAME_BYTES + 1))
                .is_err()
        );
    }

    #[test]
    fn marked_unicode_composition_undoes_once_even_when_first_update_is_unchanged() {
        let mut input = CommandInput::default();
        input.set("a".into()).unwrap();
        input.replace_composition(None, "a").unwrap();
        input.marked = Some(0..1);
        input.replace_composition(None, "a😀").unwrap();
        input.marked = Some(0..3);
        input.replace(None, "a😀").unwrap();
        assert_eq!(input.text, "a😀");
        assert_eq!(input.undo.len(), 1);
        assert!(input.undo());
        assert_eq!(input.text, "a");
        assert_eq!((input.anchor, input.cursor), (0, 1));
        assert!(input.marked.is_none());
        assert!(input.redo());
        assert_eq!(input.text, "a😀");
    }

    #[test]
    fn field_history_caps_snapshots_and_bytes_and_rejects_failed_edits() {
        let mut input = CommandInput::default();
        for index in 0..100 {
            input.anchor = 0;
            input.cursor = input.text.len();
            input
                .replace(None, &format!("{index}{}", "x".repeat(32_000)))
                .unwrap();
        }
        assert!(input.undo.len() <= 64);
        assert!(
            input
                .undo
                .iter()
                .map(|snapshot| snapshot.text.len())
                .sum::<usize>()
                <= 1024 * 1024
        );
        let before = input.text.clone();
        let count = input.undo.len();
        assert!(
            input
                .replace(None, &"x".repeat(crate::run_command::MAX_COMMAND_BYTES + 1))
                .is_err()
        );
        assert_eq!(input.text, before);
        assert_eq!(input.undo.len(), count);
        while input.undo() {}
        assert!(
            input
                .redo
                .iter()
                .map(|snapshot| snapshot.text.len())
                .sum::<usize>()
                <= 1024 * 1024
        );
    }
    #[test]
    fn edits_keep_unicode_boundaries_and_limit_input() {
        let mut input = CommandInput::default();
        input.set("café 中".into()).unwrap();
        assert!(input.set("tool\nnext".into()).is_err());
        assert!(
            input
                .set("x".repeat(crate::run_command::MAX_COMMAND_BYTES + 1))
                .is_err()
        );
        input.move_to(5, false);
        input.delete(true);
        assert_eq!(input.text, "caf 中");
        input.move_to(usize::MAX, false);
        input.delete(true);
        assert_eq!(input.text, "caf ");
        assert!(
            input
                .replace(None, &"x".repeat(crate::run_command::MAX_COMMAND_BYTES + 1))
                .is_err()
        );
        assert_eq!(input.text, "caf ");
    }
    #[test]
    fn utf16_replacement_overwrites_composition_and_filters_line_ends() {
        let mut input = CommandInput::default();
        input.set("A😀中Z".into()).unwrap();
        input.replace(Some(1..3), "é").unwrap();
        assert_eq!(input.text, "Aé中Z");
        input.marked = Some(1..2);
        input.replace(None, "界\r\n\0").unwrap();
        assert_eq!(input.text, "A界中Z");
        assert_eq!(str_byte_to_utf16(&input.text, input.cursor), 2);
        assert_eq!(str_utf16_to_byte(&input.text, 2), input.cursor);
    }
}
