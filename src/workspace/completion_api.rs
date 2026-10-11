// PowerEditor/src/ScintillaComponent/AutoCompletion.cpp:1101-1259 —
// optional per-language API definitions. Original asynchronous UI integration.
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AppContext, Context, Subscription, Task, Window, WindowId};
use uuid::Uuid;

use crate::api_registry::{ApiError, ApiRegistry};
use crate::buffer::ContentRevision;
use crate::syntax::LanguageId;
use crate::tab::InputFocus;

use super::autocomplete::CandidateKind;
use super::Workspace;

const MAX_CACHED_LANGUAGES: usize = 4;

#[derive(Clone)]
pub(crate) struct ApiSnapshot {
    pub(crate) generation: Uuid,
    pub(crate) registry: Arc<ApiRegistry>,
}

struct ApiSlot {
    language: Option<LanguageId>,
    snapshot: Option<ApiSnapshot>,
}

#[derive(Clone)]
enum ApiPurpose { Completion, FunctionHint }

#[derive(Clone)]
struct ApiRequest {
    id: Uuid,
    window: WindowId,
    input_context: gpui::InputContextToken,
    epoch: Uuid,
    language: Option<LanguageId>,
    kind: CandidateKind,
    manual: bool,
    purpose: ApiPurpose,
    tab: Uuid,
    caret: usize,
    anchor: usize,
    revision: ContentRevision,
    rich_revision: Option<ContentRevision>,
    rope: ropey::Rope,
    popup: Option<Uuid>,
    status: Option<String>,
    loading_status: bool,
}

pub(super) struct CompletionApiCache {
    slots: VecDeque<ApiSlot>,
    pending: Option<ApiRequest>,
    task: Option<Task<()>>,
    pending_blur: Option<Subscription>,
    pending_activation: Option<Subscription>,
    pub(super) epoch: Uuid,
}

impl Default for CompletionApiCache {
    fn default() -> Self {
        Self {
            slots: VecDeque::new(),
            pending: None,
            task: None,
            pending_blur: None,
            pending_activation: None,
            epoch: Uuid::new_v4(),
        }
    }
}

impl CompletionApiCache {
    fn snapshot(&self, language: Option<LanguageId>) -> Option<ApiSnapshot> {
        self.slots
            .iter()
            .find(|slot| slot.language == language)
            .and_then(|slot| slot.snapshot.clone())
    }

    fn contains(&self, language: Option<LanguageId>) -> bool {
        self.slots.iter().any(|slot| slot.language == language)
    }

    fn touch(&mut self, language: Option<LanguageId>) {
        if let Some(index) = self.slots.iter().position(|slot| slot.language == language) {
            if let Some(slot) = self.slots.remove(index) {
                self.slots.push_back(slot);
            }
        }
    }

    fn store(&mut self, language: Option<LanguageId>, snapshot: Option<ApiSnapshot>) {
        self.slots.retain(|slot| slot.language != language);
        while self.slots.len() >= MAX_CACHED_LANGUAGES {
            self.slots.pop_front();
        }
        self.slots.push_back(ApiSlot { language, snapshot });
    }
}

/// Known filenames only. TSX is an original supported language with its own file.
fn api_filename(language: Option<LanguageId>) -> &'static str {
    match language {
        None => "normal.xml",
        Some(LanguageId::Rust) => "rust.xml",
        Some(LanguageId::Python) => "python.xml",
        Some(LanguageId::C) => "c.xml",
        Some(LanguageId::CSharp) => "cs.xml",
        Some(LanguageId::JavaScript) => "javascript.xml",
        Some(LanguageId::TypeScript) => "typescript.xml",
        Some(LanguageId::Tsx) => "tsx.xml",
        Some(LanguageId::Json) => "json.xml",
        Some(LanguageId::Markdown) => "markdown.xml",
        Some(LanguageId::Bash) => "bash.xml",
        Some(LanguageId::PowerShell) => "powershell.xml",
        Some(LanguageId::Go) => "go.xml",
        Some(LanguageId::Java) => "java.xml",
        Some(LanguageId::Swift) => "swift.xml",
        Some(LanguageId::Ruby) => "ruby.xml",
        Some(LanguageId::Php) => "php.xml",
        Some(LanguageId::Cpp) => "cpp.xml",
        Some(LanguageId::Lua) => "lua.xml",
        Some(LanguageId::Html) => "html.xml",
        Some(LanguageId::Css) => "css.xml",
        Some(LanguageId::Scss) => "scss.xml",
        Some(LanguageId::Yaml) => "yaml.xml",
        Some(LanguageId::Toml) => "toml.xml",
        Some(LanguageId::Xml) => "xml.xml",
        Some(LanguageId::Sql) => "sql.xml",
    }
}

