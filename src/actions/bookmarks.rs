// Bookmark actions — Notepad++-style line bookmarks. Lives under the Search
// menu (Notepad++ convention) so the action types share the search/navigation
// surface area conceptually even though the prefix differentiates them.

gpui::actions!(
    notepad_studio,
    [
        BookmarkToggle,
        BookmarkNext,
        BookmarkPrev,
        BookmarkClearAll,
        CopyBookmarkedLines,
    ]
);
