// File menu + app-lifecycle actions.

use std::path::PathBuf;

gpui::actions!(
    notepad_studio,
    [
        Quit,
        FileNewWindow,
        FileNew,
        FileOpen,
        FileOpenContainingFolder,
        FileOpenDefaultViewer,
        FileOpenFolderAsWorkspace,
        FileReloadFromDisk,
        FileSave,
        FileSaveAs,
        FileSaveAll,
        FileCloseTab,
        FileCloseAllTabs,
        FileCloseAllButActive,
        FileCloseAllToLeft,
        FileCloseAllToRight,
        FileCloseAllUnchanged,
        FileRestoreClosedTab,
        FileClearRecent,
    ]
);

/// Action emitted by every File ▸ Open Recent submenu item. Carries the path
/// to open as a payload so the same action handler routes every entry; the
/// dynamic File menu builder constructs one of these per remembered path.
/// `no_json` because we never construct it from a keymap — only from the
/// menu.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = notepad_studio, no_json)]
pub struct FileOpenRecent {
    pub path: PathBuf,
}
