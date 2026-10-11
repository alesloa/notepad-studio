// PowerEditor/src/ScintillaComponent/FunctionCallTip.cpp:90-124,385-449 —
// parameter hints, active argument and cyclic overload navigation.
use gpui::{AppContext, Context, Subscription, Task, Window, WindowId};
use uuid::Uuid;

use crate::actions::{EditAutocompleteFuncParamsNext, EditAutocompleteFuncParamsPrevious};
use crate::buffer::ContentRevision;
use crate::call_context::{self, CallContext, CallLanguage, ScanLimit};
use crate::syntax::LanguageId;
use crate::tab::InputFocus;

use super::completion_api::ApiSnapshot;
use super::Workspace;

#[derive(Clone)]
struct HintPosition {
    window: WindowId,
    tab: Uuid,
    language: Option<LanguageId>,
    caret: usize,
    anchor: usize,
    raw: ContentRevision,
    rich: Option<ContentRevision>,
    rope: ropey::Rope,
}

#[derive(Clone)]
struct HintRequest {
    id: Uuid,
    epoch: Uuid,
    position: HintPosition,
    api: ApiSnapshot,
    manual: bool,
    status: Option<String>,
}

struct HintIntent {
    tab: Uuid,
    language: Option<LanguageId>,
    manual: bool,
    status: Option<String>,
}

#[derive(Clone)]
pub(crate) struct HintSnapshot {
    pub(crate) session: Uuid,
    pub(crate) api: ApiSnapshot,
    pub(crate) call: CallContext,
    pub(crate) overload: usize,
}

struct HintState {
    snapshot: HintSnapshot,
    position: HintPosition,
}

pub(super) struct FunctionHintRuntime {
    epoch: Uuid,
    intent: Option<HintIntent>,
    state: Option<HintState>,
    previous: Option<HintSnapshot>,
    request: Option<HintRequest>,
    task: Option<Task<()>>,
    dirty: bool,
    loading_status: Option<String>,
    blur: Option<Subscription>,
    activation: Option<Subscription>,
}

impl Default for FunctionHintRuntime {
    fn default() -> Self {
        Self {
            epoch: Uuid::new_v4(),
            intent: None,
            state: None,
            previous: None,
            request: None,
            task: None,
            dirty: false,
            loading_status: None,
            blur: None,
            activation: None,
        }
    }
}

fn lexical_mode(language: Option<LanguageId>) -> CallLanguage {
    match language {
        None
        | Some(LanguageId::Markdown | LanguageId::Yaml | LanguageId::Html | LanguageId::Xml) => {
            CallLanguage::Plain
        }
        Some(LanguageId::Rust) => CallLanguage::Rust,
        Some(LanguageId::JavaScript | LanguageId::TypeScript | LanguageId::Tsx) => {
            CallLanguage::JavaScript
        }
        Some(LanguageId::Python) => CallLanguage::Python,
        Some(LanguageId::Bash) => CallLanguage::Shell,
        Some(LanguageId::PowerShell) => CallLanguage::PowerShell,
        Some(LanguageId::Ruby) => CallLanguage::Ruby,
        Some(LanguageId::Lua) => CallLanguage::Lua,
        Some(LanguageId::Sql) => CallLanguage::Sql,
        Some(_) => CallLanguage::General,
    }
}

impl Workspace {
    pub(super) fn function_hint_epoch(&self) -> Uuid {
        self.function_hint.epoch
    }

    pub(crate) fn function_hint_active(&self) -> bool {
        self.function_hint.intent.is_some()
    }

    pub(crate) fn function_hint_visible(&self) -> bool {
        self.function_hint.state.is_some()
    }

    pub(crate) fn close_function_hint(&mut self, cx: &mut Context<Self>) {
        let had_hint = self.function_hint_active();
        if self
            .function_hint
            .loading_status
            .as_ref()
            .is_some_and(|status| self.status_message.as_ref() == Some(status))
        {
            self.status_message = None;
        }
        self.function_hint = FunctionHintRuntime::default();
        if had_hint {
            cx.notify();
        }
    }

