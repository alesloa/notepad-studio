// PowerEditor/src/WinControls/TabBar/TabBar.cpp:771 — overflowing single-line
// tabs scroll one tab at a time. TabBar.cpp:1190 — middle-click closes the
// tab under the cursor.
use super::*;
use crate::{
    persist::TempDir,
    settings::{Settings, override_for_test},
    tab::Tab,
};
use gpui::{Bounds, Modifiers, TestAppContext, VisualTestContext, WindowAppearance, size};

fn preferences() -> Settings {
    Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        ..Settings::default()
    }
}

fn fixture<'a>(
    cx: &'a mut TestAppContext,
    directory: &TempDir,
    count: usize,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            1.,
            WindowAppearance::Dark,
        );
        w.tabs = (0..count)
            .map(|i| {
                let mut tab = Tab::empty();
                tab.buffer.file_path = Some(directory.join(&format!("neutral-{i}.txt")));
                tab.buffer.rope = ropey::Rope::from_str(&format!("source-{i}"));
                tab
            })
            .collect();
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w
    });
    visual.simulate_resize(size(px(640.), px(400.)));
    visual.run_until_parked();
    (workspace, visual)
}

fn bounds(visual: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} was not painted"))
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    let at = bounds(visual, selector).center();
    visual.simulate_click(at, Modifiers::none());
    visual.run_until_parked();
}

fn middle_click(visual: &mut VisualTestContext, selector: &'static str) {
    let at = bounds(visual, selector).center();
    visual.simulate_mouse_down(at, MouseButton::Middle, Modifiers::none());
    visual.simulate_mouse_up(at, MouseButton::Middle, Modifiers::none());
    visual.run_until_parked();
}

