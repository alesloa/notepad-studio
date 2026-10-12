// Source-only G1 cases. Codec tests establish current app facts; they do not
// exercise a hash engine. Dispatch probes still use test-only action types.
use super::*;
use crate::buffer::BlockSelection;
use crate::text::{encoding::{self, Encoding}, line_ending::{self, LineEnding}};

#[test]
fn hash_tools_g1_unicode_load_facts_remove_transport_bom_and_restore_editor_eol_bytes() {
    let text = "é中\r\nnext\r\n";
    for encoding in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        let disk = encoding::encode_checked(text, encoding).unwrap();
        let decoded = encoding::decode_bytes(&disk);
        assert_eq!(decoded.encoding, encoding); assert!(!decoded.lossy);
        let (eol, normalized) = line_ending::split_line_endings(decoded.text);
        assert_eq!(eol, LineEnding::Crlf); assert_eq!(normalized, "é中\nnext\n");
        let editor_bytes = line_ending::rope_to_bytes(&ropey::Rope::from_str(&normalized), eol);
        assert_eq!(editor_bytes, text.as_bytes(), "Unicode editor bytes are UTF-8 without the transport BOM");
        if encoding != Encoding::Utf8 { assert_ne!(editor_bytes, disk); }
    }
    let disk = encoding::encode_checked("\u{feff}body", Encoding::Utf16Le).unwrap();
    assert_eq!(encoding::decode_bytes(&disk).text, "\u{feff}body", "a second BOM is literal editor text");
}

#[test]
fn hash_tools_g1_mixed_eol_and_strict_windows1252_are_existing_codec_facts() {
    let text = "é\r\n中\nlast\rtail";
    let (eol, normalized) = line_ending::split_line_endings(text.into());
    assert_eq!(eol, LineEnding::Lf); assert_eq!(normalized, text);
    assert_eq!(line_ending::rope_to_bytes(&ropey::Rope::from_str(&normalized), eol), text.as_bytes());
    let raw = [0xe9, 0x80, 0x81, 0xff, b'\r', b'\n'];
    let decoded = encoding::decode_bytes(&raw);
    assert_eq!(decoded.encoding, Encoding::Latin1); assert!(!decoded.lossy);
    assert_eq!(encoding::encode_checked(&decoded.text, Encoding::Latin1).unwrap(), raw);
    assert!(encoding::encode_checked("中", Encoding::Latin1).is_err(), "hashing must refuse rather than substitute");
}

#[test]
fn hash_tools_g1_test_only_unicode_and_ansi_selection_probes_editor_byte_contract() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!("../../../tests/data/hash_tools_vectors.json")).unwrap();
    let ansi: serde_json::Value = serde_json::from_str(include_str!("../../../tests/data/hash_tools_ansi_fixture_vectors.json")).unwrap();
    for (text, encoding, id) in [
        ("é中\r\nnext\r\n", Encoding::Utf8, "unicode_crlf"),
        ("é中\r\nnext\r\n", Encoding::Utf8Bom, "unicode_crlf"),
        ("é中\r\nnext\r\n", Encoding::Utf16Le, "unicode_crlf"),
        ("é中\r\nnext\r\n", Encoding::Utf16Be, "unicode_crlf"),
        ("é\r\n中\nlast\rtail", Encoding::Utf8Bom, "mixed_eol"),
        ("é€\u{81}ÿ\r\n", Encoding::Latin1, "windows1252_unambiguous"),
    ] {
        let disk = encoding::encode_checked(text, encoding).unwrap();
        let decoded = encoding::decode_bytes(&disk);
        let (eol, normalized) = line_ending::split_line_endings(decoded.text);
        let expected = if encoding == Encoding::Latin1 { ansi["case"]["digests"]["md5"].as_str().unwrap() }
            else { vectors["raw_vectors"].as_array().unwrap().iter().find(|row| row["id"] == id).unwrap()["digests"]["md5"].as_str().unwrap() };
        with_editor(|workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| {
                let tab = w.active_tab_mut(); tab.buffer.rope = ropey::Rope::from_str(&normalized);
                tab.buffer.encoding = encoding; tab.buffer.line_ending = eol;
                tab.selection.anchor = normalized.len(); tab.selection.head = 0;
            });
            let before = draft(workspace, visual);
            visual.dispatch_action(ToolsMd5GenerateSelection); visual.run_until_parked();
            assert!(draft(workspace, visual) == before);
            assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(expected),
                "hash supported editor bytes, preserving mixed EOL and excluding transport BOM");
        });
    }
}

