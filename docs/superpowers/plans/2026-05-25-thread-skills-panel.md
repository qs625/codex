# Thread Skills Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a persisted and realtime thread-level skills aggregate to app-server and surface it in the `apps/root-worker-prototype` right panel.

**Architecture:** Extend app-server protocol `Thread` payloads with a unified `skills` aggregate and emit incremental `thread/skillUpdated` notifications when explicit or implicit skill usage changes thread state. Persist that aggregate with thread state so `thread/read` can restore it, then wire the prototype Electron bridge and React panel to render the new `Skills` view.

**Tech Stack:** Rust app-server/app-server-protocol/core, TypeScript React + Electron, Vite

---

### Task 1: Protocol Shapes

**Files:**
- Modify: `codex-rs/app-server-protocol/src/protocol/v2/thread_data.rs`
- Modify: `codex-rs/app-server-protocol/src/protocol/common.rs`
- Test: `codex-rs/app-server/tests/suite/v2/thread_read.rs`

- [ ] **Step 1: Write the failing protocol expectation test**

Add a focused app-server test that reads a thread and expects `thread.skills` to be present on the wire, even when empty, plus a notification serialization check for the new method.

```rust
let response = client.thread_read(json!({
    "threadId": thread_id.to_string(),
    "includeTurns": false
})).await?;
let thread = response["result"]["thread"].as_object().expect("thread object");
assert!(thread.contains_key("skills"));
assert_eq!(thread["skills"], serde_json::json!([]));
```

- [ ] **Step 2: Run the targeted test to verify it fails**

Run: `cargo test -p codex-app-server thread_read -- --nocapture`
Expected: FAIL because `thread.skills` and the skill update notification type do not exist yet.

- [ ] **Step 3: Add protocol types and notification registration**

