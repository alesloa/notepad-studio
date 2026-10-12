// MISC/md5/md5Dlgs.rc:27-61, md5Dlgs.cpp:134-510 — eight persistent dialogs.
mod input;
pub(crate) mod jobs;
mod files;
pub(crate) mod selection;
#[cfg(test)]
mod tests;
use std::{ops::Range, path::PathBuf, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}};
use gpui::{App, AppContext, Bounds, Context, Entity, EntityInputHandler, FocusHandle, Focusable, Global,
    IntoElement, KeyDownEvent, Pixels, Render, TitlebarOptions, UTF16Selection, Window,
    WindowHandle, WindowKind, WindowOptions, div, prelude::*, px, rems, rgb, size};
use ropey::Rope;
use crate::{actions::*, clipboard_ownership, hash_tools::{self, Algorithm, TextMode}, theme::{Theme, ThemeMode}, workspace::Workspace};
use input::{Data, Field, InputElement};
use jobs::{Budget, Delivery};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind { Text, Files }
struct Retained { entity: Entity<Dialog>, window: Option<WindowHandle<Dialog>> }
struct Dialogs([Option<Retained>; 8]);
impl Default for Dialogs { fn default() -> Self { Self(std::array::from_fn(|_| None)) } }
impl Global for Dialogs {}
struct Pending { marker: Arc<()>, cancel: Arc<AtomicBool>, progress: Arc<AtomicU64> }
impl Drop for Pending { fn drop(&mut self) { self.cancel.store(true, Ordering::Relaxed); } }
pub(crate) struct Dialog {
    algorithm: Algorithm, kind: Kind, fields: [Field; 3], active: usize, per_line: bool,
    focus: FocusHandle, buttons: [FocusHandle; 4],
    version: Arc<()>, pending: Option<Pending>, copy_cancel: Option<Arc<AtomicBool>>, copy_progress: Option<Arc<AtomicU64>>,
    ready: bool, opened: bool, status: String, mode: ThemeMode,
}
impl Focusable for Dialog { fn focus_handle(&self, _: &App) -> FocusHandle { self.initial_focus().clone() } }
impl Dialog {
    fn initial_focus(&self) -> &FocusHandle { if self.kind == Kind::Text { &self.fields[0].focus } else { &self.buttons[0] } }
    fn focus_initial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active = if self.kind == Kind::Text { 0 } else { 1 }; window.focus(self.initial_focus(), cx);
    }
    fn focused_field(&mut self, window: &Window) -> bool {
        let Some(index) = self.fields.iter().position(|field| field.focus.is_focused(window)) else { return false; };
        self.active = index; true
    }
    fn copy_enabled(&self) -> bool {
        self.ready && self.pending.is_none() && self.fields[if self.kind == Kind::Text { 1 } else { 2 }].data.rope.len_chars() > 0
    }
    fn control(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.buttons.get(index) { window.focus(focus, cx); }
        match index {
            0 if self.kind == Kind::Text => { self.per_line = !self.per_line; self.changed(cx); }
            0 => self.choose(cx),
            1 if self.copy_enabled() => self.copy_field(true, false, cx),
            2 if self.pending.is_some() || self.copy_cancel.is_some() => self.cancel(cx),
            3 => self.close(window, cx), _ => {},
        }
    }
    fn tab(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        let order = if self.kind == Kind::Text { [0, 3, 1, 4, 5, 6] } else { [3, 1, 2, 4, 5, 6] };
        let order: Vec<usize> = order.into_iter().filter(|&node| node != 4 || self.copy_enabled())
            .filter(|&node| node != 5 || self.pending.is_some() || self.copy_cancel.is_some()).collect();
        let focus = |node: usize| if node < 3 { self.fields.get(node).map(|field| &field.focus) } else { self.buttons.get(node - 3) };
        let current = order.iter().position(|&node| focus(node).is_some_and(|focus| focus.is_focused(window)));
        let next = if reverse { current.unwrap_or(0).checked_sub(1).unwrap_or_else(|| order.len().saturating_sub(1)) }
            else { current.map(|index| index.saturating_add(1)).unwrap_or(0) % order.len().max(1) };
        if let Some(&node) = order.get(next) {
            if let Some(focus) = focus(node) { window.focus(focus, cx); }
            if node < 3 { self.active = node; } self.version = Arc::new(()); cx.notify();
        }
    }
    fn theme(&self, window: &Window) -> Theme {
        let settings = crate::settings::snapshot();
        Theme::resolve(self.mode, window.appearance(), settings.tone_palette, &settings.customized_slots_array())
    }
    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        if let Some(cancel) = self.copy_cancel.take() { cancel.store(true, Ordering::Relaxed); }
        self.copy_progress = None; self.version = Arc::new(());
        self.status = "Hash cancelled; previous results retained".into(); cx.notify();
    }
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel(cx); self.opened = false; window.remove_window();
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.version = Arc::new(()); self.ready = false;
        if let Some(cancel) = self.copy_cancel.take() { cancel.store(true, Ordering::Relaxed); }
        self.copy_progress = None;
        self.recompute(cx);
    }
    fn recompute(&mut self, cx: &mut Context<Self>) {
        if self.kind != Kind::Text { return; }
        self.pending = None;
        let budget = cx.default_global::<Budget>().clone(); let executor = cx.background_executor().clone();
        let queued = match budget.reserve(4096) {
            Ok(queued) => queued, Err(message) => { self.status = message.into(); cx.notify(); return; }
        };
        let marker = Arc::new(()); let cancel = Arc::new(AtomicBool::new(false)); let progress = Arc::new(AtomicU64::new(0));
        let pending = Pending { marker: marker.clone(), cancel: cancel.clone(), progress: progress.clone() };
        let data = self.fields[0].data.clone(); let algorithm = self.algorithm;
        let mode = if self.per_line { TextMode::PerLine } else { TextMode::Whole };
        let weak = cx.entity().downgrade(); let worker_executor = executor.clone();
        let result = jobs::run(cx, cancel.clone(), move || {
            let _queued = queued;
            if cancel.load(Ordering::Relaxed) { return Err(hash_tools::HashError::Cancelled.to_string()); }
            let output = if mode == TextMode::Whole { 4096 } else { 3 * hash_tools::MAX_TEXT_OUTPUT };
            let mut permit = budget.reserve(output + hash_tools::CHUNK).map_err(str::to_string)?;
            let text = hash_tools::text_result(algorithm, &data.rope, mode, &cancel, &progress).map_err(|error| error.to_string())?;
            let cost = Data::cost(text.len()).map_err(str::to_string)?;
            let rope = Rope::from_str(&text); drop(text); permit.shrink(cost);
            Ok::<_, String>(Data::new(rope, permit, worker_executor))
        }, move |result, cx| {
            let mut result = Delivery::new(result, executor);
            let _ = weak.update(cx, |dialog, cx| {
                if !dialog.opened || !dialog.pending.as_ref().is_some_and(|pending| Arc::ptr_eq(&pending.marker, &marker)) { return; }
                dialog.pending = None;
                match result.take() {
                    Some(Ok(data)) => { dialog.set_output(1, data); dialog.ready = true; dialog.status.clear(); }
                    Some(Err(error)) => dialog.status = error.to_string(), _ => {},
                }
                cx.notify();
            });
        });
        match result {
            Ok(()) => { self.pending = Some(pending); self.status = "Hashing text; Cancel stops this operation".into(); self.poll_progress(cx); }
            Err(message) => self.status = message.into(),
        }
        cx.notify();
    }
    fn poll_progress(&self, cx: &mut Context<Self>) {
        let Some(marker) = self.pending.as_ref().map(|pending| pending.marker.clone()) else { return; };
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                let active = this.update(cx, |dialog, cx| {
                    let active = dialog.opened && dialog.pending.as_ref().is_some_and(|pending| Arc::ptr_eq(&pending.marker, &marker));
                    if active { cx.notify(); } active
                }).unwrap_or(false);
                if !active { break; }
            }
        }).detach();
    }
    fn set_output(&mut self, index: usize, data: Arc<Data>) {
        if let Some(cancel) = self.copy_cancel.take() { cancel.store(true, Ordering::Relaxed); }
        self.copy_progress = None; self.version = Arc::new(());
        if let Some(field) = self.fields.get_mut(index) { field.generation = Arc::new(()); field.data = data; field.head = 0; field.anchor = 0; field.top = 0; field.marked = None; }
    }
    fn replace(&mut self, range: Option<Range<usize>>, text: &str, composing: bool, cx: &mut Context<Self>) {
        if self.kind != Kind::Text || self.active != 0 { return; }
        if text.len() > hash_tools::CHUNK { self.status = "Native text commit exceeds 64 KiB; paste large text instead".into(); cx.notify(); return; }
        let budget = cx.default_global::<Budget>().clone(); let range = self.fields[0].replace_range(range);
        match self.fields[0].candidate(range, text, &budget, cx.background_executor()) {
            Ok(candidate) => { self.fields[0].apply(candidate, composing); self.changed(cx); }
            Err(error) => { self.status = error; cx.notify(); }
        }
    }
    fn copy_field(&mut self, whole_output: bool, cut: bool, cx: &mut Context<Self>) {
        let index = if whole_output { if self.kind == Kind::Text { 1 } else { 2 } } else { self.active };
        if whole_output && (!self.ready || self.pending.is_some()) { return; }
        let Some(field) = self.fields.get(index) else { return; };
        let range = if whole_output { 0..field.data.rope.len_chars() } else { field.selection() };
        if range.is_empty() { return; }
        let data = field.data.clone(); let bytes = data.rope.get_slice(range.clone()).map(|s| s.len_bytes()).unwrap_or(0);
        let budget = cx.default_global::<Budget>().clone(); let executor = cx.background_executor().clone();
        let permit = match budget.reserve(bytes.saturating_mul(2).saturating_add(4096)) {
            Ok(permit) => permit, Err(message) => { self.status = message.into(); cx.notify(); return; }
        };
        if let Some(cancel) = self.copy_cancel.take() { cancel.store(true, Ordering::Relaxed); }
        let cancel = Arc::new(AtomicBool::new(false)); self.copy_cancel = Some(cancel.clone());
        let completed = cancel.clone(); let progress = Arc::new(AtomicU64::new(0)); self.copy_progress = Some(progress.clone());
        let owner = clipboard_ownership::claim(cancel.clone()); let version = self.version.clone(); let weak = cx.entity().downgrade();
        let cut = cut && self.kind == Kind::Text && index == 0;
        let worker_budget = budget.clone(); let worker_executor = executor.clone(); let selection = range.clone();
        let result = jobs::run(cx, cancel.clone(), move || {
            let mut text = String::new(); text.try_reserve_exact(bytes).map_err(|_| "Not enough memory to copy hash text")?;
            let slice = data.rope.get_slice(range).ok_or("Hash text selection changed")?;
            for chunk in slice.chunks() { if cancel.load(Ordering::Relaxed) { return Err("Hash copy cancelled"); } text.push_str(chunk); progress.fetch_add(chunk.len() as u64, Ordering::Relaxed); }
            let history = crate::clipboard_history::prepare_copy(&text, &cancel)?;
            let replacement = if cut { Some(input::candidate(&data, selection, "", &worker_budget, &worker_executor).map_err(|_| "Not enough memory to cut hash input")?) } else { None };
            Ok((text, history, replacement, permit))
        }, move |result, cx| {
            let mut delivery = Delivery::new(result, executor.clone());
            let _ = weak.update(cx, |dialog, cx| {
                if !dialog.copy_cancel.as_ref().is_some_and(|cancel| Arc::ptr_eq(cancel, &completed)) { return; }
                dialog.copy_cancel = None; dialog.copy_progress = None;
                if completed.load(Ordering::Relaxed) || !dialog.opened || !Arc::ptr_eq(&dialog.version, &version) || !clipboard_ownership::is_current(&owner) {
                    dialog.status = "Hash copy cancelled because the input or clipboard request changed".into(); cx.notify(); return;
                }
                match delivery.take() {
                    Some(Ok((text, history, replacement, _permit))) => {
                        if let Some(replacement) = replacement { dialog.fields[0].apply(replacement, false); dialog.changed(cx); }
                        jobs::dispose(crate::clipboard_history::record_copy_owned(history), &executor);
                        clipboard_ownership::publish(&owner, cx, gpui::ClipboardItem::new_string(text));
                        if !cut { dialog.status = "Copied hash text".into(); }
                    }
                    Some(Err(error)) => dialog.status = error.into(), _ => {},
                }
                cx.notify();
            });
        });
        if let Err(message) = result { self.copy_cancel = None; self.copy_progress = None; self.status = message.into(); cx.notify(); }
        else {
            self.status = "Preparing hash text copy; Cancel stops this operation".into();
            let cancel = self.copy_cancel.clone();
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                    let active = this.update(cx, |dialog, cx| {
                        let active = dialog.opened && dialog.copy_cancel.as_ref().zip(cancel.as_ref()).is_some_and(|(current, expected)| Arc::ptr_eq(current, expected));
                        if active { cx.notify(); } active
                    }).unwrap_or(false);
                    if !active { break; }
                }
            }).detach(); cx.notify();
        }
    }
    fn paste(&mut self, _: &EditPaste, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focused_field(window) || self.kind != Kind::Text || self.active != 0 { return; }
        let Some(item) = cx.read_from_clipboard() else { return; };
        if item.entries().len() > files::MAX_FILES { jobs::dispose(item, cx.background_executor()); self.status = "Hash paste supports at most 4,096 clipboard entries".into(); cx.notify(); return; }
        let bytes = item.entries().iter().filter_map(|entry| match entry { gpui::ClipboardEntry::String(s) => Some(s.text().len()), _ => None })
            .try_fold(0usize, usize::checked_add);
        let Some(bytes) = bytes.filter(|&n| n <= hash_tools::MAX_TEXT_INPUT) else {
            jobs::dispose(item, cx.background_executor()); self.status = hash_tools::HashError::InputLimit.to_string(); cx.notify(); return;
        };
        if bytes == 0 { jobs::dispose(item, cx.background_executor()); return; }
        let budget = cx.default_global::<Budget>().clone(); let executor = cx.background_executor().clone();
        let mut permit = match budget.reserve(bytes.saturating_mul(2).saturating_add(4096)) {
            Ok(permit) => permit, Err(message) => { jobs::dispose(item, &executor); self.status = message.into(); cx.notify(); return; }
        };
        self.pending = None;
        let marker = Arc::new(()); let cancel = Arc::new(AtomicBool::new(false)); let progress = Arc::new(AtomicU64::new(0));
        let pending = Pending { marker: marker.clone(), cancel: cancel.clone(), progress };
        let data = self.fields[0].data.clone(); let range = self.fields[0].selection(); let version = self.version.clone(); let weak = cx.entity().downgrade();
        let worker_executor = executor.clone();
        let result = jobs::run(cx, cancel.clone(), move || {
            let mut text = String::new(); text.try_reserve_exact(bytes).map_err(|_| "Not enough memory to paste hash input".to_string())?;
            for entry in item.into_entries() {
                if cancel.load(Ordering::Relaxed) { return Err("Hash paste cancelled".into()); }
                if let gpui::ClipboardEntry::String(entry) = entry { text.push_str(entry.text()); }
            }
            permit.shrink(bytes.saturating_add(4096));
            input::candidate(&data, range, &text, &budget, &worker_executor)
        }, move |result, cx| {
            let mut result = Delivery::new(result, executor);
            let _ = weak.update(cx, |dialog, cx| {
                if !dialog.opened || !dialog.pending.as_ref().is_some_and(|pending| Arc::ptr_eq(&pending.marker, &marker)) { return; }
                dialog.pending = None;
                if !Arc::ptr_eq(&dialog.version, &version) { dialog.status = "Hash paste cancelled because input or selection changed".into(); cx.notify(); return; }
                match result.take() {
                    Some(Ok(candidate)) => { dialog.fields[0].apply(candidate, false); dialog.changed(cx); }
                    Some(Err(error)) => dialog.status = error, _ => {},
                }
                cx.notify();
            });
        });
        match result { Ok(()) => { self.pending = Some(pending); self.status = "Preparing pasted hash input; Cancel stops this operation".into(); self.poll_progress(cx); }, Err(message) => self.status = message.into() }
        cx.notify();
    }
    fn select_all(&mut self, _: &EditSelectAll, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focused_field(window) { return; }
        self.version = Arc::new(());
        if let Some(field) = self.fields.get_mut(self.active) { field.generation = Arc::new(()); field.anchor = 0; field.head = field.data.rope.len_chars(); } cx.notify();
    }
    fn copy(&mut self, _: &EditCopy, window: &mut Window, cx: &mut Context<Self>) { if self.focused_field(window) { self.copy_field(false, false, cx); } }
    fn cut(&mut self, _: &EditCut, window: &mut Window, cx: &mut Context<Self>) { if self.focused_field(window) { self.copy_field(false, true, cx); } }
    fn undo(&mut self, _: &EditUndo, window: &mut Window, cx: &mut Context<Self>) { if self.focused_field(window) && self.active == 0 && self.kind == Kind::Text && self.fields[0].undo(false) { self.changed(cx); } }
    fn redo(&mut self, _: &EditRedo, window: &mut Window, cx: &mut Context<Self>) { if self.focused_field(window) && self.active == 0 && self.kind == Kind::Text && self.fields[0].undo(true) { self.changed(cx); } }
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str(); let mods = event.keystroke.modifiers;
        if key == "escape" { if self.pending.is_some() || self.copy_cancel.is_some() { self.cancel(cx); } else { self.close(window, cx); } cx.stop_propagation(); return; }
        if key == "tab" { self.tab(mods.shift, window, cx); cx.stop_propagation(); return; }
        if matches!(key, "enter" | "space") {
            if let Some(index) = self.buttons.iter().position(|focus| focus.is_focused(window)) {
                self.control(index, window, cx); cx.stop_propagation(); return;
            }
        }
        if !self.focused_field(window) { return; }
        if mods.control || mods.platform {
            match key { "a" => self.select_all(&EditSelectAll, window, cx), "c" => self.copy(&EditCopy, window, cx),
                "x" => self.cut(&EditCut, window, cx), "v" => self.paste(&EditPaste, window, cx),
                "z" if mods.shift => self.redo(&EditRedo, window, cx), "z" => self.undo(&EditUndo, window, cx),
                "y" => self.redo(&EditRedo, window, cx), _ => return }
            cx.stop_propagation(); return;
        }
        if matches!(key, "left" | "right" | "up" | "down" | "home" | "end") { self.version = Arc::new(()); }
        let Some(field) = self.fields.get_mut(self.active) else { return; };
        match key {
            "left" => field.move_to(field.previous(), mods.shift),
            "right" => field.move_to(field.next(), mods.shift),
            "up" | "down" => {
                let line = field.data.rope.try_char_to_line(field.head).unwrap_or(0);
                let column = field.head.saturating_sub(field.data.rope.try_line_to_char(line).unwrap_or(0));
                let target = if key == "up" { line.saturating_sub(1) } else { line.saturating_add(1).min(field.data.rope.len_lines().saturating_sub(1)) };
                if let Some(byte) = crate::workspace::column_plan::row(&field.data.rope, target).ok().and_then(|row| row.byte(column).ok()) {
                    field.move_to(field.data.rope.try_byte_to_char(byte).unwrap_or(field.head), mods.shift);
                }
            }
            "home" => { let line = field.data.rope.try_char_to_line(field.head).unwrap_or(0); field.move_to(field.data.rope.try_line_to_char(line).unwrap_or(0), mods.shift); }
            "end" => { let line = field.data.rope.try_char_to_line(field.head).unwrap_or(0); let row = crate::workspace::column_plan::row(&field.data.rope, line).ok();
                let at = row.and_then(|row| row.byte(usize::MAX).ok()).and_then(|at| field.data.rope.try_byte_to_char(at).ok()).unwrap_or(field.data.rope.len_chars()); field.move_to(at, mods.shift); }
            "enter" if self.active == 0 && self.kind == Kind::Text => { if field.marked.is_some() { return; } self.replace(None, "\r\n", false, cx); }
            "backspace" | "delete" if self.active == 0 && self.kind == Kind::Text => {
                let mut range = field.selection();
                if range.is_empty() { if key == "backspace" { range.start = field.previous(); } else { range.end = field.next(); } }
                let range = field.utf16_at(range.start)..field.utf16_at(range.end);
                self.replace(Some(range), "", false, cx);
            }
            _ => return,
        }
        cx.stop_propagation(); cx.notify();
    }
}

