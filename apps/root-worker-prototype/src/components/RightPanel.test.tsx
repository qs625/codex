import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type {
  FilePanelView,
  FilePreview,
  FileTreeEntry,
  RightPanelView,
  Thread,
  ThreadPlanUpdate,
  ThreadLifecycleStatus,
  ThreadWorkflowRunProgressKind,
  WorkflowSummary,
} from "../types";
import { CHAT_COMPAT_CWD_BASENAME } from "../lib/chatCompat";

(globalThis as typeof globalThis & { React: typeof React }).React = React;
const {
  RightPanel,
  ScheduleAgendaDateGroup,
  buildGitGraphTopology,
  filePreviewRenderMode,
  resolveMarkdownPreviewLocalFileTarget,
} = await import("./RightPanel");

const FEATURE_DEV_WORKFLOW: WorkflowSummary = {
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

function makeThread(
  items: Thread["turns"][number]["items"],
  lifecycleStatus: ThreadLifecycleStatus = { type: "complete" },
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
    lifecycleStatus,
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

function makeWorkflowProgressItem(
  kind: ThreadWorkflowRunProgressKind,
  overrides: Partial<
    Extract<Thread["turns"][number]["items"][number], { type: "workflowRunProgress" }>["event"]
  > = {},
): Extract<Thread["turns"][number]["items"][number], { type: "workflowRunProgress" }> {
  return {
    id: `workflow-${kind}-${overrides.runId ?? "wf_1"}`,
    type: "workflowRunProgress",
    event: {
      runId: "wf_1",
      workflowId: "feature-dev",
      status: kind,
      runnerStatus: kind === "started" || kind === "resumed" ? "running" : kind,
      kind,
      message: `${kind} message`,
      updatedAt: 1_785_737_385,
      ...overrides,
    },
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
    isCollapsed?: boolean;
    preview?: FilePreview | null;
    todoItems?: React.ComponentProps<typeof RightPanel>["todoItems"];
  },
) {
  return renderToStaticMarkup(
    <RightPanel
      activeView={activeView}
      availableSkillCount={0}
      availableWorkflows={[FEATURE_DEV_WORKFLOW]}
      isCollapsed={options?.isCollapsed ?? false}
      expandedTreeDirectories={options?.expandedTreeDirectories ?? []}
      filePanelView={options?.filePanelView ?? "preview"}
      fileTreeEntriesByPath={options?.fileTreeEntriesByPath ?? {}}
      fileTreeErrorsByPath={{}}
      fileTreeLoadingPath={null}
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onOpenTreeFile={() => {}}
      onSetActiveView={() => {}}
      onSetCollapsed={() => {}}
      onSetFilePanelView={() => {}}
      onToggleTreeDirectory={() => {}}
      onCancelGoal={() => {}}
      onPauseGoal={() => {}}
      onResumeGoal={() => {}}
      planUpdate={planUpdate}
      goal={null}
      goalAction={null}
      goalActionError={null}
      preview={options?.preview ?? null}
      previewError={null}
      previewLoading={false}
      skills={[]}
      thread={thread}
      todoItems={options?.todoItems ?? []}
    />,
  );
}

function makePreview(overrides: Partial<FilePreview> = {}): FilePreview {
  return {
    path: "/tmp/README.md",
    displayPath: "README.md",
    content: "",
    language: "markdown",
    line: null,
    column: null,
    lsp: {
      enabled: false,
      languageId: null,
      lspStatus: {
        phase: "plain",
        detail: null,
      },
      serverLabel: null,
      workspaceRoot: null,
      reason: null,
    },
    image: null,
    ...overrides,
  };
}

test("resolves markdown preview relative links from the current file directory", () => {
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget("/tmp/docs/README.md", "./other.md"),
    "/tmp/docs/other.md",
  );
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget("/tmp/docs/guides/README.md", "../other.md"),
    "/tmp/docs/other.md",
  );
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget("C:\\repo\\docs\\README.markdown", ".\\other.md"),
    "C:\\repo\\docs\\other.md",
  );
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget("/tmp/docs/README.md", "/tmp/other.md"),
    "/tmp/other.md",
  );
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget("/tmp/docs/README.md", "file:///tmp/other.md"),
    "file:///tmp/other.md",
  );
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget("/tmp/docs/README.md", "~/other.md"),
    "~/other.md",
  );
});

