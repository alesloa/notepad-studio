# Hash text input: Enter and paste supplement

2026-10-11 source/test preparation only. This supplements the frozen G1 packet
`b577a8d7d573eff7c55e9da62298dbaa9b34c83e301499f34c2d532b8a665973`
and its manifest `d8580dfde79935eee5453e1bd153187a1067b1ef8b04d77f17c59209dfda9ecb`.
Those artifacts and the original 23-case vector file are unchanged. No Cargo
build, dialog execution, native EDIT experiment or observed Enter defect is
claimed. The worktree remains at the reviewed b42c889 checkpoint.

## Local control trace

The oracle's `PowerEditor/src/MISC/md5/md5Dlgs.rc:35-46` declares the text dialog.
The input at line 42 is an EDITTEXT with ES_MULTILINE, ES_AUTOVSCROLL, WS_VSCROLL,
WS_HSCROLL and ES_WANTRETURN. Its output at line 43 adds ES_READONLY and omits
ES_WANTRETURN. The file paths/results at lines 29-30 are read-only multiline
EDITTEXT controls. Dialog EXSTYLE at lines 24 and 37 is the ordinary window
frame style; it is not an edit-control EOL extended style.

`md5Dlgs.cpp:514-527` initializes the text dialog with dark-mode child theming,
font selection and TextEditSelectAllProc on input/output. That procedure at
239-274 requests all keys, implements Ctrl+A selection and delegates other
characters/messages to the native control. It does not rewrite Enter or Paste.
The font setup at 712-718 only changes fonts. EN_CHANGE at 581-590 retrieves and
hashes the input; whole-text extraction at 364-372 does not normalize EOL.
Per-line processing at 422-510 is a separate operation after input retrieval.

`PowerEditor/src/NppDarkMode.cpp:2898-2932` routes EDIT theming to
subclassAndThemeListBoxOrEditControl at 3118 onward. That helper themes scrollbars
and the custom border. The inspected path contains no text/EOL conversion.
A source-wide search of the local checkout's cpp/h/rc files found no
EM_SETENDOFLINE, EM_GETENDOFLINE, EM_FMTLINES, EM_SETEXTENDEDSTYLE,
ES_EX_ALLOWEOL_CR/LF/ALL, ES_EX_CONVERT_EOL_ON_PASTE or corresponding
Edit_SetEndOfLine/FmtLines/SetExtendedStyle calls. This is negative source
inspection, not a native runtime query of default style bits.

The original oracle file SHA-256 values still apply: md5Dlgs.rc is
`215039609c40b43eade96974e3ad38fb73dadcc09c2a2bf53880727396d31e1c`,
and md5Dlgs.cpp is
`6d7e6a48ca540e602c149d231fe6e2bf2962bedf99dd3166ca5fff5f9694e1ea`.
The main G1 input-policy document pins the other studied source identities.

## Native facts and the input contract

