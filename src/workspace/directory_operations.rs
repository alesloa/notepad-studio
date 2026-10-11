use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use gpui::{AnyWindowHandle, App, AppContext, Context, EntityId, Window};

use crate::find_in_files::{FindInFilesOutcome, FindInFilesQuery};
use super::Workspace;

const STRING_LIMIT: usize = 1024 * 1024;
const SNAPSHOT_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Route { Files, Projects }

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Search, ConfirmReplace, RunningReplace, Cancelling }

#[derive(Clone, Copy, PartialEq, Eq)]
enum OriginRoot { Editor(EntityId), Dialog(EntityId) }

fn origin_root(window: &Window) -> Option<OriginRoot> {
    if let Some(root) = window.root::<Workspace>().flatten() {
        Some(OriginRoot::Editor(root.entity_id()))
    } else {
        window.root::<crate::ui::find_dialog::FindDialog>().flatten()
            .map(|root| OriginRoot::Dialog(root.entity_id()))
    }
}

fn root_matches(expected: OriginRoot, window: &Window, cx: &App, workspace: EntityId) -> bool {
    match expected {
        OriginRoot::Editor(id) => id == workspace && window.root::<Workspace>().flatten()
            .is_some_and(|root| root.entity_id() == id),
        OriginRoot::Dialog(id) => window.root::<crate::ui::find_dialog::FindDialog>().flatten()
            .is_some_and(|root| root.entity_id() == id && root.read(cx).workspace.entity_id() == workspace),
    }
}

struct Owner {
    id: uuid::Uuid,
    origin: AnyWindowHandle,
    root: OriginRoot,
    dialog: bool,
    phase: Phase,
    cancelled: Arc<AtomicBool>,
    wake: Option<futures::channel::oneshot::Sender<()>>,
    task: Option<gpui::Task<()>>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(wake) = self.wake.take() { let _ = wake.send(()); }
    }
}

#[derive(Default)]
pub(crate) struct DirectoryOperations { owner: Option<Owner> }

impl DirectoryOperations {
    pub(crate) fn busy(&self) -> bool { self.owner.is_some() }

    fn owns(&self, id: uuid::Uuid) -> bool {
        self.owner.as_ref().is_some_and(|o| o.id == id)
    }

    fn finish(&mut self, id: uuid::Uuid) -> Option<bool> {
        if !self.owns(id) { return None; }
        self.owner.take().map(|o| o.cancelled.load(Ordering::Acquire))
    }

    fn cancel(&mut self, dialog: Option<AnyWindowHandle>) {
        let Some(owner) = self.owner.as_mut() else { return; };
        if dialog.is_some_and(|handle| !owner.dialog || handle != owner.origin) { return; }
        owner.cancelled.store(true, Ordering::Release);
        if let Some(wake) = owner.wake.take() { let _ = wake.send(()); }
        if owner.phase == Phase::ConfirmReplace {
            self.owner = None;
        } else {
            owner.phase = Phase::Cancelling;
        }
    }
}

struct Snapshot {
    route: Route,
    query: FindInFilesQuery,
    replacement: String,
    roots: Vec<PathBuf>,
    retained: usize,
}

fn charge(total: &mut usize, capacity: usize) -> Result<(), String> {
    *total = total.checked_add(capacity).filter(|n| *n <= SNAPSHOT_LIMIT)
        .ok_or_else(|| "Directory operation exceeds the snapshot size limit".to_string())?;
    Ok(())
}

fn copy_string(text: &str, total: &mut usize) -> Result<String, String> {
    copy_string_reserved(text, total, |copy, len| copy.try_reserve_exact(len))
}