test("renders thread analysis title and monitor empty states", () => {
  const markup = renderRightPanel(null);

  assert.match(markup, /Thread Analysis/);
  assert.match(markup, /Context Window Used/);
  assert.match(markup, /No live commands\./);
  assert.match(markup, /No scheduled listeners\./);
});

test("renders browser panel and rail button", () => {
  const markup = renderRightPanel(makeThread([]), "browser");

  assert.match(markup, /aria-label="Browser"/);
  assert.match(markup, /Browser URL/);
  assert.match(markup, /class="browser-go-button" disabled=""/);
  assert.match(markup, /Open a page in the right panel/);
});

test("collapsed browser panel keeps rail and omits browser content", () => {
  const markup = renderRightPanel(makeThread([]), "browser", null, {
    isCollapsed: true,
  });

  assert.match(markup, /right-panel collapsed/);
  assert.match(markup, /aria-label="Browser"/);
  assert.doesNotMatch(markup, /Browser URL/);
  assert.doesNotMatch(markup, /Open a page in the right panel/);
});

test("renders workflow empty state with available workflows", () => {
  const markup = renderRightPanel(makeThread([]), "workflow");

  assert.match(markup, /No workflow activity in this thread/);
  assert.match(markup, /Feature Development/);
  assert.match(markup, /aria-label="Workflow"/);
});

test("renders workflow run summary, feature-dev graph fallback, and timeline", () => {
  const markup = renderRightPanel(
    makeThread([
      makeWorkflowProgressItem("started", {
        message: "Workflow started",
        runnerStatus: "runner_starting",
      }),
    ]),
    "workflow",
  );

  assert.match(markup, /wf_1 · Started/);
  assert.match(markup, /runner_starting/);
  assert.match(markup, /Research/);
  assert.match(markup, /Implement/);
  assert.match(markup, /Review\/Fix/);
  assert.match(markup, /Verify/);
  assert.match(markup, /Using built-in feature-dev stage fallback/);
  assert.match(markup, /Workflow started/);
});

test("renders aborted workflow progress without marking it running", () => {
  const markup = renderRightPanel(
    makeThread([
      makeWorkflowProgressItem("aborted", {
        message: "User aborted",
        runnerStatus: "aborted",
      }),
    ]),
    "workflow",
  );

  assert.match(markup, /Aborted/);
  assert.match(markup, /User aborted/);
  assert.match(markup, /workflow-status-pill aborted/);
  assert.doesNotMatch(markup, /workflow-status-pill running/);
});