fn scrolled(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> (Pixels, Pixels) {
    visual.update(|_, cx| {
        let scroll = &workspace.read(cx).tab_strip_scroll;
        (-scroll.offset().x, scroll.max_offset().width)
    })
}

fn paths(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Vec<String> {
    visual.update(|_, cx| {
        workspace
            .read(cx)
            .tabs
            .iter()
            .filter_map(|tab| tab.buffer.file_path.as_ref())
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect()
    })
}

fn near(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() < px(1.)
}

#[test]
fn step_target_moves_one_tab_edge_and_stays_in_range() {
    let starts = [px(0.), px(100.), px(220.), px(300.)];
    let max = px(250.);
    assert_eq!(step_target(&starts, px(0.), max, true), px(100.));
    assert_eq!(step_target(&starts, px(100.), max, true), px(220.));
    // The last tab start lies past the end of the scroll range.
    assert_eq!(step_target(&starts, px(220.), max, true), max);
    assert_eq!(step_target(&starts, max, max, true), max);
    assert_eq!(step_target(&starts, max, max, false), px(220.));
    // A strip scrolled part-way into a tab steps back to that tab's start.
    assert_eq!(step_target(&starts, px(150.), max, false), px(100.));
    assert_eq!(step_target(&starts, px(100.), max, false), px(0.));
    assert_eq!(step_target(&starts, px(0.), max, false), px(0.));
    // Rounding on a tab edge does not skip the next tab.
    assert_eq!(step_target(&starts, px(99.8), max, true), px(220.));
    assert_eq!(step_target(&[], px(0.), px(0.), true), px(0.));
}

#[test]
fn scroll_state_marks_each_open_end() {
    let none = TabScroll { overflow: false, back: false, forward: false };
    assert_eq!(TabScroll::from_offsets(px(0.), px(0.)), none);
    assert_eq!(
        TabScroll::from_offsets(px(0.), px(90.)),
        TabScroll { overflow: true, back: false, forward: true }
    );
    assert_eq!(
        TabScroll::from_offsets(px(40.), px(90.)),
        TabScroll { overflow: true, back: true, forward: true }
    );
    assert_eq!(
        TabScroll::from_offsets(px(90.), px(90.)),
        TabScroll { overflow: true, back: true, forward: false }
    );
}

#[test]
fn arrows_appear_only_while_single_line_tabs_overflow() {
    let _settings = override_for_test(preferences());
    let directory = TempDir::new("tab-strip-fit");
    let mut cx = TestAppContext::single();
    let (_, visual) = fixture(&mut cx, &directory, 3);
    assert!(visual.debug_bounds("tab-scroll-forward").is_none());
    assert!(visual.debug_bounds("tab-scroll-back").is_none());
    drop(cx);

    let _settings = override_for_test(Settings { tab_multi_line: true, ..preferences() });
    let mut cx = TestAppContext::single();
    let (_, visual) = fixture(&mut cx, &directory, 14);
    assert!(visual.debug_bounds("tab-scroll-forward").is_none());
    assert!(visual.debug_bounds("tab-13").is_some());
}

#[test]
fn arrows_step_one_tab_at_a_time_and_stop_at_each_end() {
    let _settings = override_for_test(preferences());
    let directory = TempDir::new("tab-strip-arrows");
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, &directory, 14);
    let first = bounds(visual, "tab-0");
    let second = bounds(visual, "tab-1");
    let forward = bounds(visual, "tab-scroll-forward");
    let back = bounds(visual, "tab-scroll-back");
    // Arrows sit at the right end, after the tabs and inside the window.
    assert!(back.right() <= forward.left() + px(0.5));
    assert!(forward.right() <= px(640.5));
    let (_, max) = scrolled(&workspace, visual);
    assert!(max > px(0.), "14 tabs should overflow a 640px window");

    click(visual, "tab-scroll-back");
    assert!(near(bounds(visual, "tab-0").left(), first.left()));

    click(visual, "tab-scroll-forward");
    assert!(near(bounds(visual, "tab-1").left(), first.left()));
    click(visual, "tab-scroll-forward");
    assert!(near(bounds(visual, "tab-2").left(), first.left()));
    click(visual, "tab-scroll-back");
    assert!(near(bounds(visual, "tab-1").left(), first.left()));
    assert!(near(bounds(visual, "tab-1").left() - bounds(visual, "tab-0").left(),
        second.left() - first.left()));

    for _ in 0..20 {
        click(visual, "tab-scroll-forward");
    }
    let (at, max) = scrolled(&workspace, visual);
    assert!(near(at, max));
    // The last tab ends where the visible strip ends.
    let strip = visual.update(|_, cx| workspace.read(cx).tab_strip_scroll.bounds());
    assert!(near(bounds(visual, "tab-13").right(), strip.right()));

    for _ in 0..20 {
        click(visual, "tab-scroll-back");
    }
    assert!(near(bounds(visual, "tab-0").left(), first.left()));
    assert_eq!(paths(&workspace, visual).len(), 14, "arrows never close tabs");
}

#[test]
fn newly_active_tab_scrolls_into_view_and_arrows_follow() {
    let _settings = override_for_test(preferences());
    let directory = TempDir::new("tab-strip-reveal");
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, &directory, 14);
    let first = bounds(visual, "tab-0");
    visual.update(|_, cx| workspace.update(cx, |w, cx| w.set_active_tab(13, cx)));
    visual.run_until_parked();
    let strip = visual.update(|_, cx| workspace.read(cx).tab_strip_scroll.bounds());
    let last = bounds(visual, "tab-13");
    assert!(last.left() >= strip.left() - px(0.5) && last.right() <= strip.right() + px(0.5));

    // The back arrow was drawn disabled before the reveal; it must work now.
    click(visual, "tab-scroll-back");
    let (at, max) = scrolled(&workspace, visual);
    assert!(at < max, "back arrow stayed disabled after the reveal");
    for _ in 0..20 {
        click(visual, "tab-scroll-back");
    }
    assert!(near(bounds(visual, "tab-0").left(), first.left()));
    // Arrow scrolling alone does not pull the active tab back into view.
    let (at, _) = scrolled(&workspace, visual);
    assert!(near(at, px(0.)));
}

#[test]
fn middle_click_closes_the_tab_under_the_cursor() {
    let _settings = override_for_test(preferences());
    let directory = TempDir::new("tab-strip-middle");
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, &directory, 3);
    middle_click(visual, "tab-1");
    assert_eq!(paths(&workspace, visual), ["neutral-0.txt", "neutral-2.txt"]);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).active), 0);

    // Left and right clicks still do not close.
    click(visual, "tab-1");
    assert_eq!(paths(&workspace, visual).len(), 2);
}

#[test]
fn middle_click_closes_a_scrolled_tab_and_a_vertical_tab() {
    let _settings = override_for_test(preferences());
    let directory = TempDir::new("tab-strip-middle-scrolled");
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, &directory, 14);
    for _ in 0..20 {
        click(visual, "tab-scroll-forward");
    }
    middle_click(visual, "tab-12");
    let names = paths(&workspace, visual);
    assert_eq!(names.len(), 13);
    assert!(!names.iter().any(|name| name == "neutral-12.txt"));
    drop(cx);

    let _settings = override_for_test(Settings { tab_vertical: true, ..preferences() });
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, &directory, 3);
    middle_click(visual, "vtab-2");
    assert_eq!(paths(&workspace, visual), ["neutral-0.txt", "neutral-1.txt"]);
}