fn copy_string_reserved(text: &str, total: &mut usize,
    reserve: impl FnOnce(&mut String, usize) -> Result<(), std::collections::TryReserveError>) -> Result<String, String>
{
    if text.len() > STRING_LIMIT { return Err("Directory query, filter or replacement exceeds 1 MiB".into()); }
    if !total.checked_add(text.len()).is_some_and(|n| n <= SNAPSHOT_LIMIT) {
        return Err("Directory operation exceeds the snapshot size limit".into());
    }
    let mut copy = String::new();
    reserve(&mut copy, text.len()).map_err(|_| "Cannot allocate directory operation snapshot".to_string())?;
    charge(total, copy.capacity())?;
    copy.push_str(text);
    Ok(copy)
}

fn copy_path(path: &Path, total: &mut usize) -> Result<PathBuf, String> {
    let len = path.as_os_str().as_encoded_bytes().len();
    if len > STRING_LIMIT { return Err("Directory path exceeds 1 MiB".into()); }
    if !total.checked_add(len).is_some_and(|n| n <= SNAPSHOT_LIMIT) {
        return Err("Directory operation exceeds the snapshot size limit".into());
    }
    let mut copy = PathBuf::new();
    copy.try_reserve_exact(len).map_err(|_| "Cannot allocate directory path snapshot".to_string())?;
    let initial_capacity = copy.capacity(); charge(total, initial_capacity)?;
    copy.push(path);
    charge(total, copy.capacity().saturating_sub(initial_capacity))?;
    Ok(copy)
}

impl Snapshot {
    fn capture(w: &Workspace, route: Route, replace: bool) -> Result<Self, String> {
        let s = &w.find_in_files;
        let mut retained = std::mem::size_of::<Self>() + 4 * std::mem::size_of::<usize>()
            + std::mem::size_of::<AtomicBool>();
        let text = copy_string(&s.query, &mut retained)?;
        let filter = copy_string(&s.filter, &mut retained)?;
        let replacement = copy_string(if replace { &s.replacement } else { "" }, &mut retained)?;
        let mut roots = Vec::new();
        roots.try_reserve_exact(3).map_err(|_| "Cannot allocate directory scope snapshot".to_string())?;
        charge(&mut retained, roots.capacity().checked_mul(std::mem::size_of::<PathBuf>())
            .ok_or_else(|| "Directory scope size overflow".to_string())?)?;
        match route {
            Route::Files => {
                let path = s.directory.as_deref().ok_or_else(|| "Pick a directory first".to_string())?;
                roots.push(copy_path(path, &mut retained)?);
            }
            Route::Projects => {
                let settings = crate::settings::snapshot();
                for (index, selected) in [s.project_panel_1, s.project_panel_2, s.project_panel_3].into_iter().enumerate() {
                    if selected {
                        if let Some(path) = settings.folder_panel_roots.get(index) {
                            roots.push(copy_path(path, &mut retained)?);
                        }
                    }
                }
            }
        }
        let first = roots.first().ok_or_else(|| "Tick a Project Panel with an open folder".to_string())?;
        let directory = copy_path(first, &mut retained)?;
        let query = FindInFilesQuery {
            text, directory, filter, match_case: s.match_case, whole_word: s.whole_word,
            regex: s.regex, extended: s.extended, dot_matches_newline: s.dot_matches_newline,
            in_subfolders: route == Route::Projects || s.in_subfolders,
            in_hidden_folders: route == Route::Files && s.in_hidden_folders,
        };
        Ok(Self { route, query, replacement, roots, retained })
    }

    fn matches(&self, w: &Workspace) -> bool {
        let s = &w.find_in_files; let q = &self.query;
        if q.text != s.query || q.filter != s.filter || self.replacement != s.replacement
            || q.match_case != s.match_case || q.whole_word != s.whole_word
            || q.regex != s.regex || q.extended != s.extended
            || q.dot_matches_newline != s.dot_matches_newline { return false; }
        match self.route {
            Route::Files => s.directory.as_ref() == self.roots.first()
                && self.roots.first().is_some_and(|p| Path::new(s.directory_text.trim()) == p)
                && q.in_subfolders == s.in_subfolders && q.in_hidden_folders == s.in_hidden_folders,
            Route::Projects => {
                let settings = crate::settings::snapshot();
                let mut roots = settings.folder_panel_roots.iter().take(3).enumerate()
                    .filter_map(|(i, p)| [s.project_panel_1, s.project_panel_2, s.project_panel_3]
                        .get(i).copied().filter(|b| *b).map(|_| p));
                self.roots.iter().all(|p| roots.next() == Some(p)) && roots.next().is_none()
            }
        }
    }

