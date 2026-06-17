# PM Progress

## Current Goal

None

## Active Work

- None. Verified on 2026-06-17 after merging command-wait-timeout-display that no
  planned, in-progress, review, testing, blocked, or ready-to-merge work remains in this
  PM tracker.

## Design Direction

- Thread lifecycle is modeled as three top-level states: `Active`, `Idle`, and `Complete`.
- `Active` means a turn is currently running, or a pending-input turn is being started immediately.
- `Idle` means no turn is running, but direct children or wait_command state still prevent completion; schedule/event-tool recurring waits are out of scope for this lifecycle decision for now.
- `Complete` means no active turn, no pending input, no incomplete direct child, no wait_command, and no active goal continuation to run.
- Lifecycle evaluation order is fixed: pending input -> incomplete direct child -> wait_command -> active goal continuation -> complete.
- The evaluator only needs direct-child status; recursive behavior is produced naturally because a child cannot notify its parent until its own direct children are complete.
- Non-management subagents do not become parent-visible complete at their own turn finish. They first send a child completion message carrying their agent path; the parent marks that direct child complete when it starts a turn that consumes the completion message.
- `agent_mode = management` is exempt from parent completion delivery and may transition directly to `Complete` when its own local lifecycle permits it.
- The fixed Rust/Cargo tester path `/root/my_codex_pm/rust_cargo_tester` is created by PM as a management task. Owner and reviewer must not create tester threads; owner sends concrete validation JSON with `followup_task` to this fixed tester path. Tester returns raw command outputs directly to the requesting owner and does not need to notify PM when it completes.
- `command_wait` display lifecycle should show a started ThreadItem at model tool-call start and a completed ThreadItem at return using the same item id; `CommandWait.wait_timeout_ms` must be the current window for that call, with the initial window derived from the originating `exec_command` effective `initial_wait_ms`.
- `ResponseItem` is for model interaction, context manager history, provider wire history, compact, guardian, and model-visible tool outputs.
- `EventMsg` is the runtime event log and canonical source for app-server/root-worker UI display.
- `ThreadItem` is the app-server/client display projection, generated from display-capable `EventMsg` variants.
- Do not make provider/model requests rebuild `ResponseItem` from `EventMsg`; model context continues to store and consume `ResponseItem`.
- Business actions that need both model visibility and UI visibility should use a helper that writes the model `ResponseItem` and emits the UI/runtime `EventMsg` together.
- Display-only items such as command wait display, workflow progress, goal lifecycle, event command display, inter-agent display, and command notifications now use dedicated `EventMsg` variants as the primary display path.
- `ResponseItem -> ThreadItem`, `RawResponseItem -> ThreadItem`, and public `TurnItem -> ThreadItem` display adapters are removed. Thread display replay only consumes display-capable `EventMsg`; old rollout/history files that only contain `ResponseItem` / `RawResponseItem` do not rebuild UI display.

## Overall Plan

1. Architecture contract:
   - Document `ResponseItem = model context/provider interaction`, `EventMsg = runtime/UI display event source`, and `ThreadItem = client display projection`.
   - Document that provider/model requests continue to consume context `ResponseItem` directly; no `EventMsg -> ResponseItem` model reconstruction is planned.
   - Document dual-write helpers for business actions that need model-visible state and UI-visible events.
2. EventMsg projection boundary:
   - Add a shared app-server/protocol adapter that consumes `EventMsg` and returns zero or more `ThreadItem` lifecycle payloads.
   - Compatibility support for `ResponseItemStarted` and `ResponseItemCompleted` has been removed; new display work must add semantic `EventMsg` variants instead of display-only `ResponseItem` variants.
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
   - Do not preserve old ResponseItem display variants through rollout compatibility.
6. ResponseItem tightening:
   - Prevent new display-only ResponseItem variants by AGENTS/review guidance and targeted code comments/tests.
   - Remove display-only ResponseItem replay once corresponding EventMsg variants cover live/read.
7. Legacy cleanup:
   - Remove `TurnItem -> ThreadItem` live display as a public adapter.
   - Remove `RawResponseItem` display use.
   - Remove root-worker marker/text/JSON envelope display parsing and raw FunctionCallOutput fallback display.
8. Validation:
   - For each migrated display family, require live EventMsg -> ThreadItem tests, thread/read replay tests, and model-context tests proving ResponseItem remains correct for provider requests.
   - Rust/Cargo validation must go through the fixed tester `/root/my_codex_pm/rust_cargo_tester`.

## Completed

