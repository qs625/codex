# Thread Skills Panel Design

## Goal

Add a right-side thread skills panel in `apps/root-worker-prototype` that shows which skills a thread has truly used, updates immediately while the thread is running, and survives reloads through app-server persistence.

## Scope

This design covers:

- a unified thread-level skill usage data structure in app-server
- persistence of that structure with thread state
- a realtime notification for incremental skill usage updates
- a new `Skills` view in the prototype right panel

This design does not add per-turn skill breakdowns or a TUI implementation.

## User-visible behavior

When a thread uses a skill, the right panel should show it immediately.

The panel should display one row per unique skill, deduped by skill path:

- first line: skill name
- second line: `kind` badge and skill path on the same line

The path should be visually weaker than the name.

The panel should reflect three kinds of usage:

- `explicit`: the thread explicitly loaded the full `SKILL.md`
- `implicit`: the thread implicitly used the skill via implicit invocation detection
- `all`: both explicit and implicit usage happened for the same skill in the thread

On reload or thread re-open, the panel should show the same data from persisted thread state even before new realtime events arrive.

## Definitions

### Thread skill record

A thread skill record represents a unique skill path that the thread has used.

Proposed wire shape:

```ts
type ThreadSkillKind = "explicit" | "implicit" | "all";

type ThreadSkill = {
  name: string;
  path: string;
  kind: ThreadSkillKind;
};
```

### Usage rules

- `explicit` means full-skill injection happened successfully. This is the existing `SkillInjection` path.
- `implicit` means implicit skill invocation detection fired for the thread.
- `all` means both signals have occurred for the same skill path in the same thread.

This is a thread-level aggregate, not a turn log.

## Architecture

### 1. Unified thread field

Add a `skills` field to app-server protocol `Thread` responses and notifications that carry a `Thread`.

The field should default to an empty list and be populated anywhere app-server materializes a `Thread`, including:

- `thread/read`
- `thread/resume`
- `thread/fork`
- `thread/start`
- thread summary notifications where a `Thread` object is already returned

The field is the single source of truth for the prototype panel and for persistence recovery.

### 2. Thread-level persistence

Persist the full `skills` aggregate as part of thread state instead of trying to reconstruct it only from lossy history.

Reason:

- explicit skill injection is visible today, but implicit usage is not represented as a persisted thread item
- existing thread history is documented as lossy, so reconstructing this panel from turns/items is not reliable

Persistence should store the unified `ThreadSkill` structure, including `kind`.

### 3. Realtime updates

Add a dedicated app-server notification for thread skill updates.

Proposed shape:

```ts
type ThreadSkillUpdatedNotification = {
  threadId: string;
  skill: ThreadSkill;
};
```

The notification fires when a thread skill aggregate changes:

- a new skill path is first observed
- an existing skill path changes from `explicit` to `all`
- an existing skill path changes from `implicit` to `all`

No notification should be sent when an event would leave the aggregate unchanged.

### 4. Aggregation rules

Aggregate by normalized skill path.

State transitions:

- missing + explicit => `explicit`
- missing + implicit => `implicit`
- explicit + implicit => `all`
- implicit + explicit => `all`
- explicit + explicit => `explicit`
- implicit + implicit => `implicit`
- all + explicit => `all`
- all + implicit => `all`

If the same path is seen again with a different name, preserve the first stable name unless there is a known stronger canonical source already present in the code path. For this feature, preserving first name is sufficient.

## Backend data sources

### Explicit path

Use the existing successful `SkillInjection` flow as the explicit source.

The signal is only valid after:

- the skill was selected by `collect_explicit_skill_mentions(...)`
- `build_skill_injections(...)` successfully read the `SKILL.md`
- the resulting `SkillInjection` exists

This avoids counting metadata visibility or failed attempts as skill usage.

### Implicit path

Use the existing implicit invocation detection path as the implicit source.

The signal is only valid when `maybe_emit_implicit_skill_invocation(...)` has found a candidate and accepted it after dedupe.

This avoids counting merely available implicit skills as used.

## Backend implementation outline

### Protocol

Update `codex-rs/app-server-protocol`:

- add `ThreadSkillKind`
- add `ThreadSkill`
- add `skills: Vec<ThreadSkill>` to `Thread`
- add `ThreadSkillUpdatedNotification`
- add notification method registration in protocol common definitions

