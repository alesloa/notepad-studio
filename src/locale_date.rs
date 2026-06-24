//! OS locale-aware date/time formatting for Edit ▸ Insert ▸ Date/Time Short.
//!
//! Notepad++ on Windows uses `GetDateFormatEx(DATE_SHORTDATE)` and
//! `GetTimeFormatEx`, which resolve the user's Control Panel short-date /
//! short-time patterns. There's no portable equivalent in chrono — chrono's
//! `unstable-locales` feature only localizes weekday/month names, not the
//! pattern itself. So we bridge per platform:
//!
//!   * macOS: `NSDateFormatter` with `NSDateFormatterShortStyle` for both
//!     date and time (existing `cocoa` deps).
//!   * Linux: `nl_langinfo(D_FMT)` + `nl_langinfo(T_FMT)` (POSIX), fed to
//!     `chrono::format` so weekday/month names still pass through.
//!   * Windows / unknown: ISO `YYYY-MM-DD HH:MM` — same fallback the v1
//!     placeholder used. Windows binary support is gated on a `windows-sys`
//!     dep that hasn't been needed yet; revisit when we ship Windows builds.
//!
//! Always returns *something* — never panics, never returns Result. A broken
//! locale bridge falls through to ISO so Insert ▸ Date/Time Short keeps
//! working.

/// Format `now()` as the user's locale-aware short date + time. Joined with a
/// single space, matching NPP's behavior of concatenating the two `GetFormat`
/// results separated by `_T(" ")`.
pub fn short_now() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Some(s) = macos::format_short_now() {
            return s;
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(s) = linux::format_short_now() {
            return s;
        }
    }
    iso_fallback()
}

fn iso_fallback() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

#[cfg(target_os = "macos")]
mod macos {
    use cocoa::base::{id, nil};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::c_void;

    // NSDateFormatterStyle values from
    // https://developer.apple.com/documentation/foundation/nsdateformatterstyle
    const NS_DATE_FORMATTER_SHORT_STYLE: u64 = 1; // NSDateFormatterShortStyle
    const NS_UTF8_STRING_ENCODING: u64 = 4;

    pub fn format_short_now() -> Option<String> {
        unsafe {
            let formatter: id = msg_send![class!(NSDateFormatter), alloc];
            let formatter: id = msg_send![formatter, init];
            if formatter == nil {
                return None;
            }
            let _: () = msg_send![formatter, setDateStyle: NS_DATE_FORMATTER_SHORT_STYLE];
            let _: () = msg_send![formatter, setTimeStyle: NS_DATE_FORMATTER_SHORT_STYLE];

            let date: id = msg_send![class!(NSDate), date];
            let ns_str: id = msg_send![formatter, stringFromDate: date];
            let _: () = msg_send![formatter, release];
            if ns_str == nil {
                return None;
            }
            ns_string_to_rust(ns_str)
        }
    }

    /// Copy an `NSString` into an owned Rust `String`. Returns `None` if the
    /// string is nil or doesn't fit UTF-8 (NSDateFormatter output always does
    /// in practice, but we don't want to panic on adversarial locale data).
    unsafe fn ns_string_to_rust(ns_str: id) -> Option<String> {
        unsafe {
            let utf8_ptr: *const c_void = msg_send![ns_str, UTF8String];
            if utf8_ptr.is_null() {
                return None;
            }
            let len: usize = msg_send![ns_str, lengthOfBytesUsingEncoding: NS_UTF8_STRING_ENCODING];
            let bytes = std::slice::from_raw_parts(utf8_ptr as *const u8, len);
            std::str::from_utf8(bytes).ok().map(|s| s.to_owned())
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use chrono::Local;
    use std::ffi::CStr;

    // `nl_langinfo` item indices from `langinfo.h`. These constants are stable
    // across glibc / musl. Pulled in directly to avoid a libc dep just for two
    // integers.
    const D_FMT: libc::nl_item = 0x2002C;
    const T_FMT: libc::nl_item = 0x2002D;

    pub fn format_short_now() -> Option<String> {
        let date_fmt = read_langinfo(D_FMT)?;
        let time_fmt = read_langinfo(T_FMT)?;
        let now = Local::now();
        // POSIX D_FMT / T_FMT use the same `%`-specifier vocabulary chrono
        // already understands. `format` panics on unknown specifiers, so probe
        // first via `StrftimeItems` and bail to ISO if anything trips.
        use chrono::format::{Item, StrftimeItems};
        let combined = format!("{} {}", date_fmt, time_fmt);
        if StrftimeItems::new(&combined).any(|i| matches!(i, Item::Error)) {
            return None;
        }
        Some(now.format(&combined).to_string())
    }

    fn read_langinfo(item: libc::nl_item) -> Option<String> {
        unsafe {
            let ptr = libc::nl_langinfo(item);
            if ptr.is_null() {
                return None;
            }
            let cs = CStr::from_ptr(ptr);
            let s = cs.to_str().ok()?.trim();
            if s.is_empty() { None } else { Some(s.to_owned()) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_now_returns_non_empty_string() {
        let s = short_now();
        assert!(!s.is_empty(), "expected a formatted timestamp");
        // Must include at least one digit — every locale's short date/time
        // is built from numeric fields.
        assert!(
            s.chars().any(|c| c.is_ascii_digit()),
            "expected digits in {s:?}"
        );
    }

    #[test]
    fn iso_fallback_matches_expected_shape() {
        let s = iso_fallback();
        // YYYY-MM-DD HH:MM — 16 chars.
        assert_eq!(s.len(), 16);
        assert_eq!(s.chars().nth(4), Some('-'));
        assert_eq!(s.chars().nth(10), Some(' '));
        assert_eq!(s.chars().nth(13), Some(':'));
    }
}
