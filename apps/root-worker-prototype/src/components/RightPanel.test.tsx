import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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
import type { RuntimeRestartProgress } from "../lib/runtimeRestartProgress";
import {
  createTerminalStateRequestSequencer,
  isTerminalCommandFocusRequestForThread,
  shouldApplyTerminalViewportFocusRequest,
} from "../lib/terminalCommandFocus";

(globalThis as typeof globalThis & { React: typeof React }).React = React;
const {
  RightPanel,
  ScheduleAgendaLayout,
  ScheduleAgendaDateGroup,
  beginFilePreviewEdit,
  beginFilePreviewSave,
  browserBoundsMatch,
  browserBoundsFromElement,
  browserTabLabel,
  buildGitGraphVisualModel,
  cancelFilePreviewEdit,
  completeFilePreviewSave,
  currentBrowserPanelApi,
  failFilePreviewSave,
  filePreviewCanEdit,
  filePreviewHeaderEditControlsVisible,
  filePreviewIdentity,
  filePreviewRenderMode,
  filePreviewSourceEditorVisible,
  GitChangeGroup,
  GitChangeRow,
  GitCommitFileRow,
  GitDiffPreviewPanel,
  gitDiffTargetForTreePath,
  gitRelativeTreePath,
  normalizeBrowserPanelState,
  nextBrowserBoundsSequence,
  resolveGitTreeFileOpen,
  resolveThreadAnalysisCommandFocus,
  resolvePreviewDefinitionPosition,
  resolveMarkdownPreviewLocalFileTarget,
  shouldClearBrowserLocalError,
  shouldClearGitDiffPreviewForFilePreviewChange,
  syncFilePreviewEditState,
  updateFilePreviewDraft,
} = await import("./RightPanel");

const FEATURE_DEV_WORKFLOW: WorkflowSummary = {
  id: "feature-dev",
  name: "Feature Development",
  description: "Research, implement, review, and verify.",
  source: "project",
  path: "/repo/.morpheus/workflows/feature-dev",
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((nextResolve) => {
    resolve = nextResolve;
  });
  return { promise, resolve };
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
    previewError?: string | null;
    previewLoading?: boolean;
    runtimeRestartProgress?: RuntimeRestartProgress | null;
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
      onPreviewUpdated={() => {}}
      previewRootId="root-1"
      onSetActiveView={() => {}}
      onSetCollapsed={() => {}}
      onSetFilePanelView={() => {}}
      onToggleTreeDirectory={() => {}}
      onCancelGoal={() => {}}
      onPauseGoal={() => {}}
      onResumeGoal={() => {}}
      planUpdate={planUpdate}
      runtimeRestartProgress={options?.runtimeRestartProgress ?? null}
      goal={null}
      goalAction={null}
      goalActionError={null}
      preview={options?.preview ?? null}
      previewError={options?.previewError ?? null}
      previewLoading={options?.previewLoading ?? false}
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
    pdf: null,
    ...overrides,
  };
}

function makeGitCommit(
  graph: string,
  hash: string,
  parents: string[] = ["parent"],
  subject = hash,
) {
  return {
    type: "commit" as const,
    graph,
    hash,
    shortHash: hash.slice(0, 7),
    parents,
    refs: [],
    subject,
    author: "Author",
    relativeTime: "now",
  };
}

function makeScheduleAgendaGroups() {
  return [
    {
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
    {
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
    },
  ];
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
  assert.match(markup, /context-section-card current-plan-card/);
  assert.match(markup, /No plan published yet\./);
  assert.match(markup, /Context Window Used/);
  assert.match(
    markup,
    /<div class="monitor-section-title"><span class="monitor-kind-dot command"><\/span><span>Live Commands<\/span><\/div><div class="monitor-empty">No live commands\.<\/div>/,
  );
  assert.match(
    markup,
    /<div class="monitor-section-title"><span class="monitor-kind-dot schedule"><\/span><span>Schedules<\/span><\/div><div class="monitor-empty">No scheduled listeners\.<\/div>/,
  );
  assert.match(markup, /No live commands\./);
  assert.match(markup, /No scheduled listeners\./);
});

test("renders browser panel and rail button", () => {
  const markup = renderRightPanel(makeThread([]), "browser");

  assert.match(markup, /aria-label="Browser"/);
  assert.match(markup, /aria-label="Browser tabs"/);
  assert.match(markup, /New tab/);
  assert.match(markup, /browser-new-tab-button/);
  assert.match(markup, /aria-label="New browser tab"/);
  assert.match(markup, /Browser URL/);
  assert.match(markup, /class="browser-go-button" disabled=""/);
  assert.match(markup, /Open a page in the right panel/);
});

test("browserBoundsFromElement measures the visible viewport rect with sequence", () => {
  const element = {
    getBoundingClientRect: () => ({
      left: 820.6,
      top: 168.5,
      width: 652.6,
      height: 782.5,
    }),
  } as HTMLElement;

  assert.deepEqual(browserBoundsFromElement(element, 12), {
    x: 821,
    y: 169,
    width: 653,
    height: 783,
    sequence: 12,
  });
});

test("nextBrowserBoundsSequence advances past manual wall-clock bounds", () => {
  const originalNow = Date.now;
  Date.now = () => 1_800_000;
  try {
    const sequence = { current: 9_901 };
    assert.equal(nextBrowserBoundsSequence(sequence), 1_800_000);
    assert.equal(sequence.current, 1_800_000);
    assert.equal(nextBrowserBoundsSequence(sequence), 1_800_001);
  } finally {
    Date.now = originalNow;
  }
});

test("browserBoundsMatch ignores sequence and detects layout movement", () => {
  assert.equal(
    browserBoundsMatch(
      { x: 900, y: 150, width: 700, height: 600, sequence: 42 },
      { x: 900, y: 150, width: 700, height: 600, sequence: 43 },
    ),
    true,
  );
  assert.equal(
    browserBoundsMatch(
      { x: 900, y: 150, width: 700, height: 600, sequence: 43 },
      { x: 1193, y: 169, width: 489, height: 888, sequence: 44 },
    ),
    false,
  );
});

