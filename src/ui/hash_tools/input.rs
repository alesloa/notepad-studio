use std::{ops::Range, sync::Arc};
use gpui::{AnyWindowHandle, App, BackgroundExecutor, Bounds, ElementInputHandler, Entity, FocusHandle,
    InputContextToken, InputHandler, IntoElement, Pixels, Point, ShapedLine, SharedString, TextRun,
    UTF16Selection, Window, fill, point, px, rgb, rgba, size};
use ropey::Rope;
use super::{Dialog, jobs::{self, Budget, Permit}};
use crate::hash_tools::{HashError, MAX_TEXT_INPUT};

pub(super) struct Data { pub(super) rope: Rope, permit: Option<Permit>, executor: BackgroundExecutor }
impl Data {
    pub(super) fn new(rope: Rope, permit: Permit, executor: BackgroundExecutor) -> Arc<Self> {
        Arc::new(Self { rope, permit: Some(permit), executor })
    }
    pub(super) fn cost(bytes: usize) -> Result<usize, &'static str> {
        bytes.checked_mul(2).and_then(|n| n.checked_add(4096)).ok_or(jobs::MEMORY)
    }
    pub(super) fn empty(budget: &Budget, executor: &BackgroundExecutor) -> Result<Arc<Self>, &'static str> {
        Ok(Self::new(Rope::new(), budget.reserve(Self::cost(0)?)?, executor.clone()))
    }
}
impl Drop for Data {
    fn drop(&mut self) { jobs::dispose((std::mem::take(&mut self.rope), self.permit.take()), &self.executor); }
}
#[derive(Clone)]
pub(super) struct Snapshot { pub(super) data: Arc<Data>, pub(super) head: usize, pub(super) anchor: usize }
pub(super) struct Field {
    pub(super) data: Arc<Data>, pub(super) head: usize, pub(super) anchor: usize,
    pub(super) marked: Option<Range<usize>>, pub(super) dragging: bool, pub(super) top: usize,
    pub(super) undo: Option<Snapshot>, pub(super) redo: Option<Snapshot>, pub(super) focus: FocusHandle,
    pub(super) generation: Arc<()>,
}
impl Field {
    pub(super) fn new(data: Arc<Data>, focus: FocusHandle) -> Self {
        Self { data, head: 0, anchor: 0, marked: None, dragging: false, top: 0, undo: None, redo: None, focus, generation: Arc::new(()) }
    }
    pub(super) fn snapshot(&self) -> Snapshot { Snapshot { data: self.data.clone(), head: self.head, anchor: self.anchor } }
    pub(super) fn selection(&self) -> Range<usize> { self.head.min(self.anchor)..self.head.max(self.anchor) }
    pub(super) fn char_at_utf16(&self, at: usize) -> usize {
        self.data.rope.try_utf16_cu_to_char(at.min(self.data.rope.len_utf16_cu())).unwrap_or(self.data.rope.len_chars())
    }
    pub(super) fn utf16_at(&self, at: usize) -> usize {
        self.data.rope.try_char_to_utf16_cu(at.min(self.data.rope.len_chars())).unwrap_or(self.data.rope.len_utf16_cu())
    }
    pub(super) fn replace_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        range.map(|r| self.char_at_utf16(r.start)..self.char_at_utf16(r.end)).unwrap_or_else(|| self.marked.clone().unwrap_or_else(|| self.selection()))
    }
    pub(super) fn candidate(&self, range: Range<usize>, text: &str, budget: &Budget,
        executor: &BackgroundExecutor) -> Result<Snapshot, String> {
        candidate(&self.data, range, text, budget, executor)
    }
    pub(super) fn apply(&mut self, snapshot: Snapshot, composing: bool) {
        self.generation = Arc::new(());
        if self.marked.is_none() { self.undo = Some(self.snapshot()); }
        self.redo = None; self.data = snapshot.data; self.head = snapshot.head; self.anchor = snapshot.anchor;
        if !composing { self.marked = None; }
        let line = self.data.rope.try_char_to_line(self.head).unwrap_or(0);
        if line < self.top || line > self.top.saturating_add(8) { self.top = line.saturating_sub(4); }
    }
    pub(super) fn move_to(&mut self, at: usize, extend: bool) {
        self.generation = Arc::new(());
        self.head = at.min(self.data.rope.len_chars()); if !extend { self.anchor = self.head; } self.marked = None;
        let line = self.data.rope.try_char_to_line(self.head).unwrap_or(0);
        if line < self.top || line > self.top.saturating_add(8) { self.top = line.saturating_sub(4); }
    }
    pub(super) fn previous(&self) -> usize {
        let at = self.head.saturating_sub(1);
        if self.data.rope.get_char(at) == Some('\n') && at > 0 && self.data.rope.get_char(at - 1) == Some('\r') { at - 1 } else { at }
    }
    pub(super) fn next(&self) -> usize {
        let at = self.head.saturating_add(1).min(self.data.rope.len_chars());
        if self.data.rope.get_char(self.head) == Some('\r') && self.data.rope.get_char(at) == Some('\n') { at.saturating_add(1).min(self.data.rope.len_chars()) } else { at }
    }
    pub(super) fn undo(&mut self, redo: bool) -> bool {
        let snapshot = if redo { self.redo.take() } else { self.undo.take() };
        let Some(snapshot) = snapshot else { return false; };
        self.generation = Arc::new(());
        let current = self.snapshot();
        if redo { self.undo = Some(current); } else { self.redo = Some(current); }
        self.data = snapshot.data; self.head = snapshot.head; self.anchor = snapshot.anchor; self.marked = None; true
    }
}
pub(super) fn candidate(data: &Arc<Data>, range: Range<usize>, text: &str, budget: &Budget,
    executor: &BackgroundExecutor) -> Result<Snapshot, String> {
    let rope = &data.rope;
    if range.start > range.end || range.end > rope.len_chars() { return Err(HashError::Input.to_string()); }
    let removed = rope.get_slice(range.clone()).ok_or_else(|| HashError::Input.to_string())?.len_bytes();
    let bytes = rope.len_bytes().checked_sub(removed).and_then(|n| n.checked_add(text.len()))
        .filter(|&n| n <= MAX_TEXT_INPUT).ok_or_else(|| HashError::InputLimit.to_string())?;
    let permit = budget.reserve(Data::cost(bytes)?).map_err(str::to_string)?;
    let mut next = rope.clone();
    next.try_remove(range.clone()).map_err(|_| HashError::Input.to_string())?;
    next.try_insert(range.start, text).map_err(|_| HashError::Input.to_string())?;
    let head = range.start.checked_add(text.chars().count()).ok_or_else(|| HashError::Input.to_string())?;
    Ok(Snapshot { data: Data::new(next, permit, executor.clone()), head, anchor: head })
}