- commit: `5090f7476`
  summary: Merged command wait and client display fixes: stable command_wait duration formatting, typed Init Context display/replay with Agent file instructions, backend ThreadStatus-driven root-worker thinking state, obsolete SendMessage display normalization, and typed list_agents display through live/read paths.
  validation: Owner reported same reviewer passed all rounds with no blockers; frontend checks passed (`rtk pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/conversation.test.ts`, `rtk pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/thread.test.ts`, `rtk pnpm --dir apps/root-worker-prototype build`); fixed tester passed Init Context tests, list_agents protocol tests, failed list_agents trace test, affected crate checks, and after stale metadata cleanup `rtk proxy cargo build -p codex-app-server --bin codex-app-server`.
  residual_risk: Full root-worker suite still has the known unrelated `contextUsage.test.ts` baseline failure; broad workspace tests were not run.
- commit: `e1ac24b86`
  summary: Fixed MultiAgent V2 parent wakeup for direct child completion when final status is advanced through raw event delivery; parent now gets the typed child completion path without needing manual `list_agents` or `wait_agent`.
  validation: Reviewer reported no blockers; fixed tester passed `rtk cargo test -p codex-core agent::control_tests::raw_final_status_wakes_parent_with_child_completion` and `rtk cargo build -p codex-app-server --bin codex-app-server`; owner `rtk git diff --check` passed.
  residual_risk: No app-server v2 projection integration assertion was added; change reuses existing typed child completion display path.
- commit: included in cleanup commit
  summary: Follow-up cleanup removes old display compatibility: thread/read and live no longer project `RolloutItem::ResponseItem`, `RawResponseItem`, `ResponseItemStarted/Completed`, or `ResponseItem::FunctionCall` into `ThreadItem`; the public `TurnItem -> ThreadItem` adapter and structured ResponseItem display projector are removed; stale tests and specs now assert EventMsg as the only display source.
  validation: `rtk git diff --check` passed locally; fixed tester request `remove-legacy-display-compat-20260617-08-protocol-rerun` passed `rtk cargo test -p codex-app-server-protocol`; fixed tester request `remove-legacy-display-compat-20260617-09-app-server-build` passed `rtk cargo build -p codex-app-server --bin codex-app-server`.
  residual_risk: Full workspace tests were not run.
- commit: `67509dd`
  summary: Dedicated display EventMsg variants landed for command wait/stdin/notification, workflow progress, goal lifecycle, event-command, event-driven tool, and inter-agent display. app-server live/read display now projects these EventMsg variants to ThreadItem directly. Schema fixtures, AGENTS.md, and EventMsg/ResponseItem specs were updated to reflect EventMsg as the display source and ResponseItem as model context/provider history.
  validation: Fixed tester `/root/my_codex_pm/rust_cargo_tester` reported targeted validation passed: `rtk cargo test -p codex-core explicit_record_conversation_items_emits_command_wait_display_event`, `rtk cargo build -p codex-app-server --bin codex-app-server`, and after schema regeneration `rtk cargo test -p codex-app-server-protocol` with 284 passed. `rtk git diff --check` also passed before commit.
  residual_risk: Full workspace tests were not run; a follow-up cleanup removes legacy rollout/history display compatibility.
- commit: `1f4e54e`
  summary: EventMsg display projection boundary landed. This historical step introduced the shared adapter and helper naming; later cleanup removed ResponseItem lifecycle display replay so EventMsg is now the only display source.
  validation: Fixed tester `/root/my_codex_pm/rust_cargo_tester` reported all targeted commands passed: `rtk cargo test -p codex-app-server-protocol response_item_`, `rtk cargo test -p codex-core explicit_record_conversation_items_emits_response_item_completed_for_command_wait`, `rtk cargo build -p codex-app-server --bin codex-app-server`; `rtk git diff --check` also passed.
  residual_risk: Dedicated semantic EventMsg variants for command notifications, collab/goal/workflow/event-command remain active follow-up work; full workspace tests were not run.
- commit: `4044884cb`
  summary: Thread lifecycle and command_wait fixes are implemented in the main checkout. Child completion is now parent-visible only when the parent starts a turn consuming the completion pending input; post-turn active checks use direct child state, running command state, pending input/mailbox, and active event subscriptions before active goal continuation; management agents bypass parent completion delivery. `command_wait` now uses the originating exec_command effective `initial_wait_ms` as its initial backoff window and emits typed started/completed lifecycle items with the same id and current wait window.
  validation: Fixed tester `/root/my_codex_pm/rust_cargo_tester` reported all targeted commands passed: `rtk cargo test -p codex-core command_wait`, `rtk cargo test -p codex-core goal_post_turn_state`, `rtk cargo test -p codex-core turn_start_consumes_child_completion_before_parent_visible_complete`, `rtk cargo test -p codex-app-server-protocol response_item_started_maps_command_wait_to_thread_item`, `rtk cargo build -p codex-app-server --bin codex-app-server`.
  residual_risk: Full workspace tests were not run.
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
