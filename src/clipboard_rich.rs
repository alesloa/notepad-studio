//! Read formatted clipboard flavors past GPUI's string-only API.
//!
//! GPUI's `ClipboardItem` only surfaces `public.utf8-plain-text` (and image
//! formats), but a paste from Safari / Chrome / Word also puts `public.html`
//! and `public.rtf` onto NSPasteboard. Phase 2g.K reads those flavors directly
//! so the paste path can offer "keep formatting" alongside "paste as plain
//! text". Linux + Windows stubs return `None` for the rich flavors — Phase 2g.K
//! is macOS-first; the workspace falls back to plain-text paste there.

/// Snapshot of the relevant pasteboard flavors. `plain` mirrors what GPUI's
/// `read_from_clipboard` returns — kept here so the workspace can decide
/// formatting-or-plain from a single source rather than reading the clipboard
/// twice (NSPasteboard contents can change between calls when an app re-writes
/// the board mid-paste, which would race the modal's "what's on the clipboard"
/// snapshot).
#[derive(Debug, Default, Clone)]
pub struct RichClipboard {
    pub plain: Option<String>,
    pub html: Option<String>,
    pub rtf: Option<String>,
}

impl RichClipboard {
    /// True iff there's any formatted flavor that differs meaningfully from
    /// the plain text — that's the gate for showing the paste-format modal.
    pub fn has_formatting(&self) -> bool {
        let html_meaningful = self
            .html
            .as_deref()
            .map(|h| !h.trim().is_empty())
            .unwrap_or(false);
        let rtf_meaningful = self
            .rtf
            .as_deref()
            .map(|r| !r.trim().is_empty())
            .unwrap_or(false);
        html_meaningful || rtf_meaningful
    }
}

#[cfg(target_os = "macos")]
pub fn read() -> RichClipboard {
    macos::read()
}

#[cfg(not(target_os = "macos"))]
pub fn read() -> RichClipboard {
    // Non-mac platforms haven't been wired yet — Phase 2g.K is mac-first.
    // The workspace treats `has_formatting() == false` as "plain paste, no
    // modal", which is the right fallback until per-platform bridges land.
    RichClipboard::default()
}

#[cfg(target_os = "macos")]
mod macos {
    use super::RichClipboard;
    use cocoa::{
        appkit::NSPasteboard,
        base::{id, nil},
        foundation::NSData,
    };
    use core::slice;
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::c_void;

    /// Build an autoreleased `NSString` from a Rust `&str`. Mirrors
    /// `gpui::platform::mac::ns_string` so the symbol footprint matches.
    unsafe fn ns_string(string: &str) -> id {
        // SAFETY: `bytes`/`len` describe the live `&str` for the whole call;
        // `initWithBytes:length:encoding:` copies them, and a nil from
        // `alloc`/`init` is a valid receiver for `autorelease` (no-op).
        unsafe {
            let cls = class!(NSString);
            let bytes = string.as_ptr() as *const c_void;
            let s: id = msg_send![cls, alloc];
            let s: id = msg_send![
                s,
                initWithBytes: bytes
                length: string.len()
                encoding: 4_u64 // NSUTF8StringEncoding
            ];
            let _: () = msg_send![s, autorelease];
            s
        }
    }

    pub fn read() -> RichClipboard {
        let mut out = RichClipboard::default();
        // SAFETY: AppKit calls on the general pasteboard; both returned
        // objects are nil-checked before use, and `read_string_flavor`
        // nil-checks everything it derives from them.
        unsafe {
            let pb: id = NSPasteboard::generalPasteboard(nil);
            if pb == nil {
                return out;
            }
            let types: id = pb.types();
            if types == nil {
                return out;
            }
            out.plain = read_string_flavor(pb, types, "public.utf8-plain-text");
            out.html = read_string_flavor(pb, types, "public.html");
            out.rtf = read_string_flavor(pb, types, "public.rtf");
        }
        out
    }

    /// Pull a UTF-8 string from `pb` for the given UTI if the pasteboard
    /// advertises it. Both `public.html` and `public.rtf` arrive as UTF-8 byte
    /// blobs in practice — even RTF, which is ASCII-with-control-words, parses
    /// cleanly as UTF-8 since RTF restricts itself to 7-bit + escape sequences.
    unsafe fn read_string_flavor(pb: id, types: id, uti: &str) -> Option<String> {
        // SAFETY: `pb` and `types` are non-nil (checked by the caller). The
        // slice is built from `NSData`'s own non-null `bytes`/`length` pair
        // and copied into a `String` before `data` can be released.
        unsafe {
            let uti_id = ns_string(uti);
            let contains: bool = msg_send![types, containsObject: uti_id];
            if !contains {
                return None;
            }
            let data = pb.dataForType(uti_id);
            if data == nil {
                return None;
            }
            let len = data.length() as usize;
            if len == 0 {
                return Some(String::new());
            }
            let bytes_ptr = data.bytes();
            if bytes_ptr.is_null() {
                return Some(String::new());
            }
            let bytes = slice::from_raw_parts(bytes_ptr as *const u8, len);
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_formatting_false_when_only_plain() {
        let c = RichClipboard {
            plain: Some("hello".into()),
            html: None,
            rtf: None,
        };
        assert!(!c.has_formatting());
    }

    #[test]
    fn has_formatting_false_when_html_is_whitespace() {
        let c = RichClipboard {
            plain: Some("hello".into()),
            html: Some("   \n  ".into()),
            rtf: None,
        };
        assert!(!c.has_formatting());
    }

    #[test]
    fn has_formatting_true_when_html_present() {
        let c = RichClipboard {
            plain: Some("hello".into()),
            html: Some("<p><b>hi</b></p>".into()),
            rtf: None,
        };
        assert!(c.has_formatting());
    }

    #[test]
    fn has_formatting_true_when_rtf_present() {
        let c = RichClipboard {
            plain: Some("hello".into()),
            html: None,
            rtf: Some("{\\rtf1 hi}".into()),
        };
        assert!(c.has_formatting());
    }
}