test("browser native view hides under app overlays and restores with measured bounds", () => {
  const rightPanelSource = readFileSync(
    new URL("./RightPanel.tsx", import.meta.url),
    "utf8",
  );
  const appSource = readFileSync(new URL("../App.tsx", import.meta.url), "utf8");

  assert.match(
    rightPanelSource,
    /if \(nativeOverlayActive\) \{[\s\S]*\.hideBrowserView\(\)/,
  );
  assert.match(
    rightPanelSource,
    /else \{[\s\S]*const bounds = measureBounds\(\)[\s\S]*\.showBrowserView\(bounds\)/,
  );
  assert.match(
    rightPanelSource,
    /requestAnimationFrame\(watchBounds\)/,
  );
  assert.match(
    appSource,
    /browserNativeOverlayActive=\{[\s\S]*isSelfCommandOpen \|\| isSettingsOpen \|\| isCreatingChatThread/,
  );
});

test("browser tab helpers preserve active tab state and readable labels", () => {
  const state = normalizeBrowserPanelState({
    url: "https://active.example/docs",
    title: "Active page",
    loading: false,
    canGoBack: true,
    canGoForward: false,
    error: null,
    activeTabId: "tab-2",
    tabs: [
      {
        id: "tab-1",
        url: "https://old.example/",
        title: "Old page",
        loading: false,
        canGoBack: false,
        canGoForward: true,
        error: null,
      },
      {
        id: "tab-2",
        url: "https://active.example/docs",
        title: "Active page",
        loading: true,
        canGoBack: true,
        canGoForward: false,
        error: "Loading took too long",
      },
    ],
  });

  assert.equal(state.url, "https://active.example/docs");
  assert.equal(state.title, "Active page");
  assert.equal(state.loading, true);
  assert.equal(state.error, "Loading took too long");
  assert.equal(browserTabLabel(state.tabs[0]), "Old page");
  assert.equal(
    browserTabLabel({
      id: "tab-host",
      url: "https://docs.example/path",
      title: null,
      loading: false,
      canGoBack: false,
      canGoForward: false,
      error: null,
    }),
    "docs.example",
  );
  assert.equal(
    browserTabLabel({
      id: "tab-empty",
      url: null,
      title: null,
      loading: false,
      canGoBack: false,
      canGoForward: false,
      error: null,
    }),
    "New tab",
  );

  const legacyState = normalizeBrowserPanelState({
    url: "https://legacy.example/",
    title: "Legacy page",
    loading: false,
    canGoBack: false,
    canGoForward: false,
    error: null,
  });
  assert.equal(legacyState.activeTabId, "browser-tab-active");
  assert.equal(legacyState.tabs.length, 1);
  assert.equal(legacyState.tabs[0]?.title, "Legacy page");
});

test("browser successful state clears stale local errors", () => {
  const state = normalizeBrowserPanelState({
    activeTabId: "tab-1",
    tabs: [
      {
        id: "tab-1",
        url: "https://example.com/",
        title: "Example Domain",
        loading: false,
        canGoBack: false,
        canGoForward: false,
        error: null,
      },
    ],
  });

  assert.equal(shouldClearBrowserLocalError(state, state.tabs[0] ?? null), true);
});

test("browser real tab error keeps local error visible", () => {
  const state = normalizeBrowserPanelState({
    activeTabId: "tab-1",
    tabs: [
      {
        id: "tab-1",
        url: "https://offline.invalid/",
        title: null,
        loading: false,
        canGoBack: false,
        canGoForward: false,
        error: "Page failed to load",
      },
    ],
  });

  assert.equal(shouldClearBrowserLocalError(state, state.tabs[0] ?? null), false);
});

test("browser API detection requires tab actions", () => {
  const originalWindow = globalThis.window;
  const baseApi = {
    browserGoBack: async () => ({}),
    browserGoForward: async () => ({}),
    hideBrowserView: async () => ({}),
    navigateBrowserView: async () => ({}),
    openLink: async () => ({ ok: true }),
    reloadBrowserView: async () => ({}),
    setBrowserViewBounds: async () => ({}),
    showBrowserView: async () => ({}),
    stopBrowserView: async () => ({}),
    subscribeBrowserState: () => () => {},
  };
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: { codexDesktop: baseApi },
  });

  try {
    assert.equal(currentBrowserPanelApi(), null);
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: {
        codexDesktop: {
          ...baseApi,
          createBrowserTab: async () => ({}),
          selectBrowserTab: async () => ({}),
          closeBrowserTab: async () => ({}),
        },
      },
    });
    assert.ok(currentBrowserPanelApi());
  } finally {
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: originalWindow,
    });
  }
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

