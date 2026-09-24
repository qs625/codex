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
import { filePreviewOpenInBrowserActionVisible } from "../lib/filePreviewBrowser";
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
  BrowserPanel,
  browserTabLabel,
  buildGitGraphVisualModel,
  cancelFilePreviewEdit,
  completeFilePreviewSave,
  currentBrowserPanelApi,
  failFilePreviewSave,
  FilePreviewPanel,
  filePreviewCanEdit,
  filePreviewHeaderEditControlsVisible,
  filePreviewIdentity,
  filePreviewRenderMode,
  filePreviewSourceEditorVisible,
  GitChangeGroup,
  GitChangeRow,
  GitCommitFileRow,
  GitDiffPreviewPanel,
  normalizeBrowserPanelState,
  nextBrowserBoundsSequence,
  openCwdTreeFilePreview,
  resolveBrowserPanelChromeLabels,
  resolveBrowserPanelTabSelection,
  resolveThreadAnalysisCommandFocus,
  resolvePreviewDefinitionPosition,
  resolveMarkdownPreviewLocalFileTarget,
  shouldClearBrowserLocalError,
  shouldClearGitDiffPreviewForFilePreviewChange,
  syncFilePreviewEditState,
  updateFilePreviewDraft,
} = await import("./RightPanel");
const {
  WORKSPACE_OBJECT_DRAG_TYPE,
  hasWorkspaceObjectDragData,
  readWorkspaceObjectDragData,
  writeWorkspaceObjectDragData,
} = await import("../lib/workspaceObjectDrag");

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
    workspaceTabsEnabled?: boolean;
  },
) {
  if (
    activeView === "preview" &&
    options?.filePanelView !== "tree" &&
    (options?.preview || options?.previewError || options?.previewLoading)
  ) {
    return renderToStaticMarkup(
      <FilePreviewPanel
        expandedTreeDirectories={options?.expandedTreeDirectories ?? []}
        filePanelView="preview"
        fileTreeEntriesByPath={options?.fileTreeEntriesByPath ?? {}}
        fileTreeErrorsByPath={{}}
        fileTreeLoadingPath={null}
        gitDiffPreview={null}
        gitDiffPreviewError={null}
        gitDiffPreviewLoading={false}
        onNavigateToSymbol={() => {}}
        onOpenPreviewExternally={() => {}}
        onOpenPreviewInBrowser={() => {}}
        onOpenTreeFile={() => {}}
        onPreviewUpdated={() => {}}
        onToggleTreeDirectory={() => {}}
        preview={options?.preview ?? null}
        previewError={options?.previewError ?? null}
        previewLoading={options?.previewLoading ?? false}
        previewRootId="root-1"
        thread={thread}
      />,
    );
  }
  return renderToStaticMarkup(
    <RightPanel
      activeView={activeView}
      availableSkillCount={0}
      availableWorkflows={[FEATURE_DEV_WORKFLOW]}
      workspaceTabsEnabled={options?.workspaceTabsEnabled ?? false}
      isCollapsed={options?.isCollapsed ?? false}
      expandedTreeDirectories={options?.expandedTreeDirectories ?? []}
      filePanelView={options?.filePanelView ?? "preview"}
      fileTreeEntriesByPath={options?.fileTreeEntriesByPath ?? {}}
      fileTreeErrorsByPath={{}}
      fileTreeLoadingPath={null}
      onNavigateToSymbol={() => {}}
      onOpenPreviewExternally={() => {}}
      onOpenPreviewInBrowser={() => {}}
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

test("right panel keeps file terminal and browser entry points by default", () => {
  const markup = renderRightPanel(makeThread([]), "skills", null, {
    preview: makePreview(),
    workspaceTabsEnabled: true,
  });

  assert.match(markup, /aria-label="File Preview"/);
  assert.match(markup, /aria-label="Terminal"/);
  assert.match(markup, /aria-label="Browser"/);
  assert.match(markup, /aria-label="Thread Analysis"/);
  assert.match(markup, /aria-label="Git Changes"/);
  assert.match(markup, /aria-label="Workflow"/);
  assert.doesNotMatch(markup, /aria-label="Search"/);
  assert.doesNotMatch(markup, /aria-label="Artifacts"/);
});

test("renders browser panel and rail button", () => {
  const markup = renderRightPanel(makeThread([]), "browser");

  assert.match(markup, /aria-label="Browser"/);
  assert.match(markup, /aria-label="Browser tabs"/);
  assert.match(markup, /New tab/);
  assert.match(markup, /browser-new-tab-button/);
  assert.match(markup, /aria-label="New browser tab"/);
  assert.doesNotMatch(markup, /Browser URL/);
  assert.doesNotMatch(markup, /class="browser-go-button" disabled=""/);
  assert.match(markup, /Open a page in the right panel/);
});

test("BrowserPanel manager selection ignores workspace-owned tabs", () => {
  const tabs = [
    {
      id: "browser-a",
      title: "A",
      url: "https://detached.example",
      loading: false,
      canGoBack: false,
      canGoForward: false,
      error: null,
    },
    {
      id: "browser-b",
      title: "B",
      url: "https://visible.example",
      loading: false,
      canGoBack: true,
      canGoForward: false,
      error: null,
    },
  ];

  const onlyDetached = resolveBrowserPanelTabSelection({
    tabs: [tabs[0]],
    activeTabId: "browser-a",
    isManagerVariant: true,
    detachedBrowserTabIds: ["browser-a"],
  });
  assert.equal(onlyDetached.activeTab, null);
  assert.deepEqual(onlyDetached.renderedTabs, []);
  assert.equal(onlyDetached.managerActiveTabDetached, true);

  const withVisibleFallback = resolveBrowserPanelTabSelection({
    tabs,
    activeTabId: "browser-a",
    isManagerVariant: true,
    detachedBrowserTabIds: ["browser-a"],
  });
  assert.equal(withVisibleFallback.activeTab?.id, "browser-b");
  assert.deepEqual(
    withVisibleFallback.renderedTabs.map((tab) => tab.id),
    ["browser-b"],
  );

  const withLocalManagerSelection = resolveBrowserPanelTabSelection({
    tabs: [
      ...tabs,
      {
        id: "browser-c",
        title: "C",
        url: "https://new-right-owned.example",
        loading: false,
        canGoBack: false,
        canGoForward: false,
        error: null,
      },
    ],
    activeTabId: "browser-a",
    managerSelectedBrowserTabId: "browser-c",
    isManagerVariant: true,
    detachedBrowserTabIds: ["browser-a"],
  });
  assert.equal(withLocalManagerSelection.activeTab?.id, "browser-c");
});

test("BrowserPanel workspace variant renders a minimal URL toolbar without manager chrome", () => {
  const markup = renderToStaticMarkup(
    <BrowserPanel
      variant="workspace"
      active
      nativeOverlayActive={false}
      resizing={false}
      navigationRequest={null}
      activeBrowserTabId="browser-tab-1"
    />,
  );

  assert.match(markup, /browser-toolbar-workspace/);
  assert.match(markup, /aria-label="Go back"/);
  assert.match(markup, /aria-label="Go forward"/);
  assert.match(markup, /aria-label="Reload"/);
  assert.match(markup, /aria-label="Workspace browser URL"/);
  assert.doesNotMatch(markup, /aria-label="Workspace browser URL"[^>]*disabled/);
  assert.match(markup, /class="browser-go-button" disabled=""/);
  assert.doesNotMatch(markup, /browser-tab-strip/);
  assert.doesNotMatch(markup, /aria-label="New browser tab"/);
  assert.doesNotMatch(markup, /browser-status-row/);
});

test("browser and terminal rail entries are real workspace drag sources", () => {
  const markup = renderRightPanel(makeThread([]), "skills");
  const writes: Array<[string, string]> = [];
  const dataTransfer = {
    effectAllowed: "none",
    setData(type: string, value: string) {
      writes.push([type, value]);
    },
  };

  assert.match(
    markup,
    /draggable="true"[^>]*aria-label="Browser"|aria-label="Browser"[^>]*draggable="true"/,
  );
  assert.match(
    markup,
    /draggable="true"[^>]*aria-label="Terminal"|aria-label="Terminal"[^>]*draggable="true"/,
  );

  writeWorkspaceObjectDragData(dataTransfer, "browser");
  assert.equal(dataTransfer.effectAllowed, "move");
  assert.equal(writes[0]?.[0], WORKSPACE_OBJECT_DRAG_TYPE);
  assert.deepEqual(JSON.parse(writes[0]?.[1] ?? "{}"), { kind: "browser" });
  assert.deepEqual(
    readWorkspaceObjectDragData({
      getData: (type: string) => (type === WORKSPACE_OBJECT_DRAG_TYPE ? "browser" : ""),
    }),
    { kind: "browser" },
  );
  assert.equal(
    hasWorkspaceObjectDragData({
      types: [WORKSPACE_OBJECT_DRAG_TYPE],
    } as Pick<DataTransfer, "types">),
    true,
  );
  assert.equal(
    hasWorkspaceObjectDragData({
      types: {
        length: 1,
        item: () => WORKSPACE_OBJECT_DRAG_TYPE,
        contains: () => true,
      },
    } as unknown as Pick<DataTransfer, "types">),
    true,
  );
});

test("workspace object drag data carries concrete browser and terminal tab identity", () => {
  const writes: Array<[string, string]> = [];
  const dataTransfer = {
    effectAllowed: "none",
    setData(type: string, value: string) {
      writes.push([type, value]);
    },
  };

  writeWorkspaceObjectDragData(dataTransfer, {
    kind: "browser",
    browserTabId: "browser-tab-2",
    title: "Docs",
    url: "https://example.test/docs",
  });
  writeWorkspaceObjectDragData(dataTransfer, {
    kind: "terminal",
    terminalTabId: "terminal-tab-7",
    sessionId: "session-7",
    threadId: "thread-1",
    title: "pnpm build",
    cwd: "/repo",
    commandItemId: "command-1",
    command: "pnpm build",
    status: "running",
  });

  assert.deepEqual(JSON.parse(writes[0]?.[1] ?? "{}"), {
    kind: "browser",
    browserTabId: "browser-tab-2",
    title: "Docs",
    url: "https://example.test/docs",
  });
  assert.deepEqual(JSON.parse(writes[1]?.[1] ?? "{}"), {
    kind: "terminal",
    terminalTabId: "terminal-tab-7",
    sessionId: "session-7",
    threadId: "thread-1",
    title: "pnpm build",
    cwd: "/repo",
    commandItemId: "command-1",
    command: "pnpm build",
    status: "running",
  });
});

test("detached workspace objects are hidden from the right panel until their workspace tab closes", () => {
  const appSource = readFileSync(new URL("../App.tsx", import.meta.url), "utf8");
  const rightPanelSource = readFileSync(
    new URL("./RightPanel.tsx", import.meta.url),
    "utf8",
  );
  const terminalPanelSource = readFileSync(
    new URL("./TerminalPanel.tsx", import.meta.url),
    "utf8",
  );
  const detachedOwnershipSource = appSource.slice(
    appSource.indexOf("const detachedWorkspaceBrowserTabIds = workspaceTabs"),
    appSource.indexOf("return (", appSource.indexOf("const detachedWorkspaceBrowserTabIds = workspaceTabs")),
  );

  assert.match(
    appSource,
    /const detachedWorkspaceBrowserTabIds = workspaceTabs[\s\S]*tab\.kind === "browser" && tab\.browserTabId[\s\S]*map\(\(tab\) => tab\.browserTabId as string\)/,
  );
  assert.match(
    appSource,
    /const detachedWorkspaceTerminalTabIds = workspaceTabs[\s\S]*tab\.kind === "terminal" && tab\.terminalTabId[\s\S]*map\(\(tab\) => tab\.terminalTabId as string\)/,
  );
  assert.doesNotMatch(
    detachedOwnershipSource,
    /activeWorkspaceTab\?\.kind === "browser"/,
  );
  assert.doesNotMatch(
    detachedOwnershipSource,
    /activeWorkspaceTab\?\.kind === "terminal"/,
  );
  assert.match(
    rightPanelSource,
    /resolveBrowserPanelTabSelection\(\{[\s\S]*detachedBrowserTabIds/,
  );
  assert.match(
    terminalPanelSource,
    /state\.tabs\.filter\(\(tab\) => !detachedTerminalTabIdSet\.has\(tab\.id\)\)/,
  );
  assert.match(rightPanelSource, /managerNativeViewBlocked/);
  assert.match(rightPanelSource, /browserSurfaceIdRef/);
  assert.match(rightPanelSource, /Browser content is open in workspace\./);
  assert.match(terminalPanelSource, /Terminal session is open in workspace\./);
});

test("workspace Browser and Terminal tabs can be returned to the right panel", () => {
  const appSource = readFileSync(new URL("../App.tsx", import.meta.url), "utf8");
  const rightPanelSource = readFileSync(
    new URL("./RightPanel.tsx", import.meta.url),
    "utf8",
  );
  const terminalPanelSource = readFileSync(
    new URL("./TerminalPanel.tsx", import.meta.url),
    "utf8",
  );

  assert.match(
    appSource,
    /function workspaceObjectDragPayloadForTab\([\s\S]*tab\.kind === "browser" && tab\.browserTabId[\s\S]*kind: "browser"[\s\S]*browserTabId: tab\.browserTabId/,
  );
  assert.match(
    appSource,
    /function workspaceObjectDragPayloadForTab\([\s\S]*tab\.kind === "terminal" && tab\.terminalTabId[\s\S]*kind: "terminal"[\s\S]*terminalTabId: tab\.terminalTabId[\s\S]*sessionId: tab\.terminalSessionId/,
  );
  assert.match(
    appSource,
    /function handleWorkspaceTabDragStart[\s\S]*event\.dataTransfer\.setData\("text\/plain", tabId\)[\s\S]*workspaceObjectDragPayloadForTab\(tab\)[\s\S]*writeWorkspaceObjectDragData\(event\.dataTransfer, payload\)/,
  );
  assert.match(
    appSource,
    /function handleReturnWorkspaceObjectToRightPanel[\s\S]*payload\.kind === "browser"[\s\S]*item\.browserTabId === payload\.browserTabId[\s\S]*closeWorkspaceTab\(tab\.id\)[\s\S]*setRightPanelView\("browser"\)[\s\S]*setRightPanelBrowserTabFocusRequest\(\(current\) => \(\{[\s\S]*tabId: payload\.browserTabId/,
  );
  assert.match(
    appSource,
    /function handleReturnWorkspaceObjectToRightPanel[\s\S]*payload\.kind === "terminal"[\s\S]*item\.terminalTabId === payload\.terminalTabId[\s\S]*closeWorkspaceTab\(tab\.id\)[\s\S]*setRightPanelView\("terminal"\)[\s\S]*setRightPanelTerminalTabFocusRequest\(\(current\) => \(\{[\s\S]*tabId: payload\.terminalTabId/,
  );
  assert.match(appSource, /browserTabFocusRequest=\{rightPanelBrowserTabFocusRequest\}/);
  assert.match(appSource, /terminalTabFocusRequest=\{rightPanelTerminalTabFocusRequest\}/);
  assert.match(appSource, /setRightPanelBrowserTabFocusRequest\(\(current\) => \(\{[\s\S]*tabId: payload\.browserTabId,[\s\S]*token: \(current\?\.token \?\? 0\) \+ 1/);
  assert.match(appSource, /setRightPanelTerminalTabFocusRequest\(\(current\) => \(\{[\s\S]*tabId: payload\.terminalTabId,[\s\S]*token: \(current\?\.token \?\? 0\) \+ 1/);
  assert.match(appSource, /onReturnWorkspaceObject=\{handleReturnWorkspaceObjectToRightPanel\}/);
  assert.match(
    rightPanelSource,
    /function getReturnableWorkspaceObject[\s\S]*view: RightPanelView \| null = null[\s\S]*payload\.kind !== "browser" && payload\.kind !== "terminal"[\s\S]*view != null && payload\.kind !== workspaceObjectKindForView\(view\)[\s\S]*return payload;/,
  );
  assert.match(
    rightPanelSource,
    /className="right-panel-content"[\s\S]*onDragOver=\{\(event\) =>[\s\S]*handleWorkspaceObjectReturnDragOver\(event, null\)[\s\S]*onDrop=\{\(event\) =>[\s\S]*handleWorkspaceObjectReturnDrop\(event, null\)/,
  );
  assert.match(
    rightPanelSource,
    /className=\{`panel-rail-button[\s\S]*onDragOver=\{\(event\) =>[\s\S]*handleWorkspaceObjectReturnDragOver\(event, item\.view\)[\s\S]*onDrop=\{\(event\) =>[\s\S]*handleWorkspaceObjectReturnDrop\(event, item\.view\)/,
  );
  assert.match(rightPanelSource, /focusBrowserTabRequest\?: \{ tabId: string; token: number \} \| null/);
  assert.match(
    rightPanelSource,
    /!renderedTabs\.some\(\(tab\) => tab\.id === focusBrowserTabRequest\.tabId\)[\s\S]*return;[\s\S]*focusBrowserTabRequest\.tabId === state\.activeTabId[\s\S]*lastBrowserTabFocusRequestTokenRef\.current = focusBrowserTabRequest\.token;[\s\S]*currentBrowserPanelApi\(\)[\s\S]*lastBrowserTabFocusRequestTokenRef\.current = focusBrowserTabRequest\.token;[\s\S]*selectBrowserTab\(focusBrowserTabRequest\.tabId\)/,
  );
  assert.match(terminalPanelSource, /focusTerminalTabRequest\?: \{ tabId: string; token: number \} \| null/);
  assert.match(
    terminalPanelSource,
    /!visibleTabs\.some\(\(tab\) => tab\.id === focusTerminalTabRequest\.tabId\)[\s\S]*return;[\s\S]*focusTerminalTabRequest\.tabId === state\.activeTabId[\s\S]*lastTerminalTabFocusRequestTokenRef\.current = focusTerminalTabRequest\.token;[\s\S]*lastTerminalTabFocusRequestTokenRef\.current = focusTerminalTabRequest\.token;[\s\S]*selectTerminalTab\(focusTerminalTabRequest\.tabId\)/,
  );
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
    /const shouldHideNativeView =[\s\S]*!active[\s\S]*nativeOverlayActive[\s\S]*resizing[\s\S]*managerNativeViewBlocked/,
  );
  assert.match(
    rightPanelSource,
    /\}, \[active, activeTab\?\.id, managerNativeViewBlocked, nativeOverlayActive, resizing\]\);/,
  );
  assert.match(
    rightPanelSource,
    /if \(shouldHideNativeView\) \{[\s\S]*\.hideBrowserView\(\{ surfaceId \}\)/,
  );
  assert.match(
    rightPanelSource,
    /else \{[\s\S]*const bounds = measureBounds\(\)[\s\S]*\.showBrowserView\(bounds\)/,
  );
  assert.match(
    rightPanelSource,
    /const passiveBoundsCorrectionRef = useRef<\(\(\) => void\) \| null>\(null\)/,
  );
  assert.match(
    rightPanelSource,
    /const applyBrowserState = \(nextState: BrowserPanelState\) => \{[\s\S]*passiveBoundsCorrectionRef\.current\?\.\(\);[\s\S]*\};/,
  );
  assert.match(
    rightPanelSource,
    /passiveBoundsCorrectionRef\.current = scheduleBoundsUpdate;[\s\S]*scheduleBoundsUpdate\(\);/,
  );
  assert.match(
    rightPanelSource,
    /const sendBounds = \(\) => \{[\s\S]*\.setBrowserViewBounds\(bounds\)/,
  );
  assert.match(
    rightPanelSource,
    /const showNativeBrowserView = async \(browserApi: BrowserPanelApi\) => \{[\s\S]*browserBoundsFromElement\([\s\S]*\.showBrowserView\(bounds\)/,
  );
  assert.match(
    rightPanelSource,
    /await showNativeBrowserView\(browserApi\);[\s\S]*await browserApi\.navigateBrowserView\(\{[\s\S]*target: normalized\.url,[\s\S]*surfaceId: browserSurfaceIdRef\.current,[\s\S]*tabId: activeTab\?\.id \?\? null/,
  );
  const showBranch = rightPanelSource.slice(
    rightPanelSource.indexOf("} else {", rightPanelSource.indexOf("if (shouldHideNativeView)")),
    rightPanelSource.indexOf("scheduleBoundsUpdate();"),
  );
  assert.doesNotMatch(showBranch, /lastSentBounds = bounds/);
  assert.doesNotMatch(
    rightPanelSource,
    /requestAnimationFrame\(watchBounds\)/,
  );
  assert.match(
    appSource,
    /browserNativeOverlayActive=\{[\s\S]*isSelfCommandOpen \|\| isSettingsOpen \|\| isCreatingChatThread/,
  );
  assert.match(appSource, /browserPanelResizing=\{isRightPanelResizing\}/);
  assert.match(appSource, /setIsRightPanelResizing\(panel === "right"\)/);
  assert.match(appSource, /setIsRightPanelResizing\(false\)/);
  assert.match(appSource, /setPointerCapture\(pointerId\)/);
  assert.match(appSource, /releasePointerCapture\(pointerCapture\.pointerId\)/);
  assert.match(appSource, /window\.addEventListener\("blur", finishResize\)/);
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

test("browser panel header stays product chrome while tabs use page titles", () => {
  const rightPanelSource = readFileSync(
    new URL("./RightPanel.tsx", import.meta.url),
    "utf8",
  );
  const baiduTab = {
    id: "tab-baidu",
    url: "https://www.baidu.com/",
    title: "百度一下，你就知道",
    loading: false,
    canGoBack: false,
    canGoForward: false,
    error: null,
  };
  const labels = resolveBrowserPanelChromeLabels(baiduTab);

  assert.equal(labels.headerTitle, "Browser");
  assert.equal(labels.activeTabTitle, "百度一下，你就知道");
  assert.equal(browserTabLabel(baiduTab), "百度一下，你就知道");
  assert.match(rightPanelSource, /<h2>\{panelChromeLabels\.headerTitle\}<\/h2>/);
  assert.match(
    rightPanelSource,
    /<span className="browser-tab-title">\{browserTabLabel\(tab\)\}<\/span>/,
  );
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

  assert.match(markup, /Feature Development/);
  assert.match(markup, /workflow-status-pill running">Started/);
  assert.match(markup, /title="wf_1">wf_1/);
  assert.match(markup, /title="Started">Started/);
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
      onOpenPreviewInBrowser={() => {}}
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

test("thread analysis active view rerenders live command section and badge from active command state", () => {
  const activeCommand = {
    type: "commandExecution",
    id: "command-1",
    command: "pnpm package:root-worker-prototype:mac",
    cwd: "/repo",
    processId: "pid-1",
    status: "running",
    aggregatedOutput: null,
    exitCode: null,
    durationMs: null,
  } satisfies NonNullable<Thread["activeCommandItems"]>[number];
  const thread = {
    ...makeThread([], { type: "idle", reason: "eventSubscription" }),
    activeCommandItems: [activeCommand],
  } satisfies Thread;
  const initialMarkup = renderRightPanel(
    makeThread([], { type: "idle", reason: "eventSubscription" }),
    "skills",
  );
  const updatedMarkup = renderRightPanel(thread, "skills");

  assert.match(initialMarkup, /No live commands\./);
  assert.doesNotMatch(initialMarkup, /pnpm package:root-worker-prototype:mac/);
  assert.match(updatedMarkup, /Live Commands/);
  assert.match(updatedMarkup, /pnpm package:root-worker-prototype:mac/);
  assert.doesNotMatch(updatedMarkup, /No live commands\./);
  assert.match(
    updatedMarkup,
    /aria-label="Thread Analysis"[\s\S]*<span class="panel-rail-badge">1<\/span>/,
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

test("workspace conversation tabs use concrete thread labels and preserve layout affordances", () => {
  const appSource = readFileSync(new URL("../App.tsx", import.meta.url), "utf8");
  const rightPanelSource = readFileSync(new URL("./RightPanel.tsx", import.meta.url), "utf8");
  const terminalPanelSource = readFileSync(new URL("./TerminalPanel.tsx", import.meta.url), "utf8");
  const panelsSource = readFileSync(new URL("./Panels.tsx", import.meta.url), "utf8");
  const agentTreeSource = readFileSync(new URL("./AgentTree.tsx", import.meta.url), "utf8");
  const stylesSource = readFileSync(new URL("../styles.css", import.meta.url), "utf8");
  const cssBlock = (selector: string, nextSelector: string) =>
    stylesSource.slice(stylesSource.indexOf(selector), stylesSource.indexOf(nextSelector));

  assert.match(appSource, /workspaceTabForThread/);
  assert.match(appSource, /workspaceTabForFile/);
  assert.match(appSource, /workspaceTabForBrowser/);
  assert.match(appSource, /workspaceTabForTerminal/);
  assert.match(appSource, /browserWorkspaceTabId\(tab\?\.browserTabId/);
  assert.match(appSource, /terminalWorkspaceTabId\(tab\?\.terminalTabId/);
  assert.match(appSource, /browserTabId: tab\?\.browserTabId/);
  assert.match(appSource, /terminalTabId: tab\?\.terminalTabId/);
  assert.match(appSource, /readWorkspaceObjectDragData\(event\.dataTransfer\)/);
  assert.match(appSource, /aria-label="Workspace object tabs"/);
  assert.match(appSource, /getWorkspaceTabThread/);
  assert.match(appSource, /getRootThreadConversationTitle\(thread\)/);
  assert.match(appSource, /getAgentRoleLabel\(thread\)/);
  assert.match(appSource, /isRootThread\(thread\)[\s\S]*\? getAgentRoleLabel\(thread\)[\s\S]*: getThreadPresenceLabel\(thread\)/);
  assert.match(appSource, /getThreadPath\(thread\)/);
  assert.match(appSource, /storedWorkspaceTabOrderRef/);
  assert.match(appSource, /applyStoredWorkspaceTabOrder\([\s\S]*storedWorkspaceTabOrderRef\.current/);
  assert.doesNotMatch(appSource, /storeWorkspaceTabOrder\(workspaceTabs/);
  assert.match(appSource, /storedWorkspaceTabOrderRef\.current = storeWorkspaceTabOrder/);
  assert.match(appSource, /current\.flatMap\(\(tab\) =>/);
  assert.match(appSource, /return thread \? \[workspaceTabForThread\(thread\)\] : \[\]/);
  assert.match(appSource, /current\.map\(\(item\) => \(item\.id === tab\.id \? tab : item\)\)/);
  assert.match(appSource, /function openConversationWorkspaceTab\(threadId: string\)/);
  assert.match(appSource, /openConversationWorkspaceTab\(threadId\);[\s\S]*setSelectedThreadId\(threadId\);/);
  assert.match(appSource, /useEffect\(\(\) => \{\s*if \(selectedThreadId\)/);
  assert.doesNotMatch(appSource, /setActiveWorkspaceTabId\(tab\.id\);\s*\}, \[selectedThread\]\)/);
  assert.match(appSource, /workspaceTabs\.some\(\(tab\) => tab\.id === current\)/);
  assert.match(appSource, /threads\.some\(\(thread\) => thread\.id === tab\.threadId\)/);
  assert.match(appSource, /const tabThread = getWorkspaceTabThread\(tab, threads\)/);
  assert.match(appSource, /workspace-tab-dot \$\{threadDisplayStatusClass\(tabThread\)\}/);
  assert.doesNotMatch(appSource, /workspace-tab-dot \$\{tab\.kind\}/);
  assert.match(appSource, /closeWorkspaceTabById\(currentTabs, tabId\)/);
  assert.match(
    appSource,
    /event\.stopPropagation\(\);\s*closeWorkspaceTab\(tab\.id, \{ closeOwnedBrowserTab: true \}\)/,
  );
  assert.match(appSource, /async function openFilePathInWorkspace\(target: string\)/);
  assert.match(appSource, /return \{ preview, rootId: requestRootId \}/);
  assert.match(appSource, /const result = await loadFilePreview\(target, \{ preserveRightPanel: true \}\)/);
  assert.match(appSource, /workspaceTabForFile\(result\.preview, result\.rootId\)/);
  assert.doesNotMatch(appSource, /workspaceTabForFile\(preview, selectedTreeRootIdRef\.current\)/);
  assert.match(appSource, /title: displayPath\.split\("\/"\)\.filter\(Boolean\)\.at\(-1\) \?\? displayPath/);
  assert.match(appSource, /kind: "terminal",[\s\S]*title: "Terminal"/);
  assert.doesNotMatch(appSource, /<span className="workspace-tab-subtitle">/);
  assert.match(appSource, /filePanelView="preview"/);
  assert.match(appSource, /\[target\]: payload\.entries/);
  assert.match(appSource, /\[payload\.path\]: payload\.entries/);
  assert.doesNotMatch(appSource, /filePanelViewRef/);
  assert.doesNotMatch(appSource, /filePanelViewRef\.current === "tree"/);
  assert.match(
    appSource,
    /setExpandedTreeDirectories\(\[\]\);[\s\S]*if \(rightPanelView === "preview" && selectedThread\?\.cwd\) \{[\s\S]*loadFileTreeDirectory\(selectedThread\.cwd\)/,
  );
  assert.match(
    appSource,
    /const previousRightPanelViewRef = useRef<RightPanelView>\(rightPanelView\)/,
  );
  assert.match(
    appSource,
    /useEffect\(\(\) => \{[\s\S]*const previousRightPanelView = previousRightPanelViewRef\.current;[\s\S]*previousRightPanelViewRef\.current = rightPanelView;[\s\S]*previousRightPanelView !== "preview"[\s\S]*rightPanelView === "preview"[\s\S]*ensureFileTreeDirectoryLoaded\(selectedThread\.cwd\)/,
  );
  assert.doesNotMatch(
    appSource,
    /function handleSetRightPanelView\(view: RightPanelView\) \{[\s\S]*ensureFileTreeDirectoryLoaded\(selectedThread\.cwd\)[\s\S]*if \(activeWorkspaceTab\?\.kind === "browser"/,
  );
  assert.match(appSource, /fileTreeEntriesByPath\[target\] \|\| fileTreeLoadingPath === target/);
  assert.match(appSource, /function handleOpenTreeFile\(target: string\) \{[\s\S]*openFilePathInWorkspace\(target\)/);
  assert.match(appSource, /if \(rightPanelView === "preview"\) \{[\s\S]*setRightPanelView\("skills"\)/);
  assert.match(appSource, /activeWorkspaceTab\?\.kind !== "file"/);
  assert.match(appSource, /activeWorkspaceTab\?\.kind !== "browser"/);
  assert.match(appSource, /activeWorkspaceTab\?\.kind !== "terminal"/);
  assert.match(appSource, /const activeTerminalThread =/);
  assert.match(appSource, /thread=\{activeTerminalThread\}/);
  assert.match(
    appSource,
    /activeWorkspaceTab\?\.kind === "browser"[\s\S]*<BrowserPanel[\s\S]*variant="workspace"[\s\S]*activeBrowserTabId=\{activeWorkspaceTab\.browserTabId \?\? null\}/,
  );
  assert.match(
    appSource,
    /activeWorkspaceTab\?\.kind === "terminal"[\s\S]*<TerminalPanel[\s\S]*variant="workspace"[\s\S]*activeTerminalTabId=\{activeWorkspaceTab\.terminalTabId \?\? null\}/,
  );
  assert.match(appSource, /function gitDiffWorkspaceTabId\(targetId: string\)/);
  assert.match(appSource, /function workspaceTabForGitDiff\(state: GitDiffPreviewState\)/);
  assert.match(appSource, /kind: "diff"/);
  assert.match(appSource, /gitDiffWorkspaceStateById/);
  assert.match(appSource, /onGitDiffPreviewChange=\{handleGitDiffPreviewChange\}/);
  assert.match(
    appSource,
    /if \(state\.loading \|\| workspaceTabsRef\.current\.some\(\(item\) => item\.id === tab\.id\)\) \{[\s\S]*upsertWorkspaceObjectTab\(tab, \{ activate: state\.loading \}\)/,
  );
  assert.match(
    appSource,
    /activeWorkspaceTab\?\.kind === "diff"[\s\S]*<GitDiffPreviewPanel[\s\S]*diff=\{activeWorkspaceDiffState\.diff\}/,
  );
  assert.match(
    appSource,
    /activeWorkspaceTab\?\.kind !== "file"[\s\S]*<FilePreviewPanel[\s\S]*variant="workspace"[\s\S]*gitDiffPreview=\{null\}/,
  );
  const workspaceBrowserPanelSource = appSource.slice(
    appSource.indexOf('activeWorkspaceTab?.kind === "browser"'),
    appSource.indexOf('activeWorkspaceTab?.kind === "terminal"'),
  );
  assert.doesNotMatch(workspaceBrowserPanelSource, /rightPanelView/);
  assert.doesNotMatch(workspaceBrowserPanelSource, /effectiveActiveView/);
  assert.doesNotMatch(
    appSource,
    /function openBrowserInWorkspace[\s\S]*setRightPanelView\("skills"\)/,
  );
  assert.doesNotMatch(
    appSource,
    /function openTerminalInWorkspace[\s\S]*setRightPanelView\("skills"\)/,
  );
  assert.doesNotMatch(appSource, /setRightPanelViewWithWorkspaceFallback/);
  assert.match(appSource, /const detachedWorkspaceBrowserTabIds = workspaceTabs[\s\S]*tab\.kind === "browser" && tab\.browserTabId[\s\S]*map\(\(tab\) => tab\.browserTabId as string\)/);
  assert.match(appSource, /const detachedWorkspaceTerminalTabIds = workspaceTabs[\s\S]*tab\.kind === "terminal" && tab\.terminalTabId[\s\S]*map\(\(tab\) => tab\.terminalTabId as string\)/);
  assert.match(appSource, /detachedBrowserTabIds=\{detachedWorkspaceBrowserTabIds\}/);
  assert.match(appSource, /detachedTerminalTabIds=\{detachedWorkspaceTerminalTabIds\}/);
  assert.match(appSource, /onReturnWorkspaceObject=\{handleReturnWorkspaceObjectToRightPanel\}/);
  assert.match(appSource, /browserTabFocusRequest=\{rightPanelBrowserTabFocusRequest\}/);
  assert.doesNotMatch(appSource, /browserNativeViewSuppressed/);
  assert.match(appSource, /terminalTabFocusRequest=\{rightPanelTerminalTabFocusRequest\}/);
  assert.match(
    appSource,
    /function handleOpenArtifactUrl\(url: string\)[\s\S]*setRightPanelView\("browser"\)/,
  );
  assert.match(
    appSource,
    /function closeWorkspaceTab\([\s\S]*options: \{ closeOwnedBrowserTab\?: boolean \} = \{\}[\s\S]*options\.closeOwnedBrowserTab[\s\S]*closingTab\?\.kind === "browser"[\s\S]*window\.codexDesktop[\s\S]*\.closeBrowserTab\(closingTab\.browserTabId\)/,
  );
  assert.match(
    appSource,
    /className="workspace-tab-close"[\s\S]*onClick=\{\(event\) => \{[\s\S]*closeWorkspaceTab\(tab\.id, \{ closeOwnedBrowserTab: true \}\);[\s\S]*onKeyDown=\{\(event\) => \{[\s\S]*closeWorkspaceTab\(tab\.id, \{ closeOwnedBrowserTab: true \}\);/,
  );
  assert.match(
    appSource,
    /function handleReturnWorkspaceObjectToRightPanel\([\s\S]*payload\.kind === "browser"[\s\S]*closeWorkspaceTab\(tab\.id\);[\s\S]*setRightPanelView\("browser"\)[\s\S]*setRightPanelBrowserTabFocusRequest/,
  );
  assert.doesNotMatch(
    appSource.slice(
      appSource.indexOf("function handleReturnWorkspaceObjectToRightPanel"),
      appSource.indexOf("function handleSetRightPanelView"),
    ),
    /closeOwnedBrowserTab/,
  );
  assert.match(
    appSource,
    /function handleThreadAnalysisCommandFocus[\s\S]*setRightPanelView\("terminal"\)/,
  );
  assert.match(appSource, /onOpenWorkspaceObject=\{openRightPanelObjectInWorkspace\}/);
  assert.match(appSource, /hasWorkspaceObjectDragData\(event\.dataTransfer\)/);
  assert.match(appSource, /const PANEL_RESIZER_WIDTH = 4/);
  assert.match(appSource, /revealThreadInSidebarState\(\{/);
  assert.match(appSource, /touchedProjectCollapseIdsRef\.current\.add\(next\.expandedProjectId\)/);
  assert.match(
    appSource,
    /const conversationCells = useMemo\(\(\) => \{[\s\S]*buildConversationState\([\s\S]*selectedThread[\s\S]*filterConversationCellsForDisplay\(nextConversationState\.cells\);[\s\S]*\}, \[selectedThread\]\);/,
  );
  assert.match(rightPanelSource, /type WorkspaceOpenableRightPanelObject/);
  assert.doesNotMatch(rightPanelSource, /panel-eyebrow/);
  assert.doesNotMatch(rightPanelSource, /preview-mode-toggle/);
  assert.doesNotMatch(rightPanelSource, /Context mix/);
  assert.doesNotMatch(rightPanelSource, /aria-label="Show current file"/);
  assert.doesNotMatch(rightPanelSource, /aria-label="Show file tree"/);
  assert.doesNotMatch(rightPanelSource, /file-object-toolbar/);
  assert.match(rightPanelSource, /draggable=\{workspaceObjectKindForView\(item\.view\) != null\}/);
  assert.match(rightPanelSource, /writeWorkspaceObjectDragData\(event\.dataTransfer, kind\)/);
  assert.match(rightPanelSource, /onOpenWorkspaceObject\?\.\(kind\)/);
  assert.match(rightPanelSource, /hasWorkspaceObjectDragData\(event\.dataTransfer\)/);
  assert.match(rightPanelSource, /browserTabDragPayload/);
  assert.match(rightPanelSource, /browserTabId: tab\.id/);
  assert.match(rightPanelSource, /onOpenBrowserTabInWorkspace/);
  assert.match(rightPanelSource, /detachedBrowserTabIds\?: string\[\]/);
  assert.match(rightPanelSource, /detachedTerminalTabIds\?: string\[\]/);
  assert.match(rightPanelSource, /onReturnWorkspaceObject\?: \(payload: WorkspaceObjectDragPayload\) => void/);
  assert.match(rightPanelSource, /activeBrowserTabId\?: string \| null/);
  assert.match(rightPanelSource, /browserTabFocusRequest\?: \{ tabId: string; token: number \} \| null/);
  assert.doesNotMatch(rightPanelSource, /browserNativeViewSuppressed\?: boolean/);
  assert.doesNotMatch(rightPanelSource, /suppressNativeView/);
  assert.match(rightPanelSource, /terminalTabFocusRequest\?: \{ tabId: string; token: number \} \| null/);
  assert.match(terminalPanelSource, /activeTerminalTabId\?: string \| null/);
  assert.match(rightPanelSource, /detachedBrowserTabIds=\{detachedBrowserTabIds\}/);
  assert.match(rightPanelSource, /detachedTerminalTabIds=\{detachedTerminalTabIds\}/);
  assert.match(rightPanelSource, /variant = "manager"/);
  assert.match(rightPanelSource, /variant\?: "manager" \| "workspace"/);
  const browserPanelSource = rightPanelSource.slice(
    rightPanelSource.indexOf("export function BrowserPanel"),
    rightPanelSource.indexOf("export function normalizeBrowserPanelState"),
  );
  const browserSelectionSource = rightPanelSource.slice(
    rightPanelSource.indexOf("export function resolveBrowserPanelTabSelection"),
    rightPanelSource.indexOf("function formatByteSize"),
  );
  const selectForSurfaceSource = browserPanelSource.slice(
    browserPanelSource.indexOf("const selectBrowserTabForSurfaceIfNeeded"),
    browserPanelSource.indexOf("useEffect(() => {", browserPanelSource.indexOf("const selectBrowserTabForSurfaceIfNeeded")),
  );
  const navigationRequestSource = browserPanelSource.slice(
    browserPanelSource.indexOf("if (!navigationRequest)"),
    browserPanelSource.indexOf("}, [", browserPanelSource.indexOf("if (!navigationRequest)")),
  );
  const focusBrowserRequestSource = browserPanelSource.slice(
    browserPanelSource.lastIndexOf("useEffect(() => {", browserPanelSource.indexOf("!focusBrowserTabRequest")),
    browserPanelSource.indexOf("}, [", browserPanelSource.indexOf("!focusBrowserTabRequest")),
  );
  const navigateSource = browserPanelSource.slice(
    browserPanelSource.indexOf("const navigate = () => {"),
    browserPanelSource.indexOf("const runCommand = ("),
  );
  const runCommandSource = browserPanelSource.slice(
    browserPanelSource.indexOf("const runCommand = ("),
    browserPanelSource.indexOf("const createTab = () => {"),
  );
  const createTabSource = browserPanelSource.slice(
    browserPanelSource.indexOf("const createTab = () => {"),
    browserPanelSource.indexOf("const selectTab = ("),
  );
  const managerNewTabButtonSource = browserPanelSource.slice(
    browserPanelSource.indexOf("browser-new-tab-button"),
    browserPanelSource.indexOf("{activeTab ? ("),
  );
  const surfaceApplySource = browserPanelSource.slice(
    browserPanelSource.indexOf("const applyBrowserState ="),
    browserPanelSource.indexOf("const showNativeBrowserView ="),
  );
  const managerChromeSource = browserPanelSource.slice(
    browserPanelSource.indexOf("{isManagerVariant ? ("),
    browserPanelSource.indexOf("{!isManagerVariant ? ("),
  );
  assert.match(rightPanelSource, /function resolveBrowserPanelTabSelection/);
  assert.match(browserSelectionSource, /const detachedBrowserTabIdSet = new Set\(detachedBrowserTabIds\)/);
  assert.match(browserSelectionSource, /tabs\.filter\(\(tab\) => !detachedBrowserTabIdSet\.has\(tab\.id\)\)/);
  assert.match(browserSelectionSource, /managerSelectedBrowserTabId \?\? activeTabId/);
  assert.match(browserPanelSource, /resolveBrowserPanelTabSelection\(\{/);
  assert.match(browserPanelSource, /activeTabId: state\.activeTabId/);
  assert.match(browserPanelSource, /managerSelectedBrowserTabId/);
  assert.doesNotMatch(rightPanelSource, /nativeViewSuppressed/);
  assert.match(browserPanelSource, /const managerNativeViewBlocked =\s*managerActiveTabDetached \|\|/);
  assert.match(browserPanelSource, /managerHasDetachedTabs && activeTab == null/);
  assert.match(selectForSurfaceSource, /if \(!isManagerVariant\) \{[\s\S]*return;[\s\S]*\}/);
  assert.match(selectForSurfaceSource, /selectBrowserTab\(targetTabId\)/);
  assert.doesNotMatch(selectForSurfaceSource, /activeBrowserTabId/);
  assert.doesNotMatch(focusBrowserRequestSource, /nativeViewSuppressed/);
  assert.match(
    rightPanelSource,
    /if \(isManagerVariant && managerHasDetachedTabs && !activeTab\) \{[\s\S]*return;[\s\S]*\}[\s\S]*const browserApi = currentBrowserPanelApi\(\);/,
  );
  assert.doesNotMatch(navigationRequestSource, /nativeViewSuppressed/);
  assert.doesNotMatch(navigateSource, /nativeViewSuppressed/);
  assert.doesNotMatch(runCommandSource, /nativeViewSuppressed/);
  assert.match(createTabSource, /createBrowserTab\(\)/);
  assert.match(managerNewTabButtonSource, /disabled=\{!hasBrowserApi\}/);
  assert.match(rightPanelSource, /const shouldHideNativeView =[\s\S]*managerNativeViewBlocked/);
  assert.match(rightPanelSource, /function nextBrowserPanelSurfaceId\(\)/);
  assert.match(rightPanelSource, /const browserSurfaceIdRef = useRef\(nextBrowserPanelSurfaceId\(\)\)/);
  assert.match(rightPanelSource, /hideBrowserView\(\{ surfaceId \}\)/);
  assert.match(rightPanelSource, /Browser content is open in workspace\./);
  assert.doesNotMatch(
    browserPanelSource,
    /runCommand\([\s\S]*selectBrowserTab\(activeBrowserTabId\)/,
  );
  assert.match(browserPanelSource, /const browserSurfaceRef = useRef\(\{/);
  assert.match(browserPanelSource, /detachedBrowserTabIds/);
  assert.match(browserPanelSource, /const addressInputFocusedRef = useRef\(false\)/);
  assert.match(browserPanelSource, /const lastAddressTabIdRef = useRef<string \| null>\(null\)/);
  assert.match(browserPanelSource, /const syncAddressFromTab = \(/);
  assert.match(browserPanelSource, /addressInputFocusedRef\.current && !tabChanged/);
  assert.match(browserPanelSource, /const handleAddressInputBlur = \(event: React\.FocusEvent<HTMLInputElement>\) => \{/);
  assert.match(
    browserPanelSource,
    /event\.currentTarget\.form\?\.contains\(nextFocusedElement\)[\s\S]*return;[\s\S]*syncAddressFromTab\(activeTab\)/,
  );
  assert.match(browserPanelSource, /onFocus=\{\(\) => \{[\s\S]*addressInputFocusedRef\.current = true/);
  assert.match(browserPanelSource, /aria-label="Browser URL"[\s\S]*onBlur=\{handleAddressInputBlur\}/);
  assert.match(browserPanelSource, /aria-label="Workspace browser URL"[\s\S]*onBlur=\{handleAddressInputBlur\}/);
  assert.match(surfaceApplySource, /const surfaceDetachedTabIds = new Set\(surface\.detachedBrowserTabIds\)/);
  assert.match(surfaceApplySource, /surface\.managerSelectedBrowserTabId/);
  assert.match(surfaceApplySource, /!surfaceDetachedTabIds\.has\(normalizedActiveTab\.id\)/);
  assert.match(surfaceApplySource, /syncAddressFromTab\(surfaceActiveTab\)/);
  assert.match(navigateSource, /await selectBrowserTabForSurfaceIfNeeded\(browserApi\)/);
  assert.match(navigateSource, /await showNativeBrowserView\(browserApi\)/);
  assert.match(navigateSource, /navigateBrowserView\(\{[\s\S]*target: normalized\.url,[\s\S]*surfaceId: browserSurfaceIdRef\.current,[\s\S]*tabId: activeTab\?\.id \?\? null/);
  assert.match(rightPanelSource, /isManagerVariant \? "browser-panel-manager" : "browser-panel-workspace"/);
  assert.match(managerChromeSource, /browser-tab-strip/);
  assert.match(managerChromeSource, /\{activeTab \? \(/);
  assert.match(managerChromeSource, /browser-toolbar/);
  assert.match(managerChromeSource, /browser-status-row/);
  assert.match(rightPanelSource, /!isManagerVariant \? \([\s\S]*browser-toolbar browser-toolbar-workspace[\s\S]*aria-label="Workspace browser URL"[\s\S]*\) : null/);
  assert.match(rightPanelSource, /<div ref=\{viewportRef\} className="browser-native-viewport">/);
  assert.match(rightPanelSource, /const fileSourcePanelView: FilePanelView =[\s\S]*\? "preview"[\s\S]*: "tree"/);
  assert.match(rightPanelSource, /!workspaceTabsEnabled &&[\s\S]*gitDiffPreview\.loading/);
  assert.match(rightPanelSource, /gitDiffRequestScopeByTargetRef/);
  assert.match(
    rightPanelSource,
    /function clearGitDiffPreview\(\) \{[\s\S]*if \(!workspaceTabsEnabled\) \{[\s\S]*gitDiffRequestScopeByTargetRef\.current\.clear\(\)/,
  );
  assert.match(rightPanelSource, /function beginGitDiffRequest\(targetId: string/);
  assert.match(rightPanelSource, /function isCurrentGitDiffRequest\(targetId: string, scope: number\)/);
  assert.match(rightPanelSource, /const targetId = `worktree:\$\{thread\.cwd\}:\$\{mode\}:\$\{change\.originalPath \?\? ""\}:\$\{change\.path\}`/);
  assert.match(rightPanelSource, /const targetId = `commit:\$\{thread\.cwd\}:\$\{commit\.hash\}:\$\{file\.originalPath \?\? ""\}:\$\{file\.path\}`/);
  assert.match(rightPanelSource, /beginGitDiffRequest\(targetId, \{ exclusive: !workspaceTabsEnabled \}\)/);
  assert.match(rightPanelSource, /isCurrentGitDiffRequest\(targetId, scope\)/);
  assert.match(rightPanelSource, /if \(!workspaceTabsEnabled\) \{[\s\S]*onSetActiveView\("preview"\)/);
  assert.match(rightPanelSource, /gitDiffPreview=\{workspaceTabsEnabled \? null : gitDiffPreview\.diff\}/);
  assert.match(rightPanelSource, /filePanelView=\{fileSourcePanelView\}/);
  assert.match(rightPanelSource, /variant = "manager"/);
  assert.match(rightPanelSource, /variant\?: "manager" \| "workspace"/);
  assert.match(rightPanelSource, /preview-workspace-status-bar/);
  assert.match(rightPanelSource, /!isWorkspaceVariant \? \([\s\S]*panel-content-header preview-header/);
  assert.match(panelsSource, /data-thread-id/);
  assert.match(panelsSource, /className="chat-list-row"[\s\S]*data-thread-id=\{node\.threadId\}/);
  assert.match(panelsSource, /scrollIntoView\(\{ block: "nearest" \}\)/);
  assert.match(agentTreeSource, /data-thread-id=\{node\.threadId\}/);
  assert.doesNotMatch(appSource, /WORKSPACE_TAB_LABELS/);
  assert.match(appSource, /<FilePreviewPanel/);
  assert.match(appSource, /<BrowserPanel/);
  assert.match(appSource, /<TerminalPanel/);
  assert.match(appSource, /gridTemplateColumns: `\$\{sidebarWidth\}px \$\{PANEL_RESIZER_WIDTH\}px minmax\(0, 1fr\) \$\{PANEL_RESIZER_WIDTH\}px/);
  assert.match(stylesSource, /\.workspace-tab-panel > \.conversation-panel/);
  assert.match(stylesSource, /width: 100%;/);
  assert.match(stylesSource, /\.workspace-tab-panel > \.conversation-panel,[\s\S]*\.workspace-tab-panel > \.browser-panel \{[\s\S]*border-top: 0;[\s\S]*box-shadow: none;[\s\S]*background-image: none;/);
  assert.match(stylesSource, /\.workspace-tab-strip \{[\s\S]*gap: 6px;[\s\S]*border-bottom: 0;[\s\S]*box-shadow: none;/);
  assert.match(stylesSource, /\.workspace-tab \{[\s\S]*border: 0;[\s\S]*border-radius: 999px;[\s\S]*background: rgba\(28, 25, 23, 0\.045\);/);
  assert.match(stylesSource, /\.workspace-tab\.active \{[\s\S]*background: rgba\(15, 118, 110, 0\.12\);[\s\S]*box-shadow: inset 0 0 0 1px/);
  assert.doesNotMatch(cssBlock(".workspace-tab {", ".workspace-tab:hover"), /cursor:/);
  assert.doesNotMatch(cssBlock(".workspace-tab-close {", ".workspace-tab-close:hover"), /cursor:/);
  assert.doesNotMatch(stylesSource, /\.workspace-tab:active \{[\s\S]*cursor:/);
  assert.match(stylesSource, /\.browser-panel-workspace \{[\s\S]*background: #ffffff;/);
  assert.match(stylesSource, /\.browser-toolbar-workspace \{[\s\S]*border-bottom: 0;[\s\S]*box-shadow: none;/);
  assert.match(stylesSource, /\.browser-panel-workspace \.browser-native-viewport \{[\s\S]*min-height: 0;/);
  assert.match(stylesSource, /\.terminal-panel-workspace \{[\s\S]*background: #f5f3f0;/);
  assert.match(stylesSource, /\.terminal-panel-workspace \.terminal-viewport-shell \{[\s\S]*border-top: 0;[\s\S]*box-shadow: none;/);
  assert.match(stylesSource, /\.preview-workspace-status-bar \{[\s\S]*min-height: 34px;/);
  assert.match(stylesSource, /\.preview-panel-workspace > \.preview-editor-shell > \.preview-utility-strip,[\s\S]*display: none;/);
  assert.match(stylesSource, /\.git-panel \{[\s\S]*--git-surface: rgba\(252, 251, 249, 0\.58\);[\s\S]*background: var\(--git-surface\);/);
  assert.match(stylesSource, /\.git-graph-section \{[\s\S]*background: var\(--git-surface\);/);
  assert.match(stylesSource, /\.git-section-header \{[\s\S]*background: var\(--git-surface-raised\);/);
  assert.match(stylesSource, /\.git-graph-row:hover \{[\s\S]*background: var\(--git-surface-hover\);/);
  assert.doesNotMatch(cssBlock(".git-section-toggle {", ".git-section-toggle:hover"), /cursor:/);
  assert.doesNotMatch(cssBlock(".git-icon-button {", ".git-icon-button:hover"), /cursor:/);
  assert.doesNotMatch(cssBlock(".drag-scroll-region {", ".drag-scroll-region.is-dragging"), /cursor:/);
  assert.doesNotMatch(cssBlock(".git-graph-row-main {", ".git-graph-lanes"), /cursor:/);
  assert.doesNotMatch(cssBlock(".git-change-group-header {", ".git-change-group-header[aria-expanded"), /cursor:/);
  assert.doesNotMatch(cssBlock(".git-change-row.clickable:hover", ".git-change-row.clickable:focus-visible"), /cursor:/);
  assert.match(stylesSource, /\.workspace-tab-strip \{[\s\S]*border-bottom: 0;/);
  assert.match(
    stylesSource,
    /UI polish: right panel content keeps structure without extra 1px separator lines\./,
  );
  assert.match(
    stylesSource,
    /\.right-panel \.panel-rail,[\s\S]*\.right-panel \.terminal-viewport-shell \{[\s\S]*border-top: 0;[\s\S]*border-bottom: 0;/,
  );
  assert.match(stylesSource, /\.right-panel \.panel-rail \{[\s\S]*border-left: 0;/);
  assert.match(
    stylesSource,
    /\.right-panel \.overview-metric,[\s\S]*\.right-panel \.workflow-status-pill \{[\s\S]*border: 0;/,
  );
  assert.match(stylesSource, /\.panel-rail-button\.active \{[\s\S]*background: rgba\(28, 25, 23, 0\.05\);/);
  assert.match(stylesSource, /\.panel-rail-button\.active::after \{[\s\S]*background: #d97706;/);
  assert.match(
    stylesSource,
    /\.workspace-tab-panel > \.conversation-panel,\s*\.conversation-panel,\s*\.conversation-scroll \{[\s\S]*border-top: 0;[\s\S]*box-shadow: none;/,
  );
  assert.match(stylesSource, /\.sidebar \{[\s\S]*border-right: 0;/);
  assert.match(stylesSource, /\.conversation-scroll \{[\s\S]*background: #ffffff;[\s\S]*background-image: none;/);
  assert.match(
    stylesSource,
    /\.compact-row::before,\s*\.archive-row::before \{[\s\S]*display: none;/,
  );
  assert.match(stylesSource, /\.panel-resizer \{[\s\S]*background: transparent;/);
  assert.match(stylesSource, /\.panel-resizer::before \{[\s\S]*left: 50%;[\s\S]*width: 1px;[\s\S]*background: rgba\(16, 24, 40, 0\.08\);/);
  assert.match(stylesSource, /\.panel-resizer:hover::before \{[\s\S]*background: rgba\(217, 119, 6, 0\.42\);/);
  assert.match(stylesSource, /\.is-resizing-panels \.panel-resizer::before \{[\s\S]*background: rgba\(217, 119, 6, 0\.68\);/);
  assert.match(appSource, /const PANEL_RESIZER_WIDTH = 4/);
  assert.match(stylesSource, /\.panel-content-header \{[\s\S]*min-height: 34px;[\s\S]*padding: 6px 10px;/);
  assert.doesNotMatch(stylesSource, /\.file-object-toolbar/);
  assert.doesNotMatch(stylesSource, /\.panel-eyebrow/);
  assert.doesNotMatch(stylesSource, /\.preview-mode-toggle/);
  assert.match(stylesSource, /\.workspace-tab-dot\.doing/);
  assert.match(stylesSource, /\.workspace-tab-dot\.waiting-subagent/);
  assert.match(stylesSource, /\.workspace-tab-dot\.waiting-eventtool/);
  assert.match(stylesSource, /\.workspace-tab-dot\.waiting-subscription/);
  assert.match(stylesSource, /\.workspace-tab-dot\.blocked/);
  assert.match(stylesSource, /\.workspace-tab-dot\.active/);
  assert.match(stylesSource, /\.workspace-tab-dot\.running/);
  assert.match(stylesSource, /\.workspace-tab-dot\.completed/);
  assert.match(stylesSource, /\.workspace-tab-dot\.inactive/);
  assert.match(stylesSource, /\.workspace-tab-close \{[\s\S]*-webkit-app-region: no-drag;/);
  assert.doesNotMatch(stylesSource, /\.conversation-header/);
  assert.doesNotMatch(
    stylesSource,
    /\.conversation-panel[^{]*\{[^}]*border-right:/,
  );
  assert.doesNotMatch(
    stylesSource,
    /\.conversation-panel[^{]*\{[^}]*border-top:(?![ \t]*0[ \t]*;)/,
  );
  assert.doesNotMatch(
    stylesSource,
    /\.conversation-panel[^{]*\{[^}]*box-shadow:(?![ \t]*none[ \t]*;)/,
  );
  assert.doesNotMatch(
    stylesSource,
    /\.conversation-scroll[^{]*\{[^}]*border-top:(?![ \t]*0[ \t]*;)/,
  );
  assert.doesNotMatch(
    stylesSource,
    /\.conversation-scroll[^{]*\{[^}]*box-shadow:(?![ \t]*none[ \t]*;)/,
  );
  assert.doesNotMatch(
    stylesSource,
    /\.workspace-tab-panel[^{]*\{[^}]*border-right:/,
  );
  assert.doesNotMatch(
    stylesSource,
    /\.workspace-tab-panel\s*\{[^}]*border-top:/,
  );
  assert.doesNotMatch(
    stylesSource,
    /\.workspace-tab-panel\s*\{[^}]*box-shadow:/,
  );
  assert.doesNotMatch(
    stylesSource,
    /\.workspace-tab-strip\s*\{[^}]*border-bottom:(?![ \t]*0[ \t]*;)/,
  );
  assert.doesNotMatch(
    cssBlock(".compact-row::before,", ".compact-icon"),
    /(?:top:|height:|background: linear-gradient)/,
  );
  assert.doesNotMatch(stylesSource, /\.workspace-main \{[^}]*border-right:/);
  assert.doesNotMatch(stylesSource, /\.composer-shell \{[^}]*border-top:/);
  assert.doesNotMatch(stylesSource, /\.composer-shell \{[^}]*box-shadow:/);
  assert.doesNotMatch(stylesSource, /scrollbar[^{}]*(?::focus|:focus-within|:active)[^{]*\{/);
  assert.doesNotMatch(stylesSource, /(?::focus|:focus-within|:active)[^{]*::-[^{]*scrollbar/);
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
  const railClickSource = source.slice(
    railClickIndex,
    source.indexOf("onSetActiveView(next.nextView);", railClickIndex),
  );

  assert.notEqual(tokenStateIndex, -1);
  assert.notEqual(propIndex, -1);
  assert.notEqual(railClickIndex, -1);
  assert.match(
    railClickSource,
    /setTerminalPanelFocusRequestToken\(\s*\(current\) => current \+ 1,\s*\);/,
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
  assert.match(markup, /File Reads/);
  assert.match(markup, /Commands/);
  assert.match(markup, /Inter-Agent/);
  assert.match(markup, /Search &amp; Media/);
  assert.match(markup, /Other Tools/);
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
      { step: "Validate parallel owner", status: "inProgress" },
      { step: "Wait for release approval", status: "blocked" },
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
  assert.match(markup, /Validate parallel owner/);
  assert.match(markup, /Wait for release approval/);
  assert.match(markup, /Run validation/);
  assert.match(markup, /In progress/);
  assert.match(markup, /Blocked/);
  assert.match(markup, /plan-status-label blocked/);
  assert.equal(markup.match(/plan-status-label inProgress/g)?.length, 2);
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
  assert.match(css, /\.plan-status-dot\.blocked/);
  assert.match(css, /\.plan-status-label\.blocked/);
  assert.match(
    css,
    /\.monitor-section\s*\{[\s\S]*padding-top: 10px;/,
  );
  assert.match(css, /\.right-panel \.monitor-section,[\s\S]*border-top: 0;/);
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

test("cwd tree file opens stay on normal file preview routing", () => {
  const calls: string[] = [];

  openCwdTreeFilePreview({
    clearGitDiffPreview: () => calls.push("clear-diff"),
    openTreeFile: (path: string) => calls.push(`open:${path}`),
    path: "/repo/src/modified.ts",
  });

  assert.deepEqual(calls, ["clear-diff", "open:/repo/src/modified.ts"]);
});

test("cwd tree file routing does not keep changed-file diff status lookups", () => {
  const source = readFileSync(new URL("./RightPanel.tsx", import.meta.url), "utf8");

  assert.doesNotMatch(source, /readGitStatusSnapshot/);
  assert.doesNotMatch(source, /resolveGitTreeFileOpen/);
  assert.doesNotMatch(source, /gitDiffTargetForTreePath/);
  assert.doesNotMatch(source, /onGitSnapshotChange/);
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

  assert.match(css, /\.git-panel \{[\s\S]*--git-surface: rgba\(252, 251, 249, 0\.58\);[\s\S]*background: var\(--git-surface\);/);
  assert.match(css, /\.git-graph-section \{[\s\S]*background: var\(--git-surface\);/);
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
  assert.match(css, /\.git-graph-dot\.main \{[\s\S]*fill: #fbfaf8;[\s\S]*stroke-width: 3\.4;/);
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

  assert.doesNotMatch(markup, /aria-label="Show current file"/);
  assert.doesNotMatch(markup, /aria-label="Show file tree"/);
  assert.doesNotMatch(markup, /CWD Tree/);
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

test("shows the Browser open action only for loaded HTML previews", () => {
  const htmlPreview = makePreview({
    path: "/tmp/docs/share page.html",
    displayPath: "docs/share page.html",
    language: "html",
    content: "<main>Hello</main>",
  });
  const htmPreview = makePreview({
    path: "/tmp/docs/share.htm",
    displayPath: "docs/share.htm",
    language: "plaintext",
    content: "<main>Hello</main>",
  });
  const nonHtmlMarkup = renderRightPanel(makeThread([]), "preview", null, {
    preview: makePreview({
      path: "/tmp/docs/share.md",
      displayPath: "docs/share.md",
      language: "markdown",
      content: "# Share",
    }),
  });
  const loadingMarkup = renderRightPanel(makeThread([]), "preview", null, {
    preview: makePreview({
      path: "/tmp/docs/share.html",
      displayPath: "docs/share.html",
      language: "html",
    }),
    previewLoading: true,
  });
  const errorMarkup = renderRightPanel(makeThread([]), "preview", null, {
    preview: makePreview({
      path: "/tmp/docs/share.html",
      displayPath: "docs/share.html",
      language: "html",
    }),
    previewError: "Failed to load",
  });
  const treeMarkup = renderRightPanel(makeThread([]), "preview", null, {
    filePanelView: "tree",
    preview: makePreview({
      path: "/tmp/docs/share.html",
      displayPath: "docs/share.html",
      language: "html",
    }),
  });

  assert.equal(
    filePreviewOpenInBrowserActionVisible({
      filePanelView: "preview",
      preview: htmlPreview,
      previewError: null,
      previewLoading: false,
    }),
    true,
  );
  assert.equal(
    filePreviewOpenInBrowserActionVisible({
      filePanelView: "preview",
      preview: htmPreview,
      previewError: null,
      previewLoading: false,
    }),
    true,
  );
  assert.doesNotMatch(nonHtmlMarkup, /Open preview in Browser/);
  assert.match(nonHtmlMarkup, /aria-label="Open preview in system editor"/);
  assert.doesNotMatch(loadingMarkup, /Open preview in Browser/);
  assert.doesNotMatch(errorMarkup, /Open preview in Browser/);
  assert.doesNotMatch(treeMarkup, /Open preview in Browser/);
  assert.equal(
    filePreviewOpenInBrowserActionVisible({
      filePanelView: "preview",
      preview: null,
      previewError: null,
      previewLoading: false,
    }),
    false,
  );
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

  assert.doesNotMatch(markup, /aria-label="Show file tree"/);
  assert.doesNotMatch(markup, /CWD Tree/);
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
      onOpenPreviewInBrowser={() => {}}
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