Regenerate app-server schema fixtures after protocol changes.

### App-server runtime state

Add thread skill aggregate storage to the app-server thread state that already survives across thread operations and can be materialized into protocol `Thread`.

Requirements:

- read current aggregate for `thread/read` and related responses
- update aggregate from both explicit and implicit sources
- emit `thread/skillUpdated` only on state change
- keep behavior thread-scoped

### Core to app-server handoff

The execution stack already knows when explicit or implicit skill usage happens, but app-server needs a stable channel to receive that signal.

Implementation should prefer the smallest integration point that already handles thread-scoped state mutation or event plumbing, rather than duplicating parsing in multiple layers.

The explicit integration point should be the successful `SkillInjection` result path.

The implicit integration point should be the accepted implicit invocation path after dedupe.

### Materializing `Thread.skills`

Whenever app-server constructs a `Thread`, include the current aggregate.

That includes:

- live thread snapshots
- stored thread snapshots
- summary notifications where a `Thread` object is returned

For threads without any recorded skill usage, return `skills: []`.

## Frontend implementation outline

### Electron bridge

Update `apps/root-worker-prototype/electron/main.cjs` to:

- normalize `thread.skills` on thread reads and thread-bearing notifications
- normalize the new realtime skill notification

### Frontend types

Update `apps/root-worker-prototype/src/types.ts` to add:

- `ThreadSkillKind`
- `ThreadSkill`
- `skills` on `Thread`
- the new notification payload shape

### App state

Update `apps/root-worker-prototype/src/App.tsx` to:

- keep `skills` on in-memory threads
- upsert `skills` from `thread/read`
- handle realtime skill notifications
- update the selected thread immediately when the notification targets the current thread

### Right panel

Update [RightPanel.tsx](/Users/bytedance/Projects/my-codex/apps/root-worker-prototype/src/components/RightPanel.tsx):

- add a `Skills` view to the panel rail
- show count badge from `thread.skills.length`
- render a skills list for the selected thread
- render empty state when no skills exist

Per-item layout:

- line 1: skill name
- line 2: kind badge on the left, path on the same line

Styling requirements:

- name is the dominant text
- path uses smaller text and lower contrast than the name
- kind badge stays visually readable without overpowering the name

## Error handling

- Failed explicit `SKILL.md` reads should not create a skill record.
- Duplicate explicit or implicit events for the same final state should be ignored.
- Realtime notification delivery loss should be recoverable by re-reading the thread and consuming persisted `skills`.
- Unknown `kind` values from the wire should fail closed in typed code and default to no render only if a temporary compatibility guard is required in the prototype bridge.

## Testing

### Backend

Add tests that cover:

- explicit-only thread skill aggregation
- implicit-only thread skill aggregation
- explicit then implicit upgrades to `all`
- implicit then explicit upgrades to `all`
- duplicate events do not emit duplicate notifications
- `thread/read` returns persisted `skills`
- live notifications deliver the updated aggregate shape

Run:

- `cargo test -p codex-app-server-protocol`
- targeted app-server tests for thread read / notifications / skill updates

If protocol shapes change, also run:

- `just write-app-server-schema`

### Frontend

Add tests for:

- right panel skills empty state
- right panel renders name + kind + path layout
- realtime notification updates selected thread view immediately
- stored `skills` populate the panel on initial thread load

Use the existing frontend test setup for the prototype if present; otherwise add focused component/state tests instead of broad renderer coverage.

## Tradeoffs

### Why not derive from thread turns

Thread history is explicitly lossy, and implicit skill usage is not currently represented as persisted thread items. A panel based on turn reconstruction would be incomplete and drift-prone.

### Why one unified structure instead of separate explicit and implicit lists

The user wants one panel and one persisted concept. A unified aggregate avoids duplicated UI rows and keeps persistence and realtime merge logic simple.

### Why `all` instead of an array of kinds

The three-state enum is enough for the current UI, reduces payload complexity, and keeps upgrade logic simple.

## Open constraints resolved

- The panel is for `apps/root-worker-prototype`, not TUI.
- The right panel should update immediately while the thread is running.
- Persistence is required, so realtime alone is insufficient.
- The UI should show both explicit and implicit skill usage.
- The path must be visually de-emphasized relative to the name.
- The second and third pieces of metadata should share one line in the UI.