    fn hint_position(&self, window: &Window) -> Option<HintPosition> {
        if !window.is_window_active()
            || !self.focus_handle.is_focused(window)
            || self.input_focus(window) != InputFocus::Editor
            || !self.autocomplete_editable()
            || self.is_composing_text(window)
        {
            return None;
        }
        let tab = self.tabs.get(self.active)?;
        let caret = super::edits::floor_rope_char_boundary(&tab.buffer.rope, tab.selection.head);
        Some(HintPosition {
            window: window.window_handle().window_id(),
            tab: tab.session_id,
            language: tab.buffer.language,
            caret,
            anchor: tab.selection.anchor,
            raw: tab.buffer.raw_revision.clone(),
            rich: tab
                .buffer
                .rich_doc
                .as_ref()
                .map(|payload| payload.revision.clone()),
            rope: tab.buffer.rope.clone(),
        })
    }

    fn hint_position_valid(&self, position: &HintPosition, window: &Window) -> bool {
        self.hint_position(window).is_some_and(|current| {
            current.window == position.window
                && current.tab == position.tab
                && current.language == position.language
                && current.caret == position.caret
                && current.anchor == position.anchor
                && current.raw == position.raw
                && current.rich == position.rich
                && current.rope.is_instance(&position.rope)
        })
    }

    pub(crate) fn function_hint_snapshot(&self, window: &Window) -> Option<HintSnapshot> {
        let state = self.function_hint.state.as_ref()?;
        (self.hint_position_valid(&state.position, window)
            && self
                .loaded_api_snapshot(state.position.language)
                .is_some_and(|api| api.generation == state.snapshot.api.generation))
        .then(|| state.snapshot.clone())
    }

    pub(super) fn own_function_hint_loading_status(&mut self) {
        self.function_hint.loading_status = self.status_message.clone();
        if let Some(intent) = self.function_hint.intent.as_mut() {
            intent.status = self.status_message.clone();
        }
    }

    fn arm_function_hint(
        &mut self,
        manual: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(position) = self.hint_position(window) else {
            self.close_function_hint(cx);
            return false;
        };
        self.function_hint.intent = Some(HintIntent {
            tab: position.tab,
            language: position.language,
            manual,
            status: self.status_message.clone(),
        });
        self.function_hint.dirty = true;
        if self.function_hint.blur.is_none() {
            let focus = self.focus_handle.clone();
            self.function_hint.blur =
                Some(cx.on_blur(&focus, window, |w, _, cx| w.close_function_hint(cx)));
            self.function_hint.activation =
                Some(cx.observe_window_activation(window, |w, window, cx| {
                    if !window.is_window_active() {
                        w.close_function_hint(cx);
                    }
                }));
        }
        true
    }

