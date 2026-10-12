use super::*;
use crate::{edit_ops::legacy_lines::override_seed, text::encoding::Encoding};

// Golden fixture bytes use the chosen encoding directly, independently of
// the Rope-to-disk writer and its format normalization.
fn fixture_bytes(text: &str,encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => [vec![0xef,0xbb,0xbf],text.as_bytes().to_vec()].concat(),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let little=encoding==Encoding::Utf16Le;
            let mut bytes=if little { vec![0xff,0xfe] } else { vec![0xfe,0xff] };
            for unit in text.encode_utf16() { bytes.extend(if little {unit.to_le_bytes()}else{unit.to_be_bytes()}); }
            bytes
        }
        Encoding::Latin1 => text.chars().map(|c| if c=='é' {0xe9} else {assert!(c.is_ascii());c as u8}).collect(),
    }
}

#[test]
fn legacy_all_twelve_encoding_eol_open_save_and_undo_redo_bytes() {
    for encoding in [Encoding::Utf8,Encoding::Utf8Bom,Encoding::Utf16Le,Encoding::Utf16Be,Encoding::Latin1] {
        for ending in ["\n","\r\n","\r"] {
            for (index,case) in CASES.iter().enumerate() {
                let _seed=override_seed(1);
                let source=case.source.split('\n').map(|row| format!("{row}é")).collect::<Vec<_>>().join(ending);
                let expected_base=if index==9 { "a\nb\nc" } else { case.expected };
                let expected=expected_base.split('\n').map(|row| format!("{row}é")).collect::<Vec<_>>().join("\n");
                let disk_expected=expected.replace('\n',ending);
                let directory=crate::persist::TempDir::new("legacy-line-format"); let path=directory.join("neutral.txt");
                let original=fixture_bytes(&source,encoding); let sorted=fixture_bytes(&disk_expected,encoding);
                std::fs::write(&path,&original).unwrap();
                with_source("",|workspace,visual| {
                    visual.update_window_entity(workspace,|w,_,cx| w.open_path(path.clone(),cx));
                    visual.run_until_parked();
                    visual.update(|_,cx| { let w=workspace.read(cx); assert_eq!(w.active,0); assert_eq!(w.tabs.len(),1);
                        assert_eq!(w.buffer().encoding,encoding); assert_eq!(w.buffer().rope.to_string(),source.replace(ending,"\n")); });
                    assert_eq!(std::fs::read(&path).unwrap(),original);
                    visual.dispatch_action(FileSave); visual.run_until_parked();
                    assert_eq!(std::fs::read(&path).unwrap(),original,"unchanged open/save bytes");
                    let before=state(workspace,visual); invoke(case,visual);
                    let status=visual.update(|_,cx| workspace.read(cx).status_message.clone());
                    assert_eq!(text(workspace,visual),expected,"{} {encoding:?} {ending:?} {status:?}",case.id);
                    let after=state(workspace,visual); assert_eq!((after.encoding,after.eol),(before.encoding,before.eol));
                    assert_eq!(after.undo,1);
                    visual.dispatch_action(FileSave); visual.run_until_parked();
                    assert_eq!(std::fs::read(&path).unwrap(),sorted);
                    visual.dispatch_action(EditUndo); visual.run_until_parked();
                    let undone=state(workspace,visual); assert_eq!(undone.text,before.text); assert_eq!(undone.selection,before.selection);
                    assert_eq!(undone.block,before.block); assert_eq!((undone.encoding,undone.eol),(before.encoding,before.eol));
                    visual.dispatch_action(FileSave); visual.run_until_parked(); assert_eq!(std::fs::read(&path).unwrap(),original);
                    visual.dispatch_action(EditRedo); visual.run_until_parked(); let redone=state(workspace,visual);
                    assert_eq!(redone.text,expected); assert_eq!(redone.selection,after.selection); assert_eq!(redone.block,after.block);
                    assert_eq!((redone.encoding,redone.eol),(before.encoding,before.eol));
                    visual.dispatch_action(FileSave); visual.run_until_parked(); assert_eq!(std::fs::read(&path).unwrap(),sorted);
                });
            }
        }
    }
}

#[test]
fn legacy_rectangle_keys_distinguish_unicode_normalized_and_ansi_offsets() {
    // A corner after é is offset 2 in normalized UTF8 and offset 1 in ANSI.
    // Those offsets index the sorter's UTF16 keys, as the older oracle does.
    for encoding in [Encoding::Utf8,Encoding::Utf8Bom,Encoding::Utf16Le,Encoding::Utf16Be,Encoding::Latin1] {
        for index in [0,1,2,3,4,5,6,7] {
            let source="outside\né29\né31\nend";
            let ascending=index%2==0; let ansi=encoding==Encoding::Latin1;
            let expected=if ascending==ansi { source } else { "outside\né31\né29\nend" };
            with_source(source,|workspace,visual| {
                visual.update_window_entity(workspace,|w,_,_| {
                    w.tabs[0].buffer.encoding=encoding;
                    w.tabs[0].block_selection=Some(BlockSelection { anchor_line:1,head_line:2,anchor_col:1,head_col:3,is_dragging:false });
                });
                let before=state(workspace,visual); invoke(&CASES[index],visual); assert_eq!(text(workspace,visual),expected);
                if expected==source { assert!(state(workspace,visual)==before); }
                else {
                    assert_eq!(state(workspace,visual).block,Some((1,1,0,0,false)));
                    visual.dispatch_action(EditUndo); visual.run_until_parked(); let after=state(workspace,visual);
                    assert_eq!(after.text,before.text); assert_eq!(after.block,before.block); assert_eq!(after.selection,before.selection);
                }
            });
        }
    }
}

#[test]
fn legacy_consecutive_rectangle_geometry_uses_internal_utf8_for_every_encoding() {
    for encoding in [Encoding::Utf8,Encoding::Utf8Bom,Encoding::Utf16Le,Encoding::Utf16Be,Encoding::Latin1] {
        with_source("outside\né\né\nz\nend",|workspace,visual| {
            visual.update_window_entity(workspace,|w,_,_| {
                w.tabs[0].buffer.encoding=encoding;
                w.tabs[0].block_selection=Some(BlockSelection { anchor_line:1,head_line:3,anchor_col:1,head_col:1,is_dragging:false });
                w.tabs[0].selection.anchor=10; w.tabs[0].selection.head=15;
            });
            let before=state(workspace,visual); invoke(&CASES[11],visual); assert_eq!(text(workspace,visual),"outside\né\nz\nend");
            assert_eq!(state(workspace,visual).block,Some((1,2,1,1,false)));
            assert_eq!(state(workspace,visual).selection,(10,12,false));
            visual.dispatch_action(EditUndo); visual.run_until_parked(); let after=state(workspace,visual);
            assert_eq!(after.block,before.block); assert_eq!(after.selection,before.selection); assert_eq!(after.text,before.text);
        });
    }
}
