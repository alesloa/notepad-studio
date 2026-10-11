# Windows locale sorting: proposed rule exception

Status: approved by the parent's decision after the user explicitly delegated it on 2026-10-11. The two-call wrapper and all six sort actions are integrated on main. Native NLS key tests, the isolated and combined Windows suites, and optimized large-file controls passed. Native visual checks remain pending. `windows-nls-seam.patch` preserves the original review proposal (SHA256 cbe19f009c78164ca1889b83bc7cba0319d0d9cc9a5761e7774fc9a931258df5); the integrated implementation and its limits are recorded in `windows-line-sorts.md`.

`AGENTS.md` section 3 previously permitted unsafe only at the plugin ABI, macOS pasteboard and Linux locale seams. Exact Windows locale sorting requires one additional Windows-only seam. The approved exception permits two audited `LCMapStringEx` calls in `src/locale_sort_windows.rs`, behind a safe `Result` interface. It does not authorize unsafe elsewhere.

Notepad++ uses Windows NLS sort keys with the user's locale. Its current menu defaults ignore case, sort digit sequences numerically, and retain accents and symbols. Behavior sources: `PowerEditor/src/MISC/Common/SortLocale.cpp:28-38,191-212`, `SortLocale.h:33-37` and `NppCommands.cpp:1066-1078`. The Rust implementation is original.

The wrapper uses the already locked Windows crate, caps each input at 32 MiB and each key at 64 MiB, checks the lengths passed to the native API and reports errors before any document changes. The background sort planner caps aggregate keys and checks cancellation. The destination buffer owns twice the declared byte capacity because the pinned Rust projection accepts a UTF-16 slice while this API's sort-key mode counts bytes. Parent review confirmed the projection's signature and constants in locked windows 0.61.3; the proposal uses the required `LPARAM(0)` handle type.

At approval, the patch had not been compiled or executed. Verification must cover real native keys, all six registered sort actions, case/numeric/accent/NUL/Unicode controls, forward and backward selections, rectangular keys, physical line endings, cancellation, saved macro replay and one Undo/Redo. The usual Windows build, full tests and Clippy remain required. Mac testing remains deferred.

API documentation: [Microsoft LCMapStringEx](https://learn.microsoft.com/en-us/windows/win32/api/winnls/nf-winnls-lcmapstringex), [Microsoft Rust projection](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Globalization/fn.LCMapStringEx.html).