    fn query_for(&self, path: &Path) -> Result<FindInFilesQuery, String> {
        let mut allocated = self.retained;
        let text = copy_string(&self.query.text, &mut allocated)?;
        let filter = copy_string(&self.query.filter, &mut allocated)?;
        let directory = copy_path(path, &mut allocated)?;
        Ok(FindInFilesQuery { text, filter, directory,
            match_case: self.query.match_case, whole_word: self.query.whole_word,
            regex: self.query.regex, extended: self.query.extended,
            dot_matches_newline: self.query.dot_matches_newline,
            in_subfolders: self.query.in_subfolders, in_hidden_folders: self.query.in_hidden_folders })
    }

    fn prompt(&self) -> Result<String, String> {
        let path_bytes = self.roots.iter().try_fold(0usize, |n, p| {
            n.checked_add(p.as_os_str().as_encoded_bytes().len().checked_mul(3)?.checked_add(4)?)
        }).ok_or_else(|| "Directory prompt size overflow".to_string())?;
        let requested = self.query.text.len().checked_add(self.replacement.len())
            .and_then(|n| n.checked_add(path_bytes)).and_then(|n| n.checked_add(256))
            .filter(|n| self.retained.checked_add(*n).is_some_and(|sum| sum <= SNAPSHOT_LIMIT))
            .ok_or_else(|| "Directory confirmation exceeds the snapshot size limit".to_string())?;
        let mut prompt = String::new();
        prompt.try_reserve_exact(requested).map_err(|_| "Cannot allocate directory confirmation".to_string())?;
        let initial_capacity = prompt.capacity();
        let mut allocated = self.retained; charge(&mut allocated, initial_capacity)?;
        write!(&mut prompt, "Replace every match of\n  {}\nwith\n  {}\nin:\n", self.query.text, self.replacement)
            .map_err(|_| "Cannot format directory confirmation".to_string())?;
        for path in &self.roots {
            writeln!(&mut prompt, "  {}", path.display()).map_err(|_| "Cannot format directory scope".to_string())?;
        }
        prompt.push_str("\nThis rewrites matching files on disk and cannot be undone from inside Notepad Studio.");
        charge(&mut allocated, prompt.capacity().saturating_sub(initial_capacity))?;
        Ok(prompt)
    }
}

impl Workspace {
    pub(crate) fn cancel_directory_operations(&mut self) {
        self.directory_operations.cancel(None);
        self.find_in_files.searching = self.directory_operations.busy();
    }

    pub(crate) fn cancel_dialog_directory_operations(&mut self, handle: AnyWindowHandle) {
        let was_busy = self.directory_operations.busy();
        self.directory_operations.cancel(Some(handle));
        self.find_in_files.searching = self.directory_operations.busy();
        if was_busy && !self.find_in_files.searching {
            self.find_in_files.status = Some("Directory replacement cancelled".into());
        }
    }