#[test]
fn hash_tools_g1_test_only_nonempty_nul_selection_probes_absent_handler() {
    for (text, expected) in [("abc\0suffix", "900150983cd24fb0d6963f7d28e17f72"),
        ("\0abc", "d41d8cd98f00b204e9800998ecf8427e")] {
        with_editor(|workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| {
                let tab = w.active_tab_mut(); tab.buffer.rope = ropey::Rope::from_str(text);
                tab.selection.anchor = text.len(); tab.selection.head = 0;
            });
            let before = draft(workspace, visual);
            visual.dispatch_action(ToolsMd5GenerateSelection); visual.run_until_parked();
            assert!(draft(workspace, visual) == before);
            assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(expected),
                "nonempty span truncates at first NUL; a leading NUL still copies the empty-byte digest");
        });
    }
}

#[test]
fn hash_tools_g1_test_only_ansi_nul_suffix_is_not_converted_or_substituted() {
    with_editor(|workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| {
            let tab = w.active_tab_mut(); tab.buffer.rope = ropey::Rope::from_str("abc\0中");
            tab.buffer.encoding = Encoding::Latin1; tab.selection.anchor = 7; tab.selection.head = 0;
        });
        let before = draft(workspace, visual);
        visual.dispatch_action(ToolsMd5GenerateSelection); visual.run_until_parked();
        assert!(draft(workspace, visual) == before);
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(),
            Some("900150983cd24fb0d6963f7d28e17f72"));
    });
}

#[test]
fn hash_tools_g1_test_only_one_row_rectangle_probes_physical_bytes_without_padding() {
    with_editor(|workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| {
            w.active_tab_mut().block_selection = Some(BlockSelection {
                anchor_line: 0, anchor_col: 7, head_line: 0, head_col: 0, is_dragging: true,
            });
        });
        let before = draft(workspace, visual);
        visual.dispatch_action(ToolsMd5GenerateSelection); visual.run_until_parked();
        assert!(draft(workspace, visual) == before);
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(),
            Some("900150983cd24fb0d6963f7d28e17f72"));
    });
}

#[test]
fn hash_tools_g1_test_only_find_focus_probe_targets_active_readonly_editor_selection() {
    with_editor(|workspace, visual| {
        visual.update_window_entity(workspace, |w, window, cx| {
            w.find_state.open = true; w.find_state.query = "different field text".into();
            window.focus(&w.find_focus, cx);
        });
        visual.run_until_parked();
        let before = draft(workspace, visual);
        visual.dispatch_action(ToolsMd5GenerateSelection); visual.run_until_parked();
        assert!(draft(workspace, visual) == before);
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(),
            Some("900150983cd24fb0d6963f7d28e17f72"));
        visual.update(|_, cx| assert_eq!(workspace.read(cx).find_state.query, "different field text"));
    });
}

#[test]
fn hash_tools_g1_test_only_empty_and_multirow_noops_preserve_sentinel_and_draft() {
    // An absent handler also passes these no-op assertions. Pair them with
    // the failing nonempty dispatch controls; these alone do not prove guards.
    for block in [None, Some(BlockSelection {
        anchor_line: 0, anchor_col: 0, head_line: 1, head_col: 2, is_dragging: true,
    }), Some(BlockSelection {
        anchor_line: 0, anchor_col: 9, head_line: 0, head_col: 7, is_dragging: true,
    })] {
        with_editor(|workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| {
                let tab = w.active_tab_mut(); tab.buffer.rope = ropey::Rope::from_str("abc\ndef");
                tab.selection.anchor = 2; tab.selection.head = 2; tab.block_selection = block;
            });
            let before = draft(workspace, visual);
            visual.dispatch_action(ToolsMd5GenerateSelection); visual.run_until_parked();
            assert!(draft(workspace, visual) == before);
            assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("sentinel"));
        });
    }
}
