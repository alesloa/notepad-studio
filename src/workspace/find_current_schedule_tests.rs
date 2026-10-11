use super::*;
use crate::{rich_doc::ast::{Block, BlockProps, InlineRun}, settings::Settings, tab::TabViewMode};
use gpui::{prelude::*, AppContext, Entity};
use std::{sync::{Arc, Mutex}, time::{Duration, Instant}};

const SAMPLE_LIMIT: usize = 4096;
#[derive(Clone, Default)]
pub(super) struct Sample {
    pub wall: Duration,
    pub guard: Duration,
    pub bytes: usize,
    pub nodes: usize,
    pub attempts: usize,
    pub capture: bool,
    pub tick: bool,
}
struct Data {
    current: Sample,
    samples: Vec<Sample>,
    overflow: usize,
    projection: Duration,
    prepare: Duration,
}
pub(super) struct Trace(Mutex<Data>);
impl Trace {
    fn new() -> Self {
        Self(Mutex::new(Data { current: Sample::default(), samples: Vec::with_capacity(SAMPLE_LIMIT),
            overflow: 0, projection: Duration::ZERO, prepare: Duration::ZERO }))
    }
    pub(super) fn begin(&self) { self.0.lock().unwrap().current = Sample::default(); }
    pub(super) fn guard(&self, elapsed: Duration) { self.0.lock().unwrap().current.guard += elapsed; }
    pub(super) fn capture(&self, bytes: usize, nodes: usize, attempts: usize) {
        let mut data = self.0.lock().unwrap();
        data.current.bytes += bytes; data.current.nodes += nodes; data.current.attempts += attempts;
        data.current.capture = true;
    }
    pub(super) fn finish(&self, started: Instant, tick: bool) {
        let mut data = self.0.lock().unwrap();
        let mut sample = data.current.clone(); sample.wall = started.elapsed(); sample.tick = tick;
        if data.samples.len() < SAMPLE_LIMIT { data.samples.push(sample); } else { data.overflow += 1; }
    }
    pub(super) fn projection(&self, elapsed: Duration) { self.0.lock().unwrap().projection += elapsed; }
    pub(super) fn prepare(&self, elapsed: Duration) { self.0.lock().unwrap().prepare += elapsed; }
}
struct NeutralWindow { workspace: Entity<Workspace> }
impl Render for NeutralWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child("Capture timing fixture")
    }
}
#[derive(Clone, Copy)]
struct Case { mib: usize, empty: usize, short: usize, long_query: bool, extra_tabs: usize }
struct Summary {
    turns: usize, handler: Duration, samples: Vec<Sample>, projection: Duration, prepare: Duration,
    logical_bytes: usize, peak: usize,
}
fn run_case(case: Case) -> Summary { run_case_with_elapsed(case, None) }
fn run_case_with_elapsed(case: Case, controlled_elapsed: Option<Duration>) -> Summary {
    println!("scheduler_fixture_manifest={}", env!("CARGO_MANIFEST_DIR"));
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None, ..Default::default()
    });
    let trace = Arc::new(Trace::new());
    let logical_bytes = case.mib * 1024 * 1024 + case.short * 2 + 4;
    let mut cx = gpui::TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (neutral, visual) = cx.add_window_view(|_, cx| NeutralWindow { workspace: cx.new(|cx| {
        let mut workspace = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut payload = crate::rich_doc::rtf::open_bytes(b"{\\rtf1\\ansi seed}".to_vec()).unwrap();
        let mut runs = Vec::with_capacity(case.empty + case.short + 1);
        runs.extend((0..case.empty).map(|_| InlineRun::plain("")));
        runs.extend((0..case.short).map(|_| InlineRun::plain("é")));
        runs.push(InlineRun::plain(format!("{} hit", "x".repeat(case.mib * 1024 * 1024))));
        payload.ast.blocks = vec![Block::Paragraph { runs, props: BlockProps::default() }];
        let start = payload.block_start(0);
        payload.selection = Some(crate::rich_doc::TextSelection { anchor: start, cursor: start });
        payload.active_block = Some(0);
        let mut tab = Tab::empty(); tab.buffer.rich_doc = Some(payload);
        tab.buffer.rope = ropey::Rope::from_str("stale raw representation");
        tab.view_mode = TabViewMode::RichDocView;
        workspace.tabs = vec![tab]; workspace.tabs.extend((0..case.extra_tabs).map(|_| Tab::empty()));
        workspace.active = 0;
        workspace.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        workspace.session_final_save_result = Some(true);
        workspace.rich_find_current.trace = Some(Arc::clone(&trace));
        workspace.rich_find_current.capture_elapsed_override = controlled_elapsed;
        workspace.find_state.query = if case.long_query { format!("{}|hit", "a".repeat(65532)) } else { "hit".into() };
        workspace.find_state.regex = case.long_query;
        workspace.find_state.open = true; workspace.dialog_tab = DialogTab::Find;
        workspace
    }) });
    let entity = visual.update(|_, cx| neutral.read(cx).workspace.clone());
    visual.run_until_parked();
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| crate::ui::find_dialog::FindDialog::new(weak, window, cx));
    let mut dialog = dialog.clone();
    dialog.update(|window, cx| { window.activate_window(); window.focus(&entity.read(cx).find_focus.clone(), cx); });
    dialog.run_until_parked();
    let button = dialog.debug_bounds("dialog-button-Find All in Current Document").unwrap();
    let started = Instant::now(); dialog.simulate_click(button.center(), gpui::Modifiers::none());
    let handler = started.elapsed();
    visual.run_until_parked();
    visual.update(|_, cx| {
        let workspace = entity.read(cx);
        assert_eq!(workspace.rich_find_current.first_scheduled_capture_bytes, None);
        assert_eq!(workspace.rich_find_current.capture_elapsed_override, controlled_elapsed);
        assert!(workspace.find_in_files.searching);
    });
    let mut turns = 0;
    while turns < 2400 {
        visual.cx.executor().advance_clock(Duration::from_millis(16)); visual.run_until_parked(); turns += 1;
        if visual.update(|_, cx| !entity.read(cx).find_in_files.searching) { break; }
    }
    let peak = visual.update(|_, cx| {
        let workspace = entity.read(cx);
        assert!(!workspace.find_in_files.searching, "bounded fixture must finish");
        assert_eq!(workspace.find_in_files.results.len(), 1);
        assert_eq!(workspace.find_state.matches, vec![logical_bytes - 3..logical_bytes]);
        assert_eq!(workspace.find_in_files.last_search_query, workspace.find_state.query);
        assert!(workspace.find_state.rich_doc_block_offsets.is_some());
        assert!(workspace.rich_find_current.owned_peak() <= 192 * 1024 * 1024);
        workspace.rich_find_current.owned_peak()
    });
    let (samples, projection, prepare) = {
        let data = trace.0.lock().unwrap(); assert_eq!(data.overflow, 0);
        (data.samples.clone(), data.projection, data.prepare)
    };
    for sample in &samples {
        assert!(sample.nodes <= 1024); assert!(sample.bytes <= 1024 * 1024); assert!(sample.attempts <= 16);
    }
    visual.update(|_, cx| entity.update(cx, |workspace, _| workspace.release_session_window()));
    visual.run_until_parked(); cx.quit();
    Summary { turns, handler, samples, projection, prepare, logical_bytes, peak }
}
#[test]
fn actual_sparse_capture_finishes_within_sixty_four_scheduled_turns() {
    let result = run_case_with_elapsed(Case { mib: 12, empty: 0, short: 0, long_query: false, extra_tabs: 0 }, Some(Duration::ZERO));
    assert!(result.turns <= 64, "actual scheduled turns={}, expected<=64", result.turns);
}
#[test]
fn actual_dense_empty_and_short_runs_preserve_aggregate_node_budget() {
    for (empty, short) in [(100_000, 0), (0, 100_000)] {
        let result = run_case_with_elapsed(Case { mib: 12, empty, short, long_query: false, extra_tabs: 0 }, Some(Duration::ZERO));
        assert!(result.samples.iter().any(|sample| sample.nodes == 1024));
        assert!(result.samples.iter().filter(|sample| sample.capture).count() >= 98);
    }
}
#[test]
fn actual_full_query_budget_and_many_tabs_preserve_accepted_output() {
    let result = run_case_with_elapsed(Case { mib: 12, empty: 0, short: 0, long_query: true, extra_tabs: 128 }, Some(Duration::from_millis(3)));
    assert!(result.samples.iter().filter(|sample| sample.capture).all(|sample| sample.attempts == 1));
}
fn micros(duration: Duration) -> u128 { duration.as_micros() }
#[test]
#[ignore = "matched warmed optimized callback diagnostic; virtual turns are not native latency"]
fn optimized_actual_capture_scheduler_callback_diagnostic() {
    let cases = [
        Case { mib: 12, empty: 0, short: 0, long_query: false, extra_tabs: 0 },
        Case { mib: 63, empty: 0, short: 0, long_query: false, extra_tabs: 0 },
        Case { mib: 12, empty: 100_000, short: 0, long_query: false, extra_tabs: 0 },
        Case { mib: 12, empty: 0, short: 100_000, long_query: false, extra_tabs: 0 },
        Case { mib: 12, empty: 0, short: 0, long_query: true, extra_tabs: 128 },
    ];
    for (case_index, case) in cases.into_iter().enumerate() {
        let _warm = run_case(case);
        for sample_index in 0..5 {
            let summary = run_case(case);
            let mut walls: Vec<_> = summary.samples.iter().map(|sample| sample.wall).collect(); walls.sort_unstable();
            let total: Duration = summary.samples.iter().map(|sample| sample.wall).sum();
            let guard: Duration = summary.samples.iter().map(|sample| sample.guard).sum();
            let captures = summary.samples.iter().filter(|sample| sample.capture && sample.tick).count();
            let overshoot = summary.samples.iter().filter(|sample| sample.wall > Duration::from_millis(2)).count();
            println!("case={case_index} sample={sample_index} bytes={} virtual_turns={} capture_ticks={} derived_capture_pacing_ms={} action_span_us={} entity_update_sum_us={} entity_update_max_us={} entity_update_p95_us={} guard_included_sum_us={} callbacks_over_2ms={} projection_sync_us={} prepare_sync_us={} tracked_peak={}",
                summary.logical_bytes, summary.turns, captures, captures * 16, micros(summary.handler), micros(total),
                micros(*walls.last().unwrap()), micros(walls[walls.len() * 95 / 100]), micros(guard), overshoot,
                micros(summary.projection), micros(summary.prepare), summary.peak);
        }
    }
}