    pub(super) fn start_directory_operation(&mut self, route: Route, replace: bool,
        window: &mut Window, cx: &mut Context<Self>)
    {
        if self.window_closed || self.directory_operations.busy() || self.find_in_files.searching { return; }
        let origin = window.window_handle(); let workspace_id = cx.entity().entity_id();
        let Some(root) = origin_root(window) else { return; };
        let dialog = matches!(root, OriginRoot::Dialog(_));
        if matches!(root, OriginRoot::Editor(id) if id != workspace_id)
            || (dialog && !self.find_dialog_window.is_some_and(|h| AnyWindowHandle::from(h) == origin)) { return; }
        if self.find_in_files.directory_text.len() > STRING_LIMIT {
            self.find_in_files.error = Some("Directory path exceeds 1 MiB".into()); cx.notify(); return;
        }
        if route == Route::Files { self.commit_directory_text(); }
        if self.find_in_files.query.trim().is_empty() {
            self.find_in_files.error = Some("Enter a search query".into()); cx.notify(); return;
        }
        let mut snapshot = match Snapshot::capture(self, route, replace) {
            Ok(snapshot) => snapshot,
            Err(error) => { self.find_in_files.error = Some(error); cx.notify(); return; }
        };
        let last_query = if replace { None } else {
            match copy_string(&snapshot.query.text, &mut snapshot.retained) {
                Ok(text) => Some(text),
                Err(error) => { self.find_in_files.error = Some(error); cx.notify(); return; }
            }
        };
        let snapshot = Arc::new(snapshot);
        // Admission covers a simultaneous worker query, before any task owns the slot.
        if let Err(error) = snapshot.query_for(&snapshot.query.directory) {
            self.find_in_files.error = Some(error); cx.notify(); return;
        }
        let prompt = if replace {
            match snapshot.prompt() {
                Ok(prompt) => Some(prompt),
                Err(error) => { self.find_in_files.error = Some(error); cx.notify(); return; }
            }
        } else { None };
        if replace {
            for (slot, value) in [
                (crate::settings::FindHistorySlot::FifQuery, &self.find_in_files.query),
                (crate::settings::FindHistorySlot::FifReplacement, &self.find_in_files.replacement),
                (crate::settings::FindHistorySlot::FifFilter, &self.find_in_files.filter),
            ] { crate::settings::push_find_history(slot, value); }
            if route == Route::Files {
                crate::settings::push_find_history(crate::settings::FindHistorySlot::FifDirectory,
                    &self.find_in_files.directory_text);
            }
        }
        let id = uuid::Uuid::new_v4(); let cancelled = Arc::new(AtomicBool::new(false));
        let (wake, stopped) = futures::channel::oneshot::channel();
        self.directory_operations.owner = Some(Owner { id, origin, root, dialog,
            phase: if replace { Phase::ConfirmReplace } else { Phase::Search },
            cancelled: cancelled.clone(), wake: Some(wake), task: None });
        self.find_in_files.searching = true;
        self.find_in_files.error = None;
        self.find_in_files.status = Some(match (route, replace) {
            (Route::Files, true) => "Confirming Replace in Files…",
            (Route::Projects, true) => "Confirming Replace in Projects…",
            (Route::Files, false) => "Searching…", (Route::Projects, false) => "Searching projects…",
        }.into());
        if !replace {
            self.find_in_files.results.clear(); self.find_in_files.truncated = false;
            self.find_in_files.last_search_query = last_query.unwrap_or_default();
            self.find_in_files.last_search_mode = super::find_in_files::describe_search_mode(
                snapshot.query.regex, snapshot.query.extended);
        }
        cx.notify();
        #[cfg(test)]
        let audit_confirmation = if replace { self.replace_audit_confirmations.pop_front() } else { None };
        #[cfg(test)]
        let audit_ack = self.replace_audit_ack.take();
        let task = cx.spawn_in(window, async move |this, cx| {
            if replace {
                #[cfg(test)]
                let confirmation = super::replace_lifecycle_audit_tests::confirmation(audit_confirmation, "", "", "");
                #[cfg(not(test))]
                let confirmation = rfd::AsyncMessageDialog::new().set_level(rfd::MessageLevel::Warning)
                    .set_title(if route == Route::Files { "Replace in Files" } else { "Replace in Projects" })
                    .set_description(prompt.as_deref().unwrap_or_default())
                    .set_buttons(rfd::MessageButtons::YesNo).show();
                futures::pin_mut!(confirmation);
                let confirm = match futures::future::select(confirmation, stopped).await {
                    futures::future::Either::Left((reply, _)) => reply,
                    futures::future::Either::Right(_) => return,
                };
                drop(prompt);
                if !matches!(confirm, rfd::MessageDialogResult::Yes) {
                    let live_origin = cx.update(|window, cx| root_matches(root, window, cx, workspace_id))
                        .unwrap_or(false);
                    let _ = this.update(cx, |w, cx| {
                        if w.directory_operations.finish(id).is_some() {
                            w.find_in_files.searching = false;
                            if live_origin && !w.window_closed {
                                w.find_in_files.status = Some("Directory replacement cancelled".into());
                            }
                            cx.notify();
                        }
                    });
                    return;
                }
                let accepted = cx.update(|window, cx| {
                    let matching_root = root_matches(root, window, cx, workspace_id);
                    this.update(cx, |w, cx| {
                    let valid = matching_root && w.directory_operations.owner.as_ref().is_some_and(|owner|
                        owner.id == id && owner.origin == origin && owner.root == root && owner.phase == Phase::ConfirmReplace
                        && !owner.cancelled.load(Ordering::Acquire)
                        && (!owner.dialog || w.find_dialog_window.is_some_and(|h| AnyWindowHandle::from(h) == origin)))
                        && !w.window_closed && snapshot.matches(w);
                    if valid {
                        if let Some(owner) = w.directory_operations.owner.as_mut() { owner.phase = Phase::RunningReplace; }
                        w.find_in_files.status = Some("Replacing…".into()); cx.notify();
                    } else if w.directory_operations.finish(id).is_some() {
                        w.find_in_files.searching = false;
                        if matching_root && !w.window_closed {
                            w.find_in_files.status = Some("Directory replacement cancelled because its scope changed or closed".into());
                        }
                        cx.notify();
                    }
                    valid
                })}).is_ok_and(|result| result.unwrap_or(false));
                if !accepted {
                    let _ = this.update(cx, |w, cx| {
                        if w.directory_operations.finish(id).is_some() {
                            w.find_in_files.searching = false;
                            cx.notify();
                        }
                    }); return;
                }
            }
            let live = cx.update(|window, cx| {
                let matching_root = root_matches(root, window, cx, workspace_id);
                this.update(cx, |w, _| {
                matching_root && !w.window_closed && w.directory_operations.owner.as_ref().is_some_and(|owner|
                    owner.id == id && owner.origin == origin && owner.root == root && !owner.cancelled.load(Ordering::Acquire)
                    && (!owner.dialog || w.find_dialog_window.is_some_and(|h| AnyWindowHandle::from(h) == origin)))
            })}).is_ok_and(|result| result.unwrap_or(false));
            if !live || cancelled.load(Ordering::Acquire) {
                let _ = this.update(cx, |w, cx| {
                    if w.directory_operations.finish(id).is_some() { w.find_in_files.searching = false; cx.notify(); }
                }); return;
            }
            let result = cx.background_spawn(async move {
                if replace { WorkResult::Replace(replace_scope(&snapshot, &cancelled)) }
                else { WorkResult::Search(search_scope(&snapshot, &cancelled)) }
            }).await;
            #[cfg(test)]
            if let Some(ack) = audit_ack { let _ = ack.await; }
            let current_origin = cx.update(|window, cx| root_matches(root, window, cx, workspace_id));
            let _ = this.update(cx, |w, cx| {
                let Some(was_cancelled) = w.directory_operations.finish(id) else { return; };
                w.find_in_files.searching = false;
                if w.window_closed || !(matches!(current_origin, Ok(true)) || (current_origin.is_err() && was_cancelled)) {
                    cx.notify(); return;
                }
                match result {
                    WorkResult::Replace(Ok(report)) => {
                        w.find_in_files.status = Some(format!("{} {} occurrence{} in {} file{}{}",
                            if was_cancelled || report.cancelled { "Cancelled after replacing" } else { "Replaced" },
                            report.replacements, if report.replacements == 1 { "" } else { "s" },
                            report.files_changed, if report.files_changed == 1 { "" } else { "s" },
                            if report.failures > 0 { report.failure_status() } else { String::new() }));
                        w.find_in_files.error = None;
                    }
                    WorkResult::Replace(Err(error)) => { w.find_in_files.error = Some(error); w.find_in_files.status = None; }
                    WorkResult::Search(outcome) => {
                        if was_cancelled { w.find_in_files.status = Some("Directory search cancelled".into()); }
                        else if w.find_in_files.open { publish_search(w, outcome); }
                    }
                }
                cx.notify();
            });
        });
        if let Some(owner) = self.directory_operations.owner.as_mut() {
            if owner.id == id { owner.task = Some(task); }
        }
    }
}

