// Rich-document toolbar actions. View toggles + Phase 2 formatting (Bold /
// Italic / Underline) target the active block on the rich-doc payload.
// Wired in `src/ui/toolbar.rs`; impls live in `src/workspace/edits.rs`.

gpui::actions!(
    notepad_studio,
    [
        RichDocView,
        RichDocRaw,
        RichDocBold,
        RichDocItalic,
        RichDocUnderline,
        RichDocStrikethrough,
        RichDocColorOpen,
        RichDocColorClear,
        RichDocLinkOpen,
        RichDocLinkClear,
        RichDocSaveAsDocx,
        RichDocPromoteHeading,
        RichDocDemoteHeading,
    ]
);
