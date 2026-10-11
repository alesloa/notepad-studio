// Actual row/footer clicks and intercepted capture, followed by the production
// binder and Workspace handlers. Tests keep overrides in memory; Save is not called.
use super::*;
use crate::{settings::Settings, workspace::Workspace};
use gpui::{TestAppContext, VisualTestContext, WindowAppearance};

struct MapperWindowGuard(Option<WindowHandle<ShortcutMapper>>);

impl Drop for MapperWindowGuard {
    fn drop(&mut self) {
        OPEN_WINDOW.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

fn mapper(cx: &mut TestAppContext) -> (Entity<ShortcutMapper>, VisualTestContext, MapperWindowGuard) {
    let (entity, visual) = cx.add_window_view(|_, cx| ShortcutMapper {
        focus: cx.focus_handle(),
        overrides: Overrides::new(),
        selected: None,
        capturing: None,
        status: None,
        is_error: false,
        pending_conflict: None,
        theme_mode: ThemeMode::Dark,
        _capture_interceptor: None,
    });
    let mut visual = visual.clone();
    let handle = visual.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).focus.clone(), cx);
        window.window_handle().downcast::<ShortcutMapper>().unwrap()
    });
    let guard = MapperWindowGuard(OPEN_WINDOW.with(|slot| slot.replace(Some(handle))));
    // Show the actual catalog in full on the test platform, without synthetic
    // row selection or desktop-size constraints.
    visual.simulate_resize(size(px(760.), px(catalog().len() as f32 * 24. + 160.)));
    visual.run_until_parked();
    (entity, visual, guard)
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    let bounds = visual.debug_bounds(selector).expect(selector);
    visual.simulate_click(bounds.center(), gpui::Modifiers::none());
    visual.run_until_parked();
}

fn capture(
    entity: &Entity<ShortcutMapper>, visual: &mut VisualTestContext,
    id: &str, selector: &'static str, chord: &str,
) -> Overrides {
    let index = catalog().iter().position(|entry| entry.id == id)
        .unwrap_or_else(|| panic!("{id} is absent from the actual Shortcut Mapper catalog"));
    click(visual, selector);
    visual.update(|_, cx| assert_eq!(entity.read(cx).selected, Some(index)));
    click(visual, "sm_button::Modify");
    visual.update(|_, cx| assert_eq!(entity.read(cx).capturing, Some(index)));
    visual.simulate_keystrokes(chord);
    visual.run_until_parked();
    visual.update(|_, cx| {
        let mapper = entity.read(cx);
        assert!(mapper.capturing.is_none());
        assert!(mapper.pending_conflict.is_none(), "{id}: unexpected conflict");
        assert_eq!(mapper.overrides.get(id), Some(&vec![chord.to_string()]));
        mapper.overrides.clone()
    })
}

fn workspace(cx: &mut TestAppContext, overrides: &Overrides) -> (Entity<Workspace>, VisualTestContext) {
    cx.update(|cx| shortcuts::reload_keymap(cx, overrides));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut workspace = Workspace::new(
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            1., WindowAppearance::Dark,
        );
        workspace.active_tab_mut().buffer.rope = ropey::Rope::from_str("café 😀\n中\n");
        workspace.active_tab_mut().buffer.encoding = crate::text::encoding::Encoding::Utf8;
        workspace.active_tab_mut().buffer.line_ending = crate::text::line_ending::LineEnding::Lf;
        workspace.window_closed = true;
        workspace
    });
    let mut visual = visual.clone();
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    (entity, visual)
}