pub(super) struct InputElement(pub(super) Entity<Dialog>, pub(super) usize);
struct NativeInput {
    inner: ElementInputHandler<Dialog>, dialog: Entity<Dialog>, index: usize,
    window: AnyWindowHandle, context: InputContextToken, generation: Arc<()>,
}
impl NativeInput {
    fn new(bounds: Bounds<Pixels>, dialog: Entity<Dialog>, index: usize, window: &Window, cx: &App) -> Self {
        Self { inner: ElementInputHandler::new(bounds, dialog.clone()), generation: dialog.read(cx).fields.get(index)
                .map(|field| field.generation.clone()).unwrap_or_else(|| Arc::new(())),
            dialog, index, window: window.window_handle(), context: window.input_context_token() }
    }
    fn current(&self, window: &Window, cx: &App) -> bool {
        let dialog = self.dialog.read(cx);
        window.window_handle() == self.window && window.is_window_active() && window.input_context_token() == self.context
            && window.root::<Dialog>().flatten().is_some_and(|root| root == self.dialog)
            && dialog.opened && dialog.active == self.index
            && dialog.fields.get(self.index).is_some_and(|field| field.focus.is_focused(window) && Arc::ptr_eq(&field.generation, &self.generation))
    }
    fn editable(&self, window: &Window, cx: &App) -> bool {
        self.current(window, cx) && self.index == 0 && self.dialog.read(cx).kind == super::Kind::Text
    }
    fn refresh(&mut self, cx: &App) { if let Some(field) = self.dialog.read(cx).fields.get(self.index) { self.generation = field.generation.clone(); } }
}
impl InputHandler for NativeInput {
    fn selected_text_range(&mut self, ignore: bool, window: &mut Window, cx: &mut App) -> Option<UTF16Selection> {
        if !self.current(window, cx) { return None; } self.inner.selected_text_range(ignore, window, cx)
    }
    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        if !self.current(window, cx) { return None; } self.inner.marked_text_range(window, cx)
    }
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, window: &mut Window, cx: &mut App) -> Option<String> {
        if !self.current(window, cx) { return None; } self.inner.text_for_range(range, adjusted, window, cx)
    }
    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut App) {
        if !self.editable(window, cx) { return; } self.inner.replace_text_in_range(range, text, window, cx); self.refresh(cx);
    }
    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>, window: &mut Window, cx: &mut App) {
        if !self.editable(window, cx) { return; } self.inner.replace_and_mark_text_in_range(range, text, selected, window, cx); self.refresh(cx);
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        if !self.editable(window, cx) { return; } self.inner.unmark_text(window, cx); self.refresh(cx);
    }
    fn bounds_for_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        if !self.current(window, cx) { return None; } self.inner.bounds_for_range(range, window, cx)
    }
    fn character_index_for_point(&mut self, point: Point<Pixels>, window: &mut Window, cx: &mut App) -> Option<usize> {
        if !self.current(window, cx) { return None; } self.inner.character_index_for_point(point, window, cx)
    }
    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool { self.editable(window, cx) && self.inner.accepts_text_input(window, cx) }
}
impl IntoElement for InputElement { type Element = Self; fn into_element(self) -> Self { self } }
struct Row { line: ShapedLine, start: usize, end: usize }
pub(super) struct Painted { focus: FocusHandle, rows: Vec<Row>, head: usize, anchor: usize, theme: crate::theme::Theme }
fn rows(field: &Field, theme: &crate::theme::Theme, count: usize, window: &mut Window) -> Vec<Row> {
    let style = window.text_style(); let mut rows = Vec::new(); let mut remaining = 16_384usize;
    for index in field.top..field.top.saturating_add(count) {
        let Some(line) = field.data.rope.get_line(index) else { break; };
        let start = field.data.rope.try_line_to_char(index).unwrap_or(field.data.rope.len_chars());
        let caret = field.head.saturating_sub(start).min(line.len_chars());
        let row_limit = (remaining / count.saturating_sub(rows.len()).max(1)).min(1024);
        let skip = if field.head >= start && field.head <= start.saturating_add(line.len_chars()) { caret.saturating_sub(row_limit / 2) } else { 0 };
        let length = line.len_chars().saturating_sub(skip).min(row_limit); remaining = remaining.saturating_sub(length);
        let Some(slice) = line.get_slice(skip..skip.saturating_add(length)) else { break; };
        let text: String = slice.chars().take_while(|c| !matches!(c, '\r' | '\n')).collect();
        let chars = text.chars().count();
        let run = TextRun { len: text.len(), font: style.font(), color: rgb(theme.text).into(),
            background_color: None, underline: None, strikethrough: None };
        rows.push(Row { line: window.text_system().shape_line(SharedString::from(text), style.font_size.to_pixels(window.rem_size()), &[run], None),
            start: start.saturating_add(skip), end: start.saturating_add(skip).saturating_add(chars) });
        if remaining == 0 { break; }
    }
    rows
}
fn byte_at(row: &Row, char_index: usize) -> usize {
    row.line.text.char_indices().nth(char_index.saturating_sub(row.start)).map(|(byte, _)| byte).unwrap_or(row.line.text.len())
}
fn hit(rows: &[Row], position: gpui::Point<Pixels>, bounds: Bounds<Pixels>, height: Pixels) -> Option<usize> {
    let index = ((position.y - bounds.top()) / height).max(0.) as usize;
    let row = rows.get(index.min(rows.len().saturating_sub(1)))?;
    let byte = row.line.closest_index_for_x(position.x - bounds.left());
    Some(row.start.saturating_add(row.line.text.get(..byte).unwrap_or_default().chars().count()))
}
impl gpui::Element for InputElement {
    type RequestLayoutState = (); type PrepaintState = Painted;
    fn id(&self) -> Option<gpui::ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&gpui::GlobalElementId>, _: Option<&gpui::InspectorElementId>,
        window: &mut Window, cx: &mut App) -> (gpui::LayoutId, ()) {
        let mut style = gpui::Style::default(); style.size.width = gpui::relative(1.).into(); style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(&mut self, _: Option<&gpui::GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>,
        _: &mut (), window: &mut Window, cx: &mut App) -> Painted {
        let dialog = self.0.read(cx); let field = dialog.fields.get(self.1).unwrap_or(&dialog.fields[0]); let theme = dialog.theme(window);
        Painted { focus: field.focus.clone(), rows: rows(field, &theme, ((bounds.size.height / window.line_height()).ceil() as usize).min(256), window),
            head: field.head, anchor: field.anchor, theme }
    }
    fn paint(&mut self, _: Option<&gpui::GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>,
        _: &mut (), state: &mut Painted, window: &mut Window, cx: &mut App) {
        let height = window.line_height(); let theme = state.theme;
        for (index, row) in state.rows.iter().enumerate() {
            let origin = point(bounds.left(), bounds.top() + height * index as f32);
            if state.head != state.anchor {
                let left = state.head.min(state.anchor).clamp(row.start, row.end); let right = state.head.max(state.anchor).clamp(row.start, row.end);
                if left < right { window.paint_quad(fill(Bounds::new(point(origin.x + row.line.x_for_index(byte_at(row, left)), origin.y),
                    size(row.line.x_for_index(byte_at(row, right)) - row.line.x_for_index(byte_at(row, left)), height)), rgba(theme.selection))); }
            }
            let _ = row.line.paint(origin, height, gpui::TextAlign::Left, None, window, cx);
            if state.focus.is_focused(window) && state.head >= row.start && state.head <= row.end {
                window.paint_quad(fill(Bounds::new(point(origin.x + row.line.x_for_index(byte_at(row, state.head)), origin.y),
                    size(px(crate::consts::CARET_WIDTH_PX), height)), rgb(theme.text)));
            }
        }
        if state.focus.is_focused(window) { window.handle_input(&state.focus, NativeInput::new(bounds, self.0.clone(), self.1, window, cx), cx); }
        let entity = self.0.clone(); let index = self.1;
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != gpui::MouseButton::Left || !bounds.contains(&event.position) { return; }
            entity.update(cx, |dialog, cx| {
                dialog.active = index; let theme = dialog.theme(window);
                dialog.version = Arc::new(());
                if let Some(field) = dialog.fields.get_mut(index) {
                    let rows = rows(field, &theme, ((bounds.size.height / window.line_height()).ceil() as usize).min(256), window); if let Some(at) = hit(&rows, event.position, bounds, window.line_height()) { field.move_to(at, event.modifiers.shift); }
                    field.dragging = true; window.focus(&field.focus, cx);
                }
                cx.notify();
            }); cx.stop_propagation();
        });
        let entity = self.0.clone();
        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble { return; }
            entity.update(cx, |dialog, cx| {
                let theme = dialog.theme(window);
                if let Some(field) = dialog.fields.get_mut(index).filter(|field| field.dragging) {
                    if let Some(at) = hit(&rows(field, &theme, ((bounds.size.height / window.line_height()).ceil() as usize).min(256), window), event.position, bounds, window.line_height()) { field.move_to(at, true); cx.notify(); }
                    dialog.version = Arc::new(());
                }
            });
        });
        let entity = self.0.clone();
        window.on_mouse_event(move |_event: &gpui::MouseUpEvent, phase, _, cx| {
            if phase == gpui::DispatchPhase::Bubble { entity.update(cx, |dialog, _| { if let Some(field) = dialog.fields.get_mut(index) { field.dragging = false; } }); }
        });
        let entity = self.0.clone();
        window.on_mouse_event(move |event: &gpui::ScrollWheelEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble || !bounds.contains(&event.position) { return; }
            let delta = event.delta.pixel_delta(window.line_height()).y;
            entity.update(cx, |dialog, cx| { if let Some(field) = dialog.fields.get_mut(index) {
                let lines = (delta / window.line_height()) as isize;
                field.top = field.top.saturating_add_signed(-lines).min(field.data.rope.len_lines().saturating_sub(1)); cx.notify();
            } }); cx.stop_propagation();
        });
    }
}
