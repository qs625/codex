# Thread Provider Architecture

This document describes the target architecture for making native Morpheus
threads and external CLI-backed threads share one lifecycle contract. The first
implementation phase adds a provider descriptor catalog and wires the New
conversation flow to it; the full runtime migration remains staged.

## Goals

- Make `EventMsg` the canonical provider output. Native runtime events and
  external provider stdout, JSON, or SSE messages must be normalized by their
  adapter before app-server, rollout, replay, or root-worker see them.
- Give clients one provider-neutral way to discover thread lifecycle support,
  agent roles, model selection, and active capabilities.
- Preserve legacy native `thread/start` behavior when provider fields are
  omitted.
- Keep external providers separate from native `agent_type`; `codex_cli`,
  `claude_cli`, and `opencode` are provider ids, not Morpheus roles.

Non-goals for the first phase:

- Do not migrate every runtime operation behind the new trait at once.
- Do not implement native-only features for external providers by pretending
  support exists.
- Do not parse raw external messages in root-worker.
- Do not change ACP or introduce a third-party ACP dependency.

## Provider Contract

The target backend contract is:

```rust
trait ThreadProvider {
    fn provider_kind(&self) -> ThreadProviderKind;
    async fn start_thread(&self, request: ThreadStartRequest) -> Result<ThreadHandle>;
    async fn send_input(&self, thread: ThreadHandle, input: ThreadInput) -> Result<()>;
    async fn close_thread(&self, thread: ThreadHandle, mode: CloseMode) -> Result<ThreadStatus>;
    async fn status(&self, thread: ThreadHandle) -> Result<ThreadStatus>;
    async fn list_children(&self, thread: ThreadHandle) -> Result<Vec<ThreadHandle>>;
    fn event_stream(&self, thread: ThreadHandle) -> BoxStream<'static, EventMsg>;
    async fn restore_thread(&self, metadata: PersistedThreadMetadata) -> Result<ThreadHandle>;
}
```

`ThreadProviderDescriptor` is the read-only discovery shape for that contract:

- `id` and `kind` identify the provider owner.
- `agentTypes` is provider scoped. Native exposes Morpheus roles; external CLI
  providers currently expose none.
- `modelSelection` describes where model choice comes from: catalog,
  provider default, none, or later an external config catalog.
- `capabilities` gates active requests and UI controls only. Event consumption
  never switches on capabilities.

## Event Normalization

Adapters own raw message parsing:

- Native Morpheus runtime emits existing `EventMsg` values directly from the
  session, tool runtime, agent control, and command runtime.
- Claude stream-json, OpenCode SSE/HTTP, and Codex CLI app-server transports
  parse provider-specific messages in the external adapter and emit bounded
  `EventMsg` values for assistant output, tool calls, tool results, lifecycle
  status, completion, and errors.
- Raw provider stdout, provider JSON envelopes, assistant marker text, and
  transport logs are never display facts. They may be retained only as bounded
  diagnostics behind adapter-owned errors.

Unsupported active operations return typed unsupported-action errors at the
provider boundary. They do not alter replay or display handling.

## API And Client Flow

The compatible migration path is:

1. `threadProvider/list` returns provider descriptors for a cwd.
2. Existing `agentType/list` and `model/list` stay available for legacy clients.
3. `ThreadStartParams.threadProvider` is optional. Omitted or `native` keeps
   current behavior. External ids are advertised but rejected by `thread/start`
   until their root start path is migrated.
4. Root-worker New conversation first selects provider, then shows provider
   scoped roles and model selection:
   provider -> agent role/type -> model provider/model -> reasoning/service
   tier -> create.
5. External providers with `modelSelection: providerDefault` disable global
   model pickers instead of borrowing unrelated config models.

## Runtime Boundaries

- `thread/start`, `thread/read`, `thread/resume`, `thread/list`, status
  notifications, followup input, close/cancel/archive, fork, compact, workflow,
  goals, schedules, command sessions, approvals, sandbox profiles, and dynamic
  tools should target a provider-neutral handle.
- Provider descriptors may disable active calls such as compact, workflow,
  command sessions, permissions, or `poll_event`; if a provider emits a valid
  event for any displayed item, downstream replay still handles it through the
  typed `EventMsg -> ThreadItem` path.
- Rollout `Limited` remains the reload contract. Any provider event needed for
  reload must be persisted in the view consumed by thread history/replay with a
  bounded payload.
- Root-worker tree and right-panel state consume normalized thread metadata and
  typed thread items. Parent-child edges must come from thread metadata/spawn
  edges, not orphan promotion in the client.

## First Phase

Implemented now:

- Protocol types for `ThreadProviderDescriptor`, provider capabilities, scoped
  model selection, and `threadProvider/list`.
- Optional `ThreadStartParams.threadProvider`; only omitted or `native` is
  accepted by the current native `thread/start`.
- App-server catalog descriptor source for native Morpheus plus external
  `claude_cli`, `opencode`, and `codex_cli` skeleton descriptors.
- Root-worker New conversation provider selector and provider-scoped gating for
  agent type and model fields.

## Later Phases

- Move native thread start/status/input/close/list/restore behind a concrete
  native `ThreadProvider`.
- Move external spawn registry and live snapshot logic behind external provider
  handles.
- Persist external provider thread metadata and bounded normalized events so
  completed external threads can be listed after reload and interrupted running
  sessions have explicit terminal state.
- Add provider-scoped external model catalogs where the provider can enumerate
  them.
- Route compact/workflow/goal/schedule/tool availability through provider
  capabilities and typed unsupported-action errors.
- Remove temporary duplicate catalog paths once root-worker and other clients
  consume provider descriptors by default.

## Test Matrix

- Native default `thread/start` without provider remains equivalent to legacy
  behavior.
- Native provider descriptor includes Morpheus roles and catalog-backed model
  providers.
- External descriptors are listed but expose no native agent types and no
  native-only capabilities.
- Root-worker provider selector defaults to native, disables model controls for
  provider-default external providers, and preserves legacy `agentType` and
  model fields for native.
- Thread read/list/resume/status display continues to consume typed
  `EventMsg -> ThreadItem` facts without raw provider parsing.