impl EntityInputHandler for Dialog {
    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool { self.kind == Kind::Text && self.active == 0 }
    fn text_for_range(&mut self, range: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let field = self.fields.get(self.active)?; let start = field.char_at_utf16(range.start); let end = field.char_at_utf16(range.end).max(start);
        let slice = field.data.rope.get_slice(start..end)?; if slice.len_bytes() > hash_tools::CHUNK { return None; }
        let mut text = String::new(); text.try_reserve_exact(slice.len_bytes()).ok()?;
        for chunk in slice.chunks() { text.push_str(chunk); }
        *actual = Some(field.utf16_at(start)..field.utf16_at(end)); Some(text)
    }
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let field = self.fields.get(self.active)?; let range = field.selection();
        Some(UTF16Selection { range: field.utf16_at(range.start)..field.utf16_at(range.end), reversed: field.head < field.anchor })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let field = self.fields.get(self.active)?; field.marked.as_ref().map(|r| field.utf16_at(r.start)..field.utf16_at(r.end))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) { if let Some(field) = self.fields.get_mut(self.active) { if field.marked.take().is_some() { field.generation = Arc::new(()); } } cx.notify(); }
    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) { self.replace(range, text, false, cx); }
    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        if self.kind != Kind::Text || self.active != 0 || text.len() > hash_tools::CHUNK { return; }
        let old = self.fields[0].data.clone(); self.replace(range, text, true, cx);
        if Arc::ptr_eq(&old, &self.fields[0].data) { return; }
        let field = &mut self.fields[0]; let end = field.head; let start = end.saturating_sub(text.chars().count()); field.marked = Some(start..end);
        if let Some(selected) = selected { let base = field.utf16_at(start); let limit = field.utf16_at(end);
            field.anchor = field.char_at_utf16(base.saturating_add(selected.start).min(limit)); field.head = field.char_at_utf16(base.saturating_add(selected.end).min(limit)); }
    }
    fn bounds_for_range(&mut self, _: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> { Some(bounds) }
    fn character_index_for_point(&mut self, _: gpui::Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> { None }
}

impl Render for Dialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme(window); let focus = self.focus.clone();
        let fields: [FocusHandle; 3] = std::array::from_fn(|index| self.fields[index].focus.clone());
        let buttons = self.buttons.clone();
        let field = |index, label, cx: &mut Context<Self>| div().flex().flex_col().flex_1().min_h_0().gap(rems(0.3)).child(label)
            .child(div().id(("hash-field", index)).debug_selector(move || format!("hash-field-{index}"))
                .track_focus(&fields[index]).flex_1().min_h_0().border_1().border_color(rgb(theme.border)).bg(rgb(theme.bg))
                .p(rems(0.3)).overflow_hidden().child(InputElement(cx.entity(), index)));
        let button = |id: &'static str, label: String, enabled: bool, index| div().id(id).debug_selector(move || id.to_string())
            .track_focus(&buttons[index]).px(rems(0.7)).py(rems(0.3))
            .border_1().border_color(rgb(theme.border)).bg(rgb(theme.button_bg))
            .text_color(rgb(if enabled { theme.text } else { theme.disabled_text })).child(label);
        let copy_enabled = self.copy_enabled();
        let mut root = div().id("hash-tools-dialog").key_context("HashTools").track_focus(&focus).size_full().flex().flex_col()
            .p(rems(0.8)).gap(rems(0.6)).bg(rgb(theme.chrome)).text_color(rgb(theme.text)).text_size(rems(0.8))
            .on_key_down(cx.listener(Self::key_down)).on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy)).on_action(cx.listener(Self::cut)).on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo)).on_action(cx.listener(Self::redo));
        if self.kind == Kind::Text {
            root = root.child(field(0, "Text to hash", cx))
                .child(div().id("hash-per-line").debug_selector(|| "hash-per-line".into()).cursor(gpui::CursorStyle::PointingHand)
                    .track_focus(&buttons[0])
                    .child(if self.per_line { "[x] Treat each line as a separate string" } else { "[ ] Treat each line as a separate string" })
                    .on_click(cx.listener(|dialog, _, window, cx| dialog.control(0, window, cx))))
                .child(field(1, "Hash", cx));
        } else {
            root = root.child(button("hash-choose", format!("Choose files to generate {}...", self.algorithm.label()), true, 0)
                .on_click(cx.listener(|dialog, _, window, cx| dialog.control(0, window, cx))))
                .child(field(1, "Files", cx)).child(field(2, "Hashes", cx));
        }
        let progress = self.pending.as_ref().map(|pending| pending.progress.load(Ordering::Relaxed))
            .or_else(|| self.copy_progress.as_ref().map(|progress| progress.load(Ordering::Relaxed)));
        root.child(div().min_h(rems(1.2)).child(if let Some(bytes) = progress { format!("{}  ({bytes} bytes)", self.status) } else { self.status.clone() }))
            .child(div().flex().justify_end().gap(rems(0.5))
                .child(button("hash-copy", "Copy to Clipboard".into(), copy_enabled, 1).on_click(cx.listener(|dialog, _, window, cx| dialog.control(1, window, cx))))
                .child(button("hash-cancel", "Cancel".into(), self.pending.is_some() || self.copy_cancel.is_some(), 2).on_click(cx.listener(|dialog, _, window, cx| dialog.control(2, window, cx))))
                .child(button("hash-close", "Close".into(), true, 3).on_click(cx.listener(|dialog, _, window, cx| dialog.control(3, window, cx)))))
    }
}
impl Dialog {
    fn choose(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        if let Some(paths) = cx.default_global::<tests::Picker>().0.take() { self.start_files(paths, None, cx); return; }
        self.pending = None; let marker = Arc::new(()); let cancel = Arc::new(AtomicBool::new(false));
        self.pending = Some(Pending { marker: marker.clone(), cancel: cancel.clone(), progress: Arc::new(AtomicU64::new(0)) });
        self.status = "Preparing hash paths; Cancel stops this operation".into();
        cx.spawn(async move |this, cx| {
            let paths = rfd::AsyncFileDialog::new().set_title("Choose files to hash").pick_files().await;
            let Some(paths) = paths else { let _ = this.update(cx, |dialog, cx| { if dialog.pending.as_ref().is_some_and(|p| Arc::ptr_eq(&p.marker, &marker)) { dialog.pending = None; cx.notify(); } }); return; };
            let permit = cx.update(|cx| cx.default_global::<Budget>().reserve(files::MAX_METADATA)).ok();
            let Some(permit) = permit else {
                jobs::dispose(paths, cx.background_executor());
                let _ = this.update(cx, |dialog, cx| { if dialog.pending.as_ref().is_some_and(|p| Arc::ptr_eq(&p.marker, &marker)) { dialog.pending = None; dialog.status = jobs::MEMORY.into(); cx.notify(); } }); return;
            };
            let executor = cx.background_executor().clone();
            let mut picked = Delivery::new((paths, permit), executor.clone()); let worker_cancel = cancel.clone(); let disposer = executor.clone();
            let (sender, receiver) = futures::channel::oneshot::channel();
            let accepted = cx.update(|cx| jobs::run(cx, cancel.clone(), move || {
                let Some((paths, permit)) = picked.take() else { return Err("Hash path preparation cancelled"); };
                let prepared = (|| {
                    if paths.len() > files::MAX_FILES { return Err("Choose at most 4,096 files to hash"); }
                    let mut chosen = Vec::new(); chosen.try_reserve_exact(paths.len()).map_err(|_| "Not enough memory to prepare hash paths")?;
                    let mut bytes = paths.len().saturating_mul(std::mem::size_of::<PathBuf>()); let mut exceeds = false;
                    for path in paths {
                        if worker_cancel.load(Ordering::Relaxed) { return Err("Hash path preparation cancelled"); }
                        bytes = bytes.checked_add(path.path().as_os_str().as_encoded_bytes().len().saturating_mul(4))
                            .filter(|&n| n <= files::MAX_METADATA).ok_or("Hash paths exceed the 16 MiB metadata limit; choose fewer files")?;
                        if !exceeds { exceeds = std::fs::metadata(path.path()).ok().is_some_and(|m| m.len() > 2 * 1024 * 1024 * 1024); }
                        chosen.push(path.path().to_path_buf());
                    }
                    Ok((chosen, exceeds))
                })(); prepared.map(|(paths, exceeds)| (paths, exceeds, permit))
            }, move |result, _| { let _ = sender.send(Delivery::new(result, disposer)); }));
            if let Err(message) = accepted {
                let _ = this.update(cx, |dialog, cx| { if dialog.pending.as_ref().is_some_and(|p| Arc::ptr_eq(&p.marker, &marker)) { dialog.pending = None; dialog.status = message.into(); cx.notify(); } }); return;
            }
            let Ok(mut prepared) = receiver.await else { return; };
            let (paths, exceeds, permit) = match prepared.take() {
                Some(Ok(prepared)) => prepared,
                Some(Err(message)) => { let _ = this.update(cx, |dialog, cx| { if dialog.pending.as_ref().is_some_and(|p| Arc::ptr_eq(&p.marker, &marker)) { dialog.pending = None; dialog.status = message.into(); cx.notify(); } }); return; }
                None => return,
            };
            let mut delivery = Delivery::new((paths, permit), executor);
            if cancel.load(Ordering::Relaxed) { return; }
            if exceeds {
                let answer = rfd::AsyncMessageDialog::new().set_title(crate::consts::APP_NAME).set_description("One or more files exceed 2 GiB. Continue hashing?")
                    .set_buttons(rfd::MessageButtons::YesNo).show().await;
                if !matches!(answer, rfd::MessageDialogResult::Yes) {
                    let _ = this.update(cx, |dialog, cx| { if dialog.opened && dialog.pending.as_ref().is_some_and(|p| Arc::ptr_eq(&p.marker, &marker)) { dialog.cancel(cx); } }); return;
                }
            }
            let _ = this.update(cx, |dialog, cx| {
                if !dialog.opened || !dialog.pending.as_ref().is_some_and(|p| Arc::ptr_eq(&p.marker, &marker)) { return; }
                if let Some((paths, permit)) = delivery.take() { dialog.start_files(paths, Some(permit), cx); }
            });
        }).detach(); cx.notify();
    }
    fn start_files(&mut self, paths: Vec<PathBuf>, permit: Option<jobs::Permit>, cx: &mut Context<Self>) {
        self.pending = None;
        if paths.is_empty() { return; }
        let bytes = paths.iter().try_fold(0usize, |sum, path| sum.checked_add(path.as_os_str().as_encoded_bytes().len().saturating_mul(4)));
        if paths.len() > files::MAX_FILES || bytes.is_none_or(|n| n > files::MAX_METADATA) {
            jobs::dispose((paths, permit), cx.background_executor()); self.status = "Choose at most 4,096 files with at most 16 MiB of path metadata".into(); cx.notify(); return;
        }
        let budget = cx.default_global::<Budget>().clone(); let executor = cx.background_executor().clone();
        let permit = match permit.map(Ok).unwrap_or_else(|| budget.reserve(files::MAX_METADATA)) { Ok(permit) => permit, Err(message) => { jobs::dispose(paths, &executor); self.status = message.into(); cx.notify(); return; } };
        let marker = Arc::new(()); let cancel = Arc::new(AtomicBool::new(false)); let progress = Arc::new(AtomicU64::new(0));
        let pending = Pending { marker: marker.clone(), cancel: cancel.clone(), progress: progress.clone() };
        let algorithm = self.algorithm; let weak = cx.entity().downgrade(); let worker_executor = executor.clone();
        let result = jobs::run(cx, cancel.clone(), move || {
            let _permit = permit; let result = files::prepare(paths, algorithm, &cancel, &progress)?;
            let paths = Data::new(Rope::from_str(&result.paths), budget.reserve(Data::cost(result.paths.len())?)?, worker_executor.clone());
            let hashes = Data::new(Rope::from_str(&result.hashes), budget.reserve(Data::cost(result.hashes.len())?)?, worker_executor);
            Ok::<_, String>((paths, hashes, result.status))
        }, move |result, cx| {
            let mut result = Delivery::new(result, executor);
            let _ = weak.update(cx, |dialog, cx| {
                if !dialog.opened || !dialog.pending.as_ref().is_some_and(|p| Arc::ptr_eq(&p.marker, &marker)) { return; }
                dialog.pending = None;
                match result.take() {
                    Some(Ok((paths, hashes, status))) => { dialog.set_output(1, paths); dialog.set_output(2, hashes); dialog.ready = true; dialog.status = status; }
                    Some(Err(error)) => dialog.status = error, _ => {},
                } cx.notify();
            });
        });
        match result { Ok(()) => { self.pending = Some(pending); self.status = "Hashing files; Cancel stops this operation".into(); self.poll_progress(cx); }, Err(message) => self.status = message.into() }
        cx.notify();
    }
}
fn slot(algorithm: Algorithm, kind: Kind) -> usize {
    let algorithm = match algorithm { Algorithm::Md5 => 0, Algorithm::Sha1 => 1, Algorithm::Sha256 => 2, Algorithm::Sha512 => 3 };
    algorithm * 2 + usize::from(kind == Kind::Files)
}
fn caption(algorithm: Algorithm, kind: Kind) -> String {
    format!("Generate {} digest{}", algorithm.label(), if kind == Kind::Files { " from files" } else { "" })
}
pub(crate) fn open(algorithm: Algorithm, kind: Kind, cx: &mut App) {
    let index = slot(algorithm, kind);
    let existing = cx.default_global::<Dialogs>().0.get(index).and_then(Option::as_ref).map(|dialog| (dialog.entity.clone(), dialog.window));
    if let Some((_, Some(handle))) = existing.as_ref() {
        if handle.update(cx, |dialog, window, cx| { dialog.opened = true; window.activate_window(); dialog.focus_initial(window, cx); cx.notify(); }).is_ok() { return; }
    }
    let source = cx.active_window().and_then(|h| h.downcast::<Workspace>()).or_else(|| cx.windows().into_iter().find_map(|h| h.downcast::<Workspace>()));
    let (mode, zoom) = source.and_then(|handle| handle.read(cx).ok().map(|w| (w.theme_mode, w.zoom_level))).unwrap_or((ThemeMode::Dark, crate::consts::ZOOM_DEFAULT));
    let entity = if let Some((entity, _)) = existing { entity } else {
        let budget = cx.default_global::<Budget>().clone(); let executor = cx.background_executor().clone();
        let empty = match Data::empty(&budget, &executor) { Ok(data) => data, Err(message) => { if let Some(source) = source { let _ = source.update(cx, |w, _, cx| { w.set_status(message); cx.notify(); }); } return; } };
        cx.new(|cx| Dialog { algorithm, kind, fields: std::array::from_fn(|_| Field::new(empty.clone(), cx.focus_handle())), active: 0, per_line: false,
            focus: cx.focus_handle(), buttons: std::array::from_fn(|_| cx.focus_handle()),
            version: Arc::new(()), pending: None, copy_cancel: None, copy_progress: None, ready: false, opened: true, status: String::new(), mode })
    };
    let bounds = crate::app::window::modal_bounds_centered_on_workspace(cx, size(px(640.), px(460.)));
    let title = caption(algorithm, kind); let window_title = title.clone(); let root = entity.clone();
    let result = cx.open_window(WindowOptions { window_bounds: Some(bounds), kind: WindowKind::Floating,
        is_resizable: true, window_min_size: Some(size(px(420.), px(300.))),
        titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }), ..Default::default() }, move |window, cx| {
        window.set_window_title(&window_title);
        window.set_rem_size(px(crate::consts::ZOOM_REM_BASE * zoom));
        root.update(cx, |dialog, cx| { dialog.opened = true; dialog.mode = mode; dialog.focus_initial(window, cx); });
        let weak = root.downgrade(); window.on_window_should_close(cx, move |_, cx| { let _ = weak.update(cx, |dialog, cx| { dialog.cancel(cx); dialog.opened = false; }); true }); root
    });
    match result {
        Ok(handle) => { if let Some(retained) = cx.default_global::<Dialogs>().0.get_mut(index) { *retained = Some(Retained { entity, window: Some(handle) }); } }
        Err(error) => { if let Some(source) = source { let _ = source.update(cx, |w, _, cx| { w.set_status(format!("Could not open hash dialog: {error}")); cx.notify(); }); } }
    }
}
