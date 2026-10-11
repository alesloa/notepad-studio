# Local XIM decoder patch

Source: https://github.com/zed-industries/xim-rs at
`16f35a2c881b815a2b6cdfd6687988e84f8447d8` (MIT).

Notepad Studio keeps the GPUI dependency name, version and git revision intact;
the application workspace overrides that source with this directory.

Changes from the pinned revision:

- `src/client.rs`: return `ClientError::InvalidReply` when compound-text
  decoding fails in ResetIcReply, CommitData::Chars or PreeditDraw. Return
  the reset handler's result instead of ignoring it.
- `xim-ctext/src/lib.rs`: reject overflowing high bytes in the Chinese and
  Korean 94N conversion branches instead of panicking or wrapping.
  Consume ESC(B as an ASCII GL designation, distinguish initial literal
  text from escaped chunks, and return errors for invalid designations.
  Known but unsupported JIS X0201 single-byte designations return
  `UnsupportedEncoding`. Keep prior GR designation across ASCII/UTF8 chunks;
  refuse high bytes after ESC(B when the prior GR set is not Latin1.
- `src/x11rb.rs` and its new test module: exercise the actual property
  transport, wire parser and client dispatch with an in-memory connection.
- Compound-text tests cover malformed high bytes and valid round trips.
- `xim-parser/src/parser.rs` and `xim-gen/res/snippet.rs`: bound hotkey
  counts and required bytes before fallible allocation, read each key's
  state without indexing past the array, and size every 16-byte entry.
  Public parser/writer regressions cover count, truncation and state errors.
  The upstream infallible writer API and general attribute-size limits
  remain unchanged; this does not make arbitrary oversized values safe.

Two imported whitespace-only lines in `xim-parser/Cargo.toml` and
`xim-parser/xim-format.yaml` were normalized for the repository diff check.
Package configuration, other source, examples and benchmarks remain unchanged.
No platform composition reset, routing or synchronous-flag behavior is changed. The test
connection does not exercise a native X11 desktop.

ASCII designation behavior follows the X Consortium Compound Text 1.1
[standard](https://xorg.freedesktop.org/archive/current/doc/xorg-docs/ctext/ctext.html),
sections Standard Character Set Encodings, Approved Standard Encodings and
Errors. GL and GR are independent; full independent-set decoding remains open.
For example, `ESC - B a1 ESC ( B a1` would need to preserve ISO-8859-2 GR
and decode to `ĄĄ`. This subset returns `UnsupportedEncoding` instead of
silently interpreting the second high byte as Latin1.
