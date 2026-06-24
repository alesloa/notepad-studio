// Notepad++ Edit menu submenu actions. Each `actions!` block stays grouped by
// submenu so parallel implementation forks can find and fill them without
// stepping on each other. UPPERCASE / lowercase / Toggle Block / Toggle Stream
// / Trim Trailing / EOL conversion / Move Line / Duplicate Line / Insert Blank
// Line / Delete Line live in `actions::edit` already and are not redeclared.

// Insert ▶
gpui::actions!(
    notepad_studio,
    [
        EditInsertDateTimeShort,
        EditInsertDateTimeLong,
        EditInsertDateTimeCustomized,
    ]
);

// Copy to Clipboard ▶
gpui::actions!(
    notepad_studio,
    [
        EditCopyFullPath,
        EditCopyFilename,
        EditCopyDirPath,
    ]
);

// Indent ▶ (Tab / Shift+Tab already exist as EditInsertTab; outdent is new)
gpui::actions!(
    notepad_studio,
    [
        EditOutdent,
    ]
);

// Convert Case to ▶ (UPPERCASE / lowercase already exist)
gpui::actions!(
    notepad_studio,
    [
        EditProperCaseForce,
        EditProperCaseBlend,
        EditSentenceCaseForce,
        EditSentenceCaseBlend,
        EditInvertCase,
        EditRandomCase,
    ]
);

// Line Operations ▶ (Duplicate / Move Up / Move Down already exist)
gpui::actions!(
    notepad_studio,
    [
        EditRemoveDuplicateLines,
        EditRemoveConsecutiveDuplicateLines,
        EditSplitLines,
        EditJoinLines,
        EditRemoveEmptyLines,
        EditRemoveEmptyLinesContainingBlank,
        EditSortLinesLexAsc,
        EditSortLinesLexDesc,
        EditSortLinesIntAsc,
        EditSortLinesIntDesc,
        EditSortLinesDecCommaAsc,
        EditSortLinesDecCommaDesc,
        EditSortLinesDecDotAsc,
        EditSortLinesDecDotDesc,
        EditReverseLineOrder,
        EditRandomizeLineOrder,
    ]
);

// Comment / Uncomment ▶ (Toggle Block + Toggle Stream already exist)
gpui::actions!(
    notepad_studio,
    [
        EditBlockCommentSet,
        EditBlockUncomment,
        EditStreamUncomment,
    ]
);

// Blank Operations ▶ (Trim Trailing already exists)
gpui::actions!(
    notepad_studio,
    [
        EditTrimLeadingWs,
        EditTrimLeadingAndTrailingWs,
        EditEolToSpace,
        EditRemoveUnnecessaryBlankAndEol,
        EditTabToSpace,
        EditSpaceToTabAll,
        EditSpaceToTabLeading,
    ]
);

// On Selection ▶
gpui::actions!(
    notepad_studio,
    [
        EditOpenSelectedFile,
        EditOpenSelectedInNewInstance,
        EditOpenContainingFolder,
        EditOpenContainingFolderInTerminal,
        EditSearchSelectionOnInternet,
        EditSendSelectionViaEmail,
    ]
);

// Begin / End Select markers
gpui::actions!(
    notepad_studio,
    [
        EditBeginEndSelect,
        EditBeginEndSelectColumnMode,
    ]
);

// Read-Only flag (fully implemented this round)
gpui::actions!(
    notepad_studio,
    [
        EditToggleReadOnly,
        EditClearReadOnlyFlag,
    ]
);

// Text Tools modal — Edit ▸ Text Tools…
gpui::actions!(
    notepad_studio,
    [
        EditOpenTextTools,
    ]
);

// Clipboard History panel — Edit ▸ Clipboard History
gpui::actions!(
    notepad_studio,
    [
        EditOpenClipboardHistory,
    ]
);

// Character Panel — Edit ▸ Character Panel
gpui::actions!(
    notepad_studio,
    [
        EditOpenCharacterPanel,
    ]
);
