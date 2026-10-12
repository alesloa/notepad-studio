// Edit menu labels and grouping follow PowerEditor/src/Notepad_plus.rc:488.
// Remaining command gaps are tracked in docs/notepadpp-parity-inventory.md.

use gpui::{Menu, MenuItem};

use crate::actions::*;
use super::MenuState;

#[cfg(test)]
#[path = "edit_line_command_tests.rs"]
mod line_command_tests;

#[cfg(test)]
#[path = "edit_clipboard_all_tests.rs"]
mod clipboard_all_tests;

#[cfg(test)]
#[path = "edit_line_sort_tests.rs"]
mod line_sort_tests;

#[cfg(test)]
#[path = "edit_legacy_line_tests.rs"]
mod legacy_line_tests;

pub(crate) fn build(_state: &MenuState) -> Menu {
    Menu {
        name: "&Edit".into(),
        items: vec![
            MenuItem::action("Undo", EditUndo),
            MenuItem::action("Redo", EditRedo),
            MenuItem::separator(),
            MenuItem::action("Cut", EditCut),
            MenuItem::action("Copy", EditCopy),
            MenuItem::action("Paste", EditPaste),
            MenuItem::action("Delete", EditDelete),
            MenuItem::separator(),
            MenuItem::action("Select All", EditSelectAll),
            MenuItem::action("Begin/End Select", EditBeginEndSelect),
            MenuItem::action("Begin/End Select in Column Mode", EditBeginEndSelectColumnMode),
            MenuItem::separator(),
            MenuItem::submenu(insert_menu()),
            MenuItem::submenu(copy_to_clipboard_menu()),
            MenuItem::submenu(indent_menu()),
            MenuItem::submenu(convert_case_menu()),
            MenuItem::submenu(line_operations_menu()),
            MenuItem::submenu(comment_menu()),
            MenuItem::submenu(autocompletion_menu()),
            MenuItem::submenu(eol_conversion_menu()),
            MenuItem::submenu(blank_operations_menu()),
            MenuItem::submenu(on_selection_menu()),
            MenuItem::separator(),
            MenuItem::action("Text Tools…", EditOpenTextTools),
            MenuItem::action("Clipboard History…", EditOpenClipboardHistory),
            MenuItem::action("Character Panel…", EditOpenCharacterPanel),
            MenuItem::submenu(gpui::Menu {
                name: "Spelling and Grammar".into(),
                items: vec![
                    MenuItem::action("Check Spelling", EditToggleSpelling)
                        .checked(crate::settings::snapshot().proofread_spelling),
                    MenuItem::action("Check Grammar", EditToggleGrammar)
                        .checked(crate::settings::snapshot().proofread_grammar),
                ],
            }),
            MenuItem::separator(),
            MenuItem::action("Set Read-Only", EditToggleReadOnly),
            MenuItem::action("Clear Read-Only Flag", EditClearReadOnlyFlag),
        ],
    }
}

fn insert_menu() -> Menu {
    Menu {
        name: "Insert".into(),
        items: vec![
            MenuItem::action("Date Time (short)", EditInsertDateTimeShort),
            MenuItem::action("Date Time (long)", EditInsertDateTimeLong),
            MenuItem::action("Date Time (customized)", EditInsertDateTimeCustomized),
        ],
    }
}

fn copy_to_clipboard_menu() -> Menu {
    Menu {
        name: "Copy to Clipboard".into(),
        items: vec![
            MenuItem::action("Current Full File path to Clipboard", EditCopyFullPath),
            MenuItem::action("Current Filename to Clipboard", EditCopyFilename),
            MenuItem::action("Current Dir. Path to Clipboard", EditCopyDirPath),
            // PowerEditor/src/Notepad_plus.rc:510-512.
            MenuItem::separator(),
            MenuItem::action("Copy All Filenames", EditCopyAllFilenames),
            MenuItem::action("Copy All File Paths", EditCopyAllFilePaths),
        ],
    }
}

fn indent_menu() -> Menu {
    Menu {
        name: "Indent".into(),
        items: vec![
            MenuItem::action("Increase Line Indent", EditIncreaseLineIndent),
            MenuItem::action("Decrease Line Indent", EditDecreaseLineIndent),
        ],
    }
}

fn convert_case_menu() -> Menu {
    Menu {
        name: "Convert Case to".into(),
        items: vec![
            MenuItem::action("UPPERCASE", EditUppercase),
            MenuItem::action("lowercase", EditLowercase),
            MenuItem::separator(),
            MenuItem::action("Proper Case", EditProperCaseForce),
            MenuItem::action("Proper Case (blend)", EditProperCaseBlend),
            MenuItem::action("Sentence case", EditSentenceCaseForce),
            MenuItem::action("Sentence case (blend)", EditSentenceCaseBlend),
            MenuItem::separator(),
            MenuItem::action("iNVERT cASE", EditInvertCase),
            MenuItem::action("rANdOm cASe", EditRandomCase),
        ],
    }
}

