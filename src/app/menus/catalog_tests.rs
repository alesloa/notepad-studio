// The installer and this contract use the same builders. Payload-bearing
// recent paths, named macros, named Run commands and plugin commands need
// per-entry binding identities; their empty Default values are not shortcuts.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn dynamic_payload(action: &dyn gpui::Action) -> bool {
    let value = action.as_any();
    value.is::<FileOpenRecent>()
        || value.is::<MacroRunSaved>()
        || value.is::<RunSavedCommand>()
        || value.is::<PluginRunCommand>()
}

fn collect(menu: &Menu, parents: &str, actions: &mut BTreeMap<String, String>) {
    let path = format!("{parents}{}", mnemonic::strip(&menu.name));
    for item in &menu.items {
        match item {
            MenuItem::Action { name, action, .. } if !dynamic_payload(action.as_ref()) => {
                actions.insert(action.name().into(), format!("{path} > {}", mnemonic::strip(name)));
            }
            MenuItem::Submenu(submenu) => collect(submenu, &format!("{path} > "), actions),
            _ => {}
        }
    }
}

#[test]
fn every_installed_static_menu_action_has_a_shortcut_mapper_row() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _macros = crate::macros::override_playback_for_test(Vec::new(), BTreeMap::new());
    let catalog: BTreeSet<_> = crate::shortcuts::catalog()
        .iter()
        .map(|entry| (entry.action)().name())
        .collect();
    let mut actions = BTreeMap::new();
    for macos in [false, true] {
        let state = MenuState::default();
        let file = file::build_with_recent(&crate::settings::Settings::default(), &[]);
        let menus = build_menus(&state, file, macos);
        assert_eq!(menus.first().unwrap().name.as_ref(), if macos { crate::consts::APP_NAME } else { "&File" });
        for menu in menus {
            collect(&menu, "", &mut actions);
        }
    }
    for required in ["RunCommand", "RunValidateShortcuts", "MacroPlayback", "PluginsOpenFolder", "FileClearRecent"] {
        assert!(actions.keys().any(|name| name.ends_with(&format!("::{required}"))), "missing fixed menu action {required}");
    }
    let missing: Vec<_> = actions.iter()
        .filter(|(name, _)| !catalog.contains(name.as_str()))
        .map(|(name, path)| format!("{name}: {path}"))
        .collect();
    assert!(missing.is_empty(), "{} static menu actions have no Shortcut Mapper row:\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn only_payload_entries_are_exempt_from_the_menu_catalog_contract() {
    let menu = Menu {
        name: "Dynamic fixtures".into(),
        items: vec![
            MenuItem::action("Recent", FileOpenRecent { path: "neutral.txt".into() }),
            MenuItem::action("Saved macro", MacroRunSaved { name: "neutral".into() }),
            MenuItem::action("Saved Run", RunSavedCommand { id: "neutral".into() }),
            MenuItem::action("Plugin command", PluginRunCommand { plugin: "neutral".into(), command_id: 1 }),
            MenuItem::action("Run", RunCommand),
            MenuItem::action("Validate shortcuts.xml", RunValidateShortcuts),
            MenuItem::action("Plugins folder", PluginsOpenFolder),
            MenuItem::action("Save macro", MacroSaveCurrent),
            MenuItem::action("Clear recent", FileClearRecent),
        ],
    };
    let mut actions = BTreeMap::new();
    collect(&menu, "", &mut actions);
    assert_eq!(actions.len(), 5);
    assert!(actions.keys().any(|name| name.ends_with("::RunCommand")));
    assert!(actions.keys().all(|name| !name.ends_with("::RunSavedCommand")));
}
