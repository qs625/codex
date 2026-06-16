# PM Progress

## Current Goal

Complete all Active Work recorded in this progress file.

## Active Work

- id: eventmsg-threaditem-architecture-plan
  owner: /root/my_codex_pm/eventmsg_threaditem_owner
  worktree: /Users/bytedance/Projects/my-codex/.worktrees/eventmsg-threaditem-display-source
  branch: agent/eventmsg-threaditem-display-source
  status: in_progress
  objective: Write and land the full architecture plan for EventMsg as runtime/UI display source and ResponseItem as model context/provider source.
  last_update: 2026-06-16
  next_action: Owner to update specs and AGENTS with the complete migration roadmap, invariants, non-goals, and validation strategy before broad code migration.
  blockers: None.
  validation: pending
  commit: pending
- id: eventmsg-threaditem-projector-boundary
  owner: pending
  worktree: pending
  branch: pending
  status: planned
  objective: Add a shared EventMsg -> ThreadItem adapter boundary in app-server/protocol while preserving existing behavior.
  last_update: 2026-06-16
  next_action: After architecture plan lands, implement the adapter for existing ResponseItemCompleted/ItemCompleted compatibility paths and cover live/read projection tests.
  blockers: eventmsg-threaditem-architecture-plan.
  validation: pending
  commit: pending
- id: eventmsg-dual-write-helpers
  owner: pending
  worktree: pending
  branch: pending
  status: planned
  objective: Introduce semantic helpers for actions that need both model ResponseItem context and UI/runtime EventMsg display.
  last_update: 2026-06-16
  next_action: Define helper APIs that append model-visible ResponseItem where needed and emit display-capable EventMsg without callers hand-rolling two paths.
  blockers: eventmsg-threaditem-projector-boundary.
  validation: pending
  commit: pending
- id: migrate-wait-command-display-events
  owner: pending
  worktree: pending
  branch: pending
  status: planned
  objective: Move command_wait, command_write_stdin, and command execution notification display semantics toward dedicated EventMsg variants.
  last_update: 2026-06-16
  next_action: Replace display-only ResponseItem usage for wait/stdin/notification with EventMsg-driven ThreadItem projection while keeping model tool outputs as ResponseItem.
  blockers: eventmsg-dual-write-helpers.
  validation: pending
  commit: pending
- id: migrate-collab-goal-workflow-display-events
  owner: pending
  worktree: pending
  branch: pending
  status: planned
  objective: Move inter-agent/child completion, goal lifecycle, workflow progress, event command, and event-driven tool display semantics toward EventMsg variants.
  last_update: 2026-06-16
  next_action: Migrate one display family at a time to EventMsg -> ThreadItem projection, with legacy ResponseItem compatibility only for old rollout/history.
  blockers: eventmsg-dual-write-helpers.
  validation: pending
  commit: pending
- id: responseitem-contract-tightening
  owner: pending
  worktree: pending
  branch: pending
  status: planned
  objective: Tighten ResponseItem so new variants are model-context/provider-facing only, not UI display-only.
  last_update: 2026-06-16
  next_action: Add compile-time/documentation guardrails, update review guidance, and deprecate display-only ResponseItem variants after EventMsg replacements exist.
  blockers: migrate-wait-command-display-events, migrate-collab-goal-workflow-display-events.
  validation: pending
  commit: pending
- id: legacy-display-path-cleanup
  owner: pending
  worktree: pending
  branch: pending
  status: planned
  objective: Remove or quarantine legacy display paths after EventMsg projection covers live/read.
  last_update: 2026-06-16
  next_action: Delete or isolate TurnItem -> ThreadItem live display, RawResponseItem display, marker/text/JSON envelope parsing, and raw tool output fallback display.
  blockers: None.
  validation: pending
  commit: pending

## Design Direction

- `ResponseItem` is for model interaction, context manager history, provider wire history, compact, guardian, and model-visible tool outputs.
- `EventMsg` is the runtime event log and canonical source for app-server/root-worker UI display.
- `ThreadItem` is the app-server/client display projection, generated from display-capable `EventMsg` variants.
- Do not make provider/model requests rebuild `ResponseItem` from `EventMsg`; model context continues to store and consume `ResponseItem`.
- Business actions that need both model visibility and UI visibility should use a helper that writes the model `ResponseItem` and emits the UI/runtime `EventMsg` together.
- Display-only items such as command wait display, workflow progress, goal lifecycle, event command display, inter-agent display, and command notifications should migrate toward dedicated `EventMsg` variants instead of new `ResponseItem` variants.
- `ResponseItem -> ThreadItem` remains only as a legacy compatibility adapter during migration, not the long-term primary display path.

## Overall Plan

1. Architecture contract:
   - Document `ResponseItem = model context/provider interaction`, `EventMsg = runtime/UI display event source`, and `ThreadItem = client display projection`.
   - Document that provider/model requests continue to consume context `ResponseItem` directly; no `EventMsg -> ResponseItem` model reconstruction is planned.
   - Document dual-write helpers for business actions that need model-visible state and UI-visible events.
2. EventMsg projection boundary:
   - Add a shared app-server/protocol adapter that consumes `EventMsg` and returns zero or more `ThreadItem` lifecycle payloads.
   - Existing `ResponseItemCompleted(ResponseItem)` support may remain as a compatibility branch, but new display work should add semantic `EventMsg` variants.
   - Thread read/history should use the same EventMsg display adapter when replaying persisted EventMsg.
3. Dual-write helpers:
   - Introduce helpers such as `record_tool_result_and_emit_event` / `record_model_item_and_emit_display_event` for actions that must update model context and UI.
   - The helper owns consistency between model `ResponseItem` writes and runtime/display `EventMsg` emission.
4. First migration family, command/wait:
   - Migrate `command_wait`, `command_write_stdin`, and command notification display to EventMsg-driven ThreadItem projection.
   - Keep ordinary tool output ResponseItems only for model interaction.
   - Ensure raw tool output JSON cannot become visible display.
5. Second migration family, collaboration/goal/workflow:
   - Migrate inter-agent display/child completion, goal lifecycle, workflow progress, event command, and event-driven tool display to EventMsg variants.
   - Preserve old ResponseItem display variants through legacy rollout compatibility only.
6. ResponseItem tightening:
   - Prevent new display-only ResponseItem variants by AGENTS/review guidance and targeted code comments/tests.
   - Deprecate or remove display-only ResponseItem variants once corresponding EventMsg variants cover live/read.
7. Legacy cleanup:
   - Remove or quarantine `TurnItem -> ThreadItem` live display as a legacy adapter.
   - Remove `RawResponseItem` display use.
   - Remove root-worker marker/text/JSON envelope display parsing and raw FunctionCallOutput fallback display.
8. Validation:
   - For each migrated display family, require live EventMsg -> ThreadItem tests, thread/read replay tests, and model-context tests proving ResponseItem remains correct for provider requests.
   - Rust/Cargo validation must go through the fixed tester `/root/my_codex_pm/rust_cargo_tester`.

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
