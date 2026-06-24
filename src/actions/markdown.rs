// Markdown formatting toolbar actions. Wired from the Markdown formatting
// group in `src/ui/toolbar.rs`; impls live in `src/workspace/markdown_format.rs`.

gpui::actions!(
    notepad_studio,
    [
        MdBold,
        MdItalic,
        MdStrikethrough,
        MdInlineCode,
        MdHeading1,
        MdHeading2,
        MdHeading3,
        MdBulletList,
        MdOrderedList,
        MdTaskList,
        MdBlockQuote,
        MdCodeBlock,
        MdLink,
        MdImage,
        MdHorizontalRule,
    ]
);
