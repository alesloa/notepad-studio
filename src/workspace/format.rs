// Per-buffer formatting toggles — currently the on-save line-ending
// (CRLF / LF / CR) and text encoding (UTF-8, UTF-8 BOM, UTF-16 LE/BE,
// Latin-1). Both surface through the status bar and a top-level menu, and
// both flip the active buffer's dirty flag because the next save would
// produce different bytes than the last one.
//
// Important: we keep the rope LF-internal and UTF-8-internal regardless of
// the chosen on-disk format. Conversion happens only at save time inside
// `write_active_to_disk`. That keeps every editor invariant (byte offsets,
// undo, find ranges, bookmarks) stable across these toggles.

use gpui::{Context, Window};

use crate::actions::{
    EditEolCycle, EditEolMacintosh, EditEolUnix, EditEolWindows, EncodingCycle, EncodingLatin1,
    EncodingUtf16Be, EncodingUtf16Le, EncodingUtf8, EncodingUtf8Bom,
};
use crate::text::encoding::Encoding;
use crate::text::line_ending::LineEnding;
use crate::workspace::Workspace;

impl Workspace {
    fn set_line_ending(&mut self, le: LineEnding, cx: &mut Context<Self>) {
        let tab = self.active_tab_mut();
        if tab.buffer.line_ending == le {
            return;
        }
        tab.buffer.line_ending = le;
        // The rope itself is LF-internal so it doesn't change shape; the
        // *file on disk* will. That's an unsaved change as far as the user
        // is concerned, so flip dirty + update status.
        tab.buffer.dirty = true;
        let label = le.status_label();
        self.set_status(format!("Line ending: {label}"));
        self.save_session();
        cx.notify();
    }

    pub(crate) fn eol_windows(
        &mut self,
        _: &EditEolWindows,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_line_ending(LineEnding::Crlf, cx);
    }

    pub(crate) fn eol_unix(&mut self, _: &EditEolUnix, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_line_ending(LineEnding::Lf, cx);
    }

    pub(crate) fn eol_macintosh(
        &mut self,
        _: &EditEolMacintosh,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_line_ending(LineEnding::Cr, cx);
    }

    /// Status-bar segment click — cycle to the next mode.
    pub(crate) fn eol_cycle(&mut self, _: &EditEolCycle, _w: &mut Window, cx: &mut Context<Self>) {
        let current = self.buffer().line_ending;
        self.set_line_ending(current.cycle_next(), cx);
    }

    fn set_encoding(&mut self, enc: Encoding, cx: &mut Context<Self>) {
        let tab = self.active_tab_mut();
        if tab.buffer.encoding == enc {
            return;
        }
        tab.buffer.encoding = enc;
        // Same reasoning as set_line_ending: rope contents don't change, but
        // the next save would produce different bytes. Flip dirty.
        //
        // v1 trade-off: we do NOT re-decode the original raw bytes. Switching
        // encoding only affects the save target. A future revision could
        // stash the raw bytes on the Buffer and re-decode here.
        tab.buffer.dirty = true;
        let label = enc.label();
        self.set_status(format!("Encoding: {label}"));
        self.save_session();
        cx.notify();
    }

    pub(crate) fn encoding_utf8(
        &mut self,
        _: &EncodingUtf8,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_encoding(Encoding::Utf8, cx);
    }

    pub(crate) fn encoding_utf8_bom(
        &mut self,
        _: &EncodingUtf8Bom,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_encoding(Encoding::Utf8Bom, cx);
    }

    pub(crate) fn encoding_utf16_le(
        &mut self,
        _: &EncodingUtf16Le,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_encoding(Encoding::Utf16Le, cx);
    }

    pub(crate) fn encoding_utf16_be(
        &mut self,
        _: &EncodingUtf16Be,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_encoding(Encoding::Utf16Be, cx);
    }

    pub(crate) fn encoding_latin1(
        &mut self,
        _: &EncodingLatin1,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_encoding(Encoding::Latin1, cx);
    }

    /// Status-bar segment click — cycle to the next encoding.
    pub(crate) fn encoding_cycle(
        &mut self,
        _: &EncodingCycle,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.buffer().encoding;
        self.set_encoding(current.cycle_next(), cx);
    }
}
