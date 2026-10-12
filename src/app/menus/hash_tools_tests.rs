// Registered Tools callers, originally frozen as missing-feature G1 controls.
use super::*;
use crate::{buffer::{ContentRevision, SavePoint}, macros, settings::Settings, tab::Tab, workspace::Workspace};
use gpui::{Action, ClipboardItem, Entity, OwnedMenu, OwnedMenuItem, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};
use std::collections::BTreeMap;

#[path = "hash_tools_policy_cases.rs"]
mod policy_cases;

#[path = "hash_tools_enter_protocol_tests.rs"]
mod enter_protocol;

const CASES: [(&str, &str, &str); 12] = [
    ("MD5", "ToolsMd5Generate", "IDM_TOOL_MD5_GENERATE"),
    ("MD5", "ToolsMd5GenerateFromFiles", "IDM_TOOL_MD5_GENERATEFROMFILE"),
    ("MD5", "ToolsMd5GenerateSelection", "IDM_TOOL_MD5_GENERATEINTOCLIPBOARD"),
    ("SHA-1", "ToolsSha1Generate", "IDM_TOOL_SHA1_GENERATE"),
    ("SHA-1", "ToolsSha1GenerateFromFiles", "IDM_TOOL_SHA1_GENERATEFROMFILE"),
    ("SHA-1", "ToolsSha1GenerateSelection", "IDM_TOOL_SHA1_GENERATEINTOCLIPBOARD"),
    ("SHA-256", "ToolsSha256Generate", "IDM_TOOL_SHA256_GENERATE"),
    ("SHA-256", "ToolsSha256GenerateFromFiles", "IDM_TOOL_SHA256_GENERATEFROMFILE"),
    ("SHA-256", "ToolsSha256GenerateSelection", "IDM_TOOL_SHA256_GENERATEINTOCLIPBOARD"),
    ("SHA-512", "ToolsSha512Generate", "IDM_TOOL_SHA512_GENERATE"),
    ("SHA-512", "ToolsSha512GenerateFromFiles", "IDM_TOOL_SHA512_GENERATEFROMFILE"),
    ("SHA-512", "ToolsSha512GenerateSelection", "IDM_TOOL_SHA512_GENERATEINTOCLIPBOARD"),
];

pub(crate) fn with_editor(test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _execution = macros::override_execution_for_test(false);
    let _macros = macros::override_playback_for_test(Vec::new(), BTreeMap::new());
    let _clipboard = crate::clipboard_ownership::override_for_test();
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty(); tab.buffer.rope = ropey::Rope::from_str("*abc");
        w.tabs = vec![tab]; w.active = 0;
        let (sink, _) = std::sync::mpsc::channel(); w.session_snapshot_sink = Some(sink); w
    });
    visual.update(|_, cx| { install_global_actions(cx); crate::shortcuts::register_catalog(cx, &crate::shortcuts::Overrides::new()); });
    visual.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked();
    visual.update_window_entity(&workspace, |w, _, cx| {
        w.replace_byte_range(0..1, "", cx); w.replace_byte_range(0..0, "?", cx);
    });
    visual.dispatch_action(EditUndo); visual.run_until_parked();
    visual.update_window_entity(&workspace, |w, _, _| {
        let tab = w.active_tab_mut(); tab.buffer.mark_saved(); tab.buffer.read_only = true;
        tab.selection.anchor = 3; tab.selection.head = 0; tab.selection.is_dragging = true;
        tab.buffer.bookmarks.insert(0); tab.buffer.marks.push(1..2);
    });
    visual.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
    test(&workspace, visual); cx.quit();
}