Introduce `ThreadSkillKind`, `ThreadSkill`, `ThreadSkillUpdatedNotification`, add `skills: Vec<ThreadSkill>` to `Thread`, and register `thread/skillUpdated` in the protocol common definitions.

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum ThreadSkillKind {
    Explicit,
    Implicit,
    All,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadSkill {
    pub name: String,
    pub path: String,
    pub kind: ThreadSkillKind,
}
```

- [ ] **Step 4: Re-run the protocol-targeted test**

Run: `cargo test -p codex-app-server thread_read -- --nocapture`
Expected: still FAIL until app-server materializes `skills`.

- [ ] **Step 5: Commit**

```bash
git add codex-rs/app-server-protocol/src/protocol/v2/thread_data.rs codex-rs/app-server-protocol/src/protocol/common.rs codex-rs/app-server/tests/suite/v2/thread_read.rs
git commit -m "feat: add thread skills protocol types"
```

### Task 2: App-server Thread Skills State

**Files:**
- Modify: `codex-rs/app-server/src/thread_state.rs`
- Modify: `codex-rs/app-server/src/request_processors/thread_processor.rs`
- Modify: `codex-rs/app-server/src/request_processors/thread_summary.rs`
- Test: `codex-rs/app-server/tests/suite/v2/thread_read.rs`

- [ ] **Step 1: Write failing aggregation tests**

Add tests for:

- explicit-only stored state returns `explicit`
- implicit-only stored state returns `implicit`
- mixed updates upgrade to `all`

```rust
assert_eq!(
    thread.skills,
    vec![ThreadSkill {
        name: "demo".to_string(),
        path: "/tmp/demo/SKILL.md".to_string(),
        kind: ThreadSkillKind::Explicit,
    }]
);
```

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run: `cargo test -p codex-app-server thread_read -- --nocapture`
Expected: FAIL because thread state does not store or materialize skills.

- [ ] **Step 3: Add thread-level aggregate state and merge rules**

Extend app-server thread state with a `skills` map keyed by normalized path and a merge helper that implements:

- missing + explicit => explicit
- missing + implicit => implicit
- explicit + implicit => all
- implicit + explicit => all

Materialize the aggregate into sorted `Vec<ThreadSkill>` whenever building `Thread`.

```rust
fn merge_kind(current: Option<ThreadSkillKind>, incoming: ThreadSkillKind) -> ThreadSkillKind {
    match (current, incoming) {
        (None, kind) => kind,
        (Some(ThreadSkillKind::All), _) => ThreadSkillKind::All,
        (Some(ThreadSkillKind::Explicit), ThreadSkillKind::Implicit) => ThreadSkillKind::All,
        (Some(ThreadSkillKind::Implicit), ThreadSkillKind::Explicit) => ThreadSkillKind::All,
        (Some(kind), _) => kind,
    }
}
```

- [ ] **Step 4: Re-run the targeted tests**

Run: `cargo test -p codex-app-server thread_read -- --nocapture`
Expected: PASS for aggregate readback tests.

- [ ] **Step 5: Commit**

```bash
git add codex-rs/app-server/src/thread_state.rs codex-rs/app-server/src/request_processors/thread_processor.rs codex-rs/app-server/src/request_processors/thread_summary.rs codex-rs/app-server/tests/suite/v2/thread_read.rs
git commit -m "feat: persist thread skills aggregate"
```

### Task 3: Explicit and Implicit Runtime Updates

**Files:**
- Modify: `codex-rs/core-skills/src/injection.rs`
- Modify: `codex-rs/core/src/skills.rs`
- Modify: `codex-rs/app-server/src/bespoke_event_handling.rs`
- Modify: `codex-rs/app-server/src/outgoing_message.rs`
- Test: `codex-rs/app-server/tests/suite/v2/thread_resume.rs`
- Test: `codex-rs/app-server/tests/suite/v2/thread_read.rs`

- [ ] **Step 1: Write failing realtime notification tests**

Add tests that start a thread, trigger explicit skill injection and implicit invocation, and assert:

- `thread/skillUpdated` is emitted on first change
- duplicate same-kind events do not emit again
- crossing from explicit/implicit to `all` emits a second update

```rust
assert_eq!(notification.method, "thread/skillUpdated");
assert_eq!(notification.params["skill"]["kind"], "all");
```

- [ ] **Step 2: Run the targeted realtime tests to verify they fail**

Run: `cargo test -p codex-app-server thread_resume -- --nocapture`
Expected: FAIL because no skill update notification is emitted.

- [ ] **Step 3: Emit and forward thread skill updates from explicit and implicit paths**

Hook explicit updates at the successful `SkillInjection` result path and implicit updates at the accepted `maybe_emit_implicit_skill_invocation(...)` path. Route both through one app-server mutation helper that updates thread state and emits `thread/skillUpdated` only on state change.

```rust
pub(crate) enum IncomingThreadSkillKind {
    Explicit,
    Implicit,
}
```

- [ ] **Step 4: Re-run the targeted realtime tests**

Run: `cargo test -p codex-app-server thread_resume -- --nocapture`
Expected: PASS for notification and upgrade semantics.

- [ ] **Step 5: Commit**

```bash
git add codex-rs/core-skills/src/injection.rs codex-rs/core/src/skills.rs codex-rs/app-server/src/bespoke_event_handling.rs codex-rs/app-server/src/outgoing_message.rs codex-rs/app-server/tests/suite/v2/thread_resume.rs codex-rs/app-server/tests/suite/v2/thread_read.rs
git commit -m "feat: emit realtime thread skill updates"
```

### Task 4: Prototype Thread Types and State

**Files:**
- Modify: `apps/root-worker-prototype/src/types.ts`
- Modify: `apps/root-worker-prototype/electron/main.cjs`
- Modify: `apps/root-worker-prototype/src/App.tsx`
- Test: `apps/root-worker-prototype/src/lib/thread.ts`

- [ ] **Step 1: Write a failing state-merge test**

Add a focused test helper around thread upsert/update logic that proves thread `skills` are preserved from reads and updated from realtime notifications by `path`.

```ts
expect(updated.skills).toEqual([
  { name: "demo", path: "/tmp/demo/SKILL.md", kind: "all" },
]);
```

- [ ] **Step 2: Run the frontend type/build verification to verify it fails**

Run: `pnpm --filter @my-codex/root-worker-prototype build`
Expected: FAIL because `Thread.skills`, `ThreadSkill`, and the new notification handling do not exist.

- [ ] **Step 3: Add thread skill types and bridge normalization**

Update the Electron bridge to normalize `thread.skills` and `thread/skillUpdated`, then update React thread state to upsert skill records by path.

```ts
type ThreadSkillKind = "explicit" | "implicit" | "all";

type ThreadSkill = {
  name: string;
  path: string;
  kind: ThreadSkillKind;
};
```

- [ ] **Step 4: Re-run the build verification**

Run: `pnpm --filter @my-codex/root-worker-prototype build`
Expected: PASS for type and state changes before UI wiring.

- [ ] **Step 5: Commit**

```bash
git add apps/root-worker-prototype/src/types.ts apps/root-worker-prototype/electron/main.cjs apps/root-worker-prototype/src/App.tsx apps/root-worker-prototype/src/lib/thread.ts
git commit -m "feat: wire prototype thread skill state"
```

### Task 5: Prototype Right Panel UI

**Files:**
- Modify: `apps/root-worker-prototype/src/components/RightPanel.tsx`
- Modify: `apps/root-worker-prototype/src/styles.css`
- Modify: `apps/root-worker-prototype/src/App.tsx`
- Test: `apps/root-worker-prototype/src/components/RightPanel.tsx`

- [ ] **Step 1: Write the failing UI expectation**

Add a focused render-level or pure-view test that expects:

- a new `Skills` rail view
- badge count from `thread.skills.length`
- two-line item layout with name on line 1 and `kind + path` on line 2

```tsx
expect(screen.getByText("demo-skill")).toBeVisible();
expect(screen.getByText("all")).toBeVisible();
expect(screen.getByText("/tmp/demo/SKILL.md")).toBeVisible();
```

- [ ] **Step 2: Run the frontend build or test command to verify it fails**

Run: `pnpm --filter @my-codex/root-worker-prototype build`
Expected: FAIL or remain incomplete because the `Skills` view is not implemented.

- [ ] **Step 3: Implement the right panel skills view**

Add the `Skills` rail entry, render empty state, and style the second line so the path is visibly de-emphasized relative to the name.

```tsx
<div className="thread-skill-meta">
  <span className={`thread-skill-kind kind-${skill.kind}`}>{skill.kind}</span>
  <span className="thread-skill-path">{skill.path}</span>
</div>
```

- [ ] **Step 4: Re-run the frontend build verification**

Run: `pnpm --filter @my-codex/root-worker-prototype build`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/root-worker-prototype/src/components/RightPanel.tsx apps/root-worker-prototype/src/styles.css apps/root-worker-prototype/src/App.tsx
git commit -m "feat: add thread skills right panel"
```

### Task 6: Formatting and Verification

**Files:**
- Modify: any files changed above

- [ ] **Step 1: Regenerate app-server schema fixtures**

Run: `just write-app-server-schema`
Expected: schema/type fixture updates included for the new thread and notification shapes.

- [ ] **Step 2: Run protocol tests**

Run: `cargo test -p codex-app-server-protocol`
Expected: PASS

- [ ] **Step 3: Run targeted app-server tests**

Run: `cargo test -p codex-app-server thread_read thread_resume -- --nocapture`
Expected: PASS

- [ ] **Step 4: Run frontend build verification**

Run: `pnpm --filter @my-codex/root-worker-prototype build`
Expected: PASS

- [ ] **Step 5: Format Rust changes**

Run: `cd codex-rs && just fmt`
Expected: PASS

- [ ] **Step 6: Run scoped lints for touched Rust crates**

Run: `cd codex-rs && just fix -p codex-app-server -p codex-app-server-protocol`
Expected: PASS
