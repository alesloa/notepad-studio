// Encoding menu actions. The Encoding menu sits as its own top-level menu in
// the menubar (mirroring Notepad++) and lets the user pick the on-disk
// encoding the active buffer will be saved with.

gpui::actions!(
    notepad_studio,
    [
        EncodingUtf8,
        EncodingUtf8Bom,
        EncodingUtf16Le,
        EncodingUtf16Be,
        EncodingLatin1,
        /// Status-bar click — cycle through the encoding list.
        EncodingCycle,
    ]
);
