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
// (Preferences dialog ▸ Multi-Instance & Date).

use std::fmt::{self, Write as _};

use chrono::format::{Item, StrftimeItems};
use chrono::{DateTime, TimeZone};
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
        // NPP's Win32 path passes unrecognised chars through as literals;
        // chrono rejects them. On a bad template insert nothing and say why.
        let Some(stamp) = format_custom(&chrono::Local::now(), &fmt) else {
            self.set_status(format!(
                "Insert Date Time: the customized format \"{fmt}\" is not valid (Preferences > Multi-Instance & Date)"
            ));
            cx.notify();
            return;
        };
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

/// `now` rendered with the user's strftime `template`, or `None` when the
/// template has an unknown specifier. chrono's `to_string()` panics on an
/// `Item::Error`; checking the items and writing through `fmt::Write` turns
/// every formatting failure into `None` instead.
fn format_custom<Tz: TimeZone>(now: &DateTime<Tz>, template: &str) -> Option<String>
where
    Tz::Offset: fmt::Display,
{
    if StrftimeItems::new(template).any(|i| matches!(i, Item::Error)) {
        return None;
    }
    let mut out = String::new();
    write!(out, "{}", now.format(template)).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn custom_format_rejects_bad_templates_without_panicking() {
        let tz = FixedOffset::east_opt(3600).expect("valid offset");
        let now = chrono::NaiveDate::from_ymd_opt(2026, 3, 4)
            .and_then(|d| d.and_hms_opt(5, 6, 7))
            .and_then(|t| t.and_local_timezone(tz).single())
            .expect("valid date");
        let cases: &[(&str, Option<&str>)] = &[
            ("%Y-%m-%d %H:%M:%S", Some("2026-03-04 05:06:07")),
            ("", Some("")),
            ("é 中 %d\r\n", Some("é 中 04\r\n")),
            ("%%", Some("%")),
            ("%Q", None),
            ("%", None),
            ("abc %", None),
            ("%Y %Ö", None),
        ];
        for (template, want) in cases {
            assert_eq!(format_custom(&now, template).as_deref(), *want, "template {template:?}");
        }
    }
}
