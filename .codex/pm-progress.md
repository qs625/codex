# PM Progress

## Current Goal

Complete all Active Work recorded in this progress file.

## Active Work

- id: goal-threaditem-display
  owner: /root/my_codex_pm/goal_threaditem_display_owner
  worktree: /Users/bytedance/Projects/my-codex/.worktrees/goal-threaditem-display
  branch: agent/goal-threaditem-display
  status: in_progress
  objective: Add typed ThreadItem/client display handling for goal tool create/update events so model-created goals appear in the client conversation.
  last_update: 2026-06-16
  next_action: Owner to trace goal tool events through ResponseItem/ThreadItem projection and root-worker display, then implement typed goal item support.
  blockers: None.
  validation: pending
  commit: pending
- id: wait-tool-backoff-semantics
  owner: /root/my_codex_pm/wait_tool_backoff_owner
  worktree: /Users/bytedance/Projects/my-codex/.worktrees/wait-tool-backoff
  branch: agent/wait-tool-backoff
  status: in_progress
  objective: Align command_wait and wait_agent with per-call wait window semantics: timeout returns to model, subsequent calls use backoff, and events reset backoff.
  last_update: 2026-06-16
  next_action: Owner to change command_wait and wait_agent so each call waits one current window and returns timeout/running, with runtime backoff persisted across calls and reset on event.
  blockers: None.
  validation: pending
  commit: pending

## Completed

- commit: `b41bdda04`
  summary: Dynamic Workflow TypeScript SDK runtime bridge completed and merged; `wf.Agent`, `agent.followup`, and `agent.wait` now bind to the current workflow tool runtime context, while `wf.shell` is an explicit unsupported structured response.
  validation: `cargo test -p codex-workflow`, `cargo build -p codex-app-server --bin codex-app-server`, `RUST_MIN_STACK=16777216 cargo test -p codex-core --test all workflow_tools -- --test-threads=1`, and `git diff --check` passed.
  residual_risk: Default-stack `codex-core` workflow_tools run still stack-overflows in the known broad test path; user said stack overflow failures are non-blocking.
- commit: `0025152a0`
  summary: Root-worker goal slash actions completed and merged.
  validation: Targeted root-worker tests, `tsc --noEmit`, root-worker build, and `git diff --check` passed.
  residual_risk: Full root-worker test suite still has the known `src/lib/contextUsage.test.ts` baseline failure.
- commit: `c04240a98`
  summary: Root-worker goal display, `/goal cancel`, and embedded `/init` system skill completed and merged.
  validation: Targeted frontend tests, `tsc --noEmit`, root-worker build, `cargo test -p codex-skills`, and `cargo build -p codex-app-server --bin codex-app-server` passed.
  residual_risk: No real Electron + app-server smoke was run.

## Known Issues

- `rtk pnpm --dir apps/root-worker-prototype test` has an existing failure in `src/lib/contextUsage.test.ts`, expected `19900` but got `3582`.
- Broad `codex-core` test filters that include known stack overflow cases may still fail; user previously confirmed stack overflow failures are not blocking.
