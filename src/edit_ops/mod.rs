// Notepad++ Edit menu submenu handlers. Each submodule is a stub-only impl
// block on `Workspace` so parallel implementation forks can fill them in
// without merge conflicts. Read-Only and Begin/End Select markers are the
// exceptions — those are wired up in this round.

pub(crate) mod begin_end_select;
pub(crate) mod blank_ops;
pub(crate) mod bookmark_clipboard;
pub(crate) mod bookmark_edits;
pub(crate) mod case_ops;
pub(crate) mod clipboard_all;
pub(crate) mod clipboard_ops;
pub(crate) mod comment_ops;
pub(crate) mod delimiter_select;
pub(crate) mod editor_keys;
pub(crate) mod indent_ops;
pub(crate) mod insert;
pub(crate) mod line_ops;
pub(crate) mod line_sort;
pub(crate) mod menu_line_commands;
pub(crate) mod on_selection;
pub(crate) mod readonly;
pub(crate) mod trim_trailing_scope;
pub(crate) mod util;

#[cfg(test)]
mod trim_trailing_scope_tests;

#[cfg(test)]
mod bookmark_edits_tests;