test("renders thread goal details in thread analysis", () => {
  const markup = renderToStaticMarkup(
    <RightPanel
      activeView="skills"
      availableSkillCount={0}
      availableWorkflows={[FEATURE_DEV_WORKFLOW]}
      isCollapsed={false}
      expandedTreeDirectories={[]}
      filePanelView="preview"
      fileTreeEntriesByPath={{}}
      fileTreeErrorsByPath={{}}
      fileTreeLoadingPath={null}
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onOpenTreeFile={() => {}}
      onSetActiveView={() => {}}
      onSetCollapsed={() => {}}
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

test("renders backend tool I/O buckets as top-level context categories", () => {
  const thread = makeThread([]);
  thread.contextUsage = {
    totalBytes: 4000,
    budgetUsedPercent: 2,
    categories: {
      compact: 0,
      skillsMetadata: 0,
      concreteSkills: 0,
      toolsMetadata: 200,
      toolCalls: 3800,
      userMessages: 0,
      llmMessages: 0,
      reasoning: 0,
    },
    loadedSkills: {
      loadedCount: 0,
      totalCount: 0,
      skills: [],
    },
    toolBreakdown: {
      applyPatch: { input: 1200, output: 300 },
      fileOperations: { input: 0, output: 0 },
      commands: { input: 700, output: 300 },
      interAgent: { input: 200, output: 300 },
      searchMedia: { input: 0, output: 0 },
      otherTools: { input: 0, output: 0 },
    },
  };

  const markup = renderRightPanel(thread);

  assert.doesNotMatch(markup, /Tool I\/O Detail/);
  assert.doesNotMatch(markup, /estimated/);
  assert.match(markup, /File Writes/);
  assert.match(markup, /Commands/);
  assert.match(markup, /Inter-Agent/);
  assert.doesNotMatch(markup, /Tool Inputs &amp; Results/);
  assert.doesNotMatch(markup, /in 1\.2 KB \/ out 300 B/);
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

  assert.match(markup, /Git graph/);
  assert.match(markup, /Graph/);
  assert.match(markup, /Changes/);
  assert.match(markup, /aria-expanded="true"/);
  assert.match(markup, /Collapse Changes/);
  assert.match(markup, /Select Git branch or ref/);
  assert.match(markup, /Resize Git graph and changes panes/);
  assert.match(markup, /panel-rail-badge">2/);
  assert.doesNotMatch(markup, /Thread File Deltas/);
});

test("builds structured git graph topology cells instead of display characters", () => {
  assert.deepEqual(buildGitGraphTopology("* "), [
    { lane: 0, commit: true, segments: ["vertical"] },
  ]);

  assert.deepEqual(buildGitGraphTopology("| * "), [
    { lane: 0, commit: false, segments: ["vertical"] },
    { lane: 1, commit: false, segments: [] },
    { lane: 2, commit: true, segments: ["vertical"] },
  ]);

  assert.deepEqual(buildGitGraphTopology("|/"), [
    { lane: 0, commit: false, segments: ["vertical"] },
    { lane: 1, commit: false, segments: ["diagonal-left"] },
  ]);

  assert.deepEqual(buildGitGraphTopology("|\\-_"), [
    { lane: 0, commit: false, segments: ["vertical"] },
    { lane: 1, commit: false, segments: ["diagonal-right"] },
    { lane: 2, commit: false, segments: ["horizontal"] },
    { lane: 3, commit: false, segments: ["horizontal"] },
  ]);
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

test("renders markdown file previews as markdown content", () => {
  const markup = renderRightPanel(makeThread([]), "preview", null, {
    preview: makePreview({
      content: "# Title\n\nThis is **bold**.\n\n[Other](./other.md)",
      language: "markdown",
    }),
  });

  assert.match(markup, /<h1>Title<\/h1>/);
  assert.match(markup, /This is <strong>bold<\/strong>\./);
  assert.match(markup, /href="#"/);
  assert.doesNotMatch(markup, /Loading editor/);
});

test("keeps non-markdown file previews on the editor render path", () => {
  assert.equal(
    filePreviewRenderMode(
      makePreview({
        path: "/tmp/src/App.tsx",
        displayPath: "src/App.tsx",
        content: "export const value = 1;",
        language: "typescript",
      }),
    ),
    "editor",
  );
});

test("keeps image file previews on the image path", () => {
  const markup = renderRightPanel(makeThread([]), "preview", null, {
    preview: makePreview({
      path: "/tmp/diagram.png",
      displayPath: "diagram.png",
      content: "",
      language: "plaintext",
      image: {
        path: "/tmp/diagram.png",
        mimeType: "image/png",
        name: "diagram.png",
        byteSize: 2048,
      },
    }),
  });

  assert.match(markup, /IMAGE/);
  assert.match(markup, /image\/png/);
  assert.match(markup, /diagram\.png/);
  assert.doesNotMatch(markup, /markdown-content/);
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
      availableWorkflows={[FEATURE_DEV_WORKFLOW]}
      isCollapsed={false}
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
      onSetCollapsed={() => {}}
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