#[cfg(not(test))]
fn api_directory() -> Option<PathBuf> {
    dirs::config_dir().map(|directory| {
        directory
            .join(crate::consts::APP_NAME)
            .join("autoCompletion")
    })
}

#[cfg(test)]
thread_local! {
    static TEST_API_DIRECTORY: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn api_directory() -> Option<PathBuf> {
    TEST_API_DIRECTORY.with(|directory| directory.borrow().clone())
}

#[cfg(test)]
pub(super) struct TestApiDirectoryGuard(Option<PathBuf>);

#[cfg(test)]
impl Drop for TestApiDirectoryGuard {
    fn drop(&mut self) {
        TEST_API_DIRECTORY.with(|directory| *directory.borrow_mut() = self.0.take());
    }
}

#[cfg(test)]
pub(super) fn override_directory_for_test(directory: PathBuf) -> TestApiDirectoryGuard {
    TestApiDirectoryGuard(TEST_API_DIRECTORY.with(|current| current.replace(Some(directory))))
}

impl Workspace {
    pub(super) fn loaded_api_snapshot(&self, language: Option<LanguageId>) -> Option<ApiSnapshot> {
        self.completion_api.snapshot(language)
    }

    pub(super) fn request_completion_api(
        &mut self,
        kind: CandidateKind,
        manual: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_language_api(kind, manual, ApiPurpose::Completion, window, cx);
    }

    pub(super) fn api_result_cached(&self, language: Option<LanguageId>) -> bool {
        self.completion_api.contains(language)
    }

    pub(super) fn api_load_pending(&self, language: Option<LanguageId>) -> bool {
        self.completion_api.pending.as_ref().is_some_and(|request| request.language == language)
    }

    pub(super) fn request_hint_api(&mut self, manual: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.request_language_api(CandidateKind::Function(self.buffer().language), manual, ApiPurpose::FunctionHint, window, cx)
    }

    fn request_language_api(&mut self, kind: CandidateKind, manual: bool, purpose: ApiPurpose,
        window: &mut Window, cx: &mut Context<Self>) -> bool {
        let language = self.buffer().language;
        self.completion_api.touch(language);
        if !manual
            && (self.completion_api.contains(language)
                || self
                    .completion_api
                    .pending
                    .as_ref()
                    .is_some_and(|request| request.language == language))
        {
            return true;
        }
        // Unit fixtures have no directory unless a scoped synthetic one is supplied.
        let Some(directory) = api_directory() else {
            return false;
        };
        let path = directory.join(api_filename(language));
        self.completion_api.task = None;
        let mut request = ApiRequest {
            id: Uuid::new_v4(),
            window: window.window_handle().window_id(),
            input_context: window.input_context_token(),
            epoch: if matches!(purpose, ApiPurpose::FunctionHint) { self.function_hint_epoch() } else { self.completion_api.epoch },
            language,
            kind,
            manual,
            purpose,
            tab: self.active_tab_mut().session_id,
            caret: self.selection().head,
            anchor: self.selection().anchor,
            revision: self.buffer().raw_revision.clone(),
            rich_revision: self.buffer().rich_doc.as_ref().map(|payload| payload.revision.clone()),
            rope: self.buffer().rope.clone(),
            popup: self.autocomplete.as_ref().map(|state| state.session_id),
            status: self.status_message.clone(),
            loading_status: false,
        };
        let ticket = request.id;
        let focus = self.focus_handle.clone();
        self.completion_api.pending_blur =
            Some(cx.on_blur(&focus, window, move |workspace, _, _| {
                if workspace
                    .completion_api
                    .pending
                    .as_ref()
                    .is_some_and(|request| request.id == ticket)
                {
                    workspace.completion_api.epoch = Uuid::new_v4();
                }
            }));
        self.completion_api.pending_activation = Some(cx.observe_window_activation(
            window,
            move |workspace, window, _| {
                if !window.is_window_active()
                    && workspace
                        .completion_api
                        .pending
                        .as_ref()
                        .is_some_and(|request| request.id == ticket)
                {
                    workspace.completion_api.epoch = Uuid::new_v4();
                }
            },
        ));
        if manual && match request.purpose {
            ApiPurpose::Completion => self.autocomplete.is_none(),
            ApiPurpose::FunctionHint => !self.function_hint_visible(),
        } {
            self.set_status(format!("Loading API definitions from {}…", path.display()));
            request.loading_status = true;
            if matches!(request.purpose, ApiPurpose::FunctionHint) { self.own_function_hint_loading_status(); }
            cx.notify();
        }
        request.status = self.status_message.clone();
        self.completion_api.pending = Some(request.clone());
        let read_path = path.clone();
        let task = cx.background_spawn(async move { ApiRegistry::load(&read_path) });
        self.completion_api.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |workspace, window, cx| {
                workspace.finish_completion_api(request, path, result, window, cx);
            });
        }));
        true
    }

    fn api_request_context_valid(&self, request: &ApiRequest, window: &Window) -> bool {
        (match request.purpose {
            ApiPurpose::Completion => self.completion_api.epoch == request.epoch
                && self.autocomplete.as_ref().map(|state| state.session_id) == request.popup,
            ApiPurpose::FunctionHint => self.function_hint_epoch() == request.epoch && !self.is_composing_text(window),
        })
            && window.is_window_active()
            && window.window_handle().window_id() == request.window
            && window.input_context_token() == request.input_context
            && self.focus_handle.is_focused(window)
            && self.input_focus(window) == InputFocus::Editor
            && self.autocomplete_editable()
            && self.tabs.get(self.active).is_some_and(|tab| {
                tab.session_id == request.tab
                    && tab.buffer.language == request.language
                    && tab.buffer.raw_revision == request.revision
                    && tab.buffer.rich_doc.as_ref().map(|payload| &payload.revision) == request.rich_revision.as_ref()
                    && tab.buffer.rope.is_instance(&request.rope)
                    && tab.selection.head == request.caret
                    && tab.selection.anchor == request.anchor
            })
    }

    fn finish_completion_api(
        &mut self,
        request: ApiRequest,
        path: PathBuf,
        result: Result<Option<ApiRegistry>, ApiError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .completion_api
            .pending
            .as_ref()
            .is_some_and(|current| current.id == request.id)
        {
            return;
        }
        self.completion_api.pending = None;
        self.completion_api.pending_blur = None;
        self.completion_api.pending_activation = None;
        let error = match result {
            Ok(registry) => {
                let snapshot = registry.map(|registry| ApiSnapshot {
                    generation: Uuid::new_v4(),
                    registry: Arc::new(registry),
                });
                self.completion_api.store(request.language, snapshot);
                None
            }
            Err(error) => {
                let last_good = self.completion_api.snapshot(request.language);
                self.completion_api.store(request.language, last_good);
                Some(error.to_string())
            }
        };
        let context_valid = self.api_request_context_valid(&request, window);
        if matches!(request.purpose, ApiPurpose::FunctionHint) {
            if context_valid && request.manual && self.status_message == request.status {
                if let Some(error) = error {
                    self.set_status(format!("Cannot load function hints from {}: {error}. The last valid definitions remain available; the file was left unchanged.", path.display()));
                    cx.notify();
                } else if self.loaded_api_snapshot(request.language).is_none() {
                    self.set_status(format!("Function hint: no API definitions at {}. Add a function with an Overload to this language XML file.", path.display()));
                    cx.notify();
                }
            }
            self.function_hint_registry_changed(window, cx);
            return;
        }
        self.function_hint_registry_changed(window, cx);
        if !context_valid {
            if request.loading_status && self.status_message == request.status {
                self.status_message = None;
                cx.notify();
            }
            return;
        }
        let previous_status = self.status_message.clone();
        let owns_status = previous_status == request.status;
        if !request.manual {
            let mode = crate::settings::snapshot().autocompletion_mode;
            if !matches!(
                (mode, request.kind),
                (
                    crate::settings::AutocompletionMode::Word,
                    CandidateKind::Word
                ) | (
                    crate::settings::AutocompletionMode::Function,
                    CandidateKind::Function(_)
                ) | (
                    crate::settings::AutocompletionMode::FunctionAndWord,
                    CandidateKind::Combined(_)
                )
            ) {
                return;
            }
        }
        if let Some(error) = error {
            if owns_status {
                self.set_status(format!("Cannot load API definitions from {}: {error}. Using the last valid definitions or original keywords; the file was left unchanged.", path.display()));
                cx.notify();
            }
            return;
        }
        let generation = self
            .loaded_api_snapshot(request.language)
            .map(|snapshot| snapshot.generation);
        if self
            .autocomplete
            .as_ref()
            .is_some_and(|state| state.api_generation() == generation)
        {
            return;
        }
        let selected = self
            .autocomplete
            .as_ref()
            .and_then(|state| state.candidates.get(state.selected))
            .cloned();
        self.open_completion_kind(request.kind, request.manual, cx);
        if !owns_status {
            self.status_message = previous_status;
        }
        if let Some(state) = self.autocomplete.as_mut() {
            if let Some(index) = selected
                .filter(|name| state.matches_live_prefix(&request.rope, request.caret, name))
                .and_then(|name| {
                    state
                        .candidates
                        .iter()
                        .position(|candidate| *candidate == name)
                })
            {
                state.selected = index;
            }
        }
        self.watch_autocomplete_focus(window, cx);
    }
}

#[cfg(test)]
#[path = "completion_api_loader_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "call_hint_api_tests.rs"]
mod hint_tests;
