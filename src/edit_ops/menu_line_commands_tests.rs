use super::*;

#[test]
fn foreground_line_work_has_exact_byte_and_physical_boundary_limits() {
    let exact = format!("{}\r\n", "x".repeat(MAX_FOREGROUND_LINE_BYTES - 2));
    let rope = Rope::from_str(&exact);
    for endpoints in [(0, exact.len()), (exact.len(), 0), (1, 1)] {
        assert!(foreground_line_work_bounded(&rope, selection(endpoints.0, endpoints.1)));
    }
    let over = format!("x{exact}");
    assert!(!foreground_line_work_bounded(&Rope::from_str(&over), selection(0, over.len())));
    for separator in ['\u{85}', '\u{b}', '\u{c}', '\u{2028}', '\u{2029}'] {
        let text = format!("{}{}é😀{}tail\r\n", "x".repeat(MAX_FOREGROUND_LINE_BYTES), separator, separator);
        let at = MAX_FOREGROUND_LINE_BYTES + separator.len_utf8();
        assert!(!foreground_line_work_bounded(&Rope::from_str(&text), selection(at, at)), "{separator:?}");
    }
    let text = format!("\u{feff}é😀\r\n{}\u{2028}tail", "x".repeat(10 * 1024 * 1024));
    let rope = Rope::from_str(&text);
    let end = "\u{feff}é😀\r\n".len();
    assert!(foreground_line_work_bounded(&rope, selection(0, end)));
    assert!(foreground_line_work_bounded(&rope, selection(end, 0)));
    assert!(foreground_line_work_bounded(&Rope::new(), selection(usize::MAX, usize::MAX)));
}

fn selection(anchor: usize, head: usize) -> Selection {
    Selection {
        anchor,
        head,
        is_dragging: false,
    }
}

fn apply(mut rope: Rope, plan: &Plan) -> String {
    for edit in plan.edits.iter().rev() {
        let range = rope.byte_to_char(edit.range.start)..rope.byte_to_char(edit.range.end);
        rope.remove(range);
        rope.insert(rope.byte_to_char(edit.range.start), &edit.text);
    }
    rope.to_string()
}

#[test]
fn line_lookup_keeps_unicode_separators_inside_cr_lf_lines() {
    let text = "a\u{2028}😀\u{85}b\r\nc\u{b}d\rf\n";
    let rope = Rope::from_str(text);
    let first_end = text.find('c').unwrap();
    for byte in 0..first_end {
        let line = line_at(&rope, byte);
        assert_eq!(
            (line.start, line.body_end, line.end),
            (0, first_end - 2, first_end),
            "at {byte}"
        );
    }
    assert_eq!(line_at(&rope, usize::MAX).start, text.len());
    let line = line_at(&Rope::new(), usize::MAX);
    assert_eq!((line.start, line.body_end, line.end), (0, 0, 0));
}

#[test]
fn multiline_indent_excludes_the_line_at_a_column_zero_endpoint_and_skips_empty_lines() {
    for reverse in [false, true] {
        let rope = Rope::from_str("a\r\n\r\n \t😀\rq");
        let end = rope.to_string().find('q').unwrap();
        let sel = if reverse {
            selection(end, 1)
        } else {
            selection(1, end)
        };
        let plan = indent_plan(&rope, sel, false, 4, true).unwrap();
        let output = apply(rope, &plan);
        assert_eq!(output, "    a\r\n\r\n        😀\rq");
        let end = output.find('q').unwrap();
        assert_eq!(
            (plan.selection.anchor, plan.selection.head),
            if reverse { (end, 0) } else { (0, end) }
        );
    }
}

#[test]
fn single_line_prefix_mapping_keeps_endpoints_in_growing_indent_and_clamps_removed_indent() {
    let rope = Rope::from_str(" \té😀");
    let grow = indent_plan(&rope, selection(1, 2), true, 4, true).unwrap();
    assert_eq!(apply(rope.clone(), &grow), "\t\té😀");
    assert_eq!((grow.selection.anchor, grow.selection.head), (1, 2));
    let shrink = indent_plan(&rope, selection(1, 4), false, 4, false).unwrap();
    assert_eq!(apply(rope, &shrink), "é😀");
    assert_eq!((shrink.selection.anchor, shrink.selection.head), (0, 2));
}