#[test]
fn mapper_captured_menu_shortcuts_dispatch_to_actual_workspace_handlers() {
    for (id, selector) in [
        ("ViewWordWrap", "row::ViewWordWrap"),
        ("EditToggleReadOnly", "row::EditToggleReadOnly"),
        ("EncodingUtf8Bom", "row::EncodingUtf8Bom"),
        ("EditEolWindows", "row::EditEolWindows"),
    ] {
        let _settings = crate::settings::override_for_test(Settings {
            toolbar_hidden: true, tab_bar_hidden: true, folder_panel_visible: false,
            ..Settings::default()
        });
        let _recent = crate::recent::override_snapshot_for_test(Vec::new());
        let _macros = crate::macros::override_playback_for_test(Vec::new(), Default::default());
        let mut cx = TestAppContext::single();
        let (mapper, mut mapper_visual, _window) = mapper(&mut cx);
        let overrides = capture(&mapper, &mut mapper_visual, id, selector, "ctrl-alt-shift-f9");
        let (workspace, mut visual) = workspace(&mut cx, &overrides);
        visual.simulate_keystrokes("ctrl-alt-shift-f9");
        visual.update(|_, cx| {
            let workspace = workspace.read(cx);
            assert_eq!(workspace.buffer().rope.to_string(), "café 😀\n中\n");
            match id {
                "ViewWordWrap" => assert!(workspace.word_wrap),
                "EditToggleReadOnly" => assert!(workspace.buffer().read_only),
                "EncodingUtf8Bom" => assert_eq!(workspace.buffer().encoding, crate::text::encoding::Encoding::Utf8Bom),
                "EditEolWindows" => assert_eq!(workspace.buffer().line_ending, crate::text::line_ending::LineEnding::Crlf),
                _ => unreachable!(),
            }
        });
        cx.quit();
    }
}

#[test]
fn mapper_capture_preserves_existing_bindings_on_cancel_and_reset() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut cx = TestAppContext::single();
    let (mapper, mut visual, _window) = mapper(&mut cx);
    let index = catalog().iter().position(|entry| entry.id == "ViewWordWrap").expect("Word Wrap row");
    click(&mut visual, "row::ViewWordWrap");
    click(&mut visual, "sm_button::Modify");
    visual.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-z" } else { "ctrl-z" });
    visual.update(|_, cx| {
        let mapper = mapper.read(cx);
        let conflict = mapper.pending_conflict.as_ref().expect("existing Undo default conflict");
        assert_eq!(conflict.row, index);
        assert_eq!(conflict.owner_id, "EditUndo");
        assert!(mapper.overrides.is_empty());
    });
    click(&mut visual, "sm_button::Cancel");
    capture(&mapper, &mut visual, "ViewWordWrap", "row::ViewWordWrap", "ctrl-alt-shift-f9");
    click(&mut visual, "sm_button::Clear");
    visual.update(|_, cx| assert_eq!(mapper.read(cx).overrides.get("ViewWordWrap"), Some(&Vec::new())));
    click(&mut visual, "sm_button::Reset Row");
    visual.update(|_, cx| {
        assert!(mapper.read(cx).overrides.is_empty());
        let undo = catalog().iter().find(|entry| entry.id == "EditUndo").unwrap();
        assert_eq!(shortcuts::effective_keys(undo, &mapper.read(cx).overrides), shortcuts::platform_default_keys(undo));
        assert!(shortcuts::effective_keys(&catalog()[index], &mapper.read(cx).overrides).is_empty());
    });
    cx.quit();
}

#[test]
fn a_captured_global_menu_shortcut_dispatches_without_workspace_context() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut cx = TestAppContext::single();
    let (mapper, mut visual, _window) = mapper(&mut cx);
    let overrides = capture(&mapper, &mut visual, "OpenShortcutMapper", "row::OpenShortcutMapper", "ctrl-alt-shift-f9");
    let count = Rc::new(std::cell::Cell::new(0));
    let observed = count.clone();
    cx.update(|cx| {
        // Observe dispatch without opening another real mapper or reading
        // shortcuts.toml through its production window constructor.
        cx.on_action(move |_: &crate::actions::OpenShortcutMapper, _| observed.set(observed.get() + 1));
        shortcuts::reload_keymap(cx, &overrides);
    });
    visual.simulate_keystrokes("ctrl-alt-shift-f9");
    assert_eq!(count.get(), 1);
    cx.quit();
}
