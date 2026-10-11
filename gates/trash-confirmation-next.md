# Gates: move-to-trash confirmation safety

Scope: confirm the existing tab-menu deletion before filesystem effects and preserve unsaved work against stale replies.

- [x] G1: oracle confirmation established and actual missing confirmation reproduced with a nondestructive test seam
  EVIDENCE: NppIO.cpp:2346-2387, Notepad_plus.cpp:2619-2628 and Buffer.cpp:1174-1197 establish confirmation before recycling. Leaf log/trash-confirmation-red.log fails through the painted MoveToTrash row because backend ran before any answer. Fake backend renames only within the temporary fixture directory; no real OS trash calls.
- [x] G2: original document/window ownership guards confirmation and trash effects; Cancel preserves document, disk and recovery
  EVIDENCE: Parent reviewed whole new trash.rs; parented async prompt captures original UUID/Arc/window/source/activity/path/encoding/EOL/savepoint/native payload. Valid reorder/switch retains original target. Last-tab recovery preflight refuses filesystem effects on failure; post-delete failure retains the exact Tab/history/native payload as dirty untitled and requires Save As.
- [x] G3: actual caller tests cover dirty content, stale tab incarnation/path/edit, window closure, errors and valid confirmation
  EVIDENCE: Main log/trash-main-focused.txt passes13 tests in0.17s. Covers Cancel/dropped reply/backend failure, clean/dirty success,13 stale source/metadata/window transitions, native window removal, both last-tab policies, CSV/transactions, native payload edit/Undo ABA and recovery failure. Snapshots use existing test sinks; Full and UntitledOnly policies are compared separately. Early CRLF/draft snapshot fixture corrections are not product REDs.
- [x] G4: focused/build/full-test/Clippy checks pass; reviewed frozen patch ready
  EVIDENCE: Frozen leaf final patch SHA256 7b1ee4a4ddf0212549ccf08f5cb596cb64bc0b1adccd5f30e95ac10b9935cfb5; manifest229ec92e90526e4d50d34d2fab5ec9fc4fa8a258b11f018d8d68431065a380c6. Build,13 focused,2431 unit plus3 integration/22 ignored, Clippy all-targets and diff check pass. Unit runtime56.19s; inherited195 app/209 test warnings with no new touched-line diagnostics. Parent source equality and13 focused pass recorded above; final joint Windows gates live in gates/resumed-parity.md. Worker cache idle.
