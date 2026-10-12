# Deletion history model measurements

The optimized control passed 24 processes: four cases, a constructed Singles
reference and the production coalescing planner, three runs each. The fixed
source was 12,600,000 ASCII bytes. Every run checked complete byte checksums,
all retained operations and revisions, first-caret metadata, expected history
entry counts, Undo/Redo fidelity and the captured-save endpoint where applicable.
All 48 fresh before/after process inventories succeeded with no competitor;
there were no rejected or retried runs. All 22 source identities matched after
compilation and after the last process.

The helper compiled the exact production `workspace/typing_undo.rs` in small
Buffer/Tab/EditOp shells with actual Rope mutations. It used the shipping
profile: optimization 3, thin LTO, one codegen unit, panic abort and stripping.
Ropey 1.6.1, smallvec 1.15.1 and str_indices 0.4.4 matched the app lock's versions,
source URLs and checksums. The retained executable was
`a42b2317d40c34398fc4093a6171bf1cea2ed141a7a7f941459f94867d536ff5`;
the frozen 22-input manifest was
`fa48ac721829964bb59c6f3349303edaebc0852f09b8028d67c9e177c6198961`.

Values below are medians of three processes. Arrows mean reference → grouped
model. Long cases removed 65,536 bytes; the fence case sealed the captured
revision halfway through and published that older marker after later deletions.

| Case | Delete ms | Undo ms | Redo ms | Peak working MiB |
|---|---:|---:|---:|---:|
| One Delete | 0.0162 → 0.0168 | 0.0164 → 0.0170 | 0.0051 → 0.0042 | 30.43 → 30.42 |
| Forward Delete | 13.8218 → 15.3664 | 9.3155 → 8.4765 | 9.9489 → 10.3048 | 44.55 → 39.04 |
| Backspace | 20.5992 → 22.8696 | 15.9470 → 15.3304 | 20.2272 → 18.4203 | 44.58 → 39.08 |
| Captured fence | 14.2095 → 16.2434 | 11.0170 → 8.9380 | 10.0210 → 12.0194 | 44.55 → 39.04 |

Grouping adds measured deletion overhead: 0.0006 ms for the short case, 1.5446 ms
for the forward batch, 2.2704 ms for Backspace and 2.0339 ms for the fence batch.
The grouped model uses one history entry, or two with the fence, versus 65,536
Singles. It retains all 65,536 operations and reduces whole-process peak working
memory by about 5.5 MiB in these batches. Batch Undo is faster in the measured model;
forward and fence Redo take slightly longer. Three runs do not establish a
universal speed advantage or a latency guarantee.

Deletion timers include fallible text/history preparation, Rope mutation,
revision updates and history publication. Undo/Redo timers cover constructed
inverse/forward walkers. File reading, Rope preparation and full checksums are
outside these operation timers and inside whole-process wall time and peaks.
This is a model measurement, not an old shipping binary comparison or native
Windows/GPUI performance result. It excludes actual history validation,
annotations, callbacks, syntax work, macros and persistence. Unicode/CRLF and
application correctness are covered by the separate
[installed-protocol checks](windows-deletion-undo-validation-20261011.md).

The immutable local evidence retains every result, stdout, empty stderr and
terminal tail, inventory and terminal receipt. Summary identity:
`c716cd2638710f2362d895a5c4e8f66d932c198364429799b2a095dff50e3739`.
The explicit terminal receipt records all 22 final hashes and zero drift.