ES_WANTRETURN makes Enter insert a newline in a multiline dialog edit control
instead of activating its default button. The oracle therefore delegates Enter
to its editable input. [Microsoft: Edit Control Styles](https://learn.microsoft.com/en-us/windows/win32/controls/edit-control-styles)

Hard edit-control line breaks consist of CRLF. EM_FMTLINES concerns optional
soft wrap markers (CR CR LF) in retrieved text and does not alter hard breaks.
The oracle has no EM_FMTLINES call. Its left-aligned WS_HSCROLL multiline input
also receives automatic horizontal scrolling rather than word wrapping.
[Microsoft: EM_FMTLINES](https://learn.microsoft.com/en-us/windows/win32/controls/em-fmtlines),
[Microsoft: About Edit Controls](https://learn.microsoft.com/en-us/windows/win32/controls/about-edit-controls)

EM_GETENDOFLINE reports the configured CRLF/CR/LF newline. EM_SETENDOFLINE can
change newly inserted breaks or detect an EOL according to enabled styles;
the oracle does not set or query it. These APIs describe configuration, not a
request to normalize all existing text. [Microsoft: EM_GETENDOFLINE](https://learn.microsoft.com/en-us/windows/win32/controls/em-getendofline),
[Microsoft: EM_SETENDOFLINE](https://learn.microsoft.com/en-us/windows/win32/controls/em-setendofline)

The required Hash dialog policy is consequently explicit: typed Enter inserts
one hard `\r\n`, replacing the current input selection. It must not insert GPUI's
usual `\n`, append a display-wrap marker, close the dialog or copy output. The
input Rope retains that pair as actual text, independent of the source tab's
encoding/EOL. Whole-text hashing consumes its UTF-8 bytes without source-editor
EOL reconstruction or normalization. For example `a`, Enter, `b` hashes
`61 0d 0a 62`, not `61 0a 62`. Per-line mode alone removes the one row-terminal CR
according to the original oracle contract. Native Unicode commits remain exact
text; a later keyboard Enter inserts CRLF and advances the UTF-16 caret by two.
This input rule applies to these tools on every supported platform, following
the Windows behavior oracle.

## Paste is a separate audit

Edit-control extended styles can enable lone CR/LF as supported breaks, and
ES_EX_CONVERT_EOL_ON_PASTE requests conversion of enabled pasted EOL to the
configured document EOL. The source does not request those styles. Ordinary
window EXSTYLE bits are a separate facility. This establishes absence of an
explicit oracle conversion request; it does not by itself prove how every
Windows EDIT implementation handles nonstandard raw LF/mixed clipboard data.
[Microsoft: Edit Control Extended Styles](https://learn.microsoft.com/en-us/windows/win32/controls/edit-control-window-extended-styles),
[Microsoft: EM_SETEXTENDEDSTYLE](https://learn.microsoft.com/en-us/windows/win32/controls/em-setextendedstyle)

The documented CF_UNICODETEXT format has CRLF line endings and ends at NUL.
Raw LF/mixed payloads in GPUI tests are therefore synthetic robustness cases;
this review cannot call their unchanged bytes a reproduced normal native paste.
[Microsoft: Standard Clipboard Formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/standard-clipboard-formats)

The proposed safe GPUI contract is to retain the text actually delivered by
Paste/native committed-text input, including raw LF/mixed bytes, rather than
silently re-encoding all prior content. A later typed Enter always adds CRLF,
even if the pasted body contains LF. Existing raw LF/mixed pure vectors remain
valid engine contracts and are unchanged. Native Notepad++ paste fidelity for
those nonstandard payloads remains pending a real control experiment. Neither
clipboard format documentation nor these synthetic tests closes that gap.
No source-editor Paste behavior or general text utility is changed.

## Prepared protocol controls

`hash_tools_enter_protocol_tests.rs` opens each text tool through its test-only
Generate action on the real app/window route. The baseline has no dialog handler
and fails at creation before an Enter/Paste event occurs. Once real wiring exists,
the tests use simulated keyboard events and the exact input handler installed by
paint, not an invented dialog transform. They require:

- `a enter b` to expose `a\r\nb` for all four algorithms;
- Select All followed by Enter to expose exactly one CRLF;
- native UTF-16 text commit of `é😀`, then Enter/x, to expose `é😀\r\nx` and caret 6;
- separately proposed raw LF/mixed Paste followed by Enter to retain the payload
  and append CRLF, without changing the clipboard.

Every control preserves the read-only source text, raw selection geometries,
annotations, encoding/EOL and both existing histories. Typed controls retain the
clipboard sentinel. The native protocol assertion proves input and caret bytes;
it does not by itself prove the digest-output/Copy button path. Actual completed
whole/per-line output and Copy tests must pair this with the pure engine vectors
in the implementation gates. New `hash_tools_input_protocol_vectors.json` adds
six independent Python hashlib cases for all four algorithms, including distinct
hard-CRLF and LF whole-text digests. The original vectors' byte hash remains
`15a0f2084ff823c56684c5333a5127872902f2201a1f6e70b927ab77440fd765`.

All controls remain unrun. Native visual/control behavior and raw-paste parity
are explicitly unverified. This supplement changes the proposed Enter input
contract, not a shipped handler or the original pure hash cases.
