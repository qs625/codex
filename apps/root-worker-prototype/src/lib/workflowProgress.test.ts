import test from "node:test";
import assert from "node:assert/strict";

import type {
  Thread,
  ThreadWorkflowRunProgressEvent,
  ThreadWorkflowRunProgressKind,
  WorkflowSummary,
} from "../types";
import {
  buildWorkflowPanelViewModel,
  collectWorkflowProgressRecords,
  formatWorkflowStatus,
} from "./workflowProgress";

const FEATURE_DEV: WorkflowSummary = {
  id: "feature-dev",
  name: "Feature Development",
  description: "Research, implement, review, and verify.",
  source: "project",
  path: "/repo/.codex/workflows/feature-dev",
  entry: "workflow.ts",
  version: "0.1.0",
  whenToUse: [],
  inputs: {},
};

function workflowEvent(
  runId: string,
  kind: ThreadWorkflowRunProgressKind,
  updatedAt: number,
  overrides: Partial<ThreadWorkflowRunProgressEvent> = {},
): ThreadWorkflowRunProgressEvent {
  return {
    runId,
    workflowId: "feature-dev",
    status: kind,
    runnerStatus: kind === "started" || kind === "resumed" ? "running" : kind,
    kind,
    message: `${kind} message`,
    updatedAt,
    ...overrides,
  };
}

function threadWithWorkflowEvents(
  events: ThreadWorkflowRunProgressEvent[],
): Thread {
  return {
    id: "thread-1",
    sessionId: "session-1",
    forkedFromId: null,
    preview: "",
    ephemeral: false,
    modelProvider: "openai",
    model: "gpt-5",
    reasoningEffort: null,
    createdAt: 1,
    updatedAt: 1,
    lifecycleStatus: { type: "complete" },
    path: null,
    cwd: "/repo",
    cliVersion: "test",
    source: "cli",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name: null,
    skills: [],
    turns: [
      {
        id: "turn-1",
        items: events.map((event, index) => ({
          type: "workflowRunProgress",
          id: `workflow-${index}`,
          event,
        })),
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 1,
        durationMs: 0,
      },
    ],
  };
}

test("collectWorkflowProgressRecords restores progress from thread items", () => {
  const records = collectWorkflowProgressRecords(
    threadWithWorkflowEvents([
      workflowEvent("wf_1", "started", 10),
      workflowEvent("wf_1", "completed", 12),
    ]),
  );

  assert.equal(records.length, 2);
  assert.equal(records[0].event.kind, "started");
  assert.equal(records[1].event.kind, "completed");
});

test("buildWorkflowPanelViewModel selects latest active run before terminal runs", () => {
  const model = buildWorkflowPanelViewModel(
    threadWithWorkflowEvents([
      workflowEvent("wf_done", "completed", 20),
      workflowEvent("wf_running", "started", 10),
    ]),
    [FEATURE_DEV],
  );

  assert.equal(model.selectedRun?.runId, "wf_running");
  assert.equal(model.selectedRun?.statusTone, "running");
  assert.deepEqual(
    model.selectedRun?.stages.map((stage) => stage.label),
    ["Research", "Implement", "Review/Fix", "Verify"],
  );
  assert.equal(model.selectedRun?.graphSource, "fallback");
});

test("buildWorkflowPanelViewModel surfaces failed and aborted terminal states", () => {
  const failed = buildWorkflowPanelViewModel(
    threadWithWorkflowEvents([workflowEvent("wf_failed", "failed", 10)]),
    [FEATURE_DEV],
  );
  const aborted = buildWorkflowPanelViewModel(
    threadWithWorkflowEvents([workflowEvent("wf_aborted", "aborted", 11)]),
    [FEATURE_DEV],
  );

  assert.equal(failed.selectedRun?.statusTone, "failed");
  assert.equal(failed.selectedRun?.stages[0]?.status, "failed");
  assert.equal(aborted.selectedRun?.statusTone, "aborted");
  assert.equal(aborted.selectedRun?.stages[0]?.status, "aborted");
});

test("buildWorkflowPanelViewModel handles missing graph metadata", () => {
  const model = buildWorkflowPanelViewModel(
    threadWithWorkflowEvents([
      workflowEvent("wf_custom", "started", 10, { workflowId: "custom-flow" }),
    ]),
    [],
  );

  assert.equal(model.selectedRun?.workflowName, "custom-flow");
  assert.equal(model.selectedRun?.graphSource, "missing");
  assert.deepEqual(model.selectedRun?.stages, []);
});

test("buildWorkflowPanelViewModel limits timeline to latest events first", () => {
  const events = Array.from({ length: 12 }, (_, index) =>
    workflowEvent("wf_1", index % 2 === 0 ? "started" : "resumed", 100 + index, {
      message: `event ${index}`,
    }),
  );
  const model = buildWorkflowPanelViewModel(threadWithWorkflowEvents(events), [FEATURE_DEV]);

  assert.equal(model.selectedRun?.timeline.length, 10);
  assert.equal(model.selectedRun?.timeline[0]?.message, "event 11");
  assert.equal(model.selectedRun?.timeline[9]?.message, "event 2");
});

test("formatWorkflowStatus handles string and tagged object status", () => {
  assert.equal(formatWorkflowStatus("running"), "running");
  assert.equal(formatWorkflowStatus({ completed: "done" }), "completed: done");
  assert.equal(formatWorkflowStatus({ aborted: null }), "aborted");
});
