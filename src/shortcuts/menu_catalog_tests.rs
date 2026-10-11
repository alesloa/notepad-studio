// Persisted action names predate menu-line command splits and catalog growth.
use super::*;

#[test]
fn old_keyboard_and_new_menu_override_names_remain_loadable_and_bindable() {
    let directory = crate::persist::TempDir::new("menu-catalog-overrides");
    let path = directory.join("shortcuts.toml");
    std::fs::write(&path, "\
[[binding]]\naction = \"EditDuplicateLine\"\nkeys = [\"ctrl-alt-shift-f7\"]\n\
[[binding]]\naction = \"EditOutdent\"\nkeys = [\"ctrl-alt-shift-f8\"]\n\
[[binding]]\naction = \"ViewWordWrap\"\nkeys = [\"ctrl-alt-shift-f9\"]\n\
[[binding]]\naction = \"FileNew\"\nkeys = []\n\
[[binding]]\naction = \"FutureAction\"\nkeys = [\"ctrl-alt-shift-f10\"]\n").unwrap();
    let overrides = load_overrides_at(&path);
    for (id, npp, chord) in [
        ("EditDuplicateLine", "SCI_SELECTIONDUPLICATE", "ctrl-alt-shift-f7"),
        ("EditOutdent", "SCI_BACKTAB", "ctrl-alt-shift-f8"),
        ("ViewWordWrap", "IDM_VIEW_WRAP", "ctrl-alt-shift-f9"),
    ] {
        let entry = catalog().iter().find(|entry| entry.id == id).expect(id);
        assert_eq!(entry.npp, Some(npp));
        assert_eq!(effective_keys(entry, &overrides), [chord]);
        let binding = load_binding(chord, (entry.action)(), entry.context, &gpui::DummyKeyboardMapper).unwrap();
        assert_eq!(binding.action().name(), (entry.action)().name());
    }
    assert_eq!(overrides.get("FileNew"), Some(&Vec::new()));
    assert_eq!(overrides.len(), 5);
    save_overrides_at(&path, &overrides).unwrap();
    assert_eq!(load_overrides_at(&path), overrides, "unknown future ids and explicit unbinding survive");
}
