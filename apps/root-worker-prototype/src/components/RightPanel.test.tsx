import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type {
  FilePanelView,
  FileTreeEntry,
  RightPanelView,
  Thread,
  ThreadPlanUpdate,
  ThreadStatus,
} from "../types";

(globalThis as typeof globalThis & { React: typeof React }).React = React;
const { RightPanel } = await import("./RightPanel");

function makeThread(
  items: Thread["turns"][number]["items"],
  status: ThreadStatus = { type: "complete" },
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
    status,
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
  options?: {
    filePanelView?: FilePanelView;
    fileTreeEntriesByPath?: Record<string, FileTreeEntry[]>;
    expandedTreeDirectories?: string[];
  },
) {
  return renderToStaticMarkup(
    <RightPanel
      activeView={activeView}
      availableSkillCount={0}
      expandedTreeDirectories={options?.expandedTreeDirectories ?? []}
      filePanelView={options?.filePanelView ?? "preview"}
      fileTreeEntriesByPath={options?.fileTreeEntriesByPath ?? {}}
      fileTreeErrorsByPath={{}}
      fileTreeLoadingPath={null}
      onCreateRootThread={() => {}}
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onOpenTreeFile={() => {}}
      onSelectTaskThread={() => {}}
      onSetActiveView={() => {}}
      onSetFilePanelView={() => {}}
      onSetTaskFilter={() => {}}
      onToggleTreeDirectory={() => {}}
      onCancelGoal={() => {}}
      onPauseGoal={() => {}}
      onResumeGoal={() => {}}
      planUpdate={planUpdate}
      goal={null}
      goalAction={null}
      goalActionError={null}
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
  assert.match(markup, /No live commands\./);
  assert.match(markup, /No scheduled listeners\./);
});

test("renders thread goal details in thread analysis", () => {
  const markup = renderToStaticMarkup(
    <RightPanel
      activeView="skills"
      availableSkillCount={0}
      expandedTreeDirectories={[]}
      filePanelView="preview"
      fileTreeEntriesByPath={{}}
      fileTreeErrorsByPath={{}}
      fileTreeLoadingPath={null}
      onCreateRootThread={() => {}}
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onOpenTreeFile={() => {}}
      onSelectTaskThread={() => {}}
      onSetActiveView={() => {}}
      onSetFilePanelView={() => {}}
      onSetTaskFilter={() => {}}
      onToggleTreeDirectory={() => {}}
      onCancelGoal={() => {}}
      onPauseGoal={() => {}}
      onResumeGoal={() => {}}
      planUpdate={null}
      goal={{
        threadId: "thread-1",
        objective: "Ship the slash goal display.",
        status: "active",
        tokenBudget: 50_000,
        tokensUsed: 12_000,
        timeUsedSeconds: 125,
        createdAt: 1,
        updatedAt: 2,
      }}
      goalAction={null}
      goalActionError={null}
      preview={null}
      previewError={null}
      previewLoading={false}
      skills={[]}
      selectedThreadId="thread-1"
      thread={makeThread([])}
      taskFilter="all"
      todoItems={[]}
    />,
  );

  assert.match(markup, /Thread Goal/);
  assert.match(markup, /Goal active/);
  assert.match(markup, /Pause/);
  assert.match(markup, /Ship the slash goal display\./);
  assert.match(markup, /12K \/ 50K tokens/);
});

test("renders live commands and schedule subscriptions", () => {
  const markup = renderRightPanel(
    makeThread(
      [
        {
          type: "commandExecution",
          id: "command-1",
          command: "tail -f /tmp/out.log",
          cwd: "/tmp",
          status: "running",
          aggregatedOutput: "changed:/tmp/out.log\n",
          exitCode: null,
          durationMs: null,
        },
        {
          type: "eventDrivenToolCall",
          id: "schedule-1",
          tool: "schedule_subscribe",
          arguments: { schedule: "once_after:60", label: "standup ping" },
          status: "completed",
          output: null,
        },
      ],
      { type: "idle", reason: "waitCommand" },
    ),
  );

  assert.match(markup, /tail -f \/tmp\/out\.log/);
  assert.doesNotMatch(markup, /changed:\/tmp\/out\.log/);
  assert.doesNotMatch(markup, /No live commands\./);
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

test("renders git panel with deduped thread file changes", () => {
  const markup = renderRightPanel(
    makeThread([
      {
        type: "fileChange",
        id: "change-1",
        status: "completed",
        changes: [
          { path: "/tmp/src/app.tsx", kind: "modified" },
          { path: "/tmp/README.md", kind: "added" },
        ],
      },
      {
        type: "fileChange",
        id: "change-2",
        status: "completed",
        changes: [
          { path: "/tmp/src/app.tsx", kind: "deleted" },
        ],
      },
    ]),
    "git",
  );

  assert.match(markup, /Git Changes/);
  assert.match(markup, /Thread File Deltas/);
  assert.match(markup, /src\/app\.tsx/);
  assert.match(markup, /README\.md/);
  assert.match(markup, /Deleted/);
  assert.match(markup, /2 updates/);
});

test("renders cwd tree inside the preview panel", () => {
  const thread = makeThread([]);
  const markup = renderRightPanel(thread, "preview", null, {
    filePanelView: "tree",
    expandedTreeDirectories: ["/tmp/src"],
    fileTreeEntriesByPath: {
      "/tmp": [
        { path: "/tmp/src", name: "src", kind: "directory" },
        { path: "/tmp/README.md", name: "README.md", kind: "file" },
      ],
      "/tmp/src": [
        { path: "/tmp/src/App.tsx", name: "App.tsx", kind: "file" },
      ],
    },
  });

  assert.match(markup, /CWD Tree/);
  assert.match(markup, /Thread cwd file tree/);
  assert.match(markup, /README\.md/);
  assert.match(markup, /App\.tsx/);
});

test("renders directory-specific cwd tree errors instead of empty state", () => {
  const thread = makeThread([]);
  const markup = renderToStaticMarkup(
    <RightPanel
      activeView="preview"
      availableSkillCount={0}
      expandedTreeDirectories={["/tmp/src"]}
      filePanelView="tree"
      fileTreeEntriesByPath={{
        "/tmp": [{ path: "/tmp/src", name: "src", kind: "directory" }],
      }}
      fileTreeErrorsByPath={{ "/tmp/src": "Permission denied" }}
      fileTreeLoadingPath={null}
      onCreateRootThread={() => {}}
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onOpenTreeFile={() => {}}
      onSelectTaskThread={() => {}}
      onSetActiveView={() => {}}
      onSetFilePanelView={() => {}}
      onSetTaskFilter={() => {}}
      onToggleTreeDirectory={() => {}}
      onCancelGoal={() => {}}
      onPauseGoal={() => {}}
      onResumeGoal={() => {}}
      planUpdate={null}
      goal={null}
      goalAction={null}
      goalActionError={null}
      preview={null}
      previewError={null}
      previewLoading={false}
      skills={[]}
      selectedThreadId="thread-1"
      thread={thread}
      taskFilter="all"
      todoItems={[]}
    />,
  );

  assert.match(markup, /Permission denied/);
  assert.doesNotMatch(markup, /Empty directory/);
});
