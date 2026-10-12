// G1 source preparation. Opening uses the labeled test-only action types.
// The baseline fails at the absent dialog, before Enter/Paste is exercised.
// These are future dialog-input contracts, not reproduced native defects.
use super::*;

fn open_text_dialog(source: &mut VisualTestContext, action: Box<dyn Action>) -> VisualTestContext {
    let previous = source.cx.windows();
    source.update(|window, cx| window.dispatch_action(action, cx)); source.run_until_parked();
    let handle = source.cx.windows().into_iter().find(|window| !previous.contains(window))
        .expect("test-only Generate action has no text dialog handler on the baseline");
    let mut dialog = VisualTestContext::from_window(handle, &source.cx);
    dialog.update(|window, cx| { window.activate_window(); window.refresh(); let _ = window.draw(cx); });
    dialog.run_until_parked(); dialog
}

fn assert_input(dialog: &mut VisualTestContext, expected: &str) {
    dialog.update(|window, cx| {
        window.refresh(); let _ = window.draw(cx);
        let mut input = window.take_registered_input_handler().expect("painted Hash input protocol");
        assert!(input.accepts_text_input(window, cx));
        let mut adjusted = None;
        assert_eq!(input.text_for_range(0..expected.encode_utf16().count(), &mut adjusted, window, cx).as_deref(),
            Some(expected), "whole-text hashing must receive these exact input bytes");
        assert_eq!(input.marked_text_range(window, cx), None);
    });
}

fn commit_input(dialog: &mut VisualTestContext, text: &str) {
    dialog.update(|window, cx| {
        let mut input = window.take_registered_input_handler().expect("painted Hash input protocol");
        input.replace_text_in_range(None, text, window, cx);
        window.refresh(); let _ = window.draw(cx);
    });
    dialog.run_until_parked();
}

#[test]
fn hash_tools_g1_typed_enter_protocol_requires_hard_crlf_for_every_text_algorithm() {
    for action in [ToolsMd5Generate.boxed_clone(), ToolsSha1Generate.boxed_clone(),
        ToolsSha256Generate.boxed_clone(), ToolsSha512Generate.boxed_clone()] {
        with_editor(|workspace, source| {
            let before = draft(workspace, source);
            let mut dialog = open_text_dialog(source, action);
            dialog.simulate_keystrokes("a enter b"); assert_input(&mut dialog, "a\r\nb");
            assert!(draft(workspace, source) == before);
            assert_eq!(dialog.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("sentinel"));
        });
    }
}

#[test]
fn hash_tools_g1_enter_replaces_selected_dialog_text_with_one_crlf() {
    with_editor(|workspace, source| {
        let before = draft(workspace, source);
        let mut dialog = open_text_dialog(source, ToolsMd5Generate.boxed_clone());
        dialog.simulate_keystrokes("a b c"); dialog.dispatch_action(EditSelectAll);
        dialog.simulate_keystrokes("enter"); assert_input(&mut dialog, "\r\n");
        assert!(draft(workspace, source) == before);
        assert_eq!(dialog.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("sentinel"));
    });
}

#[test]
fn hash_tools_g1_native_unicode_commit_then_typed_enter_retains_crlf_and_utf16_geometry() {
    with_editor(|workspace, source| {
        let before = draft(workspace, source);
        let mut dialog = open_text_dialog(source, ToolsMd5Generate.boxed_clone());
        commit_input(&mut dialog, "é😀"); dialog.simulate_keystrokes("enter x");
        dialog.update(|window, cx| {
            window.refresh(); let _ = window.draw(cx);
            let mut input = window.take_registered_input_handler().expect("painted Hash input protocol");
            let selection = input.selected_text_range(false, window, cx).expect("dialog caret");
            assert_eq!(selection.range, 6..6, "native UTF-16 caret includes both hard-EOL units");
            let mut adjusted = None;
            assert_eq!(input.text_for_range(0..6, &mut adjusted, window, cx).as_deref(), Some("é😀\r\nx"));
        });
        assert!(draft(workspace, source) == before);
        assert_eq!(dialog.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("sentinel"));
    });
}

#[test]
fn hash_tools_g1_proposed_raw_paste_protocol_retains_lf_and_mixed_bytes_separately_from_enter() {
    // Exact raw LF/mixed native EDIT paste parity remains unverified. This
    // specifies lossless GPUI input; it is not proof of Win32 conversion.
    for pasted in ["a\nb", "é\r\nx\ny\rz\r\r\n"] {
        with_editor(|workspace, source| {
            let before = draft(workspace, source);
            let mut dialog = open_text_dialog(source, ToolsMd5Generate.boxed_clone());
            dialog.write_to_clipboard(ClipboardItem::new_string(pasted.into()));
            dialog.dispatch_action(EditPaste); dialog.run_until_parked();
            dialog.simulate_keystrokes("enter");
            assert_input(&mut dialog, &format!("{pasted}\r\n"));
            assert_eq!(dialog.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(pasted));
            assert!(draft(workspace, source) == before);
        });
    }
}