fn line_operations_menu() -> Menu {
    Menu {
        name: "Line Operations".into(),
        items: vec![
            MenuItem::action("Duplicate Current Line", EditDuplicateCurrentLine),
            MenuItem::action("Remove Duplicate Lines", EditRemoveDuplicateLines),
            MenuItem::action(
                "Remove Consecutive Duplicate Lines",
                EditRemoveConsecutiveDuplicateLines,
            ),
            MenuItem::action("Split Lines", EditSplitLines),
            MenuItem::action("Join Lines", EditJoinLines),
            MenuItem::action("Move Up Current Line", EditMoveLineUp),
            MenuItem::action("Move Down Current Line", EditMoveLineDown),
            MenuItem::action("Remove Empty Lines", EditRemoveEmptyLines),
            MenuItem::action(
                "Remove Empty Lines (Containing Blank chars)",
                EditRemoveEmptyLinesContainingBlank,
            ),
            MenuItem::action("Insert Blank Line Above Current", EditInsertBlankLineAbove),
            MenuItem::action("Insert Blank Line Below Current", EditInsertBlankLineBelow),
            MenuItem::action("Reverse Line Order", EditReverseLineOrder),
            MenuItem::action("Randomize Line Order", EditRandomizeLineOrder),
            MenuItem::separator(),
            MenuItem::action("Sort Lines Lexicographically Ascending", EditSortLinesLexAsc),
            MenuItem::action("Sort Lines Lex. Ascending Ignoring Case", EditSortLinesLexIgnoreCaseAsc),
            MenuItem::action("Sort Lines In Locale Order Ascending", EditSortLinesLocaleAsc),
            MenuItem::action("Sort Lines As Integers Ascending", EditSortLinesIntAsc),
            MenuItem::action("Sort Lines As Decimals (Comma) Ascending", EditSortLinesDecCommaAsc),
            MenuItem::action("Sort Lines As Decimals (Dot) Ascending", EditSortLinesDecDotAsc),
            MenuItem::action("Sort Lines By Length Ascending", EditSortLinesLengthAsc),
            MenuItem::separator(),
            MenuItem::action("Sort Lines Lexicographically Descending", EditSortLinesLexDesc),
            MenuItem::action("Sort Lines Lex. Descending Ignoring Case", EditSortLinesLexIgnoreCaseDesc),
            MenuItem::action("Sort Lines In Locale Order Descending", EditSortLinesLocaleDesc),
            MenuItem::action("Sort Lines As Integers Descending", EditSortLinesIntDesc),
            MenuItem::action("Sort Lines As Decimals (Comma) Descending", EditSortLinesDecCommaDesc),
            MenuItem::action("Sort Lines As Decimals (Dot) Descending", EditSortLinesDecDotDesc),
            MenuItem::action("Sort Lines By Length Descending", EditSortLinesLengthDesc),
        ],
    }
}

fn comment_menu() -> Menu {
    Menu {
        name: "Comment/Uncomment".into(),
        items: vec![
            MenuItem::action("Toggle Block Comment", EditBlockComment),
            MenuItem::action("Block Comment", EditBlockCommentSet),
            MenuItem::action("Block Uncomment", EditBlockUncomment),
            MenuItem::separator(),
            MenuItem::action("Toggle Stream Comment", EditStreamComment),
            MenuItem::action("Stream Uncomment", EditStreamUncomment),
        ],
    }
}

fn autocompletion_menu() -> Menu {
    Menu {
        name: "Auto-Completion".into(),
        items: vec![
            MenuItem::action("Function Completion", EditAutocompleteFunc),
            MenuItem::action("Word Completion", EditAutocompleteWord),
            MenuItem::action("Function and Word Completion", EditAutocompleteFuncAndWord),
            MenuItem::separator(),
            MenuItem::action("Function Parameters Hint", EditAutocompleteFuncParams),
            MenuItem::action("Function Parameters Previous Hint", EditAutocompleteFuncParamsPrevious),
            MenuItem::action("Function Parameters Next Hint", EditAutocompleteFuncParamsNext),
        ],
    }
}

fn eol_conversion_menu() -> Menu {
    Menu {
        name: "EOL Conversion".into(),
        items: vec![
            MenuItem::action("Windows (CR LF)", EditEolWindows),
            MenuItem::action("Unix (LF)", EditEolUnix),
            MenuItem::action("Macintosh (CR)", EditEolMacintosh),
        ],
    }
}

fn blank_operations_menu() -> Menu {
    Menu {
        name: "Blank Operations".into(),
        items: vec![
            MenuItem::action("Trim Trailing Space", EditTrimTrailingWs),
            MenuItem::action("Trim Leading Space", EditTrimLeadingWs),
            MenuItem::action(
                "Trim Leading and Trailing Space",
                EditTrimLeadingAndTrailingWs,
            ),
            MenuItem::action("EOL to Space", EditEolToSpace),
            MenuItem::action(
                "Remove Unnecessary Blank and EOL",
                EditRemoveUnnecessaryBlankAndEol,
            ),
            MenuItem::separator(),
            MenuItem::action("TAB to Space", EditTabToSpace),
            MenuItem::action("Space to TAB (All)", EditSpaceToTabAll),
            MenuItem::action("Space to TAB (Leading)", EditSpaceToTabLeading),
            MenuItem::separator(),
            MenuItem::action("Insert Blank Line Above", EditInsertBlankLineAbove),
            MenuItem::action("Insert Blank Line Below", EditInsertBlankLineBelow),
        ],
    }
}

fn on_selection_menu() -> Menu {
    Menu {
        name: "On Selection".into(),
        items: vec![
            MenuItem::action("Open File", EditOpenSelectedFile),
            MenuItem::action("Open in New Instance", EditOpenSelectedInNewInstance),
            MenuItem::action("Open Containing Folder in Explorer", EditOpenContainingFolder),
            MenuItem::action(
                "Open Containing Folder in cmd",
                EditOpenContainingFolderInTerminal,
            ),
            MenuItem::action("Search on Internet", EditSearchSelectionOnInternet),
            MenuItem::action("Send via Email", EditSendSelectionViaEmail),
        ],
    }
}
