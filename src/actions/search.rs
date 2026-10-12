// Search menu actions — Find / Replace / Goto Line / Find in Files / Mark.
//
// The `Edit*` prefix on the find/replace/goto types is a leftover from when
// they lived alongside the rest of the editor verbs; the dialog implementation
// reuses them rather than renaming, which keeps `Search ▸ Find` keybindings
// stable for users who already memorised the chord.

gpui::actions!(
    notepad_studio,
    [
        // Open the dialog targeted at a specific tab. Each maps to a menu
        // item / keybinding; existing callers reuse `EditFind` / `EditReplace`.
        EditFind,
        EditFindClose,
        EditFindNext,
        EditFindPrev,
        EditFindCount,
        EditFindAllCurrent,
        EditFindAllOpened,
        EditReplace,
        EditReplaceCurrent,
        EditReplaceAll,
        EditReplaceAllOpened,
        EditGotoLine,
        SearchGoToMatchingBrace,
        SearchSelectInBetween,
        EditFindToggleCase,
        EditFindToggleWord,
        EditFindToggleRegex,
        EditFindToggleExtended,
        EditFindToggleWrap,
        EditFindToggleReplace,
        SearchFindInFiles,
        FindInFilesClose,
        FindInFilesRun,
        FindInFilesReplace,
        FindInFilesPickDir,
        FindInFilesToggleCase,
        FindInFilesToggleWord,
        FindInFilesToggleRegex,
        FindInFilesToggleExtended,
        FindInFilesToggleFollow,
        FindInFilesToggleSubfolders,
        FindInFilesToggleHidden,
        FindInFilesSwap,
        FindInFilesToggleTransparency,
        FindInFilesTransparencyOnFocusLoss,
        FindInFilesTransparencyAlways,
        FindInFilesTransparencyDec,
        FindInFilesTransparencyInc,
        // Find in Projects tab — NPP's "Find in Projects". Shares
        // query/replace/filter/options state with FIF; only the scope
        // changes (sidebar root(s) instead of an explicit Directory).
        SearchFindInProjects,
        FindInProjectsRun,
        FindInProjectsReplace,
        FindInProjectsTogglePanel1,
        FindInProjectsTogglePanel2,
        FindInProjectsTogglePanel3,
        FindInProjectsSwap,
        // Mark tab.
        SearchMark,
        MarkAll,
        MarkClearAll,
        MarkCopy,
        MarkToggleCase,
        MarkToggleWord,
        MarkToggleRegex,
        MarkToggleExtended,
        MarkToggleWrap,
        MarkTogglePurge,
        MarkToggleBookmarkLine,
    ]
);
