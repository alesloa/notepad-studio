# Explicit untitled-label validation, 2026-10-11

File > Rename for an untitled document now keeps that name visible when
first-line titles are enabled. An origin flag distinguishes explicit names from
generated labels, including an explicit name such as `new 37`. Saved documents
keep their path-based title. Rename preserves text, dirty state, savepoint,
history and tab identity.

The behavior oracle is `NppNotification.cpp:58–60` (explicit rename suppresses
first-line naming), `NppIO.cpp:2186` (origin flag), `Notepad_plus.cpp:6572` and
`NppIO.cpp:2614,2756` (persistence/restoration). The code is original Rust.
No dependency, plugin ABI, unsafe or platform-specific source changed.

## Compatibility

`Buffer::new_label_is_custom` survives transfer and live/closed recovery. The two
persisted fields have `#[serde(default)]`. Older manifests load their stored
name; missing origin defaults to false, retaining previous display behavior.
Older explicit and generated names cannot be distinguished reliably, so no
name-pattern migration guesses are made. A failed streaming load generates an
automatic label and resets the flag. Save-picker suggestions already used the
stored label and were not changed.

## Controls and acceptance

The frozen test-only G1 run passed three controls and failed two assertions
after actual registered Rename and isolated recovery write/restore reached the
expected precedence boundary. Earlier text/history/savepoint/identity positives
passed. G1 patch SHA256:
`65a0c478472e2e9aebaae7d9dd7bedaeb4a48f99215fa63ced1392d955919aac`.

The final leaf passes eight controls in 0.86 s: six new tests and two existing
save/close controls. They cover Unicode and generated-looking explicit names,
preference toggles, registered Paste then Undo/Redo, read-only Rename, automatic
first-line naming, older open/closed schemas, actual isolated Writer and live
restore, closed recovery and transfer. Closed recovery checks UTF16BE/CRLF and
dirty text. These tests do not establish process-crash or power-loss durability.

Earlier expanded typing fixtures wrongly assumed one or two Undo operations
remove a whole typed prefix. Their failures and a compile-only correction are
preserved separately. The final control uses one registered Paste for exact
Undo/Redo; it neither changes typing history nor establishes typing-coalescing
parity. The GPUI in-memory clipboard does not establish Windows clipboard behavior.

Final patch SHA256:
`6c593f5dfde51db17ecf3eebe1f921edc595fa689fd06da60a6c55a003e9d913`.
Final leaf source manifest SHA256:
`8ad0f97e71d5f2f4d4b97801d5fe28414aa772cae03307f9ef7b71e3937a8146`.
All seven sources were copied byte-identically into main based at `68bf410`.

Required main gates pass on all 733 unchanged source/manifest entries:

- Build: 62.729 s (`cargo build --locked`).
- Tests: 2,690 unit plus three integration tests, 30 ignored; 51.12 s unit /
  166.669 s total (`cargo test --locked -p notepad-studio`).
- Clippy: 22.365 s; unchanged 331 warning occurrences / 193 message/path pairs,
  zero additions or removals (`cargo clippy --locked -p notepad-studio --all-targets`).
- All 12 inventory tests and regenerated-output check pass. Counts remain 184
  routed and 283 missing unique static IDs; wiring is not behavior proof.

Main test EXE SHA256:
`d3d2b03354dd8e834b7a2a50e1113309ef1a8ce0a3755d102207e57210083976`.
733-entry snapshot SHA256:
`e32aed5aa78c68e9026537321e23de44212c60abe4c1ec319cdf25df554eba24`.
The 340 production Rust files have SHA256:
`032a8ba5223f22e50801f0f4794d280dda7e591cb99212d898d6ee046849b586`.
Local main logs, executable and receipts are `log/windows-untitled-label-main-*`;
leaf packets are in the detached `windows-untitled-labels/gates/` directory.
Gates and logs remain local.

## Still open

Automatic titles use `first_nonempty_line`; the oracle uses the first physical
line and retains its previous name when that line is empty. Whitespace
normalization, empty-name retention and related picker behavior need a separate
audit. The current helper materializes entire Rope lines and can scan the
document during title calculation. This source finding needs a deciding
bounded-work/performance control; no observed OOM or native latency is claimed.

Native title-bar/tab rendering and Windows input/clipboard verification remain
pending because the native bridge is unavailable. Mac testing stays deferred.
Windows Notepad++ parity remains open.
