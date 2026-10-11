use super::*;
use crate::text::encoding::{Encoding, decode_bytes, encode_checked};

const FOUR: [SortCommand; 4] = [SortCommand::IgnoreCaseAsc, SortCommand::IgnoreCaseDesc,
    SortCommand::LengthAsc, SortCommand::LengthDesc];

fn format(eol: &str, encoding: Encoding) -> SortFormat<'_> { SortFormat { eol, encoding } }

fn plan(text: &str, selection: Selection, block: Option<BlockSelection>, command: SortCommand) -> Option<SortPlan> {
    sort_plan(&Rope::from_str(text), selection, block, command, format("\n", Encoding::Utf8), None, &AtomicBool::new(false)).expect("plan")
}

fn sorted(text: &str, command: SortCommand) -> String {
    plan(text, Selection::default(), None, command).map_or_else(|| text.to_owned(), |plan| plan.text)
}

#[test]
fn ignoring_case_uses_crt_c_locale_utf16_order_without_expansions() {
    let source = "É\né\nb\nA\n_a\n[\nZ\n😀\n\u{e000}";
    assert_eq!(sorted(source, SortCommand::IgnoreCaseAsc), "[\n_a\nA\nb\nZ\nÉ\né\n😀\n\u{e000}");
    assert_eq!(sorted(source, SortCommand::IgnoreCaseDesc), "\u{e000}\n😀\né\nÉ\nZ\nb\nA\n_a\n[");
    assert_eq!(ignore_case_cmp(&"Straße".encode_utf16().collect::<Vec<_>>(), &"STRASSE".encode_utf16().collect::<Vec<_>>(), &AtomicBool::new(false)).unwrap(), Ordering::Greater);
    assert_eq!(ignore_case_cmp(&"x\0z".encode_utf16().collect::<Vec<_>>(), &"X\0a".encode_utf16().collect::<Vec<_>>(), &AtomicBool::new(false)).unwrap(), Ordering::Equal);
    assert_eq!(sorted("a\nA\nb\nB", SortCommand::IgnoreCaseDesc), "b\nB\na\nA");
}

#[test]
fn length_counts_utf16_units_and_keeps_equal_lengths_in_input_order() {
    assert_eq!(sorted("aaa\n😀\nÉ\nab\n界", SortCommand::LengthAsc), "É\n界\n😀\nab\naaa");
    assert_eq!(sorted("aaa\n😀\nÉ\nab\n界", SortCommand::LengthDesc), "aaa\n😀\nab\nÉ\n界");
    assert_eq!(sorted("e\u{301}\nÉ\n😀\na\0b", SortCommand::LengthAsc), "É\ne\u{301}\n😀\na\0b");
}

#[test]
fn whole_document_final_empty_line_joins_older_sorts() {
    assert_eq!(sorted("b\na\n", SortCommand::IgnoreCaseAsc), "\na\nb");
    assert_eq!(sorted("bbb\na\n", SortCommand::LengthAsc), "\na\nbbb");
    assert_eq!(sorted("a\nb\n", SortCommand::IgnoreCaseDesc), "b\na\n");
    assert_eq!(sorted("a\nbbb\n", SortCommand::LengthDesc), "bbb\na\n");
}

#[test]
fn normal_selection_expands_to_bodies_and_resets_older_sort_direction() {
    let text = "before\nbbb\na\nafter";
    let forward = Selection { anchor: 8, head: 13, is_dragging: true };
    let plan = plan(text, forward, None, SortCommand::LengthAsc).unwrap();
    assert_eq!(plan.range, 7..13);
    assert_eq!(plan.text, "a\nbbb\n");
    assert_eq!((plan.selection.anchor, plan.selection.head, plan.selection.is_dragging), (7, 12, false));
    let reverse = Selection { anchor: 13, head: 8, is_dragging: true };
    let plan = self::plan(text, reverse, None, SortCommand::LengthAsc).unwrap();
    assert_eq!((plan.selection.anchor, plan.selection.head), (7, 12));
    assert!(self::plan(text, Selection { anchor: 8, head: 10, is_dragging: false }, None, SortCommand::LengthAsc).is_none());
}

#[test]
fn older_changed_sort_caret_moves_to_target_start_and_unchanged_sort_preserves_it() {
    let selection = Selection { anchor: 3, head: 3, is_dragging: false };
    let result = plan("bb\na\nc", selection, None, SortCommand::IgnoreCaseAsc).unwrap();
    assert_eq!(result.selection.head, 0);
    let result = plan("a\nbb\nc", selection, None, SortCommand::IgnoreCaseAsc).unwrap();
    assert_eq!(result.selection.head, 3);
}

