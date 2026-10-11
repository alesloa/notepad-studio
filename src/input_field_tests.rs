// Exercise the rendered Workspace action path, not direct edit handlers.
use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

use crate::{
    find_state::{
        MacroPromptMode, MacroPromptState, RichDocColorBarState, RichDocLinkBarState,
        TabRenameState,
    },
    settings::Settings,
    tab::InputFocus,
    workspace::Workspace,
};

const SOURCE: &str = "first\nsecond\nthird";
const FIELD: &str = "café中";

fn preferences() -> Settings {
    Settings {
        toolbar_hidden: true,
        tab_bar_hidden: true,
        folder_panel_visible: false,
        ..Settings::default()
    }
}

fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
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
        w.active_tab_mut().buffer.rope = ropey::Rope::from_str(SOURCE);
        w.window_closed = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    (workspace, visual)
}

fn open_field(
    w: &mut Workspace,
    target: InputFocus,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) {
    match target {
        InputFocus::Goto => w.goto_state.open = true,
        InputFocus::MacroPrompt => {
            w.macro_focus = Some(cx.focus_handle());
            w.macro_prompt = Some(MacroPromptState::new(MacroPromptMode::RunCount, FIELD));
        }
        InputFocus::TabRename => {
            w.tab_rename_focus = Some(cx.focus_handle());
            w.tab_rename = Some(TabRenameState::new(0, FIELD));
        }
        InputFocus::RichDocLinkUrl => {
            w.rich_doc_link_focus = Some(cx.focus_handle());
            w.rich_doc_link_bar = Some(RichDocLinkBarState::new(0, FIELD, false));
        }
        InputFocus::RichDocColorHex => {
            w.rich_doc_color_focus = Some(cx.focus_handle());
            w.rich_doc_color_bar = Some(RichDocColorBarState::new(0, FIELD, None));
        }
        _ => panic!("unexpected fixture field"),
    }
    let (text, cursor, anchor) = w.input_field_mut(target).unwrap();
    *text = FIELD.to_string();
    *cursor = FIELD.len();
    *anchor = FIELD.len();
    w.focus_input(target, window, cx);
    cx.notify();
}

#[test]
fn inline_fields_route_select_cut_paste_and_escape_without_editing_the_document() {
    let _settings = crate::settings::override_for_test(preferences());
    for (index, target) in [
        InputFocus::Goto,
        InputFocus::MacroPrompt,
        InputFocus::MacroPrompt,
        InputFocus::TabRename,
        InputFocus::RichDocLinkUrl,
        InputFocus::RichDocColorHex,
    ]
    .into_iter()
    .enumerate()
    {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.update_window_entity(&workspace, |w, window, cx| {
            open_field(w, target, window, cx);
            if index == 2 {
                w.macro_prompt.as_mut().unwrap().mode = MacroPromptMode::SaveName;
            }
        });
        visual.run_until_parked();
        visual.simulate_keystrokes("ctrl-a");
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(w.input_focus(window) == target, "field {index} owns focus");
            assert_eq!(
                w.input_selection_text(window).as_deref(),
                Some(FIELD),
                "field {index} receives Ctrl+A"
            );
            assert!(
                w.selection().is_empty(),
                "Ctrl+A leaves document selection alone"
            );
        });
        visual.simulate_keystrokes("ctrl-c ctrl-x");
        visual.update_window_entity(&workspace, |w, _, _| {
            assert_eq!(
                w.input_field_mut(target).unwrap().0,
                "",
                "field {index} receives Cut"
            );
            assert_eq!(w.buffer().rope.to_string(), SOURCE);
        });
        visual.simulate_keystrokes("ctrl-v");
        visual.update_window_entity(&workspace, |w, _, _| {
            assert_eq!(
                w.input_field_mut(target).unwrap().0,
                FIELD,
                "field {index} receives Paste"
            );
            assert_eq!(w.buffer().rope.to_string(), SOURCE);
        });
        visual.simulate_keystrokes("escape");
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(
                w.focus_handle.is_focused(window),
                "field {index} closes and returns focus"
            );
            assert_eq!(w.buffer().rope.to_string(), SOURCE);
        });
        cx.quit();
    }
}

