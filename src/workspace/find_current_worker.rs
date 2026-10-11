//! Worker-owned matching, range banks and result snippets. No document AST here.
use std::{ops::Range, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use crate::{find_block_map::ValidatedFindBlockMap, find_in_files::{FindResult, LineWalker}};
use super::{
    find::SearchFlags,
    find_current_budget::{Credit, Ledger, Owned, Pool},
    find_current_endpoints::{Endpoint, EndpointResolver, ResolvedTargets, ENDPOINT_BYTES},
    find_current_pattern::{copy_text, pattern, QUERY_BYTES},
    find_current_projection::{Projection, ProjectionError},
};

const RANGE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const RESULT_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_ROWS: usize = 1000;
const MAX_MATCHES: usize = 1_000_000;
const SNIPPET_BYTES: usize = 2044;

pub(super) enum Failure { Budget(ProjectionError), Regex(regex::Error), Cancelled }
impl From<ProjectionError> for Failure {
    fn from(error: ProjectionError) -> Self { Self::Budget(error) }
}

// Credits stay alive until the actual row String/PathBuf allocations are freed.
pub(super) struct RowCredits { _path: Credit, _label: Credit, _snippet: Credit }
pub(super) struct Prepared {
    pub(super) rows: Owned<Vec<FindResult>>,
    pub(super) row_credits: Owned<Vec<RowCredits>>,
    pub(super) matches: Owned<Vec<Range<usize>>>,
    pub(super) marks: Owned<Vec<Range<usize>>>,
    pub(super) map: Arc<ValidatedFindBlockMap>,
    pub(super) endpoints: EndpointResolver,
    pub(super) targets: ResolvedTargets,
    pub(super) truncated: bool,
}

fn reserve_next<T>(out: &mut Owned<Vec<T>>, maximum: usize) -> Result<(), ProjectionError> {
    if out.data.len() >= maximum { return Err(ProjectionError::Limit); }
    if out.data.len() == out.data.capacity() {
        let count = out.data.capacity().max(32).saturating_mul(2).min(maximum);
        out.reserve(count.checked_mul(std::mem::size_of::<T>()).ok_or(ProjectionError::Limit)?)?;
    }
    Ok(())
}
fn push<T>(out: &mut Owned<Vec<T>>, value: T, maximum: usize) -> Result<(), ProjectionError> {
    reserve_next(out, maximum)?;
    out.data.push(value);
    Ok(())
}

fn row(
    text: &str, range: &Range<usize>, lines: &mut LineWalker<'_>, path: &Path,
    label: &str, tab_idx: usize, pool: &Pool,
) -> Result<(FindResult, RowCredits), ProjectionError> {
    let (line, start) = lines.seek(range.start);
    let content_end = lines.content_end();
    let shown_end = text.floor_char_boundary(start.saturating_add(SNIPPET_BYTES).min(content_end));
    let snippet = copy_text(text.get(start..shown_end).ok_or(ProjectionError::InvalidChunk)?, pool, SNIPPET_BYTES)?;
    let label = copy_text(label, pool, RESULT_BYTES)?;
    let mut copied_path = Owned::<PathBuf>::new(pool)?;
    copied_path.reserve(path.as_os_str().len())?;
    copied_path.data.push(path);
    if copied_path.data.capacity() != path.as_os_str().len() {
        return Err(ProjectionError::Limit);
    }
    if copied_path.data.as_os_str() != path.as_os_str() {
        return Err(ProjectionError::SourceChanged);
    }
    let (path, path_credit) = copied_path.into_parts();
    let (label, label_credit) = label.into_parts();
    let (line_text, snippet_credit) = snippet.into_parts();
    let local_start = range.start.saturating_sub(start).min(content_end.saturating_sub(start));
    let local_end = range.end.saturating_sub(start).min(content_end.saturating_sub(start)).max(local_start);
    Ok((FindResult {
        path, line, line_text, match_in_line: local_start..local_end, tab_idx: Some(tab_idx),
        display_label: Some(label), identity: Arc::new(()), source: None,
    }, RowCredits { _path: path_credit, _label: label_credit, _snippet: snippet_credit }))
}

/// The projected text is destroyed here on the worker before output notification.
/// Validated map credit moves to its final shared allocation reader.
// Borrowed search inputs and ownership budgets remain explicit at this one
// worker boundary; no document or UI entity is retained by the worker.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    projection: Projection, query: &str, flags: &SearchFlags, scope: Option<&Range<usize>>,
    path: &Path, label: &str, tab_idx: usize, ledger: &Ledger, cancel: &AtomicBool,
) -> Result<Prepared, Failure> {
    if cancel.load(Ordering::Acquire) { return Err(Failure::Cancelled); }
    let pattern = pattern(query, flags, &ledger.pool(QUERY_BYTES))?;
    // These preserve the pinned crate's existing per-NFA and hybrid-cache policy.
    // AST/HIR/compiler/cache internals are not part of the owned-capacity ledger.
    let re = regex::RegexBuilder::new(&pattern.data)
        .case_insensitive(!flags.match_case).multi_line(true)
        .dot_matches_new_line(flags.regex && flags.dot_matches_newline)
        .size_limit(10 * 1024 * 1024).dfa_size_limit(2 * 1024 * 1024)
        .build().map_err(Failure::Regex)?;
    drop(pattern);
    let Projection { text, offsets } = projection;
    let (offsets, credit) = offsets.into_parts();
    let map = Arc::new(ValidatedFindBlockMap::from_projection(&text.data, offsets, Box::new(credit))
        .ok_or(ProjectionError::InvalidChunk)?);
    let ranges_pool = ledger.pool(RANGE_BYTES);
    let results_pool = ledger.pool(RESULT_BYTES);
    // Keep row allocations before their separate credits on every success and
    // failure path, including a refusal halfway through result construction.
    let mut out = Prepared {
        rows: Owned::<Vec<FindResult>>::new(&results_pool)?,
        row_credits: Owned::<Vec<RowCredits>>::new(&results_pool)?,
        matches: Owned::<Vec<Range<usize>>>::new(&ranges_pool)?,
        marks: Owned::<Vec<Range<usize>>>::new(&ranges_pool)?,
        targets: ResolvedTargets::new(&results_pool, Arc::clone(&map))?,
        map,
        endpoints: EndpointResolver::new(&ledger.pool(ENDPOINT_BYTES), MAX_ROWS * 2)?,
        truncated: false,
    };
    let mut lines = LineWalker::new(&text.data);
    let target = scope.cloned().unwrap_or(0..text.data.len());
    for range in crate::find_in_files::scoped_match_ranges(&text.data, &re, target) {
        if cancel.load(Ordering::Acquire) { return Err(Failure::Cancelled); }
        push(&mut out.matches, range.clone(), MAX_MATCHES)?;
        push(&mut out.marks, range.clone(), MAX_MATCHES)?;
        if out.rows.data.len() == MAX_ROWS { out.truncated = true; continue; }
        let index = out.rows.data.len();
        reserve_next(&mut out.rows, MAX_ROWS)?;
        reserve_next(&mut out.row_credits, MAX_ROWS)?;
        let (result, credits) = row(&text.data, &range, &mut lines, path, label, tab_idx, &results_pool)?;
        out.rows.data.push(result);
        out.row_credits.data.push(credits);
        out.endpoints.points.data.push(Endpoint::from_global(&out.map, range.start, index, false)?);
        out.endpoints.points.data.push(Endpoint::from_global(&out.map, range.end, index, true)?);
    }
    out.endpoints.sort();
    out.targets.initialize(&out.rows.data, &out.matches.data)?;
    drop(text);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::find_current_projection::{CaptureCursor, ChunkSlots, ProjectionBuilder, ProjectionSource, RETAINED_BYTES};
    struct Source(Vec<String>);
    impl ProjectionSource for Source {
        fn block_count(&self) -> usize { self.0.len() }
        fn run_count(&self, block: usize) -> Option<usize> { self.0.get(block).map(|_| 1) }
        fn run_text(&self, block: usize, _: usize) -> Option<&str> { self.0.get(block).map(String::as_str) }
    }
    fn fixture(source: &Source, ledger: &Ledger) -> Projection {
        let pool = ledger.pool(RETAINED_BYTES); let slots = ChunkSlots::default();
        let mut cursor = CaptureCursor::default(); let mut builder = ProjectionBuilder::new(&pool).unwrap();
        loop {
            let step = cursor.step(source, slots.acquire().unwrap(), &pool).unwrap();
            let done = step.done; builder.push(step.chunk).unwrap();
            if done { return builder.finish().unwrap(); }
        }
    }
    fn flags(regex: bool) -> SearchFlags {
        SearchFlags { regex, extended: false, whole_word: false, match_case: true, dot_matches_newline: false }
    }
    #[test]
    fn full_coordinates_survive_clipped_snippets_crlf_bom_and_multibyte_text() {
        let source = Source(vec![format!("\u{feff}{}中 hit\r\nhit", "x".repeat(3000)), "hit😀".into()]);
        let ledger = Ledger::default();
        let mut out = prepare(fixture(&source, &ledger), "hit", &flags(false), None,
            Path::new("neutral.docx"), "neutral.docx", 0, &ledger, &AtomicBool::new(false)).ok().unwrap();
        assert_eq!(out.matches.data.len(), 3); assert_eq!(out.marks.data, out.matches.data);
        assert_eq!(out.rows.data[0].line_text.len(), 2044);
        assert_eq!(out.rows.data[0].match_in_line, 3007..3010);
        assert_eq!(out.rows.data[1].line, 1); assert_eq!(out.rows.data[2].line, 2);
        while !out.endpoints.done() { out.endpoints.step(&source).unwrap(); }
        assert_eq!(out.endpoints.points.data[0].position().unwrap().byte, 3007);
        assert!(ledger.used() < RETAINED_BYTES); drop(out); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn selected_search_keeps_global_anchor_context_and_bounded_target_end() {
        let source = Source(vec!["xhit tail".into()]); let ledger = Ledger::default();
        let out = prepare(fixture(&source, &ledger), "^hit", &flags(true), Some(&(1..4)),
            Path::new(""), "new 1", 0, &ledger, &AtomicBool::new(false)).ok().unwrap();
        assert!(out.rows.data.is_empty()); drop(out);
        let out = prepare(fixture(&source, &ledger), "hit.*", &flags(true), Some(&(1..4)),
            Path::new(""), "new 1", 0, &ledger, &AtomicBool::new(false)).ok().unwrap();
        assert_eq!(out.matches.data, vec![1..4]); drop(out); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn capped_rows_keep_both_complete_range_banks_and_cleanup_every_credit() {
        let source = Source(vec!["hit ".repeat(1001)]); let ledger = Ledger::default();
        let out = prepare(fixture(&source, &ledger), "hit", &flags(false), None,
            Path::new(""), "new 1", 0, &ledger, &AtomicBool::new(false)).ok().unwrap();
        assert_eq!(out.rows.data.len(), 1000); assert!(out.truncated);
        assert_eq!(out.matches.data.len(), 1001); assert_eq!(out.marks.data.len(), 1001);
        assert_eq!(out.endpoints.points.data.len(), 2000);
        drop(out); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn native_paths_preserve_exact_bytes_and_capacity_after_push() {
        let source = Source(vec!["hit".into()]); let ledger = Ledger::default();
        let mut paths = vec![PathBuf::new(), PathBuf::from("neutral/中😀é.docx")];
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::ffi::OsStringExt;
            paths.push(PathBuf::from(r"C:\neutral\中😀é.docx"));
            paths.push(PathBuf::from(r"\\?\C:\neutral\document.docx"));
            paths.push(PathBuf::from(std::ffi::OsString::from_wide(&[0x0043, 0x003a, 0x005c, 0xd800, 0x002e, 0x0078])));
        }
        for path in paths {
            let out = prepare(fixture(&source, &ledger), "hit", &flags(false), None,
                &path, "neutral", 0, &ledger, &AtomicBool::new(false)).ok().unwrap();
            assert_eq!(out.rows.data[0].path.as_os_str(), path.as_os_str());
            assert_eq!(out.rows.data[0].path.capacity(), path.as_os_str().len());
            drop(out); assert_eq!(ledger.used(), 0);
        }
    }
    #[test]
    fn result_capacity_refusal_destroys_all_rows_and_releases_shared_map_credit() {
        let source = Source(vec!["hit ".repeat(1001)]); let ledger = Ledger::default();
        let long_path = "p".repeat(4096);
        assert!(matches!(prepare(fixture(&source, &ledger), "hit", &flags(false), None,
            Path::new(&long_path), "neutral", 0, &ledger, &AtomicBool::new(false)),
            Err(Failure::Budget(ProjectionError::Limit))));
        assert_eq!(ledger.used(), 0);
        assert!(ledger.peak() <= super::super::find_current_budget::OWNED_BYTES);
    }
    #[test]
    fn cancellation_and_invalid_regex_refuse_without_partial_output() {
        let source = Source(vec!["hit".into()]); let ledger = Ledger::default();
        assert!(matches!(prepare(fixture(&source, &ledger), "hit", &flags(false), None,
            Path::new(""), "new 1", 0, &ledger, &AtomicBool::new(true)), Err(Failure::Cancelled)));
        assert_eq!(ledger.used(), 0);
        assert!(matches!(prepare(fixture(&source, &ledger), "[", &flags(true), None,
            Path::new(""), "new 1", 0, &ledger, &AtomicBool::new(false)), Err(Failure::Regex(_))));
        assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn actual_pipe_projection_and_rejected_output_release_large_capacities_on_os_worker() {
        measured_pipe(4 * 1024 * 1024, false);
    }
    #[test]
    #[ignore = "optimized phase diagnostic; run explicitly with --release"]
    fn release_rich_find_phase_diagnostic_above_streaming_threshold_and_near_limit() {
        measured_pipe(12 * 1024 * 1024, true);
        measured_pipe(63 * 1024 * 1024, true);
    }
    fn measured_pipe(source_bytes: usize, report: bool) {
        use super::super::{find_current_controller::Controller, find_current_pipe,
            find_current_projection::ProjectionBuilder};
        use std::{thread, time::Duration};
        let (drops, observed) = std::sync::mpsc::channel();
        let ledger = Ledger::observe_drops(drops);
        let pool = ledger.pool(RETAINED_BYTES);
        let metadata = ledger.pool(1024 * 1024);
        let (mut producer, consumer) = find_current_pipe::pipe(&pool, &metadata).unwrap();
        let (actor, wakes) = Controller::<find_current_pipe::Consumer, Result<Prepared, Failure>, ()>::new(&metadata).unwrap();
        let worker_actor = Arc::clone(&actor); let worker_pool = pool.clone(); let worker_ledger = ledger.clone();
        let (finished, completion) = std::sync::mpsc::channel();
        let (timing, measured) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || futures::executor::block_on(worker_actor.run(wakes,
            move |request| {
                let pool = worker_pool.clone(); let ledger = worker_ledger.clone(); let timing = timing.clone();
                async move {
                    let mut consumer = request.value;
                    let mut builder = ProjectionBuilder::new(&pool)?;
                    loop {
                        let step = consumer.next().await.ok_or(Failure::Cancelled)?;
                        let done = step.done; builder.push(step.chunk)?;
                        if done { break; }
                    }
                    let projection = builder.finish()?;
                    let started = std::time::Instant::now();
                    let output = prepare(projection, "hit", &flags(false), None, Path::new("neutral.rtf"),
                        "neutral.rtf", 0, &ledger, &request.cancel);
                    timing.send(started.elapsed()).unwrap(); output
                }
            }, move |id| { finished.send(id).unwrap(); })));
        let id = actor.request(consumer).unwrap();
        let source = Source(vec![format!("{} hit", "x".repeat(source_bytes))]);
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut capture_total = Duration::ZERO; let mut capture_max = Duration::ZERO; let mut steps = 0usize;
        loop {
            let started = std::time::Instant::now();
            let progress = producer.capture(&source).unwrap();
            let elapsed = started.elapsed(); capture_total += elapsed; capture_max = capture_max.max(elapsed);
            steps += usize::from(progress.bytes != 0 || progress.nodes != 0);
            assert!(progress.bytes <= 64 * 1024); assert!(progress.nodes <= 1024);
            if progress.done { break; }
            assert!(std::time::Instant::now() < deadline);
            if progress.blocked { thread::yield_now(); }
        }
        drop(producer);
        assert_eq!(completion.recv_timeout(Duration::from_secs(15)).unwrap(), id);
        let worker_prepare = measured.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(actor.with_output(id, |output| output.as_ref().ok().map(|out| out.rows.data.len())), Some(Some(1)));
        let started = std::time::Instant::now();
        actor.with_output(id, |output| {
            let out = output.as_mut().ok().unwrap();
            while !out.targets.done() { out.targets.step(&mut out.endpoints, &source).unwrap(); }
        }).unwrap();
        let endpoint = started.elapsed();
        if report { eprintln!("rich-phase source={} capture_steps={} capture_calls_ms={:.3} capture_max_ms={:.3} worker_prepare_ms={:.3} endpoint_ms={:.3} tracked_peak={}",
            source_bytes, steps, capture_total.as_secs_f64()*1000., capture_max.as_secs_f64()*1000.,
            worker_prepare.as_secs_f64()*1000., endpoint.as_secs_f64()*1000., ledger.peak()); }
        // Projected text is already freed before publication is offered.
        let first: Vec<_> = observed.try_iter().collect();
        assert!(first.iter().any(|(bytes, _)| *bytes >= 4 * 1024 * 1024));
        assert!(first.iter().filter(|(bytes, _)| *bytes > 64 * 1024).all(|(_, owner)| *owner == worker.thread().id()));
        let worker_id = worker.thread().id();
        actor.reject(id); actor.close(); worker.join().unwrap();
        let second: Vec<_> = observed.try_iter().collect();
        assert!(second.iter().any(|(bytes, _)| *bytes > 64 * 1024));
        assert!(second.iter().filter(|(bytes, _)| *bytes > 64 * 1024).all(|(_, owner)| *owner == worker_id));
        drop(actor); assert_eq!(ledger.used(), 0);
    }

}
