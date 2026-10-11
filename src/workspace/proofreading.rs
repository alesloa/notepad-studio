//! Source-view proofreading. Worker results and context-menu targets belong to
//! one tab incarnation and one edit activity, including edit/Undo round trips.

use std::{collections::HashSet, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};
use gpui::{Context, Task, Window};
use uuid::Uuid;
use crate::{buffer::ContentRevision, proofreading::{Issue, IssueKind}, tab::TabViewMode};
use super::Workspace;

const SOURCE_LIMIT: usize = 1024 * 1024;
const ISSUE_LIMIT: usize = 512;
const TEXT_LIMIT: usize = 1024;
const IGNORE_LIMIT: usize = 1024;

#[derive(Clone, PartialEq, Eq)]
struct Policy { spelling: bool, grammar: bool }
impl Policy {
    fn current() -> Self {
        let prefs = crate::settings::snapshot();
        Self { spelling: prefs.proofread_spelling, grammar: prefs.proofread_grammar }
    }
}

#[derive(Clone)]
struct Stamp {
    tab: Uuid,
    instance: Arc<()>,
    revision: ContentRevision,
    activity: ContentRevision,
    ignored: ContentRevision,
    generation: ContentRevision,
    policy: Policy,
    markdown: bool,
    rope: ropey::Rope,
}
impl Stamp {
    fn same(&self, other: &Self) -> bool {
        self.tab == other.tab && Arc::ptr_eq(&self.instance, &other.instance)
            && self.revision == other.revision && self.activity == other.activity
            && self.ignored == other.ignored && self.generation == other.generation && self.policy == other.policy
            && self.markdown == other.markdown && self.rope.is_instance(&other.rope)
    }
}

struct Request { id: Uuid, stamp: Stamp }
struct Cached { stamp: Stamp, issues: Vec<Issue> }
#[derive(Clone)]
pub(crate) struct CorrectionMenu { pub(crate) id: Uuid, pub(crate) issue: Issue }
struct Menu { stamp: Stamp, target: CorrectionMenu }

#[derive(Default)]
pub(super) struct Runtime {
    desired: Option<Stamp>,
    pending: Option<Request>,
    cached: Option<Cached>,
    menu: Option<Menu>,
    task: Option<Task<()>>,
    waiting: bool,
    cancel: Option<Arc<AtomicBool>>,
    ignored: HashSet<String>,
    ignored_revision: ContentRevision,
    generation: ContentRevision,
    closed: bool,
    oversized: Option<(Uuid, ContentRevision)>,
    #[cfg(test)]
    hold: bool,
    #[cfg(test)]
    held: Option<(Request, Result<Vec<Issue>, String>)>,
    #[cfg(test)]
    started: usize,
}
impl Runtime {
    pub(super) fn close(&mut self) {
        self.closed = true;
        if let Some(cancel) = &self.cancel { cancel.store(true, Ordering::Relaxed); }
        self.pending = None;
        self.desired = None;
        self.cached = None;
        self.menu = None;
    }
}
impl Drop for Runtime {
    fn drop(&mut self) { self.close(); }
}

fn valid_issues(source: &str, issues: &mut Vec<Issue>, ignored: &HashSet<String>) {
    issues.truncate(ISSUE_LIMIT);
    issues.retain_mut(|issue| {
        if issue.range.start >= issue.range.end || issue.original.len() > TEXT_LIMIT
            || source.get(issue.range.clone()) != Some(issue.original.as_str())
            || (matches!(issue.kind, IssueKind::Spelling)
                && ignored.contains(&issue.original.to_lowercase())) { return false; }
        issue.suggestions.truncate(3);
        issue.suggestions.retain(|s| s.len() <= TEXT_LIMIT && s != &issue.original);
        if issue.message.len() > TEXT_LIMIT {
            issue.message.truncate(issue.message.floor_char_boundary(TEXT_LIMIT));
        }
        true
    });
    issues.sort_unstable_by_key(|issue| (issue.range.start, issue.range.end));
}

