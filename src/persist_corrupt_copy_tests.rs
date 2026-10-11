use super::*;
use serde::Deserialize;

#[derive(Deserialize, Default, Debug, PartialEq)]
struct State {
    #[serde(default)]
    width: u32,
    #[serde(default)]
    name: String,
}

const DAMAGED: &[u8] = b"width = \"wide\"\r\nname = \"kept\"\r\n";

fn load(path: &Path) -> Loaded<State> {
    load_toml(path, |table| salvage_table(table, &[]).unwrap_or_default(), State::default)
}

#[test]
fn load_toml_salvage_keeps_document_hard_linked_to_corrupt_sibling() {
    let dir = TempDir::new("persist-linked-corrupt");
    let path = dir.join("state.toml");
    let corrupt = dir.join("state.toml.corrupt");
    let document = dir.join("document.txt");
    let original = b"unrelated document\r\nretained bytes\r\n";
    fs::write(&path, DAMAGED).unwrap();
    fs::write(&document, original).unwrap();
    fs::hard_link(&document, &corrupt).unwrap();

    let loaded = load(&path);

    assert!(matches!(loaded, Loaded::Salvaged(State { width: 0, ref name }) if name == "kept"));
    assert_eq!(fs::read(&document).unwrap(), original,
        "loading salvageable state must not overwrite an unrelated linked document");
    assert_eq!(fs::read(&path).unwrap(), DAMAGED, "the original salvage source stays intact");
}

#[test]
fn load_toml_salvage_copies_original_when_corrupt_sibling_is_absent() {
    let dir = TempDir::new("persist-new-corrupt");
    let path = dir.join("state.toml");
    fs::write(&path, DAMAGED).unwrap();

    let loaded = load(&path);

    assert!(matches!(loaded, Loaded::Salvaged(State { width: 0, ref name }) if name == "kept"));
    assert_eq!(fs::read(&path).unwrap(), DAMAGED);
    assert_eq!(fs::read(dir.join("state.toml.corrupt")).unwrap(), DAMAGED);
}

#[test]
fn load_toml_clean_state_does_not_touch_corrupt_sibling_or_linked_document() {
    let dir = TempDir::new("persist-clean-corrupt");
    let path = dir.join("state.toml");
    let corrupt = dir.join("state.toml.corrupt");
    let document = dir.join("document.txt");
    let original = b"unrelated document\r\n";
    fs::write(&path, b"width = 7\nname = \"clean\"\n").unwrap();
    fs::write(&document, original).unwrap();
    fs::hard_link(&document, &corrupt).unwrap();

    assert!(matches!(load(&path), Loaded::Clean(State { width: 7, ref name }) if name == "clean"));
    assert_eq!(fs::read(&document).unwrap(), original);
    assert_eq!(fs::read(&corrupt).unwrap(), original);
}

#[test]
fn load_toml_salvage_replaces_an_older_regular_corrupt_backup() {
    let dir = TempDir::new("persist-old-corrupt");
    let path = dir.join("state.toml");
    let corrupt = dir.join("state.toml.corrupt");
    fs::write(&path, DAMAGED).unwrap();
    fs::write(&corrupt, b"older rejected settings").unwrap();

    assert!(matches!(load(&path), Loaded::Salvaged(_)));

    assert_eq!(fs::read(&path).unwrap(), DAMAGED);
    assert_eq!(fs::read(&corrupt).unwrap(), DAMAGED, "documented fixed-name backup replacement");
}

#[test]
fn load_toml_salvage_preserves_source_hard_linked_to_corrupt_backup() {
    let dir = TempDir::new("persist-source-corrupt");
    let path = dir.join("state.toml");
    let corrupt = dir.join("state.toml.corrupt");
    fs::write(&path, DAMAGED).unwrap();
    fs::hard_link(&path, &corrupt).unwrap();

    assert!(matches!(load(&path), Loaded::Salvaged(State { width: 0, ref name }) if name == "kept"));

    assert_eq!(fs::read(&path).unwrap(), DAMAGED, "source alias cannot truncate loaded state");
    assert_eq!(fs::read(&corrupt).unwrap(), DAMAGED);
}