#[test]
fn already_sorted_selection_expands_without_creating_history() {
    let rope = Rope::from_str("aa\nbbb\ntail");
    let selection = Selection { anchor: 1, head: 5, is_dragging: true };
    let group = prepare_sort(rope, ContentRevision::default(), selection, None,
        SortCommand::LengthAsc, format("\n", Encoding::Utf8), &AtomicBool::new(false)).unwrap();
    assert!(group.ops().is_empty());
    let (_, _, selection) = group.target(false);
    assert_eq!((selection.anchor, selection.head, selection.is_dragging), (0, 6, false));
}

#[test]
fn rectangles_sort_complete_lines_by_columns_and_keep_virtual_corners() {
    let text = "top\np b tail\nq A tail\nr c tail\nbottom";
    let rectangle = BlockSelection { anchor_line: 1, head_line: 3, anchor_col: 2, head_col: 3, is_dragging: false };
    let selection = Selection { anchor: 6, head: 25, is_dragging: false };
    let plan = plan(text, selection, Some(rectangle), SortCommand::IgnoreCaseAsc).unwrap();
    assert_eq!(plan.range, 4..31);
    assert_eq!(plan.text, "q A tail\np b tail\nr c tail\n");
    assert_eq!((plan.selection.anchor, plan.selection.head), (6, 25));
    let rectangle = BlockSelection { anchor_line: 0, head_line: 3, anchor_col: 2, head_col: 20, is_dragging: false };
    let plan = self::plan("a\n😀xy\nbc\nZ", Selection::default(), Some(rectangle), SortCommand::LengthAsc).unwrap();
    assert_eq!(plan.text, "a\nbc\nZ\n😀xy");
}

#[test]
fn thin_rectangle_sorts_from_its_column_to_line_end_and_clamps_rows() {
    let rectangle = BlockSelection { anchor_line: 0, head_line: usize::MAX, anchor_col: 1, head_col: 1, is_dragging: false };
    assert_eq!(plan("aZ\nbA\ncB", Selection::default(), Some(rectangle), SortCommand::IgnoreCaseAsc).unwrap().text, "bA\ncB\naZ");
    let empty_last = BlockSelection { anchor_line: 0, head_line: 2, anchor_col: 0, head_col: 1, is_dragging: false };
    assert_eq!(plan("b\na\n", Selection::default(), Some(empty_last), SortCommand::IgnoreCaseAsc).unwrap().text, "\na\nb");
}

#[test]
fn older_rectangle_keys_use_corner_byte_offsets_as_utf16_indices() {
    let rectangle = BlockSelection { anchor_line: 0, head_line: 2, anchor_col: 1, head_col: 2, is_dragging: false };
    // Corner byte offsets are 2 and 3, selecting UTF16 unit index 2.
    let text = "éZa\n😀Ay\néBb";
    assert_eq!(plan(text, Selection::default(), Some(rectangle), SortCommand::IgnoreCaseAsc).unwrap().text,
        "éZa\n😀Ay\néBb");
    let rectangle = BlockSelection { anchor_line: 0, head_line: 2, anchor_col: 1, head_col: 1, is_dragging: false };
    // ASCII corners select unit index 1, including a lone surrogate in row 1.
    for command in [SortCommand::IgnoreCaseAsc, SortCommand::LengthAsc] {
        assert_eq!(plan("xA\n😀!\nxB", Selection::default(), Some(rectangle), command).unwrap().text, "xA\nxB\n😀!");
    }
}

#[test]
fn ansi_rectangle_offsets_use_single_byte_editing_for_all_basic_modes() {
    let text = "é\néZ\néAA";
    let rectangle = BlockSelection { anchor_line: 0, head_line: 2, anchor_col: 1, head_col: 1, is_dragging: false };
    for (command, expected) in [
        (SortCommand::IgnoreCaseAsc, "é\néAA\néZ"),
        (SortCommand::IgnoreCaseDesc, "éZ\néAA\né"),
        (SortCommand::LengthAsc, "é\néZ\néAA"),
        (SortCommand::LengthDesc, "éAA\néZ\né"),
    ] {
        let group = prepare_sort(Rope::from_str(text), ContentRevision::default(), Selection::default(),
            Some(rectangle), command, format("\n", Encoding::Latin1), &AtomicBool::new(false)).unwrap();
        assert_eq!(group.target(false).0.to_string(), expected);
        assert_eq!(encode_checked(&group.target(true).0.to_string(), Encoding::Latin1).unwrap(), encode_checked(text, Encoding::Latin1).unwrap());
    }
    for encoding in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        let group = prepare_sort(Rope::from_str(text), ContentRevision::default(), Selection::default(),
            Some(rectangle), SortCommand::LengthDesc, format("\n", encoding), &AtomicBool::new(false)).unwrap();
        assert_eq!(group.target(false).0.to_string(), "éAA\né\néZ", "Unicode files use UTF8 editing offsets");
    }
}

