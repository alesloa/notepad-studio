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
- `src/x11rb.rs` and its new test module: exercise the actual property
  transport, wire parser and client dispatch with an in-memory connection.
- Compound-text tests cover malformed high bytes and valid round trips.

Two imported whitespace-only lines in `xim-parser/Cargo.toml` and
`xim-parser/xim-format.yaml` were normalized for the repository diff check.
Package configuration, other source, examples and benchmarks remain unchanged.
No composition reset, routing or synchronous-flag behavior is changed. The test
connection does not exercise a native X11 desktop.
