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
- `xim-parser/src/parser.rs` and `xim-gen/res/snippet.rs`: preserve the
  synchronous bit in character and keysym commits. `src/client.rs` forwards
  Both text through the character callback and acknowledges successful
  synchronous Chars, Keysym and Both requests exactly once. Keysym-only
  commits still do not dispatch keys. Wire tests cover every flag, Unicode
  delivery, callback-before-reply ordering and send/callback errors.
- `src/x11rb.rs`: validate XCONNECT format and reject unimplemented transport
  versions before changing state or sending Connect. Version 0.0 retains its
  single-ClientMessage/property transport with a threshold capped at20 bytes.
  Shared sender calls return a typed error for unsupported multi-CM instead
  of panicking. Tests exercise actual negotiation and serialized property
  packets, boundary sizes, refusal state and send errors.

Two imported whitespace-only lines in `xim-parser/Cargo.toml` and
`xim-parser/xim-format.yaml` were normalized for the repository diff check.
Package configuration, other source, examples and benchmarks remain unchanged.
No platform composition reset or routing is changed. The test connection
does not exercise a native X11 desktop.

Commit flags and replies follow the X11 XIM
[specification](https://xorg.freedesktop.org/archive/X11R7.6/doc/libX11/specs/XIM/xim.html),
sections Synchronizing with the IM Server and Sending a committed string.
As before, malformed text or a refused callback returns an error before
acknowledging; Both uses that same behavior. Recovery of a synchronous
request after these errors remains open: a caller that only logs the error
  can leave the server waiting. This patch establishes valid-path behavior.

X transport version handling follows Table D.3 and its dividing-size rule in
the same specification. The client implements multi-ClientMessage transport
0.1; mixed transport 0.2 and PropertyNotify support remain open. Version 0.2
below its dividing size is not silently rerouted to
properties; it is refused before use. GPUI currently tears down XIM on an
error and falls back to ordinary keys, so transport 0.2 and PropertyNotify
modes remain unavailable.

ASCII designation behavior follows the X Consortium Compound Text 1.1
[standard](https://xorg.freedesktop.org/archive/current/doc/xorg-docs/ctext/ctext.html),
sections Standard Character Set Encodings, Approved Standard Encodings and
Errors. GL and GR are independent; full independent-set decoding remains open.
For example, `ESC - B a1 ESC ( B a1` would need to preserve ISO-8859-2 GR
and decode to `ĄĄ`. This subset returns `UnsupportedEncoding` instead of
silently interpreting the second high byte as Latin1.

Client transport 0.1 follows Appendix D, Tables D.4/D.5. The client lazily
interns `_XIM_MOREDATA` after validating XCONNECT, sends ordered 20-byte
chunks with one `_XIM_PROTOCOL` final, and pads its unused tail with zero.
Exact multiples do not produce an extra empty message. The private
`src/x11rb/client_transport.rs` reassembles only negotiated multi-CM input,
reserves its CARD16-declared frame fallibly (at most 262,144 bytes), rejects
early/extra finals and nonzero final padding, and dispatches once after
completion. Related framing/decoder/handler errors, successful reconnect,
and disconnect retire pending bytes; unrelated events and successful
independent outbound activity preserve them. Existing 0.0 property behavior
and the generic server sender/error bounds remain unchanged. Native editor
composition origin and bounded/coalesced property input remain separate work.

Actual negotiated wire regressions cover Unicode fragments, callback-before-
synchronous-ACK ordering, exact 20/40-byte boundaries, middle/final/flush
errors, maximum frames, refusal without publication, reconnect and clean
continuation after failure. The ignored release benchmark times a maximum
valid transport frame with an intentionally unknown protocol opcode; it
measures reassembly and parser refusal, not native desktop input latency.
