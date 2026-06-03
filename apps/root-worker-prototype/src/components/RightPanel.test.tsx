import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type { RightPanelView, Thread, ThreadPlanUpdate } from "../types";

(globalThis as typeof globalThis & { React: typeof React }).React = React;
const { RightPanel } = await import("./RightPanel");

function makeThread(items: Thread["turns"][number]["items"]): Thread {
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
    status: { type: "idle" },
    path: null,
    cwd: "/tmp",
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
        items,
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

function renderRightPanel(
  thread: Thread | null,
  activeView: RightPanelView = "skills",
  planUpdate: ThreadPlanUpdate | null = thread?.latestPlan ?? null,
) {
  return renderToStaticMarkup(
    <RightPanel
      activeView={activeView}
      availableSkillCount={0}
      onCreateRootThread={() => {}}
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onSelectTaskThread={() => {}}
      onSetActiveView={() => {}}
      onSetTaskFilter={() => {}}
      planUpdate={planUpdate}
      preview={null}
      previewError={null}
      previewLoading={false}
      skills={[]}
      selectedThreadId={null}
      thread={thread}
      taskFilter="all"
      todoItems={[]}
    />,
  );
}

test("renders thread analysis title and monitor empty states", () => {
  const markup = renderRightPanel(null);

  assert.match(markup, /Thread Analysis/);
  assert.match(markup, /Context Window Used/);
  assert.match(markup, /No file watches\./);
  assert.match(markup, /No process listeners\./);
  assert.match(markup, /No scheduled listeners\./);
});

test("renders filesystem, process, and schedule subscriptions", () => {
  const markup = renderRightPanel(
    makeThread([
      {
        type: "eventDrivenToolCall",
        id: "fs-1",
        tool: "fs_subscribe",
        arguments: { path: "/tmp/out.log", label: "build log" },
        status: "completed",
        output: null,
      },
      {
        type: "eventDrivenToolCall",
        id: "process-1",
        tool: "process_exit_subscribe",
        arguments: { session_id: 42 },
        status: "completed",
        output: null,
      },
      {
        type: "eventDrivenToolCall",
        id: "schedule-1",
        tool: "schedule_subscribe",
        arguments: { schedule: "once_after:60", label: "standup ping" },
        status: "completed",
        output: null,
      },
      {
        type: "eventDrivenTool",
        id: "fs-event-1",
        tool: "fs_subscribe",
        title: "File watch triggered",
        text: "[File subscription (build log)] File changed: /tmp/out.log",
      },
    ]),
  );

  assert.doesNotMatch(markup, /build log/);
  assert.doesNotMatch(markup, /File changed: \/tmp\/out\.log/);
  assert.match(markup, /No file watches\./);
  assert.match(markup, /Session 42/);
  assert.match(markup, /standup ping/);
  assert.match(markup, /once_after:60/);
});

test("renders the current thread plan in the todo panel", () => {
  const planUpdate = {
    threadId: "thread-1",
    turnId: "turn-1",
    explanation: "Keep the change scoped.",
    plan: [
      { step: "Filter direct child tasks", status: "completed" },
      { step: "Render current thread plan", status: "inProgress" },
      { step: "Run validation", status: "pending" },
    ],
  } satisfies ThreadPlanUpdate;
  const markup = renderRightPanel(
    makeThread([]),
    "todo",
    planUpdate,
  );

  assert.match(markup, /Keep the change scoped\./);
  assert.match(markup, /Filter direct child tasks/);
  assert.match(markup, /Render current thread plan/);
  assert.match(markup, /Run validation/);
  assert.match(markup, /In progress/);
});