impl Workspace {
    fn proofreading_source_eligible(tab: &crate::tab::Tab) -> bool {
        tab.view_mode == TabViewMode::Edit && tab.buffer.rich_doc.is_none()
            && tab.buffer.loading.is_none() && tab.csv_draft.is_none()
            && !crate::ui::csv_doc::is_csv_path(tab.buffer.file_path.as_deref())
            && matches!(tab.buffer.language, None | Some(crate::syntax::LanguageId::Markdown))
    }

    fn proofreading_stamp(&mut self) -> Option<Stamp> {
        if self.window_closed || self.proofreader.closed { return None; }
        let policy = Policy::current();
        if !policy.spelling && !policy.grammar { return None; }
        let tab = self.tabs.get_mut(self.active)?;
        if !Self::proofreading_source_eligible(tab) { return None; }
        tab.observe_content_activity();
        if tab.buffer.rope.len_bytes() > SOURCE_LIMIT { return None; }
        Some(Stamp {
            tab: tab.session_id, instance: tab.instance.clone(),
            revision: tab.buffer.raw_revision.clone(), activity: tab.content_activity.clone(),
            ignored: self.proofreader.ignored_revision.clone(), policy,
            generation: self.proofreader.generation.clone(),
            markdown: tab.buffer.language == Some(crate::syntax::LanguageId::Markdown)
                || tab.buffer.file_path.as_deref().and_then(|p| p.extension()).and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")),
            rope: tab.buffer.rope.clone(),
        })
    }