enum WorkResult {
    Search(FindInFilesOutcome),
    Replace(Result<super::find_in_files::ReplaceInFilesReport, String>),
}

fn search_scope(snapshot: &Snapshot, cancelled: &AtomicBool) -> FindInFilesOutcome {
    let mut merged = Vec::new(); let mut files = 0usize; let mut truncated = false; let mut elapsed = 0u64;
    for root in &snapshot.roots {
        if cancelled.load(Ordering::Acquire) { break; }
        let query = match snapshot.query_for(root) { Ok(q) => q, Err(e) => return FindInFilesOutcome::InvalidRegex(e) };
        match crate::find_in_files::run_search_cancelled(&query, cancelled) {
            FindInFilesOutcome::Ok { results, searched_files, truncated: cut, elapsed_ms } => {
                let remaining = crate::find_in_files::FIND_IN_FILES_RESULT_CAP.saturating_sub(merged.len());
                truncated |= results.len() > remaining;
                merged.extend(results.into_iter().take(remaining)); files = files.saturating_add(searched_files);
                truncated |= cut; elapsed = elapsed.saturating_add(elapsed_ms);
                if merged.len() >= crate::find_in_files::FIND_IN_FILES_RESULT_CAP { truncated = true; break; }
            }
            FindInFilesOutcome::InvalidRegex(error) => return FindInFilesOutcome::InvalidRegex(error),
            FindInFilesOutcome::DirectoryMissing if snapshot.route == Route::Files => return FindInFilesOutcome::DirectoryMissing,
            FindInFilesOutcome::DirectoryMissing => {}
        }
    }
    FindInFilesOutcome::Ok { results: merged, searched_files: files, truncated, elapsed_ms: elapsed }
}

