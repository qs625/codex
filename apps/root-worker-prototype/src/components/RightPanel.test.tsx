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
import { CHAT_COMPAT_CWD_BASENAME } from "../lib/chatCompat";

(globalThis as typeof globalThis & { React: typeof React }).React = React;
const { RightPanel, ScheduleAgendaDateGroup } = await import("./RightPanel");

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
    todoItems?: React.ComponentProps<typeof RightPanel>["todoItems"];
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
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onOpenTreeFile={() => {}}
      onSetActiveView={() => {}}
      onSetFilePanelView={() => {}}
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
      thread={thread}
      todoItems={options?.todoItems ?? []}
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
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onOpenTreeFile={() => {}}
      onSetActiveView={() => {}}
      onSetFilePanelView={() => {}}
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
      thread={makeThread([])}
      todoItems={[]}
    />,
  );

  assert.match(markup, /Thread Goal/);
  assert.match(markup, /Goal active/);
  assert.match(markup, /Pause/);
  assert.match(markup, /Ship the slash goal display\./);
  assert.match(markup, /12K \/ 50K tokens/);
});

test("omits plan work queue from thread analysis", () => {
  const markup = renderRightPanel(makeThread([]), "skills", null, {
    todoItems: [
      {
        id: "task-1",
        title: "Wire plan into analysis",
        ownerPath: "/my_codex/owner_dev",
        status: "doing",
        statusLabel: "Running",
        updatedLabel: "just now",
        summary: "Move the existing work queue into the analysis view.",
        threadId: "thread-1",
      },
    ],
  });

  assert.match(markup, /Thread Analysis/);
  assert.doesNotMatch(markup, /Plan Work/);
  assert.doesNotMatch(markup, /Execution Queue/);
  assert.doesNotMatch(markup, /Open Project/);
  assert.doesNotMatch(markup, /Wire plan into analysis/);
  assert.doesNotMatch(markup, /New Task/);
  assert.doesNotMatch(markup, /Todo List/);
  assert.doesNotMatch(markup, /Todo Board/);
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
          type: "builtinToolCall",
          id: "schedule-1",
          tool: "schedule_subscribe",
          arguments: {
            schedule: { kind: "every_interval", interval_ms: 21_600_000 },
            label: "standup ping",
          },
          status: "completed",
          output: {
            subscription_id: "sub-schedule",
            schedule_summary: "every 21600000 ms",
          },
        },
      ],
      { type: "idle", reason: "waitCommand" },
    ),
  );

  assert.match(markup, /tail -f \/tmp\/out\.log/);
  assert.doesNotMatch(markup, /changed:\/tmp\/out\.log/);
  assert.doesNotMatch(markup, /No live commands\./);
  assert.match(markup, /standup ping/);
  assert.match(markup, /every_interval 6h/);
  assert.match(markup, /Upcoming/);
  assert.match(markup, /aria-expanded="true"/);
  assert.match(markup, /2 items/);
  assert.match(markup, /Every 6 hours/);
  assert.doesNotMatch(markup, /every 21600000 ms/);
});

test("renders schedule agenda groups expanded by default", () => {
  const markup = renderToStaticMarkup(
    <ScheduleAgendaDateGroup
      group={{
        dateKey: "2026-07-13",
        dateLabel: "Today",
        items: [
          {
            id: "schedule-1:2026-07-13T09:00:00.000Z",
            subscriptionId: "schedule-1",
            label: "standup ping",
            rule: "Every 6 hours",
            startsAt: "2026-07-13T09:00:00.000Z",
            timeLabel: "09:00",
          },
        ],
      }}
      collapsed={false}
      onToggle={() => {}}
    />,
  );

  assert.match(markup, /aria-expanded="true"/);
  assert.match(markup, /aria-controls="schedule-agenda-items-2026-07-13"/);
  assert.match(markup, /Today/);
  assert.match(markup, /standup ping/);
  assert.match(markup, /Every 6 hours/);
});

test("toggles schedule agenda groups from the date header", () => {
  let clicked = false;
  const element = ScheduleAgendaDateGroup({
    group: {
      dateKey: "2026-07-13",
      dateLabel: "Today",
      items: [
        {
          id: "schedule-1:2026-07-13T09:00:00.000Z",
          subscriptionId: "schedule-1",
          label: "standup ping",
          rule: "Every 6 hours",
          startsAt: "2026-07-13T09:00:00.000Z",
          timeLabel: "09:00",
        },
      ],
    },
    collapsed: false,
    onToggle: () => {
      clicked = true;
    },
  });
  const [button] = React.Children.toArray(
    (element.props as { children: React.ReactNode }).children,
  ) as React.ReactElement<{ onClick: () => void }>[];

  button.props.onClick();
  assert.equal(clicked, true);
});

