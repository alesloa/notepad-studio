// Menu bar keyboard entry points on Windows and Linux (`crate::ui::menu_bar`).
// These are the Win32 menu-bar keys, not Notepad++ commands, so they stay out
// of the Shortcut Mapper catalog.

// `MenuBarAltTap`: Alt pressed and released alone. `MenuBarActivate`: F10.
gpui::actions!(notepad_studio, [MenuBarAltTap, MenuBarActivate]);

/// Alt+<key>: open the top-level menu whose title carries `key`.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = notepad_studio, no_json)]
pub struct MenuBarOpen {
    pub key: char,
}
