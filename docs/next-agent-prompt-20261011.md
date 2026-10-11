# Next agent prompt

Target: a fresh Codex coding session in T3 Code, with this repository open.

```text
Continue the active goal of finishing Notepad Studio's Windows Notepad++ behavior parity in C:\Users\Ale\Documents\GitHub\notepad-studio. Mac testing is deferred by the user; do not start Linux, WSL or VMs. Preserve the Rust/GPUI architecture, Markdown Reader and other existing views, Apache-2.0 clean-room boundary, and local English-only proofreading.

Read AGENTS.md, README.md and docs/agent-handoff-20261011.md before editing. Tested checkpoints include c077e2d (17 File/bookmark/sort commands) and 10d4723 (mandatory synced save rescue). Column/physical-line fixes are also integrated: build, 2652 unit plus 3 integration tests, Clippy with zero new diagnostics, inventory checks and scoped release controls pass. Current Rust inventory SHA is c71335ee3eaa3d371bef0f956e95ea302ca23a719329a7c54e15832b3c26c339. Verify the actual current branch, log and working tree. Completed worktree patches are already integrated; gates/ is ignored and retains local evidence.

Use available parallel agents in isolated worktrees for independent, reproduced bugs or confirmed parity gaps. Do not reapply the completed save/column/physical-line patches. Active next leaves are bounded restoration (actual bounded-memory contract RED reviewed; fixed 64KiB pump implementation authorized) and source-only hash-tool G1 preparation (no Cargo yet). Neither next leaf is integrated. Read the handoff for exact source/evidence ownership, policy and limits. Normal Save background work, large columns, older line operations and native verification remain open. The inventory has 182 routed and 285 missing unique IDs; keep generator outputs and docs/windows-parity-behavior.json current. Preserve legacy macro refusals, background continuation and full-history limits. Use CodeGraph as instructed. Study Notepad++ for behavior and write original Rust.

Integrate each reviewed batch, run the required Windows gates, update release notes and records, and commit using $commit. Preserve user edits and unsaved application state. Do not push.

Report evidence and remaining gaps plainly. Native visuals/IME/OS clipboard and Mac compilation of new local changes remain unverified. Root approved only the two audited Windows LCMapStringEx calls in the bounded safe wrapper; no general unsafe exception exists. End a checkpoint with tested, committed source, workers idle and the handoff updated. Keep the overall goal active until the full behavior backlog and required verification are fulfilled.
```

The prompt keeps execution instructions short; the handoff holds the saved evidence and task-specific limits.