test("keeps live commands visible without output while rendering schedules", () => {
  const largeOutput = `${"changed:/tmp/out.log\n".repeat(400)}UNBOUNDED_RIGHT_PANEL_OUTPUT`;
  const activeCommand = {
    type: "commandExecution",
    id: "command-1",
    command: "tail -f /tmp/out.log",
    cwd: "/tmp",
    processId: "pid-1",
    status: "running",
    aggregatedOutput: largeOutput,
    exitCode: null,
    durationMs: null,
  } satisfies NonNullable<Thread["activeCommandItems"]>[number];
  const thread = {
    ...makeThread(
      [
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
    activeCommandItems: [activeCommand],
    stats: { compactionCount: 2 },
  } satisfies Thread;
  const markup = renderRightPanel(thread);

  assert.match(markup, /Live Commands/);
  assert.match(markup, /tail -f \/tmp\/out\.log/);
  assert.match(markup, /Open terminal for tail -f \/tmp\/out\.log/);
  assert.match(markup, /Lifetime/);
  assert.match(markup, /<span>Compactions<\/span><strong>2<\/strong>/);
  assert.doesNotMatch(markup, /changed:\/tmp\/out\.log/);
  assert.doesNotMatch(markup, /UNBOUNDED_RIGHT_PANEL_OUTPUT/);
  assert.doesNotMatch(markup, /No live commands\./);
  assert.match(markup, /standup ping/);
  assert.match(markup, /every_interval 6h/);
  assert.match(markup, /Upcoming/);
  assert.match(markup, /aria-expanded="true"/);
  assert.match(markup, /2 items/);
  assert.match(markup, /Every 6 hours/);
  assert.doesNotMatch(markup, /every 21600000 ms/);
  assert.deepEqual(
    resolveThreadAnalysisCommandFocus(thread, {
      id: "command-1",
      subscriptionId: "command-1",
      kind: "command",
      label: "tail -f /tmp/out.log",
      detail: "/tmp",
      status: "Running",
      eventCount: 0,
      latestEvent: null,
    }),
    {
      threadId: "thread-1",
      commandItemId: "command-1",
      processId: "pid-1",
      command: "tail -f /tmp/out.log",
      cwd: "/tmp",
      status: "running",
    },
  );
  assert.equal(
    resolveThreadAnalysisCommandFocus(thread, {
      id: "schedule-1",
      subscriptionId: "sub-schedule",
      kind: "schedule",
      label: "standup ping",
      detail: "every_interval 6h",
      status: "Active",
      eventCount: 0,
      latestEvent: null,
    }),
    null,
  );
});

test("keeps newer terminal focus request current over initial state load", () => {
  const sequencer = createTerminalStateRequestSequencer();
  const initialLoad = sequencer.begin();
  const focusRequest = sequencer.begin();

  assert.equal(sequencer.isCurrent(initialLoad), false);
  assert.equal(sequencer.isCurrent(focusRequest), true);
});

test("rejects stale terminal focus requests from another thread", () => {
  assert.equal(
    isTerminalCommandFocusRequestForThread(
      {
        threadId: "thread-a",
        commandItemId: "command-1",
        processId: "pid-1",
        token: 1,
      },
      "thread-b",
    ),
    false,
  );
  assert.equal(
    isTerminalCommandFocusRequestForThread(
      {
        threadId: "thread-b",
        commandItemId: "command-1",
        processId: "pid-1",
        token: 1,
      },
      "thread-b",
    ),
    true,
  );
});

test("right panel terminal rail click is the explicit terminal panel focus source", () => {
  const source = readFileSync(new URL("./RightPanel.tsx", import.meta.url), "utf8");
  const tokenStateIndex = source.indexOf(
    "const [terminalPanelFocusRequestToken, setTerminalPanelFocusRequestToken]",
  );
  const propIndex = source.indexOf(
    "focusPanelRequestToken={terminalPanelFocusRequestToken}",
  );
  const railClickIndex = source.indexOf("if (item.view === \"terminal\") {");
  const incrementIndex = source.indexOf(
    "setTerminalPanelFocusRequestToken((current) => current + 1);",
    railClickIndex,
  );

  assert.notEqual(tokenStateIndex, -1);
  assert.notEqual(propIndex, -1);
  assert.notEqual(railClickIndex, -1);
  assert.notEqual(incrementIndex, -1);
  assert.ok(
    incrementIndex < source.indexOf("onSetActiveView(next.nextView);", railClickIndex),
    "TerminalPanel should receive the focus token as part of the explicit open action",
  );
});

test("targeted terminal focus waits for the selected tab to become active", () => {
  const request = { token: 2, tabId: "target-tab" };

  assert.equal(
    shouldApplyTerminalViewportFocusRequest({
      request,
      lastAppliedToken: 1,
      activeTabId: "old-tab",
      terminalAvailable: true,
    }),
    false,
  );
  assert.equal(
    shouldApplyTerminalViewportFocusRequest({
      request,
      lastAppliedToken: 1,
      activeTabId: "target-tab",
      terminalAvailable: false,
    }),
    false,
  );
  assert.equal(
    shouldApplyTerminalViewportFocusRequest({
      request,
      lastAppliedToken: 1,
      activeTabId: "target-tab",
      terminalAvailable: true,
    }),
    true,
  );
  assert.equal(
    shouldApplyTerminalViewportFocusRequest({
      request,
      lastAppliedToken: 2,
      activeTabId: "target-tab",
      terminalAvailable: true,
    }),
    false,
  );
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

test("renders runtime restart progress in thread analysis", () => {
  const markup = renderRightPanel(makeThread([]), "skills", null, {
    runtimeRestartProgress: {
      status: "active",
      requestId: "restart-1",
      originThreadId: "thread-1",
      stage: "shuttingDownAppServer",
      stageLabel: "Stopping app-server",
      message: "Stopping the current app-server before switching capsules.",
      reason: null,
      activationId: "activation-1",
      releaseId: "release-1",
      updatedAtMs: 123,
    },
  });

  assert.match(markup, /Runtime Restart/);
  assert.match(markup, /Stopping app-server/);
  assert.match(markup, /Running/);
  assert.match(markup, /restart-1/);
  assert.match(markup, /thread-1/);
  assert.match(markup, /release-1/);
  assert.match(markup, /activation-1/);
});

test("hides runtime restart progress while idle and surfaces failures", () => {
  const idleMarkup = renderRightPanel(makeThread([]));
  assert.doesNotMatch(idleMarkup, /Runtime Restart/);

  const failedMarkup = renderRightPanel(makeThread([]), "skills", null, {
    runtimeRestartProgress: {
      status: "failed",
      requestId: "restart-failed",
      originThreadId: "thread-1",
      stage: "failed",
      stageLabel: "Failed",
      message: "Build failed",
      reason: "Build failed",
      activationId: null,
      releaseId: null,
      updatedAtMs: 123,
    },
  });

  assert.match(failedMarkup, /Runtime Restart/);
  assert.match(failedMarkup, /Failed/);
  assert.match(failedMarkup, /restart-failed/);
  assert.match(failedMarkup, /Build failed/);
});

test("renders schedule agenda with an overall disclosure header", () => {
  const markup = renderToStaticMarkup(
    <ScheduleAgendaLayout
      groups={makeScheduleAgendaGroups()}
      collapsed={false}
      collapsedDateKeys={new Set()}
      onToggleCollapsed={() => {}}
      onToggleDateKey={() => {}}
    />,
  );

  assert.match(markup, /aria-label="Upcoming schedule events"/);
  assert.match(markup, /aria-expanded="true"/);
  assert.match(markup, /aria-controls="schedule-agenda-groups"/);
  assert.match(markup, /Upcoming/);
  assert.match(markup, /2 items/);
  assert.match(markup, /Today/);
  assert.match(markup, /Tomorrow/);
  assert.match(markup, /standup ping/);
  assert.match(markup, /daily digest/);
});

test("collapses the whole schedule agenda without rendering date rows", () => {
  const markup = renderToStaticMarkup(
    <ScheduleAgendaLayout
      groups={makeScheduleAgendaGroups()}
      collapsed={true}
      collapsedDateKeys={new Set()}
      onToggleCollapsed={() => {}}
      onToggleDateKey={() => {}}
    />,
  );

  assert.match(markup, /aria-expanded="false"/);
  assert.match(markup, /Upcoming/);
  assert.match(markup, /2 items/);
  assert.doesNotMatch(markup, /Today/);
  assert.doesNotMatch(markup, /Tomorrow/);
  assert.doesNotMatch(markup, /standup ping/);
  assert.doesNotMatch(markup, /daily digest/);
});

test("toggles the whole schedule agenda from the agenda header", () => {
  let clicked = false;
  const element = ScheduleAgendaLayout({
    groups: makeScheduleAgendaGroups(),
    collapsed: false,
    collapsedDateKeys: new Set(),
    onToggleCollapsed: () => {
      clicked = true;
    },
    onToggleDateKey: () => {},
  });
  if (element === null) {
    throw new Error("schedule agenda layout should render");
  }
  const [button] = React.Children.toArray(
    (element.props as { children: React.ReactNode }).children,
  ) as React.ReactElement<{ onClick: () => void }>[];

  button.props.onClick();
  assert.equal(clicked, true);
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
  assert.match(markup, /context-section-card current-plan-card/);
  assert.match(markup, /Keep the change scoped\./);
  assert.match(markup, /Filter direct child tasks/);
  assert.match(markup, /Render current thread plan/);
  assert.match(markup, /Run validation/);
  assert.match(markup, /In progress/);
  assert.doesNotMatch(markup, /Plan Work/);
  assert.doesNotMatch(markup, /Execution Queue/);
  assert.doesNotMatch(markup, /Todo List/);
});

test("keeps plan and monitor activity on compact right panel layout rules", () => {
  const css = readFileSync(new URL("../styles.css", import.meta.url), "utf8");
  const source = readFileSync(new URL("./RightPanel.tsx", import.meta.url), "utf8");

  assert.match(
    source,
    /className="context-section-card current-plan-card"/,
  );
  assert.doesNotMatch(css, /\.current-plan-card\s*\{[^}]*margin:/);
  assert.match(
    css,
    /\.context-section-card\.current-plan-card\s*\{[\s\S]*padding: 10px 12px;/,
  );
  assert.match(
    css,
    /\.monitor-section\s*\{[\s\S]*border-top: 1px solid rgba\(16, 24, 40, 0\.06\);[\s\S]*padding-top: 10px;/,
  );
  assert.match(css, /\.monitor-kind-dot\.command,\s*\.monitor-kind-dot\.process/);
  assert.doesNotMatch(css, /\.monitor-empty\s*\{[^}]*border-top:/);
});

test("renders long current plan steps without dropping status labels", () => {
  const planUpdate = {
    threadId: "thread-1",
    turnId: "turn-1",
    explanation: "Keep the status summary compact.",
    plan: [
      {
        step: "Audit the right panel monitor activity layout with a deliberately long plan step that should wrap inside the inspector column instead of pushing status labels out of alignment",
        status: "inProgress",
      },
      { step: "Run focused UI validation", status: "pending" },
    ],
  } satisfies ThreadPlanUpdate;
  const markup = renderRightPanel(makeThread([]), "skills", planUpdate);

  assert.match(markup, /2 steps/);
  assert.match(markup, /current-plan-step/);
  assert.match(markup, /current-plan-count/);
  assert.match(markup, /plan-status-label inProgress/);
  assert.match(markup, /In progress/);
  assert.match(markup, /Run focused UI validation/);
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
  assert.match(markup, /graph-toolbar/);
  assert.match(markup, />Auto</);
  assert.doesNotMatch(markup, /Focus current Git ref/);
  assert.doesNotMatch(markup, /Fetch Git refs/);
  assert.doesNotMatch(markup, /Pull Git refs/);
  assert.doesNotMatch(markup, /More Git actions/);
  assert.match(markup, /Changes/);
  assert.match(markup, /aria-expanded="true"/);
  assert.match(markup, /Collapse Changes/);
  assert.match(markup, /Select Git branch or ref/);
  assert.match(markup, /Refresh Git view/);
  assert.match(markup, /Resize Git graph and changes panes/);
  assert.match(markup, /panel-rail-badge">2/);
  assert.doesNotMatch(markup, /Thread File Deltas/);
});

test("git change groups hide rows when collapsed and expose row diff navigation semantics", () => {
  const change = {
    path: "src/App.tsx",
    originalPath: "src/OldApp.tsx",
    stagedStatus: "R",
    unstagedStatus: null,
    staged: true,
    unstaged: false,
  };
  const expandedMarkup = renderToStaticMarkup(
    <GitChangeGroup
      changes={[change]}
      collapsed={false}
      mode="staged"
      onOpenDiff={() => {}}
      onToggle={() => {}}
      title="Staged Changes"
    />,
  );
  const collapsedMarkup = renderToStaticMarkup(
    <GitChangeGroup
      changes={[change]}
      collapsed={true}
      mode="staged"
      onOpenDiff={() => {}}
      onToggle={() => {}}
      title="Staged Changes"
    />,
  );
  const rowMarkup = renderToStaticMarkup(
    <GitChangeRow change={change} mode="staged" onOpenDiff={() => {}} />,
  );

  assert.match(expandedMarkup, /aria-expanded="true"/);
  assert.match(expandedMarkup, /Open staged diff for src\/App\.tsx/);
  assert.match(expandedMarkup, /from src\/OldApp\.tsx/);
  assert.match(collapsedMarkup, /aria-expanded="false"/);
  assert.doesNotMatch(collapsedMarkup, /Open staged diff for src\/App\.tsx/);
  assert.match(rowMarkup, /role="button"/);
  assert.match(rowMarkup, /tabindex="0"/);
});

test("git commit file rows expose commit diff navigation without toggling the commit row", () => {
  const file = {
    path: "src/thread.ts",
    originalPath: "src/old-thread.ts",
    status: "R",
    score: "91",
  };
  let openedFile: typeof file | null = null;
  let stopped = false;
  const row = GitCommitFileRow({
    file,
    onOpenDiff: (nextFile: typeof file) => {
      openedFile = nextFile;
    },
  }) as React.ReactElement<{
    "aria-label": string;
    className: string;
    onClick: (event: { stopPropagation: () => void }) => void;
    onKeyDown: (event: { stopPropagation: () => void }) => void;
    title: string;
    type: string;
  }>;
  const markup = renderToStaticMarkup(
    <GitCommitFileRow file={file} onOpenDiff={() => {}} />,
  );

  row.props.onClick({
    stopPropagation: () => {
      stopped = true;
    },
  });
  let keyStopped = false;
  row.props.onKeyDown({
    stopPropagation: () => {
      keyStopped = true;
    },
  });

  assert.equal(row.props.type, "button");
  assert.equal(row.props["aria-label"], "Open commit diff for src/thread.ts");
  assert.equal(row.props.title, "Open commit diff for src/thread.ts");
  assert.equal(stopped, true);
  assert.equal(keyStopped, true);
  assert.equal(openedFile, file);
  assert.match(markup, /git-commit-file-row/);
  assert.match(markup, /Open commit diff for src\/thread\.ts/);
  assert.match(markup, /from src\/old-thread\.ts/);
});

test("git diff previews render as read-only preview content without edit controls", () => {
  const markup = renderToStaticMarkup(
    <GitDiffPreviewPanel
      diff={{
        available: false,
        root: "/repo",
        path: "src/App.tsx",
        originalPath: null,
        staged: false,
        status: "M",
        language: "typescript",
        oldLabel: "Index",
        newLabel: "Working tree",
        oldContent: "",
        newContent: "",
        unifiedDiff: "",
        error: "Binary files cannot be previewed as side-by-side text.",
        binary: true,
      }}
      error={null}
      loading={false}
    />,
  );

  assert.match(markup, />DIFF</);
  assert.match(markup, />unstaged</);
  assert.doesNotMatch(markup, /preview-edit-action/);
});

test("git diff previews label commit file diffs as commit scope", () => {
  const markup = renderToStaticMarkup(
    <GitDiffPreviewPanel
      diff={{
        available: true,
        root: "/repo",
        path: "src/thread.ts",
        originalPath: null,
        staged: false,
        status: "M",
        language: "typescript",
        oldLabel: "abc1234^",
        newLabel: "abc1234",
        oldContent: "before\n",
        newContent: "after\n",
        unifiedDiff: "",
        error: null,
        binary: false,
        modeLabel: "commit",
        commit: "abc1234",
        parent: "abc1234^",
      }}
      error={null}
      loading={false}
    />,
  );

  assert.match(markup, />commit</);
  assert.doesNotMatch(markup, />unstaged</);
});

test("git diff preview clears when normal file preview changes target", () => {
  const firstPreview = makePreview({
    path: "/repo/src/App.tsx",
    line: 1,
    column: 1,
  });
  const samePreview = makePreview({
    path: "/repo/src/App.tsx",
    line: 1,
    column: 1,
  });
  const nextPreview = makePreview({
    path: "/repo/src/Other.tsx",
    line: 1,
    column: 1,
  });

  const baseKey = filePreviewIdentity(firstPreview, "root-1");

  assert.equal(filePreviewIdentity(samePreview, "root-1"), baseKey);
  assert.equal(
    shouldClearGitDiffPreviewForFilePreviewChange({
      active: true,
      basePreviewKey: baseKey,
      currentPreviewKey: filePreviewIdentity(samePreview, "root-1"),
    }),
    false,
  );
  assert.equal(
    shouldClearGitDiffPreviewForFilePreviewChange({
      active: true,
      basePreviewKey: baseKey,
      currentPreviewKey: filePreviewIdentity(nextPreview, "root-1"),
    }),
    true,
  );
  assert.equal(
    shouldClearGitDiffPreviewForFilePreviewChange({
      active: true,
      basePreviewKey: baseKey,
      currentPreviewKey: filePreviewIdentity(firstPreview, "root-2"),
    }),
    true,
  );
  assert.equal(
    shouldClearGitDiffPreviewForFilePreviewChange({
      active: false,
      basePreviewKey: baseKey,
      currentPreviewKey: filePreviewIdentity(nextPreview, "root-1"),
    }),
    false,
  );
});

test("git tree paths resolve to changed-file diff targets with unstaged priority", () => {
  const stagedChange = {
    path: "src/staged.ts",
    originalPath: null,
    stagedStatus: "M",
    unstagedStatus: null,
    staged: true,
    unstaged: false,
  };
  const unstagedChange = {
    path: "src/unstaged.ts",
    originalPath: null,
    stagedStatus: null,
    unstagedStatus: "M",
    staged: false,
    unstaged: true,
  };
  const bothChange = {
    path: "src/both.ts",
    originalPath: null,
    stagedStatus: "M",
    unstagedStatus: "M",
    staged: true,
    unstaged: true,
  };
  const snapshot = {
    available: true,
    root: "/repo",
    treeRoot: "/repo",
    branch: "main",
    selectedRef: null,
    refs: [],
    graph: [],
    changes: [stagedChange, unstagedChange, bothChange],
    error: null,
  };

  assert.deepEqual(gitDiffTargetForTreePath(snapshot, "/repo/src/unstaged.ts"), {
    change: unstagedChange,
    mode: "unstaged",
  });
  assert.deepEqual(gitDiffTargetForTreePath(snapshot, "/repo/src/staged.ts"), {
    change: stagedChange,
    mode: "staged",
  });
  assert.deepEqual(gitDiffTargetForTreePath(snapshot, "/repo/src/both.ts"), {
    change: bothChange,
    mode: "unstaged",
  });
  assert.deepEqual(
    gitDiffTargetForTreePath(
      { ...snapshot, root: "/private/var/folders/repo", treeRoot: "/var/folders/repo" },
      "/var/folders/repo/src/both.ts",
    ),
    {
      change: bothChange,
      mode: "unstaged",
    },
  );
  assert.equal(gitDiffTargetForTreePath(snapshot, "/repo/src/clean.ts"), null);
  assert.equal(gitDiffTargetForTreePath({ ...snapshot, available: false }, "/repo/src/both.ts"), null);
});

test("git tree path normalization keeps repo-relative status matching bounded", () => {
  assert.equal(gitRelativeTreePath("/repo", "/repo/src/App.tsx"), "src/App.tsx");
  assert.equal(gitRelativeTreePath("C:\\repo", "C:\\repo\\src\\App.tsx"), "src/App.tsx");
  assert.equal(gitRelativeTreePath("/repo", "/repo-other/src/App.tsx"), null);
  assert.equal(gitRelativeTreePath("/repo", "src/App.tsx"), "src/App.tsx");
  assert.equal(
    gitRelativeTreePath(
      "/private/var/folders/repo",
      "/var/folders/repo/src/App.tsx",
      "/var/folders/repo",
    ),
    "src/App.tsx",
  );
});

test("git tree cache miss resolves status before opening modified files", async () => {
  const modifiedChange = {
    path: "src/modified.ts",
    originalPath: null,
    stagedStatus: null,
    unstagedStatus: "M",
    staged: false,
    unstaged: true,
  };

  const decision = await resolveGitTreeFileOpen({
    cachedSnapshot: null,
    cwd: "/repo",
    treePath: "/repo/src/modified.ts",
    scope: 1,
    isScopeCurrent: (scope: number) => scope === 1,
    readGitStatusSnapshot: async () => ({
      available: true,
      root: "/repo",
      treeRoot: "/repo",
      changes: [modifiedChange],
      error: null,
    }),
  });

  assert.deepEqual(decision, {
    kind: "diff",
    change: modifiedChange,
    mode: "unstaged",
    snapshot: {
      available: true,
      root: "/repo",
      treeRoot: "/repo",
      changes: [modifiedChange],
      error: null,
    },
  });
});

test("git tree cache miss opens clean files normally after status resolves", async () => {
  const decision = await resolveGitTreeFileOpen({
    cachedSnapshot: null,
    cwd: "/repo",
    treePath: "/repo/src/clean.ts",
    scope: 1,
    isScopeCurrent: (scope: number) => scope === 1,
    readGitStatusSnapshot: async () => ({
      available: true,
      root: "/repo",
      treeRoot: "/repo",
      changes: [],
      error: null,
    }),
  });

  assert.deepEqual(decision, {
    kind: "file",
    snapshot: {
      available: true,
      root: "/repo",
      treeRoot: "/repo",
      changes: [],
      error: null,
    },
  });
});

test("git tree async status decisions ignore stale clicks", async () => {
  const modifiedChange = {
    path: "src/a.ts",
    originalPath: null,
    stagedStatus: null,
    unstagedStatus: "M",
    staged: false,
    unstaged: true,
  };
  const firstStatus = deferred<{
    available: boolean;
    root: string | null;
    treeRoot: string | null;
    changes: typeof modifiedChange[];
    error: string | null;
  }>();
  let currentScope = 1;

  const firstDecision = resolveGitTreeFileOpen({
    cachedSnapshot: null,
    cwd: "/repo",
    treePath: "/repo/src/a.ts",
    scope: 1,
    isScopeCurrent: (scope: number) => scope === currentScope,
    readGitStatusSnapshot: async () => firstStatus.promise,
  });

  currentScope = 2;
  const secondDecision = await resolveGitTreeFileOpen({
    cachedSnapshot: null,
    cwd: "/repo",
    treePath: "/repo/src/b.ts",
    scope: 2,
    isScopeCurrent: (scope: number) => scope === currentScope,
    readGitStatusSnapshot: async () => ({
      available: true,
      root: "/repo",
      treeRoot: "/repo",
      changes: [],
      error: null,
    }),
  });
  firstStatus.resolve({
    available: true,
    root: "/repo",
    treeRoot: "/repo",
    changes: [modifiedChange],
    error: null,
  });

  assert.equal(secondDecision.kind, "file");
  assert.deepEqual(await firstDecision, { kind: "stale" });
});

test("git tree async status decisions ignore external preview takeover", async () => {
  const modifiedChange = {
    path: "src/a.ts",
    originalPath: null,
    stagedStatus: null,
    unstagedStatus: "M",
    staged: false,
    unstaged: true,
  };
  const status = deferred<{
    available: boolean;
    root: string | null;
    treeRoot: string | null;
    changes: typeof modifiedChange[];
    error: string | null;
  }>();
  let currentScope = 1;

  const decision = resolveGitTreeFileOpen({
    cachedSnapshot: null,
    cwd: "/repo",
    treePath: "/repo/src/a.ts",
    scope: 1,
    isScopeCurrent: (scope: number) => scope === currentScope,
    readGitStatusSnapshot: async () => status.promise,
  });

  currentScope = 2;
  status.resolve({
    available: true,
    root: "/repo",
    treeRoot: "/repo",
    changes: [modifiedChange],
    error: null,
  });

  assert.deepEqual(await decision, { kind: "stale" });
});

test("builds a commit-level git graph visual model with a spine and curved branches", () => {
  const graph = [
    makeGitCommit("* ", "merge-a", ["parent-a", "parent-b"], "Merge feature"),
    { type: "connector" as const, graph: "|\\" },
    makeGitCommit("| * ", "side-a", ["merge-a"], "feature work"),
    { type: "connector" as const, graph: "|/" },
    makeGitCommit("* ", "merge-b", ["parent-c", "side-a"], "Merge feature"),
  ];

  const visualModel = buildGitGraphVisualModel(graph);

  assert.equal(visualModel.width, 52.5);
  assert.equal(visualModel.height, 126);
  assert.deepEqual(
    visualModel.commits.map((commit) => ({
      hash: commit.commit.hash,
      lane: commit.lane,
      x: commit.x,
      y: commit.y,
    })),
    [
      { hash: "merge-a", lane: 0, x: 18, y: 21 },
      { hash: "side-a", lane: 1, x: 41, y: 63 },
      { hash: "merge-b", lane: 0, x: 18, y: 105 },
    ],
  );
  assert.deepEqual(visualModel.paths, [
    {
      id: "main-spine",
      lane: 0,
      colorLane: 0,
      kind: "main",
      d: "M 18 21 L 18 105",
    },
    {
      id: "branch:1:2:1",
      lane: 1,
      colorLane: 1,
      kind: "branch",
      d: "M 18 21 C 41 21 41 48.72 41 63 C 41 77.28 41 105 18 105",
    },
  ]);

  const expandedVisualModel = buildGitGraphVisualModel(graph, {
    expandedHeightsByHash: { "merge-a": 62 },
  });
  assert.equal(expandedVisualModel.height, 188);
  assert.deepEqual(
    expandedVisualModel.commits.map((commit) => ({ hash: commit.commit.hash, y: commit.y })),
    [
      { hash: "merge-a", y: 21 },
      { hash: "side-a", y: 125 },
      { hash: "merge-b", y: 167 },
    ],
  );
});

test("git graph styles keep a light theme and full-size visible rail overlay", () => {
  const css = readFileSync(new URL("../styles.css", import.meta.url), "utf8");
  const source = readFileSync(new URL("./RightPanel.tsx", import.meta.url), "utf8");

  assert.match(css, /\.git-panel \{[\s\S]*background: #f8fafc;/);
  assert.match(css, /\.git-graph-section \{[\s\S]*background: #f8fafc;/);
  assert.doesNotMatch(css, /\.git-panel \{[\s\S]*background: #0f1419;/);
  assert.match(css, /\.git-graph-overlay \{[\s\S]*width: var\(--git-graph-visual-width, 58px\);/);
  assert.match(css, /\.git-graph-overlay \{[\s\S]*height: var\(--git-graph-visual-height, 42px\);/);
  assert.match(css, /\.git-graph-row-main \{[\s\S]*min-height: 42px;/);
  assert.match(css, /\.git-graph-list,[\s\S]*\.git-changes-list \{[\s\S]*overflow-x: hidden;/);
  assert.match(css, /\.git-graph-visual-stack \{[\s\S]*width: 100%;[\s\S]*max-width: 100%;/);
  assert.match(
    css,
    /\.git-graph-row-main \{[\s\S]*grid-template-columns: var\(--git-graph-visual-width, 58px\) minmax\(0, 1fr\) 28px;/,
  );
  assert.match(css, /\.git-graph-copy \{[\s\S]*overflow: hidden;/);
  assert.match(css, /\.git-commit-file-row \{[\s\S]*grid-template-columns: 20px minmax\(0, 1fr\) 22px;/);
  assert.match(css, /\.git-change-row \{[\s\S]*grid-template-columns: 22px minmax\(0, 1fr\) 20px;/);
  assert.match(css, /\.git-graph-dot\.main \{[\s\S]*fill: #f8fafc;[\s\S]*stroke-width: 3\.4;/);
  assert.match(css, /\.git-graph-dot\.branch \{[\s\S]*fill: currentColor;/);
  assert.match(css, /\.git-head-ref \{[\s\S]*background: #2563eb;/);
  assert.match(source, /"--git-graph-visual-height": `\$\{visualModel\.height\}px`/);
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
  assert.match(markup, /title="\/tmp\/src"/);
  assert.match(markup, /title="\/tmp\/src\/App\.tsx"/);
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
  assert.equal(
    filePreviewRenderMode(
      makePreview({
        path: "/tmp/src/main.go",
        displayPath: "src/main.go",
        content: "package main\n\nfunc main() {}\n",
        language: "go",
      }),
    ),
    "editor",
  );
});

test("enables edit actions for editable text previews while keeping image and PDF read-only", () => {
  const editorPreview = makePreview({
    path: "/tmp/src/App.tsx",
    displayPath: "src/App.tsx",
    content: "export const value = 1;",
    language: "typescript",
  });
  const markdownPreview = makePreview({
    path: "/tmp/README.md",
    displayPath: "README.md",
    content: "# Title",
    language: "markdown",
  });
  const imagePreview = makePreview({
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
  });
  const pdfPreview = makePreview({
    path: "/tmp/spec.pdf",
    displayPath: "spec.pdf",
    content: "",
    language: "pdf",
    pdf: {
      path: "/tmp/spec.pdf",
      mimeType: "application/pdf",
      name: "spec.pdf",
      byteSize: 512,
      url: "morpheus-file-preview://pdf/token-1/spec.pdf",
    },
  });

  const markdownMarkup = renderRightPanel(makeThread([]), "preview", null, {
    preview: markdownPreview,
  });
  const imageMarkup = renderRightPanel(makeThread([]), "preview", null, {
    preview: imagePreview,
  });
  const pdfMarkup = renderRightPanel(makeThread([]), "preview", null, {
    preview: pdfPreview,
  });

  assert.equal(filePreviewCanEdit(editorPreview), true);
  assert.equal(filePreviewCanEdit(markdownPreview), true);
  assert.equal(filePreviewCanEdit(imagePreview), false);
  assert.equal(filePreviewCanEdit(pdfPreview), false);
  assert.match(
    markdownMarkup,
    /<div class="preview-header-actions">[\s\S]*preview-edit-action[\s\S]*<\/header>/,
  );
  assert.match(markdownMarkup, />Edit<\/button>/);
  assert.doesNotMatch(markdownMarkup, /preview-utility-strip[\s\S]*preview-edit-action/);
  assert.doesNotMatch(imageMarkup, /preview-edit-action/);
  assert.doesNotMatch(pdfMarkup, /preview-edit-action/);
});

test("header edit controls appear only for loaded editable previews", () => {
  const markdownPreview = makePreview({
    path: "/tmp/README.md",
    displayPath: "README.md",
    content: "# Title",
    language: "markdown",
  });
  const editorPreview = makePreview({
    path: "/tmp/src/App.tsx",
    displayPath: "src/App.tsx",
    content: "export const value = 1;",
    language: "typescript",
  });
  const imagePreview = makePreview({
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
  });

  assert.equal(
    filePreviewHeaderEditControlsVisible({
      filePanelView: "preview",
      preview: markdownPreview,
      previewError: null,
      previewLoading: false,
    }),
    true,
  );
  assert.equal(
    filePreviewHeaderEditControlsVisible({
      filePanelView: "preview",
      preview: editorPreview,
      previewError: null,
      previewLoading: false,
    }),
    true,
  );
  assert.equal(
    filePreviewHeaderEditControlsVisible({
      filePanelView: "tree",
      preview: markdownPreview,
      previewError: null,
      previewLoading: false,
    }),
    false,
  );
  assert.equal(
    filePreviewHeaderEditControlsVisible({
      filePanelView: "preview",
      preview: markdownPreview,
      previewError: null,
      previewLoading: true,
    }),
    false,
  );
  assert.equal(
    filePreviewHeaderEditControlsVisible({
      filePanelView: "preview",
      preview: markdownPreview,
      previewError: "Failed",
      previewLoading: false,
    }),
    false,
  );
  assert.equal(
    filePreviewHeaderEditControlsVisible({
      filePanelView: "preview",
      preview: imagePreview,
      previewError: null,
      previewLoading: false,
    }),
    false,
  );
  assert.equal(
    filePreviewHeaderEditControlsVisible({
      filePanelView: "preview",
      preview: null,
      previewError: null,
      previewLoading: false,
    }),
    false,
  );

  const loadingMarkup = renderRightPanel(makeThread([]), "preview", null, {
    preview: markdownPreview,
    previewLoading: true,
  });
  const errorMarkup = renderRightPanel(makeThread([]), "preview", null, {
    preview: markdownPreview,
    previewError: "Failed to load",
  });
  const emptyMarkup = renderRightPanel(makeThread([]), "preview", null);

  assert.doesNotMatch(loadingMarkup, /preview-edit-action/);
  assert.doesNotMatch(errorMarkup, /preview-edit-action/);
  assert.doesNotMatch(emptyMarkup, /preview-edit-action/);
});

test("markdown previews keep rendered readonly mode until editing or saving", () => {
  const markdownPreview = makePreview({
    path: "/tmp/README.md",
    displayPath: "README.md",
    content: "# Title",
    language: "markdown",
  });
  const editorPreview = makePreview({
    path: "/tmp/src/App.tsx",
    displayPath: "src/App.tsx",
    content: "export const value = 1;",
    language: "typescript",
  });
  const imagePreview = makePreview({
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
  });
  const markup = renderRightPanel(makeThread([]), "preview", null, {
    preview: markdownPreview,
  });

  assert.match(markup, /<h1>Title<\/h1>/);
  assert.doesNotMatch(markup, /Loading editor/);
  assert.equal(filePreviewSourceEditorVisible(markdownPreview, "readonly"), false);
  assert.equal(filePreviewSourceEditorVisible(markdownPreview, "editing"), true);
  assert.equal(filePreviewSourceEditorVisible(markdownPreview, "saving"), true);
  assert.equal(filePreviewSourceEditorVisible(editorPreview, "readonly"), true);
  assert.equal(filePreviewSourceEditorVisible(imagePreview, "editing"), false);
});

test("file preview edit state saves, cancels, keeps failures, and resets on file switch", () => {
  const preview = makePreview({
    path: "/tmp/src/App.tsx",
    displayPath: "src/App.tsx",
    content: "const value = 1;",
    language: "typescript",
  });
  const nextPreview = makePreview({
    path: "/tmp/src/Other.tsx",
    displayPath: "src/Other.tsx",
    content: "const other = 1;",
    language: "typescript",
  });

  let state = syncFilePreviewEditState(
    {
      mode: "readonly",
      path: null,
      baseContent: "",
      draft: "",
      error: null,
    },
    preview,
    "editor",
  );
  state = beginFilePreviewEdit(state);
  state = updateFilePreviewDraft(state, "const value = 2;");

  assert.equal(state.mode, "editing");
  assert.equal(state.draft, "const value = 2;");

  const saving = beginFilePreviewSave(state);
  assert.equal(saving.mode, "saving");
  const failed = failFilePreviewSave(saving, "Permission denied");
  assert.equal(failed.mode, "editing");
  assert.equal(failed.draft, "const value = 2;");
  assert.equal(failed.error, "Permission denied");

  const saved = completeFilePreviewSave(failed, failed.draft);
  assert.equal(saved.mode, "readonly");
  assert.equal(saved.baseContent, "const value = 2;");
  assert.equal(saved.draft, "const value = 2;");

  const editing = updateFilePreviewDraft(beginFilePreviewEdit(saved), "dirty");
  const cancelled = cancelFilePreviewEdit(editing);
  assert.equal(cancelled.mode, "readonly");
  assert.equal(cancelled.draft, "const value = 2;");
  assert.equal(cancelled.error, null);

  const switched = syncFilePreviewEditState(editing, nextPreview, "editor");
  assert.equal(switched.mode, "readonly");
  assert.equal(switched.path, "/tmp/src/Other.tsx");
  assert.equal(switched.draft, "const other = 1;");
});

test("file preview save button and Cmd+S use the same save handler", () => {
  const source = readFileSync(new URL("./RightPanel.tsx", import.meta.url), "utf8");

  assert.match(source, /onClick=\{\(\) => void savePreviewDraft\(\)\}/);
  assert.match(source, /monaco\.KeyMod\.CtrlCmd \| monaco\.KeyCode\.KeyS/);
  assert.match(source, /savePreviewDraftRef\.current\(\)/);
});

test("renders PDF file previews with an embedded PDF object", () => {
  const markup = renderRightPanel(makeThread([]), "preview", null, {
    preview: makePreview({
      path: "/tmp/Project Docs/spec.PDF",
      displayPath: "Project Docs/spec.PDF",
      content: "",
      language: "pdf",
      pdf: {
        path: "/tmp/Project Docs/spec.PDF",
        mimeType: "application/pdf",
        name: "spec.PDF",
        byteSize: 4096,
        url: "morpheus-file-preview://pdf/token-1/spec.PDF",
      },
    }),
  });

  assert.equal(
    filePreviewRenderMode(
      makePreview({
        language: "markdown",
        pdf: {
          path: "/tmp/spec.pdf",
          mimeType: "application/pdf",
          name: "spec.pdf",
          byteSize: 512,
          url: "morpheus-file-preview://pdf/token-2/spec.pdf",
        },
      }),
    ),
    "pdf",
  );
  assert.match(markup, /PDF/);
  assert.match(markup, /application\/pdf/);
  assert.match(markup, /4\.0 KB/);
  assert.match(markup, /aria-label="PDF preview for spec\.PDF"/);
  assert.match(markup, /data="morpheus-file-preview:\/\/pdf\/token-1\/spec\.PDF"/);
  assert.doesNotMatch(markup, /Loading editor/);
});

test("resolves preview definition clicks to a column inside the current word", () => {
  const editor = {
    getModel() {
      return {
        getWordAtPosition() {
          return {
            word: "Button",
            startColumn: 12,
            endColumn: 18,
          };
        },
      };
    },
  };

  assert.deepEqual(
    resolvePreviewDefinitionPosition(editor, { lineNumber: 3, column: 18 }),
    { lineNumber: 3, column: 17 },
  );
  assert.deepEqual(
    resolvePreviewDefinitionPosition(editor, { lineNumber: 3, column: 14 }),
    { lineNumber: 3, column: 14 },
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