    pub(super) fn show_function_hint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.arm_function_hint(true, window, cx) {
            return;
        }
        // Hint-only loading must not invoke or replace a completion list.
        self.request_hint_api(true, window, cx);
        self.refresh_function_hint_for_frame(window, cx);
    }

    /// Called only for committed Source input, including an IME final commit.
    pub(crate) fn update_function_hint_after_input(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.hint_position(window).is_none() {
            self.close_function_hint(cx);
            return;
        }
        let api = self.loaded_api_snapshot(self.buffer().language);
        let (start, separator) = api
            .as_ref()
            .map(|api| {
                let env = &api.registry.environment;
                (env.start, env.separator)
            })
            .unwrap_or(('(', ','));
        let trigger = text
            .chars()
            .next_back()
            .is_some_and(|ch| ch == start || ch == separator);
        if !self.function_hint_active()
            && crate::settings::snapshot().autocompletion_func_params_hint
            && trigger
        {
            self.arm_function_hint(false, window, cx);
        }
        self.refresh_function_hint_for_frame(window, cx);
    }

    pub(crate) fn invalidate_function_hint_after_edit(&mut self, cx: &mut Context<Self>) {
        if !self.function_hint_active() {
            return;
        }
        self.function_hint.epoch = Uuid::new_v4();
        self.function_hint.task = None;
        self.function_hint.request = None;
        if let Some(state) = self.function_hint.state.take() {
            self.function_hint.previous = Some(state.snapshot);
        }
        self.function_hint.dirty = true;
        cx.notify();
    }

    /// O(1) lifecycle/cache checks. Rope flattening and lexical scanning are background work.
    pub(crate) fn refresh_function_hint_for_frame(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(intent) = self.function_hint.intent.as_ref() else {
            return;
        };
        let Some(position) = self.hint_position(window) else {
            self.close_function_hint(cx);
            return;
        };
        if position.tab != intent.tab || position.language != intent.language {
            self.close_function_hint(cx);
            return;
        }
        let manual = intent.manual;
        let status = intent.status.clone();
        let api = self.loaded_api_snapshot(position.language);
        let generation = api.as_ref().map(|api| api.generation);
        if self
            .function_hint
            .request
            .as_ref()
            .is_some_and(|request| !self.hint_position_valid(&request.position, window))
        {
            self.close_function_hint(cx);
            return;
        }
        if self.function_hint.state.as_ref().is_some_and(|state| {
            !self.hint_position_valid(&state.position, window)
                || Some(state.snapshot.api.generation) != generation
        }) || self
            .function_hint
            .request
            .as_ref()
            .is_some_and(|request| Some(request.api.generation) != generation)
        {
            self.invalidate_function_hint_after_edit(cx);
        }
        if !self.function_hint.dirty {
            return;
        }
        let Some(api) = api else {
            if self.api_result_cached(position.language)
                && !self.api_load_pending(position.language)
            {
                self.close_function_hint(cx);
            } else if !self.request_hint_api(false, window, cx) {
                self.close_function_hint(cx);
                if manual {
                    self.set_status(
                        "Function hint: the application config directory is unavailable.",
                    );
                    cx.notify();
                }
            }
            return;
        };
        if position.caret > call_context::MAX_CALL_SCAN_BYTES {
            self.close_function_hint(cx);
            if manual {
                self.set_status(
                    "Function hint: scanning is limited to 8 MiB of text before the caret.",
                );
                cx.notify();
            }
            return;
        }
        self.function_hint.dirty = false;
        let request = HintRequest {
            id: Uuid::new_v4(),
            epoch: self.function_hint.epoch,
            position,
            api,
            manual,
            status,
        };
        self.function_hint.request = Some(request.clone());
        self.function_hint.task = None;
        let snapshot = request.clone();
        let scan = cx.background_spawn(async move {
            let rope = &snapshot.position.rope;
            let prefix = rope
                .get_byte_slice(..snapshot.position.caret)
                .map(|prefix| prefix.to_string());
            prefix.map_or(Ok(None), |prefix| {
                call_context::find_call(
                    &prefix,
                    prefix.len(),
                    &snapshot.api.registry.environment,
                    lexical_mode(snapshot.position.language),
                )
            })
        });
        self.function_hint.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = scan.await;
            let _ = this.update_in(cx, |w, window, cx| {
                w.finish_function_hint(request, result, window, cx)
            });
        }));
    }

    pub(super) fn function_hint_registry_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_function_hint_for_frame(window, cx);
    }

    fn finish_function_hint(
        &mut self,
        request: HintRequest,
        result: Result<Option<CallContext>, ScanLimit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .function_hint
            .request
            .as_ref()
            .is_some_and(|current| current.id == request.id)
            || self.function_hint.epoch != request.epoch
            || !self.hint_position_valid(&request.position, window)
            || self
                .loaded_api_snapshot(request.position.language)
                .is_none_or(|api| api.generation != request.api.generation)
        {
            return;
        }
        self.function_hint.request = None;
        let owns_status = self.status_message == request.status;
        let call = match result {
            Ok(Some(call)) => call,
            result => {
                self.close_function_hint(cx);
                if request.manual && owns_status {
                    self.set_status(match result {
                        Err(ScanLimit::PrefixTooLong) => {
                            "Function hint: scanning is limited to 8 MiB before the caret."
                        }
                        Err(ScanLimit::NestingTooDeep) => {
                            "Function hint: call nesting exceeds the safe scan limit."
                        }
                        Err(ScanLimit::LiteralDelimiterTooLong) => {
                            "Function hint: a literal delimiter exceeds the safe scan limit."
                        }
                        _ => "Function hint: there is no open function call at the caret.",
                    });
                    cx.notify();
                }
                return;
            }
        };
        let Some(entry) = request
            .api
            .registry
            .find(&call.name)
            .filter(|entry| entry.is_function && !entry.overloads.is_empty())
        else {
            self.close_function_hint(cx);
            if request.manual && owns_status {
                self.set_status("Function hint: this call has no function signature in the language API definitions.");
                cx.notify();
            }
            return;
        };
        let previous = self
            .function_hint
            .state
            .as_ref()
            .map(|state| &state.snapshot)
            .or(self.function_hint.previous.as_ref());
        let mut overload = previous
            .filter(|previous| {
                previous.call.name == call.name
                    && previous.call.open_byte == call.open_byte
                    && previous.api.generation == request.api.generation
            })
            .map_or(0, |previous| {
                previous
                    .overload
                    .min(entry.overloads.len().saturating_sub(1))
            });
        // Preserve explicit choice until the argument outgrows this overload.
        if entry
            .overloads
            .get(overload)
            .is_some_and(|overload| call.argument > overload.parameters.len())
        {
            if let Some(index) = entry
                .overloads
                .iter()
                .position(|overload| call.argument <= overload.parameters.len())
            {
                overload = index;
            }
        }
        self.function_hint.previous = None;
        self.function_hint.state = Some(HintState {
            position: request.position,
            snapshot: HintSnapshot {
                session: Uuid::new_v4(),
                api: request.api,
                call,
                overload,
            },
        });
        if owns_status && request.manual {
            self.status_message = None;
        }
        // Diagnostics belong to the explicit request. Later typing only
        // updates or dismisses the hint.
        if let Some(intent) = self.function_hint.intent.as_mut() {
            intent.manual = false;
        }
        cx.notify();
    }

    pub(crate) fn navigate_function_hint_session(
        &mut self,
        session: Uuid,
        delta: i32,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self
            .function_hint_snapshot(window)
            .filter(|snapshot| snapshot.session == session)
        else {
            return;
        };
        let Some(entry) = snapshot.api.registry.find(&snapshot.call.name) else {
            return;
        };
        let count = entry.overloads.len();
        if count == 0 {
            return;
        }
        if let Some(state) = self.function_hint.state.as_mut() {
            let offset = delta.rem_euclid(count as i32) as usize;
            state.snapshot.overload = (state.snapshot.overload % count + offset) % count;
            cx.notify();
        }
    }

    pub(crate) fn dismiss_function_hint_session(&mut self, session: Uuid, cx: &mut Context<Self>) {
        if self
            .function_hint
            .state
            .as_ref()
            .is_some_and(|state| state.snapshot.session == session)
        {
            self.close_function_hint(cx);
        }
    }

    pub(crate) fn function_hint_previous(
        &mut self,
        _: &EditAutocompleteFuncParamsPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = self.function_hint_snapshot(window) {
            self.navigate_function_hint_session(state.session, -1, window, cx);
        }
    }

    pub(crate) fn function_hint_next(
        &mut self,
        _: &EditAutocompleteFuncParamsNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = self.function_hint_snapshot(window) {
            self.navigate_function_hint_session(state.session, 1, window, cx);
        }
    }
}

#[cfg(test)]
#[path = "call_hint_tests.rs"]
mod tests;