#[derive(PartialEq, Eq)]
pub(crate) struct Draft {
    text: String, selection: (usize, usize, bool),
    block: Option<(usize, usize, usize, usize, bool)>,
    undo: Vec<u8>, redo: Vec<u8>, revision: ContentRevision, savepoint: Option<SavePoint>,
    bookmarks: Vec<usize>, marks: Vec<std::ops::Range<usize>>,
    dirty: bool, readonly: bool, mode: crate::tab::TabViewMode,
    encoding: crate::text::encoding::Encoding, ending: crate::text::line_ending::LineEnding,
}
pub(crate) fn draft(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Draft {
    visual.update(|_, cx| {
        let w = workspace.read(cx); let tab = &w.tabs[0];
        Draft { text: tab.buffer.rope.to_string(), selection: (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging),
            block: tab.block_selection.map(|b| (b.anchor_line, b.anchor_col, b.head_line, b.head_col, b.is_dragging)),
            undo: crate::buffer::serialize_undo_stack(&tab.undo_stack), redo: crate::buffer::serialize_undo_stack(&tab.redo_stack),
            revision: tab.buffer.raw_revision.clone(), savepoint: tab.buffer.savepoint.clone(),
            bookmarks: tab.buffer.bookmarks.iter().copied().collect(), marks: tab.buffer.marks.clone(),
            dirty: tab.buffer.dirty, readonly: tab.buffer.read_only, mode: tab.view_mode,
            encoding: tab.buffer.encoding, ending: tab.buffer.line_ending }
    })
}

fn collect_routes(menu: &OwnedMenu, parent: &str, output: &mut BTreeMap<String, String>) {
    let path = format!("{parent}{}", mnemonic::strip(&menu.name));
    for item in &menu.items {
        match item {
            OwnedMenuItem::Submenu(sub) => collect_routes(sub, &format!("{path} > "), output),
            OwnedMenuItem::Action { name, action, .. } => { output.insert(action.name().into(), format!("{path} > {}", mnemonic::strip(name))); },
            _ => {},
        }
    }
}

#[test]
fn hash_tools_g1_installed_menu_has_tools_between_settings_and_macro_and_all_twelve_routes() {
    with_editor(|_, visual| {
        let observer = publication_fixture::observe();
        visual.update(|_, cx| install_menus(cx));
        let (count, menus) = observer.snapshot(); assert!(count > 0, "actual App::set_menus input was captured");
        let mut routes = BTreeMap::new(); for menu in &menus { collect_routes(menu, "", &mut routes); }
        let missing: Vec<_> = CASES.iter().filter(|(_, id, _)| !routes.keys().any(|name| name.ends_with(&format!("::{id}"))))
            .map(|(_, id, _)| *id).collect();
        assert!(missing.is_empty(), "installed menu is missing Tools routes: {missing:?}");
        let names: Vec<_> = menus.iter().map(|menu| mnemonic::strip(&menu.name)).collect();
        let tools = names.iter().position(|name| name == "Tools").expect("Tools top-level menu");
        assert_eq!(names.get(tools.wrapping_sub(1)).map(String::as_str), Some("Settings"));
        assert_eq!(names.get(tools + 1).map(String::as_str), Some("Macro"));
        let algorithm_menus: Vec<_> = menus[tools].items.iter().filter_map(|item| match item {
            OwnedMenuItem::Submenu(menu) => Some(menu), _ => None,
        }).collect();
        assert_eq!(algorithm_menus.iter().map(|menu| mnemonic::strip(&menu.name)).collect::<Vec<_>>(),
            ["MD5", "SHA-1", "SHA-256", "SHA-512"]);
        for menu in algorithm_menus {
            let labels: Vec<_> = menu.items.iter().filter_map(|item| match item {
                OwnedMenuItem::Action { name, .. } => Some(mnemonic::strip(name)), _ => None,
            }).collect();
            assert_eq!(labels, ["Generate...", "Generate from files...", "Generate from selection into clipboard"]);
            assert_eq!(menu.items.len(), 3, "oracle has no separators within a hash submenu");
        }
    });
}

#[test]
fn hash_tools_g1_catalog_exposes_twelve_rebindable_commands_without_default_chords() {
    let missing: Vec<_> = CASES.iter().filter(|(_, id, _)| !crate::shortcuts::catalog().iter().any(|row| row.id == *id))
        .map(|(_, id, _)| *id).collect();
    assert!(missing.is_empty(), "Shortcut Mapper is missing Tools actions: {missing:?}");
    for (_, id, npp) in CASES {
        let row = crate::shortcuts::catalog().iter().find(|row| row.id == id).unwrap();
        assert_eq!(row.npp, Some(npp)); assert!(row.keys.is_empty() && row.mac_keys.is_empty());
        assert!((row.action)().name().ends_with(&format!("::{id}")));
    }
}

#[test]
fn hash_tools_g1_test_only_text_and_file_action_dispatch_probes_find_no_dialog_handler() {
    with_editor(|workspace, visual| {
        let before = draft(workspace, visual);
        let mut missing = Vec::new();
        for action in [ToolsMd5Generate.boxed_clone(), ToolsMd5GenerateFromFiles.boxed_clone(),
            ToolsSha1Generate.boxed_clone(), ToolsSha1GenerateFromFiles.boxed_clone(),
            ToolsSha256Generate.boxed_clone(), ToolsSha256GenerateFromFiles.boxed_clone(),
            ToolsSha512Generate.boxed_clone(), ToolsSha512GenerateFromFiles.boxed_clone()] {
            let name = action.name(); let count = visual.update(|_, cx| cx.windows().len());
            visual.update(|window, cx| window.dispatch_action(action, cx)); visual.run_until_parked();
            if visual.update(|_, cx| cx.windows().len()) <= count { missing.push(name); }
        }
        assert!(draft(workspace, visual) == before, "hash dialog commands must preserve the document/history");
        assert!(missing.is_empty(), "test-only declarations dispatched on actual app/window; no dialog handler: {missing:?}");
    });
}

fn selection_probe(action: Box<dyn Action>, expected: &str) {
    with_editor(|workspace, visual| {
        let before = draft(workspace, visual);
        visual.update(|window, cx| window.dispatch_action(action, cx)); visual.run_until_parked();
        assert!(draft(workspace, visual) == before, "selection hash must preserve source, geometries, read-only and history");
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(expected),
            "test-only action declaration has no registered selection handler on the baseline");
    });
}
#[test]
fn hash_tools_g1_test_only_md5_selection_dispatch_probes_absent_handler() {
    selection_probe(ToolsMd5GenerateSelection.boxed_clone(), "900150983cd24fb0d6963f7d28e17f72");
}
#[test]
fn hash_tools_g1_test_only_sha1_selection_dispatch_probes_absent_handler() {
    selection_probe(ToolsSha1GenerateSelection.boxed_clone(), "a9993e364706816aba3e25717850c26c9cd0d89d");
}
#[test]
fn hash_tools_g1_test_only_sha256_selection_dispatch_probes_absent_handler() {
    selection_probe(ToolsSha256GenerateSelection.boxed_clone(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}
#[test]
fn hash_tools_g1_test_only_sha512_selection_dispatch_probes_absent_handler() {
    selection_probe(ToolsSha512GenerateSelection.boxed_clone(), "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f");
}
#[test]
fn hash_tools_g1_existing_readonly_copy_control_reaches_the_editor_listener() {
    with_editor(|workspace, visual| {
        let before = draft(workspace, visual); visual.dispatch_action(EditCopy); visual.run_until_parked();
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("abc"));
        assert!(draft(workspace, visual) == before);
    });
}
