use super::*;
use crate::text::{encoding::Encoding, line_ending::LineEnding};

fn expected_file(text: &str, encoding: Encoding, eol: LineEnding) -> Vec<u8> {
    let ending = match eol { LineEnding::Crlf => "\r\n", LineEnding::Lf => "\n", LineEnding::Cr => "\r" };
    let text = text.replace('\n', ending);
    match encoding {
        Encoding::Utf8 => text.into_bytes(),
        Encoding::Utf8Bom => { let mut bytes = vec![0xef, 0xbb, 0xbf]; bytes.extend_from_slice(text.as_bytes()); bytes },
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let mut bytes = if encoding == Encoding::Utf16Le { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
            for unit in text.encode_utf16() { bytes.extend_from_slice(&if encoding == Encoding::Utf16Le { unit.to_le_bytes() } else { unit.to_be_bytes() }); }
            bytes
        },
        Encoding::Latin1 => text.chars().map(|ch| u8::try_from(u32::from(ch)).expect("fixture Latin-1 character")).collect(),
    }
}

fn save_to(visual: &mut VisualTestContext, path: &Path) {
    let (reply, selected) = futures::channel::oneshot::channel();
    let _dialog = crate::io::SaveAsDialogOverrideForTest::new(selected);
    visual.dispatch_action(FileSave); visual.run_until_parked();
    reply.send(Some(path.to_path_buf())).expect("controlled Save As reply"); visual.run_until_parked();
}

#[test]
fn automatic_title_five_encodings_three_eols_preserve_exact_document_save_and_history() {
    for encoding in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for eol in [LineEnding::Crlf, LineEnding::Lf, LineEnding::Cr] {
            let directory = crate::persist::TempDir::new("automatic-title-formats"); let path = directory.join("neutral.txt");
            let text = if encoding == Encoding::Latin1 { "café\0hidden\nsecond" } else { "café\u{2028}中😀\0hidden\nsecond" };
            let expected_name = if encoding == Encoding::Latin1 { "café" } else { "café\u{2028}中😀" };
            let expected = expected_file(text, encoding, eol);
            let mut buffer = Buffer::empty(); buffer.encoding = encoding; buffer.line_ending = eol;
            buffer.new_label = Some("new 1".into());
            with_editor(buffer, |entity, visual| {
                let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
                let _settings = crate::settings::override_for_test(settings);
                automatic_title_paste(visual, text);
                assert_eq!(automatic_title_name(entity, visual), expected_name);
                let geometry = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0]; (tab.selection.anchor, tab.selection.head, tab.block_selection.map(|block| (block.anchor_line, block.anchor_col, block.head_line, block.head_col, block.is_dragging))) });
                save_to(visual, &path);
                assert_eq!(std::fs::read(&path).expect("saved bytes"), expected, "{encoding:?} {eol:?}");
                visual.update(|_, cx| {
                    let tab = &entity.read(cx).tabs[0]; assert_eq!(tab.buffer.rope.to_string(), text);
                    assert_eq!((tab.buffer.encoding, tab.buffer.line_ending), (encoding, eol));
                    assert!(tab.buffer.new_label.is_none()); assert!(!tab.buffer.new_label_is_custom);
                    assert_eq!((tab.selection.anchor, tab.selection.head, tab.block_selection.map(|block| (block.anchor_line, block.anchor_col, block.head_line, block.head_col, block.is_dragging))), geometry);
                    assert_eq!(tab.buffer.display_short_name().as_ref(), "neutral.txt"); assert!(!tab.buffer.dirty);
                });
                visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
                visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0]; assert_eq!(tab.buffer.rope.to_string(), "");
                    assert!(tab.buffer.dirty); assert_eq!(tab.buffer.display_short_name().as_ref(), "neutral.txt"); });
                visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
                visual.update(|_, cx| {
                    let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), text); assert!(!w.buffer().dirty);
                    assert_eq!((w.buffer().encoding, w.buffer().line_ending), (encoding, eol)); assert!(!w.automatic_titles_pending());
                });
                visual.dispatch_action(FileSave); visual.run_until_parked();
                assert_eq!(std::fs::read(&path).expect("unchanged save"), expected);
                let opened = crate::io::read_buffer(&path).expect("decode saved format");
                assert_eq!(opened.rope.to_string(), text); assert_eq!((opened.encoding, opened.line_ending), (encoding, eol));
            });
        }
    }
}

#[test]
fn automatic_title_surrogate_presentation_does_not_replace_utf16_document_scalar() {
    let text = format!("{}😀tail\nsecond", "a".repeat(62));
    let shown = format!("{}\u{fffd}", "a".repeat(62));
    let directory = crate::persist::TempDir::new("automatic-title-surrogate"); let path = directory.join("neutral.txt");
    let mut buffer = Buffer::empty(); buffer.encoding = Encoding::Utf16Le; buffer.line_ending = LineEnding::Crlf;
    with_editor(buffer, |entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _settings = crate::settings::override_for_test(settings);
        automatic_title_paste(visual, &text); assert_eq!(automatic_title_name(entity, visual), shown);
        save_to(visual, &path); assert_eq!(std::fs::read(&path).expect("UTF16 bytes"), expected_file(&text, Encoding::Utf16Le, LineEnding::Crlf));
        visual.update(|_, cx| { let buffer = &entity.read(cx).tabs[0].buffer;
            assert_eq!(buffer.rope.to_string(), text); assert!(buffer.new_label.is_none());
            assert_eq!(buffer.display_short_name().as_ref(), "neutral.txt"); });
    });
}