test("collapses one schedule agenda date without hiding other dates", () => {
  const collapsedMarkup = renderToStaticMarkup(
    <ScheduleAgendaDateGroup
      group={{
        dateKey: "2026-07-13",
        dateLabel: "Today",
        items: [
          {
            id: "schedule-1:2026-07-13T09:00:00.000Z",
            subscriptionId: "schedule-1",
            label: "standup ping",
            rule: "Every 6 hours",
            startsAt: "2026-07-13T09:00:00.000Z",
            timeLabel: "09:00",
          },
        ],
      }}
      collapsed={true}
      onToggle={() => {}}
    />,
  );
  const expandedMarkup = renderToStaticMarkup(
    <ScheduleAgendaDateGroup
      group={{
        dateKey: "2026-07-14",
        dateLabel: "Tomorrow",
        items: [
          {
            id: "schedule-2:2026-07-14T10:00:00.000Z",
            subscriptionId: "schedule-2",
            label: "daily digest",
            rule: "Daily 10:00 UTC",
            startsAt: "2026-07-14T10:00:00.000Z",
            timeLabel: "10:00",
          },
        ],
      }}
      collapsed={false}
      onToggle={() => {}}
    />,
  );

  assert.match(collapsedMarkup, /aria-expanded="false"/);
  assert.doesNotMatch(collapsedMarkup, /standup ping/);
  assert.match(expandedMarkup, /aria-expanded="true"/);
  assert.match(expandedMarkup, /daily digest/);
});

test("renders the current thread plan in thread analysis", () => {
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
    "skills",
    planUpdate,
  );

  assert.match(markup, /Thread Analysis/);
  assert.match(markup, /Keep the change scoped\./);
  assert.match(markup, /Filter direct child tasks/);
  assert.match(markup, /Render current thread plan/);
  assert.match(markup, /Run validation/);
  assert.match(markup, /In progress/);
  assert.doesNotMatch(markup, /Plan Work/);
  assert.doesNotMatch(markup, /Execution Queue/);
  assert.doesNotMatch(markup, /Todo List/);
});

test("does not render todo items inside thread analysis", () => {
  const markup = renderRightPanel(makeThread([]), "skills", null, {
    todoItems: [
      {
        id: "task-1",
        title: "Wire plan into analysis",
        ownerPath: "/my_codex/owner_dev",
        status: "doing",
        statusLabel: "Running",
        updatedLabel: "just now",
        summary: "Move the existing work queue into the analysis view.",
        threadId: "thread-1",
      },
    ],
  });

  assert.match(markup, /Thread Analysis/);
  assert.doesNotMatch(markup, /Wire plan into analysis/);
  assert.doesNotMatch(markup, /Move the existing work queue into the analysis view\./);
  assert.doesNotMatch(markup, /\/my_codex\/owner_dev/);
  assert.doesNotMatch(markup, /No tasks for this filter/);
  assert.doesNotMatch(markup, /Todo List/);
  assert.doesNotMatch(markup, /Todo Board/);
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

test("hides chat compat cwd from the preview tree", () => {
  const thread = {
    ...makeThread([]),
    cwd: `/tmp/root-worker/${CHAT_COMPAT_CWD_BASENAME}`,
  };
  const markup = renderRightPanel(thread, "preview", null, {
    filePanelView: "tree",
    fileTreeEntriesByPath: {
      [thread.cwd]: [{ path: `${thread.cwd}/scratch.txt`, name: "scratch.txt", kind: "file" }],
    },
  });

  assert.match(markup, /CWD Tree/);
  assert.match(markup, /This chat has no project cwd to browse\./);
  assert.doesNotMatch(markup, /Thread cwd file tree/);
  assert.doesNotMatch(markup, /scratch\.txt/);
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
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onOpenTreeFile={() => {}}
      onSetActiveView={() => {}}
      onSetFilePanelView={() => {}}
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
      thread={thread}
      todoItems={[]}
    />,
  );

  assert.match(markup, /Permission denied/);
  assert.doesNotMatch(markup, /Empty directory/);
});
