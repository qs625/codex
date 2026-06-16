# PM Progress

## Current Goal

Complete all Active Work recorded in this progress file.

## Active Work

None.

## Completed

- commit: `c2a93ce7b`
  summary: `command_wait` and `wait_agent` now use per-call backoff windows with reset-on-event behavior, typed wait timeout display, and root-worker filtering for raw wait tool output JSON.
  validation: `cargo test -p codex-command-runtime`, `cargo test -p codex-core command_wait_`, `cargo test -p codex-app-server response_item_completed_emits_command_wait_thread_item`, `cargo build -p codex-app-server --bin codex-app-server`, root-worker conversation tests, and `git diff --check` passed.
  residual_risk: Full workspace tests were not run; `wait_agent` broader integration coverage can still be expanded later.
- commit: `a231dfc2c`
  summary: Goal lifecycle updates now produce typed conversation items via `ResponseItem::ThreadGoalUpdate -> ThreadItem::ThreadGoalUpdate`, with root-worker rendering and schema fixtures updated.
  validation: `cargo test -p codex-app-server-protocol --test schema_fixtures`, `cargo test -p codex-core goal_tool`, `cargo build -p codex-app-server --bin codex-app-server`, root-worker build, and targeted conversation tests passed.
  residual_risk: Full root-worker test suite still has the known unrelated `contextUsage.test.ts` baseline failure.
- commit: `56f781919`
  summary: Child completion live display deduplicated by removing the extra raw `ItemCompleted(CollabAgentMessage)` emission and keeping the typed `ResponseItem::InterAgentCommunication -> ThreadItem` path.
  validation: `cargo test -p codex-core inter_agent_child_completion_live_item_waits_for_typed_recording`, `cargo build -p codex-app-server --bin codex-app-server`, and `git diff --check` passed.
  residual_risk: Coverage is focused on core live event emission; app-server projection relies on existing typed projector behavior.
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