    fn proofreading_valid(&self, stamp: &Stamp) -> bool {
        !self.window_closed && !self.proofreader.closed && stamp.policy == Policy::current()
            && stamp.ignored == self.proofreader.ignored_revision
            && stamp.generation == self.proofreader.generation
            && self.tabs.get(self.active).is_some_and(|tab| {
                tab.session_id == stamp.tab && Arc::ptr_eq(&tab.instance, &stamp.instance)
                    && tab.content_activity == stamp.activity && tab.buffer.raw_revision == stamp.revision
                    && tab.buffer.rope.is_instance(&stamp.rope) && tab.buffer.loading.is_none()
                    && tab.buffer.rich_doc.is_none() && tab.view_mode == TabViewMode::Edit
                    && matches!(tab.buffer.language, None | Some(crate::syntax::LanguageId::Markdown))
                    && stamp.markdown == (tab.buffer.language == Some(crate::syntax::LanguageId::Markdown)
                        || tab.buffer.file_path.as_deref().and_then(|p| p.extension()).and_then(|ext| ext.to_str())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")))
                    && tab.csv_draft.is_none()
                    && !crate::ui::csv_doc::is_csv_path(tab.buffer.file_path.as_deref())
            })
    }

    pub(crate) fn ensure_proofreading(&mut self, cx: &mut Context<Self>) {
        let policy = Policy::current();
        let oversized = self.tabs.get(self.active).filter(|tab| (policy.spelling || policy.grammar)
            && Self::proofreading_source_eligible(tab) && tab.buffer.rope.len_bytes() > SOURCE_LIMIT)
            .map(|tab| (tab.session_id, tab.buffer.raw_revision.clone()));
        if oversized != self.proofreader.oversized {
            self.proofreader.oversized = oversized;
            if self.proofreader.oversized.is_some() {
                self.set_status("Spelling and grammar checking is paused for documents over 1 MiB");
            }
        }
        let mut next = self.proofreading_stamp();
        if match (&next, &self.proofreader.desired) {
            (Some(a), Some(b)) => a.same(b), (None, None) => true, _ => false,
        } { self.start_proofreading(cx); return; }
        self.proofreader.generation = Default::default();
        if let Some(stamp) = &mut next { stamp.generation = self.proofreader.generation.clone(); }
        self.proofreader.cached = None;
        self.proofreader.menu = None;
        // A timer owns no reader payload. Restart that single timer on typing;
        // a running worker keeps its slot until delivery and retirement finish.
        if self.proofreader.waiting {
            self.proofreader.task = None;
            self.proofreader.waiting = false;
        }
        if let Some(cancel) = &self.proofreader.cancel { cancel.store(true, Ordering::Relaxed); }
        self.proofreader.pending = None;
        self.proofreader.desired = next.clone();
        if let Some(stamp) = next {
            self.proofreader.pending = Some(Request { id: Uuid::new_v4(), stamp });
        }
        self.start_proofreading(cx);
    }

    fn start_proofreading(&mut self, cx: &mut Context<Self>) {
        if self.proofreader.task.is_some() || self.proofreader.pending.is_none() { return; }
        let executor = cx.background_executor().clone();
        self.proofreader.waiting = true;
        self.proofreader.task = Some(cx.spawn(async move |this, cx| {
            loop {
                let ticket = this.update(cx, |w, _| {
                    w.proofreader.waiting = true;
                    w.proofreader.pending.as_ref().map(|r| r.id)
                });
                let Ok(Some(ticket)) = ticket else { break; };
                executor.timer(Duration::from_millis(250)).await;
                let request = this.update(cx, |w, _| {
                    w.proofreader.waiting = false;
                    if w.proofreader.pending.as_ref().map(|r| r.id) != Some(ticket) { return None; }
                    let request = w.proofreader.pending.take()?;
                    if !w.proofreading_valid(&request.stamp) { return None; }
                    let cancel = Arc::new(AtomicBool::new(false));
                    w.proofreader.cancel = Some(cancel.clone());
                    #[cfg(test)] { w.proofreader.started += 1; }
                    Some((request, cancel, w.proofreader.ignored.clone()))
                });
                let Ok(Some((request, cancel, ignored))) = request else { continue; };
                let parsed = executor.spawn(async move {
                    let source = request.stamp.rope.to_string();
                    let mut result = crate::proofreading::check(&source, request.stamp.markdown,
                        request.stamp.policy.spelling, request.stamp.policy.grammar, &cancel);
                    if let Ok(issues) = &mut result { valid_issues(&source, issues, &ignored); }
                    (request, result)
                }).await;
                let mut delivery = Some(parsed);
                let _ = this.update(cx, |w, cx| {
                    #[cfg(test)]
                    if w.proofreader.hold { w.proofreader.held = delivery.take(); return; }
                    if let Some((request, result)) = delivery.take() { w.finish_proofreading(request, result, cx); }
                });
                // Physical retirement precedes another request, including a lost editor.
                executor.spawn(async move { drop(delivery); }).await;
                #[cfg(test)]
                if this.update(cx, |w, _| w.proofreader.held.is_some()).unwrap_or(false) { return; }
            }
            let _ = this.update(cx, |w, _| { w.proofreader.task = None; w.proofreader.waiting = false; });
        }));
    }

    fn finish_proofreading(&mut self, request: Request, result: Result<Vec<Issue>, String>, cx: &mut Context<Self>) {
        if self.proofreading_valid(&request.stamp) {
            match result {
                Ok(issues) => self.proofreader.cached = Some(Cached { stamp: request.stamp, issues }),
                Err(error) => self.set_status(format!("Spelling and grammar check failed: {error}")),
            }
            cx.notify();
        }
        self.proofreader.cancel = None;
    }

    pub(crate) fn proofreading_issues(&self) -> &[Issue] {
        self.proofreader.cached.as_ref().filter(|cache| self.proofreading_valid(&cache.stamp))
            .map_or(&[], |cache| cache.issues.as_slice())
    }
    pub(crate) fn proofreading_menu(&self) -> Option<CorrectionMenu> {
        self.proofreader.menu.as_ref().filter(|menu| self.proofreading_valid(&menu.stamp))
            .map(|menu| menu.target.clone())
    }
    pub(crate) fn capture_proofreading_menu(&mut self, byte: usize, _cx: &mut Context<Self>) {
        self.proofreader.menu = self.proofreader.cached.as_ref()
            .filter(|cache| self.proofreading_valid(&cache.stamp))
            .and_then(|cache| cache.issues.iter().find(|issue| issue.range.contains(&byte))
                .map(|issue| Menu { stamp: cache.stamp.clone(), target: CorrectionMenu {
                    id: Uuid::new_v4(), issue: issue.clone(),
                } }));
    }
    pub(crate) fn clear_proofreading_menu(&mut self) { self.proofreader.menu = None; }

    pub(crate) fn apply_proofreading_suggestion(&mut self, menu_id: Uuid, index: usize, cx: &mut Context<Self>) {
        // A callback from a retired menu must not dismiss its replacement.
        if !self.proofreader.menu.as_ref().is_some_and(|menu| menu.target.id == menu_id) { return; }
        let valid = self.proofreader.menu.as_ref().is_some_and(|menu|
            menu.target.id == menu_id && self.proofreading_valid(&menu.stamp));
        let target = if valid { self.proofreader.menu.take().map(|menu| menu.target) } else { None };
        self.hide_context_menu(cx);
        let Some(target) = target else { self.set_status("This suggestion is out of date"); cx.notify(); return; };
        let Some(replacement) = target.issue.suggestions.get(index) else { return; };
        if self.correct_proofreading_range(target.issue.range, &target.issue.original, replacement, false, cx) {
            crate::macros::record(crate::macros::MacroEvent::CorrectText {
                original: target.issue.original, replacement: replacement.clone(),
            });
        }
    }

    fn correct_proofreading_range(&mut self, range: std::ops::Range<usize>, original: &str,
        replacement: &str, replay: bool, cx: &mut Context<Self>) -> bool {
        if original.is_empty() || original == replacement || original.len() > TEXT_LIMIT || replacement.len() > TEXT_LIMIT { return false; }
        let Some(tab) = self.tabs.get(self.active) else { return false; };
        if tab.buffer.read_only { self.set_status("Document is read-only"); cx.notify(); return false; }
        if tab.buffer.rich_doc.is_some() || tab.view_mode != TabViewMode::Edit || tab.buffer.loading.is_some()
            || (!replay && tab.edit_transaction.is_some()) || tab.block_selection.is_some() || self.bulk_edit
            || self.ime_composition.is_some() || self.pending_prepared_edit.is_some() { return false; }
        let Some(text) = tab.buffer.rope.get_byte_slice(range.clone()) else { return false; };
        if text.len_bytes() != original.len() || !text.chars().eq(original.chars()) { return false; }
        self.replace_byte_range(range, replacement, cx);
        self.ensure_proofreading(cx);
        true
    }

    pub(crate) fn replay_proofreading_correction(&mut self, original: &str, replacement: &str,
        cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false; };
        let selection = tab.selection.ordered();
        let range = if selection.is_empty() {
            let Some(start) = selection.end.checked_sub(original.len()) else { return false; };
            start..selection.end
        } else { selection };
        self.correct_proofreading_range(range, original, replacement, true, cx)
    }