fn replace_scope(snapshot: &Snapshot, cancelled: &AtomicBool) -> Result<super::find_in_files::ReplaceInFilesReport, String> {
    replace_scope_with_reader(snapshot, cancelled, crate::find_in_files::read_capped)
}

fn replace_scope_with_reader(snapshot: &Snapshot, cancelled: &AtomicBool,
    mut read: impl FnMut(&Path, u64) -> std::io::Result<Option<Vec<u8>>>) -> Result<super::find_in_files::ReplaceInFilesReport, String>
{
    let mut total = super::find_in_files::ReplaceInFilesReport::default();
    for root in &snapshot.roots {
        if cancelled.load(Ordering::Acquire) { total.cancelled = true; break; }
        let query = match snapshot.query_for(root) {
            Ok(query) => query,
            Err(error) if snapshot.route == Route::Files => return Err(error),
            Err(error) => { record_root_failure(&mut total, root, error); break; }
        };
        match super::find_in_files::run_replace_in_files_with_reader_cancelled(&query, &snapshot.replacement, &mut read, cancelled) {
            Ok(report) => {
                total.files_changed = total.files_changed.saturating_add(report.files_changed);
                total.replacements = total.replacements.saturating_add(report.replacements);
                total.failures = total.failures.saturating_add(report.failures);
                total.cancelled |= report.cancelled;
                if total.first_failure.is_none() { total.first_failure = report.first_failure; }
            }
            Err(error) if snapshot.route == Route::Files => return Err(error),
            Err(error) => record_root_failure(&mut total, root, error),
        }
    }
    total.cancelled |= cancelled.load(Ordering::Acquire);
    Ok(total)
}

