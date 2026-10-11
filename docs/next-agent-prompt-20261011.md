# Next agent prompt

Target: a fresh Codex coding session in T3 Code, with this repository open.

```text
Continue Notepad Studio in C:\Users\Ale\Documents\GitHub\notepad-studio toward Notepad++ behavior parity. Focus on Windows and macOS; do not start Linux, WSL or VMs. Preserve the Rust/GPUI architecture, Markdown Reader and other existing views, Apache-2.0 clean-room boundary, and local English-only proofreading.

Read AGENTS.md, README.md and docs/agent-handoff-20261011.md before editing. The implementation checkpoint is 647b0ff; verify the actual current branch and working tree. Completed worktree patches are already integrated.

Use available parallel agents in isolated worktrees for independent, reproduced bugs or confirmed parity gaps. Prioritize crashes/data loss, native completion/proofreading verification, then search macro parameter capture and menu gaps. Use CodeGraph as instructed. Study Notepad++ for behavior and write original Rust.

Integrate each reviewed batch, run the required Windows gates, update release notes and records, and commit using $commit. Preserve user edits and unsaved application state. Do not push.

Report evidence and remaining gaps plainly. Native visuals and Mac compilation remain unverified. End a session at a tested, committed checkpoint with workers idle and the handoff updated; claim full parity only after verifying the inventory.
```

The prompt keeps execution instructions short; the handoff holds the saved evidence and task-specific limits.
