// Actual menu/catalog contracts with a neutral recent snapshot, no user-state reads.
use super::*;

fn label(item: &MenuItem) -> String {
    match item {
        MenuItem::Action { name, .. } => mnemonic::strip(name),
        MenuItem::Submenu(menu) => mnemonic::strip(&menu.name),
        MenuItem::Separator => "---".into(),
        _ => "other".into(),
    }
}

#[test]
fn file_tab_commands_follow_oracle_positions() {
    let menu = build_with_recent(&settings::Settings::default(), &[]);
    let names: Vec<String> = menu.items.iter().map(label).collect();
    for required in [
        "Open Containing Folder",
        "Open in Default Viewer",
        "Reload from Disk",
        "Close Multiple Documents",
    ] {
        assert!(
            names.iter().any(|name| name == required),
            "File menu is missing {required}"
        );
    }
    let pos = |name: &str| {
        names
            .iter()
            .position(|item| item == name)
            .expect("required File row")
    };
    assert!(pos("Open…") < pos("Open Containing Folder"));
    assert!(pos("Open Containing Folder") < pos("Open in Default Viewer"));
    assert!(pos("Open in Default Viewer") < pos("Open Folder as Workspace…"));
    assert!(pos("Open Folder as Workspace…") < pos("Reload from Disk"));
    assert!(pos("Reload from Disk") < pos("Save"));
    assert!(pos("Close All Tabs") < pos("Close Multiple Documents"));
    assert!(pos("Close Multiple Documents") < pos("Restore Recent Closed Tab"));
    let close = menu
        .items
        .iter()
        .find_map(|item| match item {
            MenuItem::Submenu(sub) if mnemonic::strip(&sub.name) == "Close Multiple Documents" => {
                Some(sub)
            }
            _ => None,
        })
        .expect("Close Multiple submenu");
    assert_eq!(
        close.items.iter().map(label).collect::<Vec<_>>(),
        [
            "Close All but Active Document",
            "Close All to the Left",
            "Close All to the Right",
            "Close All Unchanged",
        ]
    );
}

#[test]
fn file_tab_commands_have_rebindable_catalog_ids_and_defaults() {
    let expected = [
        ("FileOpenContainingFolder", "IDM_FILE_OPEN_FOLDER"),
        ("FileOpenDefaultViewer", "IDM_FILE_OPEN_DEFAULT_VIEWER"),
        ("FileReloadFromDisk", "IDM_FILE_RELOAD"),
        ("FileCloseAllButActive", "IDM_FILE_CLOSEALL_BUT_CURRENT"),
        ("FileCloseAllToLeft", "IDM_FILE_CLOSEALL_TOLEFT"),
        ("FileCloseAllToRight", "IDM_FILE_CLOSEALL_TORIGHT"),
        ("FileCloseAllUnchanged", "IDM_FILE_CLOSEALL_UNCHANGED"),
    ];
    let menu = build_with_recent(&settings::Settings::default(), &[]);
    fn contains_action(menu: &Menu, name: &str) -> bool {
        menu.items.iter().any(|item| match item {
            MenuItem::Action { action, .. } => action.name() == name,
            MenuItem::Submenu(sub) => contains_action(sub, name),
            _ => false,
        })
    }
    for (id, npp) in expected {
        let entry = crate::shortcuts::catalog()
            .iter()
            .find(|entry| entry.id == id)
            .expect(id);
        assert_eq!(entry.npp, Some(npp));
        assert_eq!(entry.context, Some("Workspace"));
        assert!(
            contains_action(&menu, (entry.action)().name()),
            "{id} absent from menu"
        );
        if id == "FileReloadFromDisk" {
            assert_eq!(entry.keys, ["ctrl-r"]);
            assert_eq!(entry.mac_keys, ["cmd-r"]);
        } else {
            assert!(entry.keys.is_empty());
            assert!(entry.mac_keys.is_empty());
        }
    }
    let fill = crate::shortcuts::catalog()
        .iter()
        .find(|entry| entry.id == "EditFillRight")
        .expect("Fill Right catalog");
    assert_eq!(fill.mac_keys, ["cmd-alt-r"]);
}

#[test]
fn supplied_recent_paths_keep_flat_and_submenu_behavior() {
    let paths = [std::env::temp_dir().join("neutral & café.txt")];
    for nested in [false, true] {
        let prefs = settings::Settings {
            recent_files_in_submenu: nested,
            recent_files_show_full_path: false,
            ..settings::Settings::default()
        };
        let menu = build_with_recent(&prefs, &paths);
        let items = if nested {
            &menu
                .items
                .iter()
                .find_map(|item| match item {
                    MenuItem::Submenu(sub) if sub.name.as_ref() == "Open Recent" => Some(sub),
                    _ => None,
                })
                .expect("Open Recent submenu")
                .items
        } else {
            &menu.items
        };
        let (name, action) = items
            .iter()
            .find_map(|item| match item {
                MenuItem::Action { name, action, .. } => action
                    .as_any()
                    .downcast_ref::<FileOpenRecent>()
                    .map(|action| (name, action)),
                _ => None,
            })
            .expect("recent action");
        assert_eq!(mnemonic::strip(name), "neutral & café.txt");
        assert!(name.contains("&&"));
        assert_eq!(action.path, paths[0]);
    }
    let menu = build_with_recent(&settings::Settings::default(), &[]);
    let folders = menu
        .items
        .iter()
        .find_map(|item| match item {
            MenuItem::Submenu(sub) if mnemonic::strip(&sub.name) == "Open Containing Folder" => {
                Some(sub)
            }
            _ => None,
        })
        .expect("containing folder submenu");
    assert_eq!(
        folders.items.len(),
        1,
        "only audited file manager action is exposed"
    );
    assert_eq!(label(&folders.items[0]), file_manager_label());
}