#[test]
fn physical_endings_and_unicode_separators_preserve_every_body_and_terminator() {
    let endings = ["\n", "\r", "\r\n"];
    for eol in endings {
        let text = format!("bbb{eol}a{eol}é\u{85}界{eol}😀");
        for command in FOUR {
            let out = sorted(&text, command);
            let mut before = lines_in(&text, true, &AtomicBool::new(false)).unwrap().into_iter().map(|line| line.body.to_owned()).collect::<Vec<_>>();
            let mut after = lines_in(&out, true, &AtomicBool::new(false)).unwrap().into_iter().map(|line| line.body.to_owned()).collect::<Vec<_>>();
            before.sort(); after.sort(); assert_eq!(after, before, "{command:?} {eol:?}");
            assert_eq!(out.len(), text.len());
            assert_eq!(out.matches(eol).count(), 3);
        }
    }
    assert_eq!(sorted("b\r\na\nc\rzz", SortCommand::IgnoreCaseAsc), "a\r\nb\nc\rzz");
    assert!(sort_plan(&Rope::from_str("a\rb\n\n"), Selection::default(), None,
        SortCommand::IgnoreCaseAsc, format("\n", Encoding::Utf8), None, &AtomicBool::new(false)).is_err());
}

#[test]
fn all_offsets_clamp_to_boundaries_and_never_change_outside_the_block() {
    let inputs = ["", "é", "bbb\na\r\n😀\n", "b\u{85}z\ra\n界\u{2028}tail"];
    for text in inputs {
        let rope = Rope::from_str(text);
        for anchor in (0..=text.len().saturating_add(2)).chain([usize::MAX]) {
            for head in (0..=text.len().saturating_add(2)).chain([usize::MAX]) {
                for command in FOUR {
                    let result = sort_plan(&rope, Selection { anchor, head, is_dragging: false }, None,
                        command, format("\n", Encoding::Utf8), None, &AtomicBool::new(false));
                    if let Ok(Some(plan)) = result {
                        assert!(text.is_char_boundary(plan.range.start)); assert!(text.is_char_boundary(plan.range.end));
                        let mut changed = text.to_owned(); changed.replace_range(plan.range.clone(), &plan.text);
                        assert!(changed.starts_with(&text[..plan.range.start])); assert!(changed.ends_with(&text[plan.range.end..]));
                        assert!(changed.is_char_boundary(plan.selection.anchor)); assert!(changed.is_char_boundary(plan.selection.head));
                        assert_eq!(plan.text.len(), plan.range.len());
                    }
                }
            }
        }
    }
}

#[test]
fn bom_and_utf16_save_fidelity_survives_sort_and_undo() {
    for encoding in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        let (original, expected) = if encoding == Encoding::Latin1 { ("bbb\r\na\r\nÉ", "a\r\nÉ\r\nbbb") }
            else { ("bbb\r\na\r\n😀", "a\r\n😀\r\nbbb") };
        let original_bytes = encode_checked(original, encoding).unwrap();
        let decoded = decode_bytes(&original_bytes);
        assert_eq!(decoded.encoding, encoding);
        let rope = Rope::from_str(&decoded.text);
        let group = prepare_sort(rope, ContentRevision::default(), Selection::default(), None,
            SortCommand::LengthAsc, format("\r\n", encoding), &AtomicBool::new(false)).unwrap();
        assert_eq!(group.ops().len(), 1);
        let (sorted, _, _) = group.target(false);
        assert_eq!(encode_checked(&sorted.to_string(), encoding).unwrap(), encode_checked(expected, encoding).unwrap());
        let (undone, _, _) = group.target(true);
        assert_eq!(encode_checked(&undone.to_string(), encoding).unwrap(), original_bytes);
    }
}

