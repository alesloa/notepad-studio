# Next agent prompt

Target: a fresh Codex coding session in T3 Code, with this repository open.

```text
Continue the active goal of finishing Notepad Studio's Windows Notepad++ behavior parity in C:\Users\Ale\Documents\GitHub\notepad-studio. Mac testing is deferred by the user; do not start Linux, WSL or VMs. Preserve the Rust/GPUI architecture, Markdown Reader and other existing views, Apache-2.0 clean-room boundary, and local English-only proofreading.

Read AGENTS.md, README.md and docs/agent-handoff-20261011.md before editing. The latest implementation checkpoint is c077e2d (17 File/bookmark/sort commands), with build, 2577 combined tests, Clippy and scoped default-release controls passing. Verify the actual current branch and working tree. Completed worktree patches are already integrated; gates/ is ignored and retains local evidence.

Use available parallel agents in isolated worktrees for independent, reproduced bugs or confirmed parity gaps. Next safety investigations are interrupted-save recovery, hostile rectangle geometry and Unicode versus physical line boundaries; read their contracts/preparations in the handoff. Prove each actual caller before claiming a defect. The inventory has 182 routed and 285 missing unique IDs; keep generator outputs and docs/windows-parity-behavior.json current. Preserve legacy macro refusals, background continuation and full-history limits. Use CodeGraph as instructed. Study Notepad++ for behavior and write original Rust.

Integrate each reviewed batch, run the required Windows gates, update release notes and records, and commit using $commit. Preserve user edits and unsaved application state. Do not push.

Report evidence and remaining gaps plainly. Native visuals/IME/OS clipboard and Mac compilation of new local changes remain unverified. Root approved only the two audited Windows LCMapStringEx calls in the bounded safe wrapper; no general unsafe exception exists. End a checkpoint with tested, committed source, workers idle and the handoff updated. Keep the overall goal active until the full behavior backlog and required verification are fulfilled.
```

The prompt keeps execution instructions short; the handoff holds the saved evidence and task-specific limits.