fn record_root_failure(report: &mut super::find_in_files::ReplaceInFilesReport, root: &Path, error: String) {
    report.failures = report.failures.saturating_add(1);
    if report.first_failure.is_none() { report.first_failure = Some(format!("{}: {error}", root.display())); }
}

fn publish_search(w: &mut Workspace, outcome: FindInFilesOutcome) {
    match outcome {
        FindInFilesOutcome::Ok { results, searched_files, truncated, elapsed_ms } => {
            let count = results.len(); w.find_in_files.results = results;
            w.find_in_files.truncated = truncated; w.find_in_files.error = None;
            w.find_in_files.last_search_files_searched = searched_files;
            w.find_in_files.status = Some(format!("{count} match{} in {searched_files} file{} · {elapsed_ms}ms{}",
                if count == 1 { "" } else { "es" }, if searched_files == 1 { "" } else { "s" },
                if truncated { " (truncated)" } else { "" }));
        }
        FindInFilesOutcome::InvalidRegex(error) => { w.find_in_files.error = Some(error); w.find_in_files.status = None; }
        FindInFilesOutcome::DirectoryMissing => { w.find_in_files.error = Some("Directory not found".into()); w.find_in_files.status = None; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_string_capacity_and_byte_boundary() {
        let text = "x".repeat(STRING_LIMIT); let mut retained = 0;
        let copy = copy_string(&text, &mut retained).unwrap();
        assert_eq!(copy, text); assert_eq!(retained, copy.capacity());
        assert!(copy_string(&(text + "x"), &mut retained).is_err());
        assert_eq!(retained, copy.capacity());
    }

    #[test]
    fn snapshot_checks_budget_before_reserve_and_actual_capacity_before_copy() {
        let mut retained = SNAPSHOT_LIMIT;
        assert!(copy_string_reserved("x", &mut retained, |_, _| panic!("reservation must not run")).is_err());
        assert_eq!(retained, SNAPSHOT_LIMIT);
        retained = SNAPSHOT_LIMIT - 2;
        assert!(copy_string_reserved("x", &mut retained, |copy, _| copy.try_reserve_exact(1024)).is_err());
        assert_eq!(retained, SNAPSHOT_LIMIT - 2);
    }

    #[test]
    fn snapshot_fallible_reservation_and_checked_budget() {
        let mut retained = 0;
        assert!(copy_string_reserved("x", &mut retained, |copy, _| copy.try_reserve_exact(usize::MAX)).is_err());
        assert_eq!(retained, 0);
        retained = usize::MAX;
        assert!(copy_string("x", &mut retained).is_err());
        assert_eq!(retained, usize::MAX);
    }

    fn project_snapshot(roots: Vec<PathBuf>) -> Snapshot {
        Snapshot { route: Route::Projects, query: FindInFilesQuery {
            text: "hit".into(), filter: "*.txt".into(), directory: roots.first().unwrap().clone(),
            match_case: false, whole_word: false, regex: false, extended: false,
            dot_matches_newline: false, in_subfolders: true, in_hidden_folders: false,
        }, replacement: "done".into(), roots, retained: 1024 }
    }

    #[test]
    fn project_search_merge_has_one_global_result_cap() {
        let dir = tempfile::tempdir().unwrap(); let first = dir.path().join("one"); let second = dir.path().join("two");
        std::fs::create_dir(&first).unwrap(); std::fs::create_dir(&second).unwrap();
        for root in [&first, &second] { std::fs::write(root.join("neutral.txt"), "hit\r\n".repeat(600)).unwrap(); }
        let snapshot = project_snapshot(vec![first, second]);
        match search_scope(&snapshot, &AtomicBool::new(false)) {
            FindInFilesOutcome::Ok { results, truncated, searched_files, .. } => {
                assert_eq!(results.len(), crate::find_in_files::FIND_IN_FILES_RESULT_CAP);
                assert!(truncated); assert_eq!(searched_files, 2);
            }
            _ => panic!("actual project search must succeed"),
        }
    }

    #[test]
    fn project_cancelled_first_root_never_reads_second_root() {
        let dir = tempfile::tempdir().unwrap(); let first = dir.path().join("one"); let second = dir.path().join("two");
        std::fs::create_dir(&first).unwrap(); std::fs::create_dir(&second).unwrap();
        for root in [&first, &second] { std::fs::write(root.join("neutral.txt"), b"hit\r\n").unwrap(); }
        let snapshot = project_snapshot(vec![first.clone(), second.clone()]); let token = AtomicBool::new(false);
        let mut reads = 0;
        let report = replace_scope_with_reader(&snapshot, &token, |path, limit| {
            reads += 1; assert!(path.starts_with(&first));
            let read = crate::find_in_files::read_capped(path, limit)?;
            token.store(true, Ordering::Release); Ok(read)
        }).unwrap();
        assert!(report.cancelled); assert_eq!(reads, 1);
        assert_eq!((report.files_changed, report.replacements, report.failures), (0, 0, 0));
        assert_eq!(std::fs::read(second.join("neutral.txt")).unwrap(), b"hit\r\n");
    }

    fn root_failure_report(missing_first: bool) {
        let dir = tempfile::tempdir().unwrap();
        let valid = dir.path().join("valid"); let missing = dir.path().join("missing");
        std::fs::create_dir(&valid).unwrap();
        let file = valid.join("neutral.txt"); std::fs::write(&file, b"hit\r\n").unwrap();
        let roots = if missing_first { vec![missing.clone(), valid] } else { vec![valid, missing.clone()] };
        let report = replace_scope(&project_snapshot(roots), &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read(file).unwrap(), b"done\r\n");
        assert_eq!((report.files_changed, report.replacements, report.failures), (1, 1, 1));
        assert!(report.first_failure.as_deref().unwrap().contains(&missing.display().to_string()));
        assert!(report.first_failure.as_deref().unwrap().contains("Directory not found"));
    }

    #[test] fn project_root_failure_before_success_is_reported() { root_failure_report(true); }
    #[test] fn project_root_failure_after_success_is_reported() { root_failure_report(false); }

    #[test]
    fn project_all_missing_roots_keep_total_and_first_failure() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("missing-one"); let second = dir.path().join("missing-two");
        let report = replace_scope(&project_snapshot(vec![first.clone(), second]), &AtomicBool::new(false))
            .unwrap_or_else(|error| panic!("missing roots must produce counted failures: {error}"));
        assert_eq!((report.files_changed, report.replacements, report.failures), (0, 0, 2));
        assert!(report.first_failure.as_deref().unwrap().contains(&first.display().to_string()));
    }

    #[test]
    fn files_missing_directory_keeps_the_direct_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut snapshot = project_snapshot(vec![dir.path().join("missing")]); snapshot.route = Route::Files;
        assert_eq!(replace_scope(&snapshot, &AtomicBool::new(false)).err().as_deref(), Some("Directory not found"));
    }

    #[test]
    fn project_root_failure_keeps_prior_file_failure_and_counts_both() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("neutral.txt"); std::fs::write(&file, b"hit\r\n").unwrap();
        let snapshot = project_snapshot(vec![dir.path().to_path_buf(), dir.path().join("missing")]);
        let report = replace_scope_with_reader(&snapshot, &AtomicBool::new(false), |_, _|
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "synthetic read refusal"))).unwrap();
        assert_eq!((report.files_changed, report.replacements, report.failures), (0, 0, 2));
        let failure = report.first_failure.as_deref().unwrap();
        assert!(failure.contains(&file.display().to_string())); assert!(failure.contains("synthetic read refusal"));
        assert_eq!(std::fs::read(file).unwrap(), b"hit\r\n");
    }
}