#[test]
fn goto_enter_returns_keyboard_editing_to_the_editor() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx);
    visual.simulate_keystrokes("ctrl-g");
    visual.simulate_input("2");
    visual.simulate_keystrokes("enter");
    visual.update(|window, cx| {
        let w = workspace.read(cx);
        assert!(
            !w.goto_state.open,
            "Enter commits the visible Go to Line field"
        );
        assert_eq!(w.selection().head, 6);
        assert!(
            w.focus_handle.is_focused(window),
            "closed field returns editor focus"
        );
    });
    visual.simulate_input("中");
    visual.update(|_, cx| {
        assert_eq!(
            workspace.read(cx).buffer().rope.to_string(),
            "first\n中second\nthird"
        )
    });
    cx.quit();
}

#[test]
fn multiline_field_text_keeps_its_value_and_shapes_without_a_panic() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx);
    visual.simulate_keystrokes("ctrl-g");
    visual.simulate_input("café\r\n中\nend\r");
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-a");
    visual.update(|window, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.goto_state.text, "café\r\n中\nend\r");
        assert_eq!(
            w.input_selection_text(window).as_deref(),
            Some("café\r\n中\nend\r")
        );
        assert_eq!(w.buffer().rope.to_string(), SOURCE);
    });
    assert_eq!(
        super::single_line_display("café\r\n中\nend\r").as_ref(),
        "café  中 end "
    );
    assert_eq!(
        super::single_line_display("café\r\n中\nend\r").len(),
        "café\r\n中\nend\r".len()
    );
    let bounds = visual.debug_bounds("goto-input").unwrap();
    visual.simulate_click(bounds.center(), gpui::Modifiers::none());
    visual.update(|_, cx| assert_eq!(workspace.read(cx).goto_state.text, "café\r\n中\nend\r"));
    cx.quit();
}

#[test]
fn clicking_inline_input_padding_keeps_its_keyboard_focus() {
    let _settings = crate::settings::override_for_test(preferences());
    for (target, selector) in [
        (InputFocus::Goto, "goto-input"),
        (InputFocus::MacroPrompt, "macro-input"),
        (InputFocus::TabRename, "tab-rename-input"),
        (InputFocus::RichDocLinkUrl, "rich-link-input"),
        (InputFocus::RichDocColorHex, "rich-color-input"),
    ] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.update_window_entity(&workspace, |w, window, cx| {
            open_field(w, target, window, cx);
            window.focus(&w.focus_handle, cx);
        });
        visual.run_until_parked();
        let bounds = visual.debug_bounds(selector).unwrap();
        for position in [
            gpui::point(bounds.left() + gpui::px(1.), bounds.center().y),
            bounds.center(),
        ] {
            visual.update_window_entity(&workspace, |w, window, cx| {
                window.focus(&w.focus_handle, cx)
            });
            visual.simulate_click(position, gpui::Modifiers::none());
            visual.update(|window, cx| {
                let w = workspace.read(cx);
                assert!(
                    w.input_focus(window) == target,
                    "{selector} keeps focus after clicking {position:?}"
                );
                assert_eq!(w.buffer().rope.to_string(), SOURCE);
            });
            visual.simulate_keystrokes("ctrl-a");
            visual.update(|window, cx| {
                assert_eq!(
                    workspace.read(cx).input_selection_text(window).as_deref(),
                    Some(FIELD)
                )
            });
        }
        cx.quit();
    }
}

#[test]
fn invalid_goto_values_keep_focus_and_show_an_actionable_error() {
    let _settings = crate::settings::override_for_test(preferences());
    for text in [
        "0".to_string(),
        "-1".to_string(),
        "garbage".to_string(),
        format!("{}0", usize::MAX),
        "café\r\n2".to_string(),
    ] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.simulate_keystrokes("ctrl-g");
        visual.simulate_input(&text);
        visual.simulate_keystrokes("enter");
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(w.goto_state.open);
            assert!(w.goto_focus.is_focused(window));
            assert_eq!(w.goto_state.text, text);
            assert_eq!(
                w.status_message.as_deref(),
                Some("Go to Line needs a positive line number")
            );
            assert_eq!(w.selection().head, 0);
            assert_eq!(w.buffer().rope.to_string(), SOURCE);
        });
        cx.quit();
    }
}

