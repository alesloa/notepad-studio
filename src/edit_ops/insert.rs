// Insert ▶ submenu — Date/Time short, long, customized.
//
// Notepad++ source:
//   PowerEditor/src/NppCommands.cpp:137 — IDM_EDIT_INSERT_DATETIME_SHORT/LONG
//   PowerEditor/src/NppCommands.cpp:174 — IDM_EDIT_INSERT_DATETIME_CUSTOMIZED
//
// NPP uses Windows GetDateFormatEx + GetTimeFormatEx with DATE_SHORTDATE /
// DATE_LONGDATE — locale-aware. SHORT routes through `crate::locale_date`
// which bridges to NSDateFormatter (macOS) / nl_langinfo (Linux) and falls
// back to ISO on Windows + unknown. LONG keeps the explicit long-form spec
// from the directive (no NPP equivalent surfaced enough to bridge yet).
// Customized reads its strftime template from `Settings::datetime_custom_format`
// (Preferences dialog ▸ Insert tab).

use chrono::format::{Item, StrftimeItems};
use gpui::{Context, Window};

use crate::actions::*;
use crate::tab::InputFocus;
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn insert_datetime_short(
        &mut self,
        _: &EditInsertDateTimeShort,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        // NPP's Windows path resolves the active locale's short-date pattern
        // via `GetDateFormatEx(DATE_SHORTDATE)` + `GetTimeFormatEx`. Mirrored
        // per-OS in `crate::locale_date`: macOS uses `NSDateFormatter`, Linux
        // uses `nl_langinfo(D_FMT/T_FMT)`, Windows + unknown fall back to
        // ISO `YYYY-MM-DD HH:MM`.
        let stamp = crate::locale_date::short_now();
        self.insert_text_in_editor(&stamp, cx);
        crate::macros::record(crate::macros::MacroEvent::InsertDateTimeShort);
    }

    pub(crate) fn insert_datetime_long(
        &mut self,
        _: &EditInsertDateTimeLong,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let stamp = chrono::Local::now()
            .format("%A, %B %-d, %Y %H:%M:%S")
            .to_string();
        self.insert_text_in_editor(&stamp, cx);
        crate::macros::record(crate::macros::MacroEvent::InsertDateTimeLong);
    }

    pub(crate) fn insert_datetime_customized(
        &mut self,
        _: &EditInsertDateTimeCustomized,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let fmt = self.settings().datetime_custom_format;
        // chrono panics inside `to_string()` if the format yields an
        // `Item::Error` (unknown `%X` specifier). NPP's Win32 path silently
        // passes unrecognised chars through as literals; chrono doesn't.
        // Walk the items first; on any Error, treat as a no-op so a typo
        // in the user's format string can't crash the editor.
        if StrftimeItems::new(&fmt).any(|i| matches!(i, Item::Error)) {
            return;
        }
        let stamp = chrono::Local::now().format(&fmt).to_string();
        self.insert_text_in_editor(&stamp, cx);
        crate::macros::record(crate::macros::MacroEvent::InsertDateTimeCustomized);
    }

    /// Replace the active selection with `text`. Routes through the same
    /// undoable buffer path that `paste` uses so the insertion lands as a
    /// single undo step.
    fn insert_text_in_editor(&mut self, text: &str, cx: &mut Context<Self>) {
        let r = self.selection().ordered();
        self.replace_byte_range(r, text, cx);
    }
}