#[test]
fn cancellation_and_limits_leave_the_source_untouched() {
    let rope = Rope::from_str("z\na");
    assert_eq!(sort_plan(&rope, Selection::default(), None, SortCommand::IgnoreCaseAsc,
        format("\n", Encoding::Utf8), None, &AtomicBool::new(true)).err(), Some(CANCELLED));
    let rope = Rope::from_str(&"z\n".repeat(MAX_SORT_BYTES / 2 + 1));
    assert!(sort_plan(&rope, Selection::default(), None, SortCommand::LengthAsc,
        format("\n", Encoding::Utf8), None, &AtomicBool::new(false)).err().unwrap().contains("32 MiB"));
    assert_eq!(rope.len_bytes(), MAX_SORT_BYTES + 2);
    let rope = Rope::from_str(&"\n".repeat(MAX_SORT_LINES));
    assert!(sort_plan(&rope, Selection::default(), None, SortCommand::LengthAsc,
        format("\n", Encoding::Utf8), None, &AtomicBool::new(false)).err().unwrap().contains("1,048,576"));
}

fn injected_key(text: &str) -> Result<Vec<u8>, &'static str> { Ok(text.as_bytes().to_vec()) }

#[test]
fn locale_interface_uses_supplied_keys_and_oracle_eol_caret_rules() {
    let rope = Rope::from_str("b\r\na\n");
    let selection = Selection { anchor: 1, head: 1, is_dragging: false };
    let plan = sort_plan(&rope, selection, None, SortCommand::LocaleAsc, format("\r\n", Encoding::Utf8),
        Some(injected_key), &AtomicBool::new(false)).unwrap().unwrap();
    assert_eq!(plan.text, "a\nb\r\n");
    assert_eq!(plan.selection.head, 3);
    let rope = Rope::from_str("b\r\na");
    let plan = sort_plan(&rope, Selection { anchor: 1, head: 4, is_dragging: false }, None,
        SortCommand::LocaleAsc, format("\r\n", Encoding::Utf8), Some(injected_key), &AtomicBool::new(false)).unwrap().unwrap();
    assert_eq!(plan.text, "a\r\nb"); assert_eq!((plan.selection.anchor, plan.selection.head), (0, 4));
    let failed = sort_plan(&rope, Selection::default(), None, SortCommand::LocaleAsc,
        format("\n", Encoding::Utf8), None, &AtomicBool::new(false));
    assert!(failed.err().unwrap().contains("native NLS"));
}

#[cfg(windows)]
#[test]
fn native_locale_sort_preserves_actual_endings_selection_direction_and_caret() {
    let native = Some(crate::locale_sort_windows::sort_key as LocaleKey);
    for (text, command, expected) in [
        ("Item10\r\nitem2\nITEM1", SortCommand::LocaleAsc, "ITEM1\r\nitem2\nItem10"),
        ("Item10\r\nitem2\nITEM1", SortCommand::LocaleDesc, "Item10\r\nitem2\nITEM1"),
        ("b\ra\r", SortCommand::LocaleAsc, "a\rb\r"),
        ("b\r\na\n", SortCommand::LocaleAsc, "a\nb\r\n"),
        ("b\na", SortCommand::LocaleAsc, "a\r\nb"),
    ] {
        let result = sort_plan(&Rope::from_str(text), Selection::default(), None, command,
            format("\r\n", Encoding::Utf8), native, &AtomicBool::new(false)).unwrap().unwrap();
        assert_eq!(result.text, expected);
    }
    let rope = Rope::from_str("outside\nb\r\na\nend");
    for (selection, expected_selection) in [
        (Selection { anchor: 9, head: 12, is_dragging: false }, (8, 13)),
        (Selection { anchor: 12, head: 9, is_dragging: false }, (13, 8)),
    ] {
        let result = sort_plan(&rope, selection, None, SortCommand::LocaleAsc,
            format("\n", Encoding::Utf8), native, &AtomicBool::new(false)).unwrap().unwrap();
        assert_eq!(result.range, 8..13); assert_eq!(result.text, "a\nb\r\n");
        assert_eq!((result.selection.anchor, result.selection.head), expected_selection);
    }
    let rope = Rope::from_str("b\r\na\n");
    let result = sort_plan(&rope, Selection { anchor: 2, head: 2, is_dragging: false }, None,
        SortCommand::LocaleAsc, format("\n", Encoding::Utf8), native, &AtomicBool::new(false)).unwrap().unwrap();
    assert_eq!(result.selection.head, 4, "caret on LF follows its original CRLF line");
}