#[test]
fn enter_reaches_the_visible_inline_prompt_handler() {
    let _settings = crate::settings::override_for_test(preferences());
    for target in [
        InputFocus::MacroPrompt,
        InputFocus::TabRename,
        InputFocus::RichDocLinkUrl,
        InputFocus::RichDocColorHex,
    ] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.update_window_entity(&workspace, |w, window, cx| {
            if matches!(
                target,
                InputFocus::RichDocLinkUrl | InputFocus::RichDocColorHex
            ) {
                w.active_tab_mut().buffer.rich_doc = Some(
                    crate::rich_doc::rtf::open_bytes(b"{\\rtf1\\ansi alpha}".to_vec()).unwrap(),
                );
            }
            open_field(w, target, window, cx);
        });
        visual.run_until_parked();
        visual.simulate_keystrokes("ctrl-a");
        visual.simulate_input(match target {
            InputFocus::MacroPrompt => "0", // Validate without replay or saving.
            InputFocus::RichDocLinkUrl => "https://example.com",
            InputFocus::RichDocColorHex => "0066CC",
            _ => "draft.txt", // Untitled fixture cannot rename a real file.
        });
        visual.simulate_keystrokes("enter");
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(w.focus_handle.is_focused(window));
            assert!(
                w.macro_prompt.is_none()
                    && w.tab_rename.is_none()
                    && w.rich_doc_link_bar.is_none()
                    && w.rich_doc_color_bar.is_none()
            );
            assert_eq!(w.buffer().rope.to_string(), SOURCE);
            if target == InputFocus::RichDocLinkUrl {
                assert_eq!(
                    w.buffer()
                        .rich_doc
                        .as_ref()
                        .unwrap()
                        .block_link(0)
                        .as_deref(),
                    Some("https://example.com")
                );
            } else if target == InputFocus::RichDocColorHex {
                assert_eq!(
                    w.buffer().rich_doc.as_ref().unwrap().block_color(0),
                    Some(0x0066CC)
                );
            }
        });
        cx.quit();
    }
}

#[test]
fn tracked_folder_draft_keeps_clipboard_and_escape_routing() {
    let directory = crate::persist::TempDir::new("inline-input");
    let _settings = crate::settings::override_for_test(Settings {
        folder_panel_visible: true,
        folder_panel_roots: vec![directory.path().to_path_buf()],
        folder_panel_expanded: vec![directory.path().to_path_buf()],
        ..preferences()
    });
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx);
    visual.update_window_entity(&workspace, |w, window, cx| {
        w.folder_inline_create_focus = Some(cx.focus_handle());
        w.folder_inline_create = Some(crate::find_state::FolderInlineCreateState::new(
            directory.path().to_path_buf(),
            crate::find_state::FolderInlineCreateKind::File,
        ));
        let (text, cursor, anchor) = w.input_field_mut(InputFocus::FolderInlineCreate).unwrap();
        *text = FIELD.into();
        *cursor = FIELD.len();
        *anchor = FIELD.len();
        w.focus_input(InputFocus::FolderInlineCreate, window, cx);
        cx.notify();
    });
    visual.run_until_parked();
    let bounds = visual.debug_bounds("folder-draft-input").unwrap();
    visual.simulate_click(
        gpui::point(bounds.left() + gpui::px(1.), bounds.center().y),
        gpui::Modifiers::none(),
    );
    visual.simulate_keystrokes("ctrl-a ctrl-c ctrl-x ctrl-v");
    visual.update_window_entity(&workspace, |w, window, _| {
        assert!(w.input_focus(window) == InputFocus::FolderInlineCreate);
        assert_eq!(w.folder_inline_create.as_ref().unwrap().text, FIELD);
        assert_eq!(w.buffer().rope.to_string(), SOURCE);
    });
    visual.simulate_keystrokes("escape");
    visual.update(|window, cx| {
        let w = workspace.read(cx);
        assert!(w.folder_inline_create.is_none());
        assert!(w.focus_handle.is_focused(window));
    });
    assert_eq!(directory.path().read_dir().unwrap().count(), 0);
    cx.quit();
}