#[test]
fn unindented_decrease_is_a_noop_and_single_empty_line_increases() {
    let rope = Rope::from_str("😀");
    assert!(indent_plan(&rope, selection(0, 4), false, 4, false)
        .unwrap()
        .edits
        .is_empty());
    let rope = Rope::new();
    let plan = indent_plan(&rope, selection(0, 0), true, 4, true).unwrap();
    assert_eq!(apply(rope, &plan), "\t");
    assert_eq!(plan.selection.head, 1);
}

#[test]
fn duplicate_uses_configured_eol_and_keeps_existing_terminators_and_original_caret() {
    for (text, at, eol, want) in [
        ("é😀\r\nq", 2, "\n", "é😀\né😀\r\nq"),
        ("a\rb", 3, "\r\n", "a\rb\r\nb"),
        ("a\n", 2, "\r", "a\n\r"),
        ("", 0, "\r\n", "\r\n"),
        ("x\u{2028}y\nq", 4, "\n", "x\u{2028}y\nx\u{2028}y\nq"),
    ] {
        let rope = Rope::from_str(text);
        let plan = duplicate_plan(&rope, selection(at, at), eol).unwrap();
        assert_eq!(apply(rope, &plan), want);
        assert_eq!(plan.selection.head, at);
    }
}

#[test]
fn duplicate_multiline_selection_moves_only_endpoints_after_its_caret_line() {
    let rope = Rope::from_str("abc\ndef\nq");
    let plan = duplicate_plan(&rope, selection(9, 1), "\n").unwrap();
    assert_eq!(apply(rope, &plan), "abc\nabc\ndef\nq");
    assert_eq!((plan.selection.anchor, plan.selection.head), (13, 1));
}

#[test]
fn hostile_offsets_and_widths_are_bounded_without_changing_text() {
    let rope = Rope::from_str(" \té😀\r\nx");
    for anchor in [0, 1, 3, 5, usize::MAX] {
        for head in [0, 2, 4, 6, usize::MAX] {
            let plan = indent_plan(&rope, selection(anchor, head), false, 4, true).unwrap();
            let output = apply(rope.clone(), &plan);
            assert!(output.is_char_boundary(plan.selection.anchor));
            assert!(output.is_char_boundary(plan.selection.head));
            let plan = duplicate_plan(&rope, selection(anchor, head), "\n").unwrap();
            let output = apply(rope.clone(), &plan);
            assert!(output.is_char_boundary(plan.selection.anchor));
            assert!(output.is_char_boundary(plan.selection.head));
        }
    }
    for width in [0, 33, usize::MAX] {
        assert!(indent_plan(&rope, selection(0, 0), false, width, true).is_err());
    }
    let rope = Rope::from_str(&" ".repeat(MAX_PREFIX_BYTES + 1));
    assert!(indent_plan(&rope, selection(0, 0), true, 4, true).is_err());
}

#[test]
fn oversized_line_batch_refuses_before_allocating_edits_and_endpoint_cap_is_exact() {
    let rope = Rope::from_str(&"a\n".repeat(MAX_LINE_EDITS + 1));
    assert!(indent_plan(&rope, selection(0, rope.len_bytes()), false, 4, true).is_err());
    let end = rope.line_to_byte(MAX_LINE_EDITS - 1) + 1;
    let plan = indent_plan(&rope, selection(0, end), false, 4, true).unwrap();
    assert_eq!(plan.edits.len(), MAX_LINE_EDITS);
    let plan = indent_plan(
        &rope,
        selection(0, rope.line_to_byte(MAX_LINE_EDITS)),
        false,
        4,
        true,
    )
    .unwrap();
    assert_eq!(plan.edits.len(), MAX_LINE_EDITS);
}

#[test]
fn persisted_new_menu_events_round_trip_without_renaming_legacy_events() {
    for event in [
        MacroEvent::IncreaseLineIndent,
        MacroEvent::DecreaseLineIndent,
        MacroEvent::DuplicateCurrentLine,
        MacroEvent::InsertTab,
        MacroEvent::DuplicateLine,
    ] {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Saved {
            events: Vec<MacroEvent>,
        }
        let encoded = toml::to_string(&Saved {
            events: vec![event.clone()],
        })
        .unwrap();
        let decoded: Saved = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.events.len(), 1);
        assert_eq!(
            std::mem::discriminant(&decoded.events[0]),
            std::mem::discriminant(&event)
        );
    }
}