    pub(crate) fn ignore_proofreading_word(&mut self, menu_id: Uuid, cx: &mut Context<Self>) {
        let Some(menu) = self.proofreader.menu.as_ref() else { return; };
        if menu.target.id != menu_id || !self.proofreading_valid(&menu.stamp)
            || !matches!(menu.target.issue.kind, IssueKind::Spelling) { return; }
        let word = menu.target.issue.original.to_lowercase();
        if word.len() > 128 || self.proofreader.ignored.len() >= IGNORE_LIMIT {
            self.set_status("The session's ignored-word list is full"); cx.notify(); return;
        }
        self.proofreader.ignored.insert(word);
        self.proofreader.ignored_revision = Default::default();
        self.hide_context_menu(cx);
        self.ensure_proofreading(cx);
        cx.notify();
    }

    pub(crate) fn toggle_spelling(&mut self, _: &crate::actions::EditToggleSpelling, _: &mut Window, cx: &mut Context<Self>) {
        let mut prefs = crate::settings::snapshot(); prefs.proofread_spelling = !prefs.proofread_spelling;
        crate::settings::replace_and_save(prefs); self.ensure_proofreading(cx); cx.notify();
    }
    pub(crate) fn toggle_grammar(&mut self, _: &crate::actions::EditToggleGrammar, _: &mut Window, cx: &mut Context<Self>) {
        let mut prefs = crate::settings::snapshot(); prefs.proofread_grammar = !prefs.proofread_grammar;
        crate::settings::replace_and_save(prefs); self.ensure_proofreading(cx); cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::syntax_refresh_tests::with_sources;
    use crate::actions::{EditRedo, EditUndo};

    fn issue(range: std::ops::Range<usize>, original: &str, replacement: &str) -> Issue {
        Issue { range, original: original.into(), kind: IssueKind::Spelling,
            message: "Possible spelling mistake".into(), suggestions: vec![replacement.into()] }
    }

    fn prepare(entity: &gpui::Entity<Workspace>, visual: &mut gpui::VisualTestContext, source: &str) {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.proofreader.close();
            w.proofreader = Runtime::default();
            w.tabs[0].buffer.language = None;
            w.tabs[0].buffer.rope = ropey::Rope::from_str(source);
            w.tabs[0].buffer.raw_revision = Default::default();
            w.tabs[0].buffer.mark_saved();
            w.tabs[0].selection.collapse_to(0);
            w.tabs[0].undo_stack.clear();
            w.tabs[0].redo_stack.clear();
            w.proofreader.hold = true;
            w.ensure_proofreading(cx);
            cx.notify();
        }));
        visual.run_until_parked();
    }

    fn hold(entity: &gpui::Entity<Workspace>, visual: &mut gpui::VisualTestContext) {
        visual.run_until_parked();
        visual.cx.executor().advance_clock(Duration::from_millis(300));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.proofreader.held.is_some(), "actual admitted worker reached delivery");
            assert_eq!(w.proofreader.started, 1);
        });
    }

    fn release(w: &mut Workspace, cx: &mut Context<Workspace>) {
        let (request, _) = w.proofreader.held.take().expect("held worker");
        w.proofreader.task = None;
        // Deterministic engine output isolates lifecycle/correction tests from dictionary changes.
        w.finish_proofreading(request, Ok(vec![issue(0..3, "Ths", "This")]), cx);
        w.start_proofreading(cx);
    }

    fn publish(entity: &gpui::Entity<Workspace>, visual: &mut gpui::VisualTestContext) {
        prepare(entity, visual, "Ths caf\u{e9} \u{1f600}.\n");
        hold(entity, visual);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            release(w, cx);
            assert_eq!(w.proofreading_issues().len(), 1);
            w.capture_proofreading_menu(1, cx);
        }));
    }

    #[test]
    fn proofreading_validates_unicode_byte_spans_and_bounds_output() {
        let source = "caf\u{e9} \u{1f600} Ths";
        let mut issues = vec![issue(0..5, "caf\u{e9}", "cafe"), issue(3..4, "x", "y"),
            issue(6..10, "\u{1f600}", "smile"), issue(11..14, "Ths", "This"),
            issue(14..usize::MAX, "x", "y")];
        valid_issues(source, &mut issues, &HashSet::new());
        assert_eq!(issues.iter().map(|i| i.range.clone()).collect::<Vec<_>>(), vec![0..5, 6..10, 11..14]);
        let mut many = (0..600).map(|_| issue(11..14, "Ths", "This")).collect();
        valid_issues(source, &mut many, &HashSet::new());
        assert_eq!(many.len(), ISSUE_LIMIT);
        let mut ignored = HashSet::new(); ignored.insert("ths".into());
        valid_issues(source, &mut many, &ignored);
        assert!(many.is_empty());
    }

    #[test]
    fn proofreading_actual_worker_coalesces_burst_and_rejects_edit_undo_aba() {
        with_sources(|entity, visual| {
            prepare(entity, visual, "Ths word.\n");
            hold(entity, visual);
            visual.simulate_input("x");
            visual.dispatch_action(EditUndo);
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.ensure_proofreading(cx);
                assert_eq!(w.tabs[0].buffer.rope.to_string(), "Ths word.\n");
                assert_eq!(w.proofreader.started, 1);
                assert!(w.proofreader.pending.is_some());
                release(w, cx);
                assert!(w.proofreading_issues().is_empty());
            }));
        });
    }

    #[test]
    fn proofreading_actual_typing_burst_keeps_one_timer_and_latest_source() {
        with_sources(|entity, visual| {
            prepare(entity, visual, "Ths word.\n");
            visual.simulate_input("a");
            visual.simulate_input("b");
            visual.simulate_input("c");
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.ensure_proofreading(cx);
                assert_eq!(w.proofreader.started, 0);
                assert!(w.proofreader.task.is_some());
                assert_eq!(w.proofreader.pending.as_ref().expect("latest").stamp.rope.to_string(), "abcThs word.\n");
            }));
            hold(entity, visual);
            visual.update(|_, cx| {
                let w = entity.read(cx);
                assert_eq!(w.proofreader.held.as_ref().expect("delivery").0.stamp.rope.to_string(), "abcThs word.\n");
                assert!(w.proofreader.pending.is_none());
            });
        });
    }

    #[test]
    fn proofreading_actual_worker_rejects_tab_switch_reopen_and_closed_window() {
        for change in 0..3 {
            with_sources(|entity, visual| {
                prepare(entity, visual, "Ths word.\n"); hold(entity, visual);
                visual.update(|_, cx| entity.update(cx, |w, cx| {
                    match change {
                        0 => w.set_active_tab(1, cx),
                        1 => w.tabs[0].instance = Arc::new(()),
                        _ => { w.window_closed = true; w.proofreader.close(); },
                    }
                    release(w, cx);
                    assert!(w.proofreading_issues().is_empty());
                }));
            });
        }
    }

    #[test]
    fn proofreading_actual_menu_corrects_once_and_preserves_savepoint_unicode_metadata() {
        with_sources(|entity, visual| {
            publish(entity, visual);
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                let menu = w.proofreading_menu().expect("menu");
                let encoding = w.tabs[0].buffer.encoding;
                let eol = w.tabs[0].buffer.line_ending;
                w.apply_proofreading_suggestion(menu.id, 0, cx);
                assert_eq!(w.tabs[0].buffer.rope.to_string(), "This caf\u{e9} \u{1f600}.\n");
                assert_eq!(w.tabs[0].undo_stack.len(), 1);
                assert!(w.tabs[0].buffer.dirty);
                assert_eq!(w.tabs[0].buffer.encoding, encoding);
                assert_eq!(w.tabs[0].buffer.line_ending, eol);
            }));
            visual.dispatch_action(EditUndo);
            visual.update(|_, cx| {
                let w = entity.read(cx);
                assert_eq!(w.tabs[0].buffer.rope.to_string(), "Ths caf\u{e9} \u{1f600}.\n");
                assert!(!w.tabs[0].buffer.dirty);
            });
            visual.dispatch_action(EditRedo);
            visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.rope.to_string(), "This caf\u{e9} \u{1f600}.\n"));
        });
    }

    #[test]
    fn proofreading_actual_stale_menu_and_readonly_never_edit() {
        for change in 0..5 {
            with_sources(|entity, visual| {
                publish(entity, visual);
                visual.update(|_, cx| entity.update(cx, |w, cx| {
                    let menu = w.proofreading_menu().expect("menu");
                    match change {
                        0 => { w.replace_byte_range(0..0, "x", cx); },
                        1 => w.set_active_tab(1, cx),
                        2 => w.tabs[0].instance = Arc::new(()),
                        3 => w.tabs[0].buffer.read_only = true,
                        _ => w.capture_proofreading_menu(1, cx),
                    }
                    let before = w.tabs.iter().map(|t| (t.buffer.rope.to_string(), t.undo_stack.len(), t.buffer.dirty)).collect::<Vec<_>>();
                    w.apply_proofreading_suggestion(menu.id, 0, cx);
                    assert_eq!(before, w.tabs.iter().map(|t| (t.buffer.rope.to_string(), t.undo_stack.len(), t.buffer.dirty)).collect::<Vec<_>>());
                    if change == 4 { assert!(w.proofreading_menu().is_some()); }
                }));
            });
        }
    }

    #[test]
    fn proofreading_actual_policy_source_modes_and_size_cap_do_not_publish() {
        for change in 0..4 {
            with_sources(|entity, visual| {
                prepare(entity, visual, "Ths word.\n"); hold(entity, visual);
                let mut prefs = crate::settings::snapshot(); prefs.proofread_spelling = false; prefs.proofread_grammar = false;
                let _policy = (change == 0).then(|| crate::settings::override_for_test(prefs));
                visual.update(|_, cx| entity.update(cx, |w, cx| {
                    match change {
                        1 => w.tabs[0].view_mode = TabViewMode::MarkdownPreview,
                        2 => w.tabs[0].buffer.language = Some(crate::syntax::LanguageId::Rust),
                        3 => { w.tabs[0].buffer.rope = ropey::Rope::from_str(&"x".repeat(SOURCE_LIMIT + 1)); w.tabs[0].buffer.raw_revision = Default::default(); },
                        _ => {},
                    }
                    w.ensure_proofreading(cx); release(w, cx);
                    assert!(w.proofreading_issues().is_empty());
                    assert!(w.proofreader.pending.is_none());
                    if change == 3 { assert!(w.status_message.as_deref().is_some_and(|s| s.contains("1 MiB"))); }
                }));
            });
        }
    }

    #[test]
    fn proofreading_actual_settings_aba_rejects_cancelled_result_and_disabled_caps_keep_status() {
        with_sources(|entity, visual| {
            prepare(entity, visual, "Ths word.\n"); hold(entity, visual);
            let mut prefs = crate::settings::snapshot(); prefs.proofread_spelling = false; prefs.proofread_grammar = false;
            {
                let _off = crate::settings::override_for_test(prefs);
                visual.update(|_, cx| entity.update(cx, |w, cx| w.ensure_proofreading(cx)));
            }
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.ensure_proofreading(cx);
                release(w, cx);
                assert!(w.proofreading_issues().is_empty());
            }));
        });
        with_sources(|entity, visual| {
            let mut prefs = crate::settings::snapshot(); prefs.proofread_spelling = false; prefs.proofread_grammar = false;
            let _off = crate::settings::override_for_test(prefs);
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.tabs[0].buffer.language = None;
                w.tabs[0].buffer.rope = ropey::Rope::from_str(&"x".repeat(SOURCE_LIMIT + 1));
                w.set_status("Existing file-load warning");
                w.ensure_proofreading(cx);
                assert_eq!(w.status_message.as_deref(), Some("Existing file-load warning"));
                assert!(w.proofreader.pending.is_none());
            }));
        });
    }

    #[test]
    fn proofreading_actual_ime_composition_refuses_correction_replay() {
        with_sources(|entity, visual| {
            prepare(entity, visual, "Ths word.\n");
            visual.update(|window, cx| entity.update(cx, |w, cx| {
                assert!(w.replace_ime_text(Some(0..3), "Ths", Some(3..3), true, window, cx));
                assert!(w.ime_composition.is_some());
                let before = (w.tabs[0].buffer.rope.to_string(), w.tabs[0].undo_stack.len());
                assert!(!w.replay_proofreading_correction("Ths", "This", cx));
                assert_eq!(before, (w.tabs[0].buffer.rope.to_string(), w.tabs[0].undo_stack.len()));
            }));
        });
    }

    #[test]
    fn proofreading_correction_replay_requires_exact_selection_or_preceding_caret() {
        with_sources(|entity, visual| {
            prepare(entity, visual, "\u{1f600} Ths caf\u{e9}");
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.tabs[0].selection.collapse_to(8);
                assert!(w.replay_proofreading_correction("Ths", "This", cx));
                assert_eq!(w.tabs[0].buffer.rope.to_string(), "\u{1f600} This caf\u{e9}");
                let text = w.tabs[0].buffer.rope.to_string();
                assert!(!w.replay_proofreading_correction("Ths", "Wrong", cx));
                w.tabs[0].selection.collapse_to(1);
                assert!(!w.replay_proofreading_correction("s", "Wrong", cx));
                assert_eq!(w.tabs[0].buffer.rope.to_string(), text);
            }));
        });
    }

    #[test]
    fn proofreading_real_engine_right_click_correction_records_and_replays_one_undo_step() {
        use gpui::{Modifiers, MouseButton, point, px};
        with_sources(|entity, visual| {
            let _recorder = crate::macros::override_execution_for_test(true);
            let source = "\u{FEFF}This is a sentnce.\r\nhttps://sentnce.example/mispell";
            prepare(entity, visual, source);
            hold(entity, visual);
            let typo = source.find("sentnce").unwrap();
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                let (request, result) = w.proofreader.held.take().expect("real delivery");
                w.proofreader.task = None;
                assert!(result.as_ref().unwrap().iter().any(|issue|
                    issue.original == "sentnce" && issue.suggestions.iter().any(|s| s == "sentence")));
                w.finish_proofreading(request, result, cx);
            }));
            visual.run_until_parked();
            let position = visual.update(|_, cx| {
                let w = entity.read(cx);
                assert!(w.proofreading_issues().iter().all(|issue| issue.range.end <= source.find("https").unwrap()));
                let row = w.last_lines.iter().find(|row| row.line_byte_start <= typo
                    && row.line_byte_start.saturating_add(row.line_byte_len) > typo).expect("painted typo row");
                point(row.bounds.left() + row.shaped.x_for_index(typo - row.line_byte_start) + px(2.), row.bounds.center().y)
            });
            visual.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
            visual.run_until_parked();
            visual.update(|_, cx| {
                let menu = entity.read(cx).proofreading_menu().expect("right-click target");
                assert_eq!(menu.issue.original, "sentnce");
                assert_eq!(menu.issue.suggestions[0], "sentence");
            });
            let row = visual.debug_bounds("proofreading-first-suggestion").expect("painted correction row");
            visual.simulate_click(row.center(), Modifiers::none());
            crate::macros::stop_recording();
            let events = crate::macros::current();
            assert!(matches!(events.as_slice(), [crate::macros::MacroEvent::CorrectText { original, replacement }]
                if original == "sentnce" && replacement == "sentence"));
            visual.dispatch_action(EditUndo);
            visual.update(|window, cx| entity.update(cx, |w, cx| {
                assert_eq!(w.buffer().rope.to_string(), source);
                assert!(!w.buffer().dirty);
                w.tabs[0].selection.collapse_to(typo + "sentnce".len());
                assert!(w.replay_macro(&events, 1, window, cx));
                assert_eq!(w.buffer().rope.to_string(), source.replacen("sentnce", "sentence", 1));
                assert_eq!(w.tabs[0].undo_stack.len(), 1);
            }));
            visual.dispatch_action(EditUndo);
            visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), source));
        });
    }

    #[test]
    fn proofreading_session_ignore_rechecks_without_editing_the_document() {
        with_sources(|entity, visual| {
            let source = "This is a sentnce. This is another sentnce.";
            prepare(entity, visual, source);
            hold(entity, visual);
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                let (request, result) = w.proofreader.held.take().unwrap();
                w.proofreader.task = None;
                w.finish_proofreading(request, result, cx);
                w.capture_proofreading_menu(source.find("sentnce").unwrap(), cx);
                let id = w.proofreading_menu().unwrap().id;
                w.ignore_proofreading_word(id, cx);
                assert!(w.proofreading_menu().is_none());
                assert!(w.proofreading_issues().is_empty());
                assert_eq!(w.buffer().rope.to_string(), source);
                assert!(!w.buffer().dirty);
                assert!(w.tabs[0].undo_stack.is_empty());
            }));
            visual.run_until_parked();
            visual.cx.executor().advance_clock(Duration::from_millis(300));
            visual.run_until_parked();
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                let (request, result) = w.proofreader.held.take().expect("ignored-word recheck");
                w.proofreader.task = None;
                w.finish_proofreading(request, result, cx);
                assert!(w.proofreading_issues().iter().all(|issue| issue.original != "sentnce"));
                assert_eq!(w.buffer().rope.to_string(), source);
            }));
        });
    }
}
