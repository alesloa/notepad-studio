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
the same specification. The client implements 0.0 property-with-CM, 0.1
multi-ClientMessage, and 0.2 mixed CM/property transports. PropertyNotify
modes remain refused before use. GPUI tears down XIM on a transport error
and falls back to ordinary keys.

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
independent outbound activity preserve them. The generic server sender/error
bounds remain unchanged; the property receiver patch below changes client
0.0 input framing. Native editor composition origin and coalesced
PropertyNotify input remain separate work.

Actual negotiated wire regressions cover Unicode fragments, callback-before-
synchronous-ACK ordering, exact 20/40-byte boundaries, middle/final/flush
errors, maximum frames, refusal without publication, reconnect and clean
continuation after failure. The ignored release benchmark times a maximum
valid transport frame with an intentionally unknown protocol opcode; it
measures reassembly and parser refusal, not native desktop input latency.

Client transport 0.0 property input now lives in the private
`src/x11rb/client_property.rs` receiver. It validates announced byte lengths
(4..262,144, four-byte alignment) before requesting exactly the matching
four-byte units from GetProperty, with a checked per-atom CARD32 offset.
APPEND packets retain their next offset until delete=true consumes the last
property byte; deletion resets the entry for reuse. Only atoms with unread
tails occupy the bounded 256-entry map, reserved fallibly. Current pending
tails plus an unknown incoming frame are bounded at 64 MiB; already tracked
frames can drain their backlog without counting the same bytes twice. A full
map still accepts a new atom whose property is consumed completely.

STRING/format8 metadata, exact value length, remaining-byte alignment, XIM
header length and decoded Request.size must agree before a callback. The
padded request size accepts legitimate protocol padding while rejecting a
known opcode followed by header-announced trailing garbage. A related reply
or callback error retires only its own atom; valid reconnect/disconnect and
a server Window error clear all property offsets. The existing 0.1 fragment
state stays independent and rejects mixed property input before any fetch.
PropertyNotify modes remain unsupported. No generic sender,
server, parser/generator, or GPUI composition routing changes are included.

Tests use dynamic and sparse GetProperty stores with real offset/unit/delete
semantics. They cover ordered variable packets, 256-atom/64-MiB boundaries,
reply/header/consumption errors, handler errors, lifecycle resets, and a
continuously appended stream that advances beyond 64 MiB of historical
traffic while retaining one small tail. The malformed value_len fixture
serializes a coherent reply before mutating raw metadata; its old baseline
already refused that case, so it is a positive control rather than a client
bug reproduction. These wire tests do not exercise a native X11 desktop.

Client transport 0.2 uses an explicit private mixed mode and preserves the
server-announced dividing size. Complete CM/multi-CM and bounded property
packets dispatch in order; a property cannot interrupt an incomplete CM.
Pending property tails plus a new CM frame share the 64 MiB budget before
CM allocation. Each incoming notification still publishes one complete
protocol request, preserving the caller's single-callback contract.

A private client-only property writer selects APPEND/STRING/format8 at or
above the dividing size, checks the packet cap before interning/writing,
and reserves the serialized buffer fallibly. Below the threshold, existing
ordered CM chunks are used. Equality follows the existing 0.0 writer's
property convention. The existing u16 sequence and finite _XIM_DATA_N
namespace are retained. Intern success advances the sequence; append/send/
flush errors clear outbound bytes and incomplete CM state. The writer does
not delete potentially pending properties after an error. Generic server
transport, PropertyNotify subscriptions and GPUI callback routing remain
unchanged. Tests cover threshold boundaries, Unicode, sync ACK ordering,
APPEND offsets/deletion, shared budgets and lifecycle/error isolation.
