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

const SOURCE_PATHS = {
  agentTree: "./AgentTree.tsx",
  app: "../App.tsx",
  browserPanel: "./BrowserPanel.tsx",
  panels: "./Panels.tsx",
  rightPanel: "./RightPanel.tsx",
  styles: "../styles.css",
  terminalPanel: "./TerminalPanel.tsx",
} as const;

type SourceName = keyof typeof SOURCE_PATHS;
type SourceMap<T extends SourceName> = { [K in T as `${K}Source`]: string };

function readSource<T extends SourceName>(name: T): string {
  return readFileSync(new URL(SOURCE_PATHS[name], import.meta.url), "utf8");
}

function readSources<T extends SourceName>(names: readonly T[]): SourceMap<T> {
  return Object.fromEntries(
    names.map((name) => [`${name}Source`, readSource(name)]),
  ) as SourceMap<T>;
}

function sourceSlice(source: string, start: string, end: string): string {
  return source.slice(source.indexOf(start), source.indexOf(end));
}

function assertMatches(source: string, patterns: RegExp[]): void {
  for (const pattern of patterns) {
    assert.match(source, pattern, `Expected source to match ${pattern}`);
  }
}

function assertDoesNotMatchAny(source: string, patterns: RegExp[]): void {
  for (const pattern of patterns) {
    assert.doesNotMatch(
      source,
      pattern,
      `Expected source not to match ${pattern}`,
    );
  }
}

function cssBlock(
  stylesSource: string,
  selector: string,
  nextSelector: string,
): string {
  return sourceSlice(stylesSource, selector, nextSelector);
}

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
    Extract<
      Thread["turns"][number]["items"][number],
      { type: "workflowRunProgress" }
    >["event"]
  > = {},
): Extract<
  Thread["turns"][number]["items"][number],
  { type: "workflowRunProgress" }
> {
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
    fileTreeErrorsByPath?: Record<string, string>;
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
        fileTreeErrorsByPath={options?.fileTreeErrorsByPath ?? {}}
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
      fileTreeErrorsByPath={options?.fileTreeErrorsByPath ?? {}}
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

function renderPreviewPanel(options?: Parameters<typeof renderRightPanel>[3]) {
  return renderRightPanel(makeThread([]), "preview", null, options);
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

function makeTypescriptPreview(
  overrides: Partial<FilePreview> = {},
): FilePreview {
  return makePreview({
    path: "/tmp/src/App.tsx",
    displayPath: "src/App.tsx",
    content: "export const value = 1;",
    language: "typescript",
    ...overrides,
  });
}

function makeMarkdownPreview(
  overrides: Partial<FilePreview> = {},
): FilePreview {
  return makePreview({
    path: "/tmp/README.md",
    displayPath: "README.md",
    content: "# Title",
    language: "markdown",
    ...overrides,
  });
}

function makeImagePreview(overrides: Partial<FilePreview> = {}): FilePreview {
  return makePreview({
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
    ...overrides,
  });
}

function makePdfPreview(overrides: Partial<FilePreview> = {}): FilePreview {
  return makePreview({
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
    ...overrides,
  });
}

function previewHeaderControlsVisible(
  options: Partial<
    Parameters<typeof filePreviewHeaderEditControlsVisible>[0]
  > = {},
) {
  return filePreviewHeaderEditControlsVisible({
    filePanelView: "preview",
    preview: makeMarkdownPreview(),
    previewError: null,
    previewLoading: false,
    ...options,
  });
}

function previewOpenInBrowserVisible(
  options: Partial<
    Parameters<typeof filePreviewOpenInBrowserActionVisible>[0]
  > = {},
) {
  return filePreviewOpenInBrowserActionVisible({
    filePanelView: "preview",
    preview: null,
    previewError: null,
    previewLoading: false,
    ...options,
  });
}

function makeRuntimeRestartProgress(
  overrides: Partial<RuntimeRestartProgress> = {},
): RuntimeRestartProgress {
  return {
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

type GitDiffPreview = React.ComponentProps<typeof GitDiffPreviewPanel>["diff"];

function makeGitDiffPreview(
  overrides: Partial<GitDiffPreview> = {},
): GitDiffPreview {
  return {
    available: true,
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
    error: null,
    binary: false,
    ...overrides,
  };
}

function renderGitDiffPreview(diff: GitDiffPreview) {
  return renderToStaticMarkup(
    <GitDiffPreviewPanel diff={diff} error={null} loading={false} />,
  );
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

function makeScheduleAgendaGroup(index: number) {
  return makeScheduleAgendaGroups()[index]!;
}

function renderScheduleAgendaLayout(collapsed: boolean) {
  return renderToStaticMarkup(
    <ScheduleAgendaLayout
      groups={makeScheduleAgendaGroups()}
      collapsed={collapsed}
      collapsedDateKeys={new Set()}
      onToggleCollapsed={() => {}}
      onToggleDateKey={() => {}}
    />,
  );
}

function renderScheduleAgendaDateGroup(index: number, collapsed: boolean) {
  return renderToStaticMarkup(
    <ScheduleAgendaDateGroup
      group={makeScheduleAgendaGroup(index)}
      collapsed={collapsed}
      onToggle={() => {}}
    />,
  );
}

test("resolves markdown preview relative links from the current file directory", () => {
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget("/tmp/docs/README.md", "./other.md"),
    "/tmp/docs/other.md",
  );
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget(
      "/tmp/docs/guides/README.md",
      "../other.md",
    ),
    "/tmp/docs/other.md",
  );
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget(
      "C:\\repo\\docs\\README.markdown",
      ".\\other.md",
    ),
    "C:\\repo\\docs\\other.md",
  );
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget(
      "/tmp/docs/README.md",
      "/tmp/other.md",
    ),
    "/tmp/other.md",
  );
  assert.equal(
    resolveMarkdownPreviewLocalFileTarget(
      "/tmp/docs/README.md",
      "file:///tmp/other.md",
    ),
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

test("BrowserPanel workspace selection does not fall back after its tab closes", () => {
  const tabs = [
    {
      id: "browser-a",
      title: "A",
      url: "https://closed.example",
      loading: false,
      canGoBack: false,
      canGoForward: false,
      error: null,
    },
    {
      id: "browser-b",
      title: "B",
      url: "https://still-open.example",
      loading: false,
      canGoBack: true,
      canGoForward: false,
      error: null,
    },
  ];

  const selectedWorkspaceTab = resolveBrowserPanelTabSelection({
    tabs,
    activeTabId: "browser-b",
    activeBrowserTabId: "browser-a",
    isManagerVariant: false,
  });
  assert.equal(selectedWorkspaceTab.activeTab?.id, "browser-a");

  const closedWorkspaceTab = resolveBrowserPanelTabSelection({
    tabs: [tabs[1]],
    activeTabId: "browser-b",
    activeBrowserTabId: "browser-a",
    isManagerVariant: false,
  });
  assert.equal(closedWorkspaceTab.activeTab, null);
  assert.deepEqual(
    closedWorkspaceTab.renderedTabs.map((tab) => tab.id),
    ["browser-b"],
  );

  const blankWorkspaceBrowser = resolveBrowserPanelTabSelection({
    tabs: [tabs[1]],
    activeTabId: "browser-b",
    activeBrowserTabId: null,
    isManagerVariant: false,
  });
  assert.equal(blankWorkspaceBrowser.activeTab, null);
});

test("BrowserPanel workspace variant waits for a concrete tab without manager chrome", () => {
  const markup = renderToStaticMarkup(
    <BrowserPanel
      variant="workspace"
      active
      nativeOverlayActive={false}
      resizing={false}
      navigationRequest={null}
      activeBrowserTabId={null}
    />,
  );

  assert.match(markup, /Browser tab closed\./);
  assert.doesNotMatch(markup, /browser-toolbar-workspace/);
  assert.doesNotMatch(markup, /browser-tab-strip/);
  assert.doesNotMatch(markup, /aria-label="New browser tab"/);
  assert.doesNotMatch(markup, /browser-status-row/);
});

test("BrowserPanel workspace variant hides controls for a missing explicit tab", () => {
  const markup = renderToStaticMarkup(
    <BrowserPanel
      variant="workspace"
      active
      nativeOverlayActive={false}
      resizing={false}
      navigationRequest={null}
      activeBrowserTabId="closed-browser-tab"
    />,
  );

  assert.match(markup, /Browser tab closed\./);
  assert.doesNotMatch(markup, /browser-toolbar-workspace/);
  assert.doesNotMatch(markup, /aria-label="Workspace browser URL"/);
  assert.doesNotMatch(markup, /class="browser-go-button"/);
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
      getData: (type: string) =>
        type === WORKSPACE_OBJECT_DRAG_TYPE ? "browser" : "",
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

test("detached workspace objects are hidden from the right panel while workspace-owned", () => {
  const { appSource, browserPanelSource, terminalPanelSource } = readSources([
    "app",
    "browserPanel",
    "terminalPanel",
  ]);
  const detachedOwnershipSource = appSource.slice(
    appSource.indexOf("const detachedWorkspaceBrowserTabIds = useMemo"),
    appSource.indexOf(
      "return (",
      appSource.indexOf("const detachedWorkspaceBrowserTabIds = useMemo"),
    ),
  );

  assert.match(
    appSource,
    /const detachedWorkspaceBrowserTabIds = useMemo\([\s\S]*workspaceTabs[\s\S]*tab\.kind === "browser" && tab\.browserTabId[\s\S]*map\(\(tab\) => tab\.browserTabId as string\)[\s\S]*\.\.\.closingWorkspaceBrowserTabIds,[\s\S]*\.\.\.closedWorkspaceBrowserTabIds,[\s\S]*closedWorkspaceBrowserTabIds,[\s\S]*closingWorkspaceBrowserTabIds,[\s\S]*workspaceTabs,[\s\S]*\]\s*,\s*\);/,
  );
  assert.match(
    appSource,
    /const detachedWorkspaceTerminalTabIds = useMemo\([\s\S]*workspaceTabs[\s\S]*tab\.kind === "terminal" && tab\.terminalTabId[\s\S]*map\(\(tab\) => tab\.terminalTabId as string\)[\s\S]*\.\.\.closingWorkspaceTerminalTabIds[\s\S]*\[closingWorkspaceTerminalTabIds, workspaceTabs\]/,
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
    browserPanelSource,
    /resolveBrowserPanelTabSelection\(\{[\s\S]*detachedBrowserTabIds/,
  );
  assert.match(
    terminalPanelSource,
    /state\.tabs\.filter\(\(tab\) => !detachedTerminalTabIdSet\.has\(tab\.id\)\)/,
  );
  assert.match(browserPanelSource, /managerNativeViewBlocked/);
  assert.match(browserPanelSource, /browserSurfaceIdRef/);
  assert.match(browserPanelSource, /Browser content is open in workspace\./);
  assert.match(terminalPanelSource, /Terminal session is open in workspace\./);
});

test("workspace Browser and Terminal tabs can be explicitly returned to the right panel", () => {
  const {
    appSource,
    rightPanelSource,
    browserPanelSource,
    terminalPanelSource,
  } = readSources(["app", "rightPanel", "browserPanel", "terminalPanel"]);

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
  assert.match(
    appSource,
    /browserTabFocusRequest=\{rightPanelBrowserTabFocusRequest\}/,
  );
  assert.match(
    appSource,
    /terminalTabFocusRequest=\{rightPanelTerminalTabFocusRequest\}/,
  );
  assert.match(
    appSource,
    /setRightPanelBrowserTabFocusRequest\(\(current\) => \(\{[\s\S]*tabId: payload\.browserTabId,[\s\S]*token: \(current\?\.token \?\? 0\) \+ 1/,
  );
  assert.match(
    appSource,
    /setRightPanelTerminalTabFocusRequest\(\(current\) => \(\{[\s\S]*tabId: payload\.terminalTabId,[\s\S]*token: \(current\?\.token \?\? 0\) \+ 1/,
  );
  assert.match(
    appSource,
    /onReturnWorkspaceObject=\{handleReturnWorkspaceObjectToRightPanel\}/,
  );
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
  assert.match(
    browserPanelSource,
    /focusBrowserTabRequest\?: \{ tabId: string; token: number \} \| null/,
  );
  assert.match(
    browserPanelSource,
    /!renderedTabs\.some\(\(tab\) => tab\.id === focusBrowserTabRequest\.tabId\)[\s\S]*return;[\s\S]*focusBrowserTabRequest\.tabId === state\.activeTabId[\s\S]*lastBrowserTabFocusRequestTokenRef\.current = focusBrowserTabRequest\.token;[\s\S]*currentBrowserPanelApi\(\)[\s\S]*lastBrowserTabFocusRequestTokenRef\.current = focusBrowserTabRequest\.token;[\s\S]*selectBrowserTab\(focusBrowserTabRequest\.tabId\)/,
  );
  assert.match(
    terminalPanelSource,
    /focusTerminalTabRequest\?: \{ tabId: string; token: number \} \| null/,
  );
  assert.match(
    terminalPanelSource,
    /!visibleTabs\.some\(\(tab\) => tab\.id === focusTerminalTabRequest\.tabId\)[\s\S]*return;[\s\S]*focusTerminalTabRequest\.tabId === state\.activeTabId[\s\S]*lastTerminalTabFocusRequestTokenRef\.current =[\s\S]*focusTerminalTabRequest\.token;[\s\S]*lastTerminalTabFocusRequestTokenRef\.current = focusTerminalTabRequest\.token;[\s\S]*\.selectTerminalTab\(focusTerminalTabRequest\.tabId\)[\s\S]*requestTerminalViewportFocus\(focusTerminalTabRequest\.tabId\)/,
  );
});

test("workspace Browser and Terminal close buttons close owned instances without returning them right", () => {
  const { appSource } = readSources(["app"]);
  const closeWorkspaceTabSource = sourceSlice(
    appSource,
    "function closeWorkspaceTab(",
    "function handleWorkspaceObjectDragOver",
  );
  const closeButtonStart = appSource.indexOf('className="workspace-tab-close"');
  const closeButtonSource = appSource.slice(
    closeButtonStart,
    appSource.indexOf("</span>", closeButtonStart),
  );

  assert.match(
    closeWorkspaceTabSource,
    /options: \{[\s\S]*closeOwnedBrowserTab\?: boolean;[\s\S]*closeOwnedTerminalTab\?: boolean;[\s\S]*\} = \{\}/,
  );
  assert.match(
    closeWorkspaceTabSource,
    /options\.closeOwnedBrowserTab[\s\S]*rememberClosingWorkspaceBrowserTab\(closingTab\.browserTabId\)[\s\S]*suppressClosedWorkspaceBrowserTab\(closingTab\.browserTabId\)[\s\S]*\.closeBrowserTab\(closingTab\.browserTabId\)[\s\S]*isBrowserTabNotFoundError\(error\)[\s\S]*return;[\s\S]*releaseClosedWorkspaceBrowserTab\(closingTab\.browserTabId!\)[\s\S]*restoreWorkspaceTabAfterFailedClose\(closingTab\)[\s\S]*forgetClosingWorkspaceBrowserTab\(closingTab\.browserTabId!\)/,
  );
  assert.match(
    closeWorkspaceTabSource,
    /options\.closeOwnedTerminalTab[\s\S]*rememberClosingWorkspaceTerminalTab\(closingTab\.terminalTabId\)[\s\S]*\.closeTerminalTab\(closingTab\.terminalTabId\)[\s\S]*isTerminalTabNotFoundError\(error\)[\s\S]*restoreWorkspaceTabAfterFailedClose\(closingTab\)[\s\S]*forgetClosingWorkspaceTerminalTab\(closingTab\.terminalTabId!\)/,
  );
  assert.match(
    closeButtonSource,
    /closeWorkspaceTab\(tab\.id,[\s\S]*closeOwnedBrowserTab: true,[\s\S]*closeOwnedTerminalTab: true/,
  );
  assert.doesNotMatch(closeButtonSource, /setRightPanelView/);
  assert.doesNotMatch(closeButtonSource, /FocusRequest/);
});

test("workspace close-owned Browser remains suppressed until browser state confirms it is gone", () => {
  const { appSource } = readSources(["app"]);
  const pruneMissingWorkspaceBrowserTabsSource = sourceSlice(
    appSource,
    "function pruneMissingWorkspaceBrowserTabs(",
    "function closeWorkspaceTab(",
  );
  const closeWorkspaceTabSource = sourceSlice(
    appSource,
    "function closeWorkspaceTab(",
    "function handleWorkspaceObjectDragOver",
  );
  const detachedIdsSource = sourceSlice(
    appSource,
    "const detachedWorkspaceBrowserTabIds = useMemo",
    "const detachedWorkspaceTerminalTabIds = useMemo",
  );
  const returnSource = sourceSlice(
    appSource,
    "function handleReturnWorkspaceObjectToRightPanel",
    "function handleWorkspaceTabDragStart",
  );

  assert.match(
    appSource,
    /const \[closedWorkspaceBrowserTabIds, setClosedWorkspaceBrowserTabIds\] =\s*useState<string\[\]>\(\[\]\)/,
  );
  assert.match(
    appSource,
    /function suppressClosedWorkspaceBrowserTab\(tabId: string\)[\s\S]*current\.includes\(tabId\) \? current : \[\.\.\.current, tabId\]/,
  );
  assert.match(
    appSource,
    /function releaseClosedWorkspaceBrowserTab\(tabId: string\)[\s\S]*current\.filter\(\(id\) => id !== tabId\)/,
  );
  assert.match(
    pruneMissingWorkspaceBrowserTabsSource,
    /const liveBrowserTabIds = new Set\(browserTabIds\);[\s\S]*setClosedWorkspaceBrowserTabIds\(\(current\) =>[\s\S]*current\.filter\(\(id\) => liveBrowserTabIds\.has\(id\)\)/,
  );
  assert.match(
    closeWorkspaceTabSource,
    /rememberClosingWorkspaceBrowserTab\(closingTab\.browserTabId\);[\s\S]*suppressClosedWorkspaceBrowserTab\(closingTab\.browserTabId\);[\s\S]*\.closeBrowserTab\(closingTab\.browserTabId\)/,
  );
  assert.match(
    closeWorkspaceTabSource,
    /if \(isBrowserTabNotFoundError\(error\)\) \{[\s\S]*return;[\s\S]*\}[\s\S]*releaseClosedWorkspaceBrowserTab\(closingTab\.browserTabId!\);[\s\S]*restoreWorkspaceTabAfterFailedClose\(closingTab\)/,
  );
  assert.match(
    detachedIdsSource,
    /\.\.\.closingWorkspaceBrowserTabIds,[\s\S]*\.\.\.closedWorkspaceBrowserTabIds/,
  );
  assert.doesNotMatch(returnSource, /suppressClosedWorkspaceBrowserTab/);
  assert.doesNotMatch(returnSource, /closeOwnedBrowserTab/);
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
  const { appSource, browserPanelSource } = readSources([
    "app",
    "browserPanel",
  ]);

  assert.match(
    browserPanelSource,
    /const shouldHideNativeView =[\s\S]*!active[\s\S]*nativeOverlayActive[\s\S]*resizing[\s\S]*managerNativeViewBlocked/,
  );
  assert.match(
    browserPanelSource,
    /\}, \[[\s\S]*active,[\s\S]*activeTab\?\.id,[\s\S]*managerNativeViewBlocked,[\s\S]*nativeOverlayActive,[\s\S]*resizing,[\s\S]*\]\);/,
  );
  assert.match(
    browserPanelSource,
    /if \(shouldHideNativeView\) \{[\s\S]*\.hideBrowserView\(\{ surfaceId \}\)/,
  );
  assert.match(
    browserPanelSource,
    /else \{[\s\S]*const bounds = measureBounds\(\)[\s\S]*\.showBrowserView\(bounds\)/,
  );
  assert.match(
    browserPanelSource,
    /const passiveBoundsCorrectionRef = useRef<\(\(\) => void\) \| null>\(null\)/,
  );
  assert.match(
    browserPanelSource,
    /const applyBrowserState = \(nextState: BrowserPanelState\) => \{[\s\S]*passiveBoundsCorrectionRef\.current\?\.\(\);[\s\S]*\};/,
  );
  assert.match(
    browserPanelSource,
    /passiveBoundsCorrectionRef\.current = scheduleBoundsUpdate;[\s\S]*scheduleBoundsUpdate\(\);/,
  );
  assert.match(
    browserPanelSource,
    /const sendBounds = \(\) => \{[\s\S]*\.setBrowserViewBounds\(bounds\)/,
  );
  assert.match(
    browserPanelSource,
    /const showNativeBrowserView = async \(browserApi: BrowserPanelApi\) => \{[\s\S]*browserBoundsFromElement\([\s\S]*\.showBrowserView\(bounds\)/,
  );
  assert.match(
    browserPanelSource,
    /await showNativeBrowserView\(browserApi\);[\s\S]*await browserApi\.navigateBrowserView\(\{[\s\S]*target: normalized\.url,[\s\S]*surfaceId: browserSurfaceIdRef\.current,[\s\S]*tabId: activeTab\?\.id \?\? null/,
  );
  const showBranch = browserPanelSource.slice(
    browserPanelSource.indexOf(
      "} else {",
      browserPanelSource.indexOf("if (shouldHideNativeView)"),
    ),
    browserPanelSource.indexOf("scheduleBoundsUpdate();"),
  );
  assert.doesNotMatch(showBranch, /lastSentBounds = bounds/);
  assert.doesNotMatch(
    browserPanelSource,
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
  const browserPanelSource = readSource("browserPanel");
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
  assert.match(
    browserPanelSource,
    /<h2>\{panelChromeLabels\.headerTitle\}<\/h2>/,
  );
  assert.match(
    browserPanelSource,
    /<span className="browser-tab-title">[\s\S]*\{browserTabLabel\(tab\)\}[\s\S]*<\/span>/,
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

  assert.equal(
    shouldClearBrowserLocalError(state, state.tabs[0] ?? null),
    true,
  );
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

  assert.equal(
    shouldClearBrowserLocalError(state, state.tabs[0] ?? null),
    false,
  );
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
  const {
    appSource,
    rightPanelSource,
    browserPanelSource,
    terminalPanelSource,
    panelsSource,
    agentTreeSource,
    stylesSource,
  } = readSources([
    "app",
    "rightPanel",
    "browserPanel",
    "terminalPanel",
    "panels",
    "agentTree",
    "styles",
  ]);

  assertMatches(appSource, [
    /workspaceTabForThread/,
    /workspaceTabForFile/,
    /workspaceTabForBrowser/,
    /workspaceTabForTerminal/,
    /browserWorkspaceTabId\(tab\?\.browserTabId/,
    /terminalWorkspaceTabId\(tab\?\.terminalTabId/,
    /browserTabId: tab\?\.browserTabId/,
    /terminalTabId: tab\?\.terminalTabId/,
    /readWorkspaceObjectDragData\(event\.dataTransfer\)/,
    /aria-label="Workspace object tabs"/,
    /getWorkspaceTabThread/,
    /getRootThreadConversationTitle\(thread\)/,
    /getAgentRoleLabel\(thread\)/,
    /const title = isRootThread\(thread\)[\s\S]*\? getRootThreadConversationTitle\(thread\)[\s\S]*: getThreadPath\(thread\)/,
    /subtitle: isRootThread\(thread\)[\s\S]*\? getAgentRoleLabel\(thread\)[\s\S]*: getThreadPresenceLabel\(thread\)/,
    /getThreadPath\(thread\)/,
    /storedWorkspaceTabOrderRef/,
    /applyStoredWorkspaceTabOrder\([\s\S]*storedWorkspaceTabOrderRef\.current/,
    /storedWorkspaceTabOrderRef\.current = storeWorkspaceTabOrder/,
    /current\.flatMap\(\(tab\) =>/,
    /return thread \? \[workspaceTabForThread\(thread\)\] : \[\]/,
    /current\.map\(\(item\) => \(item\.id === tab\.id \? tab : item\)\)/,
    /function openConversationWorkspaceTab\(threadId: string\)/,
    /openConversationWorkspaceTab\(threadId\);[\s\S]*setSelectedThreadId\(threadId\);/,
    /const \[workspaceAddMenuOpen, setWorkspaceAddMenuOpen\] = useState\(false\)/,
    /workspaceAddButtonRef = useRef<HTMLButtonElement \| null>\(null\)/,
    /workspaceAddMenuRef = useRef<HTMLDivElement \| null>\(null\)/,
    /function closeWorkspaceAddMenuOnPointerDown\([\s\S]*workspaceAddButtonRef\.current\?\.contains\(target\)[\s\S]*workspaceAddMenuRef\.current\?\.contains\(target\)[\s\S]*setWorkspaceAddMenuOpen\(false\)/,
    /function closeWorkspaceAddMenuOnKeyDown\(event: KeyboardEvent\)[\s\S]*event\.key === "Escape"[\s\S]*setWorkspaceAddMenuOpen\(false\)/,
    /document\.addEventListener\([\s\S]*"pointerdown"[\s\S]*closeWorkspaceAddMenuOnPointerDown[\s\S]*true/,
    /document\.addEventListener\("keydown", closeWorkspaceAddMenuOnKeyDown\)/,
    /window\.addEventListener\("resize", closeWorkspaceAddMenuOnViewportChange\)/,
    /function toggleWorkspaceAddMenu\(\)[\s\S]*workspaceAddButtonRef\.current[\s\S]*getBoundingClientRect\(\)[\s\S]*setWorkspaceAddMenuPosition/,
    /function selectWorkspaceAddMenuItem\([\s\S]*kind: "terminal" \| "browser" \| "chat"[\s\S]*setWorkspaceAddMenuOpen\(false\)[\s\S]*openTerminalInWorkspace\(\)[\s\S]*openBrowserInWorkspace\(\)[\s\S]*void createBlankChatThread\(\)/,
    /aria-label="Add workspace tab"[\s\S]*aria-haspopup="menu"[\s\S]*aria-expanded=\{workspaceAddMenuOpen\}[\s\S]*onClick=\{toggleWorkspaceAddMenu\}/,
    /className="workspace-tab-list"[\s\S]*role="tablist"[\s\S]*aria-label="Workspace object tabs"/,
    /role="menu"[\s\S]*aria-label="Add workspace tab"/,
    /selectWorkspaceAddMenuItem\("terminal"\)[\s\S]*Terminal/,
    /selectWorkspaceAddMenuItem\("browser"\)[\s\S]*Browser/,
    /disabled=\{isCreatingChatThread\}[\s\S]*selectWorkspaceAddMenuItem\("chat"\)[\s\S]*Chat/,
    /useEffect\(\(\) => \{\s*if \(selectedThreadId\)/,
    /const selectedThreadWorkspaceTabId = selectedThread[\s\S]*conversationWorkspaceTabId\(selectedThread\.id\)/,
    /return resolveActiveWorkspaceTabId\([\s\S]*workspaceTabs,[\s\S]*current,[\s\S]*selectedThreadWorkspaceTabId/,
    /const visibleWorkspaceTabId = resolveActiveWorkspaceTabId\([\s\S]*workspaceTabs,[\s\S]*activeWorkspaceTabId,[\s\S]*selectedThreadWorkspaceTabId/,
    /workspaceTabs\.find\(\(tab\) => tab\.id === visibleWorkspaceTabId\)/,
    /const active = tab\.id === visibleWorkspaceTabId;/,
    /threads\.some\(\(thread\) => thread\.id === tab\.threadId\)/,
    /const tabThread = getWorkspaceTabThread\(tab, threads\)/,
    /workspace-tab-dot \$\{threadDisplayStatusClass\(tabThread\)\}/,
    /const currentVisibleWorkspaceTabId = resolveActiveWorkspaceTabId\([\s\S]*currentTabs,[\s\S]*activeWorkspaceTabId,[\s\S]*selectedThreadWorkspaceTabId/,
    /closeWorkspaceTabById\(currentTabs, tabId\)/,
    /function activateWorkspaceFallbackTab\(tab: WorkspaceObjectTab \| null\) \{[\s\S]*setActiveWorkspaceTabId\(tab\?\.id \?\? null\)[\s\S]*tab\?\.kind === "conversation"[\s\S]*selectThread\(tab\.threadId\)[\s\S]*tab\?\.kind === "terminal"[\s\S]*setTerminalPanelFocusRequestToken[\s\S]*tab\?\.kind === "file"[\s\S]*loadFilePreview\(tab\.path, \{ preserveRightPanel: true \}\)/,
    /if \(currentVisibleWorkspaceTabId !== tabId\) \{[\s\S]*return;[\s\S]*\}[\s\S]*const fallback =[\s\S]*next\[closingIndex\][\s\S]*activateWorkspaceFallbackTab\(fallback\)/,
    /function pruneMissingWorkspaceBrowserTabs\(browserTabIds: string\[\]\)[\s\S]*tab\.kind === "browser"[\s\S]*tab\.browserTabId[\s\S]*!liveBrowserTabIds\.has\(tab\.browserTabId\)[\s\S]*setWorkspaceTabs\(next\)[\s\S]*activateWorkspaceFallbackTab/,
    /function bindWorkspaceBrowserTab\([\s\S]*setActiveWorkspaceTabId\(\(current\) =>[\s\S]*current === workspaceTabId \? nextTab\.id : current/,
    /function bindWorkspaceTerminalTab\([\s\S]*setActiveWorkspaceTabId\(\(current\) =>[\s\S]*current === workspaceTabId \? nextTab\.id : current/,
    /function isBrowserTabNotFoundError\(error: unknown\)[\s\S]*browser tab not found/,
    /event\.stopPropagation\(\);[\s\S]*closeWorkspaceTab\(tab\.id,[\s\S]*closeOwnedBrowserTab: true/,
    /async function openFilePathInWorkspace\(target: string\)/,
    /return \{ preview, rootId: requestRootId \}/,
    /const result = await loadFilePreview\(target, \{ preserveRightPanel: true \}\)/,
    /workspaceTabForFile\(result\.preview, result\.rootId\)/,
    /title: displayPath\.split\("\/"\)\.filter\(Boolean\)\.at\(-1\) \?\? displayPath/,
    /kind: "terminal",[\s\S]*title: "Terminal"/,
    /filePanelView="preview"/,
    /\[target\]: payload\.entries/,
    /\[payload\.path\]: payload\.entries/,
    /setExpandedTreeDirectories\(\[\]\);[\s\S]*if \(rightPanelView === "preview" && selectedThread\?\.cwd\) \{[\s\S]*loadFileTreeDirectory\(selectedThread\.cwd\)/,
    /const previousRightPanelViewRef = useRef<RightPanelView>\(rightPanelView\)/,
    /useEffect\(\(\) => \{[\s\S]*const previousRightPanelView = previousRightPanelViewRef\.current;[\s\S]*previousRightPanelViewRef\.current = rightPanelView;[\s\S]*previousRightPanelView !== "preview"[\s\S]*rightPanelView === "preview"[\s\S]*ensureFileTreeDirectoryLoaded\(selectedThread\.cwd\)/,
    /fileTreeEntriesByPath\[target\] \|\| fileTreeLoadingPath === target/,
    /function handleOpenTreeFile\(target: string\) \{[\s\S]*openFilePathInWorkspace\(target\)/,
    /if \(rightPanelView === "preview"\) \{[\s\S]*setRightPanelView\("skills"\)/,
    /activeWorkspaceTab\?\.kind !== "file"/,
    /activeWorkspaceTab\?\.kind !== "browser"/,
    /activeWorkspaceTab\?\.kind !== "terminal"/,
    /const activeTerminalThread =/,
    /thread=\{activeTerminalThread\}/,
    /activeWorkspaceTab\?\.kind === "browser"[\s\S]*<BrowserPanel[\s\S]*variant="workspace"[\s\S]*activeBrowserTabId=\{activeWorkspaceTab\.browserTabId \?\? null\}/,
    /activeWorkspaceTab\?\.kind === "browser"[\s\S]*navigationRequest=\{null\}/,
    /activeWorkspaceTab\?\.kind === "browser"[\s\S]*onBrowserTabIdsChange=\{pruneMissingWorkspaceBrowserTabs\}/,
    /activeWorkspaceTab\?\.kind === "browser"[\s\S]*onWorkspaceBrowserTabBound=\{\(tab\) =>[\s\S]*bindWorkspaceBrowserTab\(activeWorkspaceTab\.id, tab\)/,
    /activeWorkspaceTab\?\.kind === "terminal"[\s\S]*<TerminalPanel[\s\S]*variant="workspace"[\s\S]*activeTerminalTabId=\{activeWorkspaceTab\.terminalTabId \?\? null\}/,
    /activeWorkspaceTab\?\.kind === "terminal"[\s\S]*onWorkspaceTerminalTabBound=\{\(tab\) =>[\s\S]*bindWorkspaceTerminalTab\(activeWorkspaceTab\.id, tab\)/,
    /function gitDiffWorkspaceTabId\(targetId: string\)/,
    /function workspaceTabForGitDiff\([\s\S]*state: GitDiffPreviewState,[\s\S]*\): WorkspaceObjectTab \| null/,
    /kind: "diff"/,
    /gitDiffWorkspaceStateById/,
    /onGitDiffPreviewChange=\{handleGitDiffPreviewChange\}/,
    /state\.loading \|\|[\s\S]*workspaceTabsRef\.current\.some\(\(item\) => item\.id === tab\.id\)[\s\S]*upsertWorkspaceObjectTab\(tab, \{ activate: state\.loading \}\)/,
    /activeWorkspaceTab\?\.kind === "diff"[\s\S]*<GitDiffPreviewPanel[\s\S]*diff=\{activeWorkspaceDiffState\.diff\}/,
    /activeWorkspaceTab\?\.kind !== "file"[\s\S]*<FilePreviewPanel[\s\S]*variant="workspace"[\s\S]*gitDiffPreview=\{null\}/,
  ]);
  assertDoesNotMatchAny(appSource, [
    /storeWorkspaceTabOrder\(workspaceTabs/,
    /setActiveWorkspaceTabId\(tab\.id\);\s*\}, \[selectedThread\]\)/,
    /workspace-tab-dot \$\{tab\.kind\}/,
    /workspaceTabForFile\(preview, selectedTreeRootIdRef\.current\)/,
    /<span className="workspace-tab-subtitle">/,
    /filePanelViewRef/,
    /filePanelViewRef\.current === "tree"/,
    /function handleSetRightPanelView\(view: RightPanelView\) \{[\s\S]*ensureFileTreeDirectoryLoaded\(selectedThread\.cwd\)[\s\S]*if \(activeWorkspaceTab\?\.kind === "browser"/,
    /document\.addEventListener\([\s\S]*"scroll"[\s\S]*closeWorkspaceAddMenuOnViewportChange/,
  ]);
  const workspaceBrowserPanelSource = sourceSlice(
    appSource,
    'activeWorkspaceTab?.kind === "browser"',
    'activeWorkspaceTab?.kind === "terminal"',
  );
  assertDoesNotMatchAny(workspaceBrowserPanelSource, [
    /rightPanelView/,
    /effectiveActiveView/,
  ]);
  assertDoesNotMatchAny(appSource, [
    /function openBrowserInWorkspace[\s\S]*setRightPanelView\("skills"\)/,
    /function openTerminalInWorkspace[\s\S]*setRightPanelView\("skills"\)/,
    /setRightPanelViewWithWorkspaceFallback/,
    /browserNativeViewSuppressed/,
  ]);
  assertMatches(appSource, [
    /const detachedWorkspaceBrowserTabIds = useMemo\([\s\S]*workspaceTabs[\s\S]*tab\.kind === "browser" && tab\.browserTabId[\s\S]*map\(\(tab\) => tab\.browserTabId as string\)[\s\S]*\.\.\.closingWorkspaceBrowserTabIds,[\s\S]*\.\.\.closedWorkspaceBrowserTabIds,[\s\S]*closedWorkspaceBrowserTabIds,[\s\S]*closingWorkspaceBrowserTabIds,[\s\S]*workspaceTabs,[\s\S]*\]\s*,\s*\);/,
    /const detachedWorkspaceTerminalTabIds = useMemo\([\s\S]*workspaceTabs[\s\S]*tab\.kind === "terminal" && tab\.terminalTabId[\s\S]*map\(\(tab\) => tab\.terminalTabId as string\)[\s\S]*\.\.\.closingWorkspaceTerminalTabIds[\s\S]*\[closingWorkspaceTerminalTabIds, workspaceTabs\]/,
    /detachedBrowserTabIds=\{detachedWorkspaceBrowserTabIds\}/,
    /onBrowserTabIdsChange=\{pruneMissingWorkspaceBrowserTabs\}/,
    /detachedTerminalTabIds=\{detachedWorkspaceTerminalTabIds\}/,
    /onReturnWorkspaceObject=\{handleReturnWorkspaceObjectToRightPanel\}/,
    /browserTabFocusRequest=\{rightPanelBrowserTabFocusRequest\}/,
    /onBrowserTabIdsChange=\{pruneMissingWorkspaceBrowserTabs\}/,
    /terminalTabFocusRequest=\{rightPanelTerminalTabFocusRequest\}/,
    /function handleOpenArtifactUrl\(url: string\)[\s\S]*setRightPanelView\("browser"\)/,
    /function closeWorkspaceTab\([\s\S]*closeOwnedBrowserTab\?: boolean;[\s\S]*closeOwnedTerminalTab\?: boolean;[\s\S]*options\.closeOwnedBrowserTab[\s\S]*closingTab\?\.kind === "browser"[\s\S]*window\.codexDesktop[\s\S]*\.closeBrowserTab\(closingTab\.browserTabId\)/,
    /function closeWorkspaceTab\([\s\S]*options\.closeOwnedTerminalTab[\s\S]*closingTab\?\.kind === "terminal"[\s\S]*window\.codexDesktop[\s\S]*\.closeTerminalTab\(closingTab\.terminalTabId\)/,
    /\.catch\(\(error\) => \{[\s\S]*isBrowserTabNotFoundError\(error\)[\s\S]*return;[\s\S]*setError\(toErrorMessage\(error\)\)/,
    /function closeWorkspaceTab\([\s\S]*const currentVisibleWorkspaceTabId = resolveActiveWorkspaceTabId\([\s\S]*currentTabs,[\s\S]*activeWorkspaceTabId,[\s\S]*selectedThreadWorkspaceTabId,[\s\S]*\);[\s\S]*if \(currentVisibleWorkspaceTabId !== tabId\)[\s\S]*const fallback =[\s\S]*activateWorkspaceFallbackTab\(fallback\)/,
    /className="workspace-tab-close"[\s\S]*onClick=\{\(event\) => \{[\s\S]*closeWorkspaceTab\(tab\.id,[\s\S]*closeOwnedBrowserTab: true,[\s\S]*closeOwnedTerminalTab: true[\s\S]*onKeyDown=\{\(event\) => \{[\s\S]*closeWorkspaceTab\(tab\.id,[\s\S]*closeOwnedBrowserTab: true,[\s\S]*closeOwnedTerminalTab: true/,
    /function handleReturnWorkspaceObjectToRightPanel\([\s\S]*payload\.kind === "browser"[\s\S]*closeWorkspaceTab\(tab\.id\);[\s\S]*setRightPanelView\("browser"\)[\s\S]*setRightPanelBrowserTabFocusRequest/,
  ]);
  assert.doesNotMatch(
    appSource.slice(
      appSource.indexOf("function handleReturnWorkspaceObjectToRightPanel"),
      appSource.indexOf("function handleSetRightPanelView"),
    ),
    /closeOwnedBrowserTab/,
  );
  assert.doesNotMatch(
    appSource.slice(
      appSource.indexOf("function handleReturnWorkspaceObjectToRightPanel"),
      appSource.indexOf("function handleSetRightPanelView"),
    ),
    /closeOwnedTerminalTab/,
  );
  assertMatches(appSource, [
    /function handleThreadAnalysisCommandFocus[\s\S]*setRightPanelView\("terminal"\)/,
    /onOpenWorkspaceObject=\{openRightPanelObjectInWorkspace\}/,
    /function lazyRightPanelComponent\(exportName: LazyRightPanelExport\)/,
    /await import\("\.\/components\/RightPanel"\)/,
    /const RightPanel = lazyRightPanelComponent\("RightPanel"\)/,
    /<Suspense[\s\S]*Loading panel\.\.\.[\s\S]*<RightPanel/,
  ]);
  assert.deepEqual(
    appSource.match(
      /^import\s+\{[^}]*\}\s+from "\.\/components\/RightPanel";/gm,
    ),
    ['import { type GitDiffPreviewState } from "./components/RightPanel";'],
  );
  assert.doesNotMatch(appSource, /^import "\.\/components\/RightPanel";/m);
  assertMatches(appSource, [
    /hasWorkspaceObjectDragData\(event\.dataTransfer\)/,
    /const PANEL_RESIZER_WIDTH = 4/,
    /revealThreadInSidebarState\(\{/,
    /touchedProjectCollapseIdsRef\.current\.add\(next\.expandedProjectId\)/,
    /const conversationCells = useMemo\(\(\) => \{[\s\S]*buildConversationState\([\s\S]*selectedThread[\s\S]*filterConversationCellsForDisplay\(nextConversationState\.cells\);[\s\S]*\}, \[selectedThread\]\);/,
  ]);
  assertMatches(rightPanelSource, [
    /type WorkspaceOpenableRightPanelObject/,
    /draggable=\{workspaceObjectKindForView\(item\.view\) != null\}/,
    /writeWorkspaceObjectDragData\(event\.dataTransfer, kind\)/,
    /onOpenWorkspaceObject\?\.\(kind\)/,
    /hasWorkspaceObjectDragData\(event\.dataTransfer\)/,
    /detachedBrowserTabIds\?: string\[\]/,
    /onBrowserTabIdsChange\?: \(tabIds: string\[\]\) => void/,
    /detachedTerminalTabIds\?: string\[\]/,
    /onReturnWorkspaceObject\?: \(payload: WorkspaceObjectDragPayload\) => void/,
    /browserTabFocusRequest\?: \{ tabId: string; token: number \} \| null/,
    /terminalTabFocusRequest\?: \{ tabId: string; token: number \} \| null/,
    /detachedBrowserTabIds=\{detachedBrowserTabIds\}/,
    /onBrowserTabIdsChange=\{onBrowserTabIdsChange\}/,
    /detachedTerminalTabIds=\{detachedTerminalTabIds\}/,
  ]);
  assertDoesNotMatchAny(rightPanelSource, [
    /panel-eyebrow/,
    /preview-mode-toggle/,
    /Context mix/,
    /aria-label="Show current file"/,
    /aria-label="Show file tree"/,
    /file-object-toolbar/,
    /browserNativeViewSuppressed\?: boolean/,
    /suppressNativeView/,
  ]);
  assertMatches(browserPanelSource, [
    /browserTabDragPayload/,
    /browserTabId: tab\.id/,
    /onOpenBrowserTabInWorkspace/,
    /onBrowserTabIdsChange\?: \(tabIds: string\[\]\) => void/,
    /onWorkspaceBrowserTabBound\?: \(tab: BrowserWorkspaceTabDescriptor\) => void/,
    /activeBrowserTabId\?: string \| null/,
    /variant = "manager"/,
    /variant\?: "manager" \| "workspace"/,
  ]);
  assert.match(terminalPanelSource, /activeTerminalTabId\?: string \| null/);
  assert.match(
    terminalPanelSource,
    /onWorkspaceTerminalTabBound\?: \([\s\S]*WorkspaceObjectDragPayload[\s\S]*\) => void/,
  );
  assertMatches(terminalPanelSource, [
    /const \[terminalStateLoaded, setTerminalStateLoaded\] = useState\(false\)/,
    /setTerminalStateLoaded\(false\)[\s\S]*getTerminalState\(thread\?\.id \?\? null\)[\s\S]*setTerminalStateLoaded\(true\)/,
    /useEffect\(\(\) => \{[\s\S]*isManagerVariant[\s\S]*!terminalStateLoaded[\s\S]*activeTerminalTabId != null[\s\S]*workspaceTerminalCreatePendingRef\.current[\s\S]*return;[\s\S]*createTerminal\(\{ bindWorkspaceTab: true \}\);/,
  ]);
  const browserSelectionSource = sourceSlice(
    browserPanelSource,
    "export function resolveBrowserPanelTabSelection",
    "export function BrowserPanel",
  );
  const selectForSurfaceSource = browserPanelSource.slice(
    browserPanelSource.indexOf("const selectBrowserTabForSurfaceIfNeeded"),
    browserPanelSource.indexOf(
      "useEffect(() => {",
      browserPanelSource.indexOf("const selectBrowserTabForSurfaceIfNeeded"),
    ),
  );
  const navigationRequestSource = browserPanelSource.slice(
    browserPanelSource.indexOf("if (!navigationRequest)"),
    browserPanelSource.indexOf(
      "}, [",
      browserPanelSource.indexOf("if (!navigationRequest)"),
    ),
  );
  const focusBrowserRequestSource = browserPanelSource.slice(
    browserPanelSource.lastIndexOf(
      "useEffect(() => {",
      browserPanelSource.indexOf("!focusBrowserTabRequest"),
    ),
    browserPanelSource.indexOf(
      "}, [",
      browserPanelSource.indexOf("!focusBrowserTabRequest"),
    ),
  );
  const navigateSource = sourceSlice(
    browserPanelSource,
    "const navigate = () => {",
    "const runCommand = (",
  );
  const runCommandSource = sourceSlice(
    browserPanelSource,
    "const runCommand = (",
    "const createTab = () => {",
  );
  const createTabSource = sourceSlice(
    browserPanelSource,
    "const createTab = () => {",
    "const selectTab = (",
  );
  const managerNewTabButtonSource = sourceSlice(
    browserPanelSource,
    "browser-new-tab-button",
    "{activeTab ? (",
  );
  const surfaceApplySource = sourceSlice(
    browserPanelSource,
    "const applyBrowserState =",
    "const showNativeBrowserView =",
  );
  const managerChromeSource = sourceSlice(
    browserPanelSource,
    "{isManagerVariant ? (",
    "{!isManagerVariant && !workspaceSelectionMissing ? (",
  );
  assertMatches(browserSelectionSource, [
    /const detachedBrowserTabIdSet = new Set\(detachedBrowserTabIds\)/,
    /tabs\.filter\(\(tab\) => !detachedBrowserTabIdSet\.has\(tab\.id\)\)/,
    /managerSelectedBrowserTabId \?\? activeTabId/,
    /const hasExplicitWorkspaceSelection = !isManagerVariant/,
    /hasExplicitWorkspaceSelection \? null : \(renderedTabs\[0\] \?\? null\)/,
  ]);
  assertMatches(browserPanelSource, [
    /function resolveBrowserPanelTabSelection/,
    /resolveBrowserPanelTabSelection\(\{/,
    /activeTabId: state\.activeTabId/,
    /managerSelectedBrowserTabId/,
    /const tabs = useMemo\(/,
    /const \{[\s\S]*renderedTabs,[\s\S]*activeTab,[\s\S]*managerHasDetachedTabs,[\s\S]*managerActiveTabDetached,[\s\S]*\} = useMemo\(/,
    /const browserTabDragPayload = useCallback\(/,
    /const managerNativeViewBlocked =\s*managerActiveTabDetached \|\|/,
    /managerHasDetachedTabs && activeTab == null/,
    /if \(isManagerVariant && managerHasDetachedTabs && !activeTab\) \{[\s\S]*return;[\s\S]*\}[\s\S]*const browserApi = currentBrowserPanelApi\(\);/,
    /const shouldHideNativeView =[\s\S]*managerNativeViewBlocked/,
    /const shouldHideNativeView =[\s\S]*!activeTab[\s\S]*managerNativeViewBlocked/,
    /const workspaceSelectionMissing = !isManagerVariant && activeTab == null/,
    /workspaceBrowserTabCreatePendingRef\.current/,
    /createBrowserTab\(\{ activate: true \}\)[\s\S]*onWorkspaceBrowserTabBound\?\.\(\{[\s\S]*browserTabId: createdTab\.id/,
    /if \(workspaceSelectionMissing\) \{[\s\S]*return;[\s\S]*\}[\s\S]*const normalized = normalizeBrowserUrl\(address\)/,
    /if \(workspaceSelectionMissing\) \{[\s\S]*return;[\s\S]*\}[\s\S]*const browserApi = currentBrowserPanelApi\(\)/,
    /function nextBrowserPanelSurfaceId\(\)/,
    /const browserSurfaceIdRef = useRef\(nextBrowserPanelSurfaceId\(\)\)/,
    /hideBrowserView\(\{ surfaceId \}\)/,
    /Browser tab closed\./,
    /Browser content is open in workspace\./,
    /const browserSurfaceRef = useRef\(\{/,
    /detachedBrowserTabIds/,
    /const onBrowserTabIdsChangeRef = useRef\(onBrowserTabIdsChange\)/,
    /onBrowserTabIdsChangeRef\.current = onBrowserTabIdsChange/,
    /onBrowserTabIdsChangeRef\.current\?\.[\s\S]*normalizedState\.tabs\.map\(\(tab\) => tab\.id\)/,
    /const addressInputFocusedRef = useRef\(false\)/,
    /const lastAddressTabIdRef = useRef<string \| null>\(null\)/,
    /const syncAddressFromTab = \(/,
    /addressInputFocusedRef\.current && !tabChanged/,
    /const handleAddressInputBlur = \([\s\S]*event: React\.FocusEvent<HTMLInputElement>,[\s\S]*\) => \{/,
    /event\.currentTarget\.form\?\.contains\(nextFocusedElement\)[\s\S]*return;[\s\S]*syncAddressFromTab\(activeTab\)/,
    /onFocus=\{\(\) => \{[\s\S]*addressInputFocusedRef\.current = true/,
    /aria-label="Browser URL"[\s\S]*onBlur=\{handleAddressInputBlur\}/,
    /aria-label="Workspace browser URL"[\s\S]*onBlur=\{handleAddressInputBlur\}/,
    /isManagerVariant \? "browser-panel-manager" : "browser-panel-workspace"/,
    /!isManagerVariant && !workspaceSelectionMissing \? \([\s\S]*browser-toolbar browser-toolbar-workspace[\s\S]*aria-label="Workspace browser URL"[\s\S]*\) : null/,
    /<div ref=\{viewportRef\} className="browser-native-viewport">/,
  ]);
  assertDoesNotMatchAny(rightPanelSource, [/nativeViewSuppressed/]);
  assertDoesNotMatchAny(browserPanelSource, [
    /nativeViewSuppressed/,
    /runCommand\([\s\S]*selectBrowserTab\(activeBrowserTabId\)/,
  ]);
  assertMatches(selectForSurfaceSource, [
    /if \(!isManagerVariant\) \{[\s\S]*return;[\s\S]*\}/,
    /selectBrowserTab\(targetTabId\)/,
  ]);
  assertDoesNotMatchAny(selectForSurfaceSource, [/activeBrowserTabId/]);
  assertDoesNotMatchAny(focusBrowserRequestSource, [/nativeViewSuppressed/]);
  assertDoesNotMatchAny(navigationRequestSource, [/nativeViewSuppressed/]);
  assertMatches(navigationRequestSource, [
    /onNavigationRequestHandled\?\.\(navigationRequest\.token\)/,
    /if \(workspaceSelectionMissing\) \{[\s\S]*return;[\s\S]*\}[\s\S]*const normalized = normalizeBrowserUrl\(navigationRequest\.url\)/,
  ]);
  assertDoesNotMatchAny(navigateSource, [/nativeViewSuppressed/]);
  assertDoesNotMatchAny(runCommandSource, [/nativeViewSuppressed/]);
  assert.match(createTabSource, /createBrowserTab\(\)/);
  assert.match(managerNewTabButtonSource, /disabled=\{!hasBrowserApi\}/);
  assertMatches(surfaceApplySource, [
    /const surfaceDetachedTabIds = new Set\(surface\.detachedBrowserTabIds\)/,
    /surface\.managerSelectedBrowserTabId/,
    /surface\.activeBrowserTabId[\s\S]*\?\s*\(normalizedState\.tabs\.find\([\s\S]*surface\.activeBrowserTabId[\s\S]*\) \?\? null\)/,
    /!surfaceDetachedTabIds\.has\(normalizedActiveTab\.id\)/,
    /syncAddressFromTab\(surfaceActiveTab\)/,
  ]);
  assertMatches(navigateSource, [
    /await selectBrowserTabForSurfaceIfNeeded\(browserApi\)/,
    /await showNativeBrowserView\(browserApi\)/,
    /navigateBrowserView\(\{[\s\S]*target: normalized\.url,[\s\S]*surfaceId: browserSurfaceIdRef\.current,[\s\S]*tabId: activeTab\?\.id \?\? null/,
  ]);
  assertMatches(managerChromeSource, [
    /browser-tab-strip/,
    /\{activeTab \? \(/,
    /browser-toolbar/,
    /browser-status-row/,
  ]);
  assertMatches(rightPanelSource, [
    /const fileSourcePanelView: FilePanelView =[\s\S]*\? "preview"[\s\S]*: "tree"/,
    /!workspaceTabsEnabled &&[\s\S]*gitDiffPreview\.loading/,
    /gitDiffRequestScopeByTargetRef/,
    /function clearGitDiffPreview\(\) \{[\s\S]*if \(!workspaceTabsEnabled\) \{[\s\S]*gitDiffRequestScopeByTargetRef\.current\.clear\(\)/,
    /function beginGitDiffRequest\([\s\S]*targetId: string/,
    /function isCurrentGitDiffRequest\(targetId: string, scope: number\)/,
    /const targetId = `worktree:\$\{thread\.cwd\}:\$\{mode\}:\$\{change\.originalPath \?\? ""\}:\$\{change\.path\}`/,
    /const targetId = `commit:\$\{thread\.cwd\}:\$\{commit\.hash\}:\$\{file\.originalPath \?\? ""\}:\$\{file\.path\}`/,
    /beginGitDiffRequest\(targetId, \{[\s\S]*exclusive: !workspaceTabsEnabled,[\s\S]*\}\)/,
    /isCurrentGitDiffRequest\(targetId, scope\)/,
    /if \(!workspaceTabsEnabled\) \{[\s\S]*onSetActiveView\("preview"\)/,
    /gitDiffPreview=\{[\s\S]*workspaceTabsEnabled \? null : gitDiffPreview\.diff[\s\S]*\}/,
    /filePanelView=\{fileSourcePanelView\}/,
    /variant = "manager"/,
    /variant\?: "manager" \| "workspace"/,
    /preview-workspace-status-bar/,
    /!isWorkspaceVariant \? \([\s\S]*panel-content-header preview-header/,
  ]);
  assertMatches(panelsSource, [
    /data-thread-id/,
    /className="chat-list-row"[\s\S]*data-thread-id=\{node\.threadId\}/,
    /scrollIntoView\(\{ block: "nearest" \}\)/,
  ]);
  assert.match(agentTreeSource, /data-thread-id=\{node\.threadId\}/);
  assert.doesNotMatch(appSource, /WORKSPACE_TAB_LABELS/);
  assertMatches(appSource, [
    /<FilePreviewPanel/,
    /<BrowserPanel/,
    /<TerminalPanel/,
    /gridTemplateColumns: `\$\{sidebarWidth\}px \$\{PANEL_RESIZER_WIDTH\}px minmax\(0, 1fr\) \$\{PANEL_RESIZER_WIDTH\}px/,
  ]);
  assertMatches(stylesSource, [
    /\.workspace-tab-panel > \.conversation-panel/,
    /width: 100%;/,
    /\.workspace-tab-panel > \.conversation-panel,[\s\S]*\.workspace-tab-panel > \.browser-panel \{[\s\S]*border-top: 0;[\s\S]*box-shadow: none;[\s\S]*background-image: none;/,
    /\.workspace-tab-strip \{[\s\S]*gap: 6px;[\s\S]*border-bottom: 0;[\s\S]*box-shadow: none;/,
    /\.workspace-tab-list \{[\s\S]*max-width: calc\(100% - 34px\);[\s\S]*flex: 0 1 auto;[\s\S]*overflow-x: auto;/,
    /\.workspace-tab \{[\s\S]*border: 0;[\s\S]*border-radius: 999px;[\s\S]*background: rgba\(28, 25, 23, 0\.045\);/,
    /\.workspace-tab\.active \{[\s\S]*background: rgba\(15, 118, 110, 0\.12\);[\s\S]*box-shadow: inset 0 0 0 1px/,
    /\.workspace-tab-add-button \{[\s\S]*width: 28px;[\s\S]*border-radius: 999px;[\s\S]*background: rgba\(28, 25, 23, 0\.045\);/,
    /\.workspace-tab-add-button:hover,[\s\S]*\.workspace-tab-add-button\.active \{[\s\S]*background: rgba\(15, 118, 110, 0\.12\);/,
    /\.workspace-tab-add-menu \{[\s\S]*position: fixed;[\s\S]*width: 224px;[\s\S]*border-radius: 10px;/,
    /\.workspace-tab-add-menu-item \{[\s\S]*min-height: 42px;[\s\S]*border-radius: 8px;/,
    /\.workspace-tab-add-menu-label \{[\s\S]*font-size: 12px;[\s\S]*font-weight: 700;/,
    /\.browser-panel-workspace \{[\s\S]*background: #ffffff;/,
    /\.browser-toolbar-workspace \{[\s\S]*border-bottom: 0;[\s\S]*box-shadow: none;/,
    /\.browser-panel-workspace \.browser-native-viewport \{[\s\S]*min-height: 0;/,
    /\.terminal-panel-workspace \{[\s\S]*background: #f5f3f0;/,
    /\.terminal-panel-workspace \.terminal-viewport-shell \{[\s\S]*border-top: 0;[\s\S]*box-shadow: none;/,
    /\.preview-workspace-status-bar \{[\s\S]*min-height: 34px;/,
    /\.preview-panel-workspace > \.preview-editor-shell > \.preview-utility-strip,[\s\S]*display: none;/,
    /\.git-panel \{[\s\S]*--git-surface: rgba\(252, 251, 249, 0\.58\);[\s\S]*background: var\(--git-surface\);/,
    /\.git-graph-section \{[\s\S]*background: var\(--git-surface\);/,
    /\.git-section-header \{[\s\S]*background: var\(--git-surface-raised\);/,
    /\.git-graph-row:hover \{[\s\S]*background: var\(--git-surface-hover\);/,
    /\.workspace-tab-strip \{[\s\S]*border-bottom: 0;/,
  ]);
  assertDoesNotMatchAny(stylesSource, [
    /\.workspace-tab:active \{[\s\S]*cursor:/,
  ]);
  for (const [selector, nextSelector] of [
    [".workspace-tab {", ".workspace-tab:hover"],
    [".workspace-tab-close {", ".workspace-tab-close:hover"],
    [".workspace-tab-add-button {", ".workspace-tab-add-button:hover"],
    [".workspace-tab-add-menu-item {", ".workspace-tab-add-menu-item:hover"],
    [".git-section-toggle {", ".git-section-toggle:hover"],
    [".git-icon-button {", ".git-icon-button:hover"],
    [".drag-scroll-region {", ".drag-scroll-region.is-dragging"],
    [".git-graph-row-main {", ".git-graph-lanes"],
    [".git-change-group-header {", ".git-change-group-header[aria-expanded"],
    [
      ".git-change-row.clickable:hover",
      ".git-change-row.clickable:focus-visible",
    ],
  ] as const) {
    assert.doesNotMatch(
      cssBlock(stylesSource, selector, nextSelector),
      /cursor:/,
    );
  }
  assertMatches(stylesSource, [
    /UI polish: right panel content keeps structure without extra 1px separator lines\./,
    /\.right-panel \.panel-rail,[\s\S]*\.right-panel \.terminal-viewport-shell \{[\s\S]*border-top: 0;[\s\S]*border-bottom: 0;/,
    /\.right-panel \.panel-rail \{[\s\S]*border-left: 0;/,
    /\.right-panel \.overview-metric,[\s\S]*\.right-panel \.workflow-status-pill \{[\s\S]*border: 0;/,
    /\.panel-rail-button\.active \{[\s\S]*background: rgba\(28, 25, 23, 0\.05\);/,
    /\.panel-rail-button\.active::after \{[\s\S]*background: #d97706;/,
    /\.workspace-tab-panel > \.conversation-panel,\s*\.conversation-panel,\s*\.conversation-scroll \{[\s\S]*border-top: 0;[\s\S]*box-shadow: none;/,
    /\.sidebar \{[\s\S]*border-right: 0;/,
    /\.conversation-scroll \{[\s\S]*background: #ffffff;[\s\S]*background-image: none;/,
    /\.compact-row::before,\s*\.archive-row::before \{[\s\S]*display: none;/,
    /\.panel-resizer \{[\s\S]*background: transparent;/,
    /\.panel-resizer::before \{[\s\S]*left: 50%;[\s\S]*width: 1px;[\s\S]*background: rgba\(16, 24, 40, 0\.08\);/,
    /\.panel-resizer:hover::before \{[\s\S]*background: rgba\(217, 119, 6, 0\.42\);/,
    /\.is-resizing-panels \.panel-resizer::before \{[\s\S]*background: rgba\(217, 119, 6, 0\.68\);/,
  ]);
  assert.match(appSource, /const PANEL_RESIZER_WIDTH = 4/);
  assertMatches(stylesSource, [
    /\.panel-content-header \{[\s\S]*min-height: 34px;[\s\S]*padding: 6px 10px;/,
    /\.workspace-tab-dot\.doing/,
    /\.workspace-tab-dot\.waiting-subagent/,
    /\.workspace-tab-dot\.waiting-eventtool/,
    /\.workspace-tab-dot\.waiting-subscription/,
    /\.workspace-tab-dot\.blocked/,
    /\.workspace-tab-dot\.active/,
    /\.workspace-tab-dot\.running/,
    /\.workspace-tab-dot\.completed/,
    /\.workspace-tab-dot\.inactive/,
    /\.workspace-tab-close \{[\s\S]*-webkit-app-region: no-drag;/,
    /\.workspace-tab-add-button \{[\s\S]*-webkit-app-region: no-drag;/,
    /\.workspace-tab-add-menu \{[\s\S]*-webkit-app-region: no-drag;/,
  ]);
  assertDoesNotMatchAny(stylesSource, [
    /\.file-object-toolbar/,
    /\.panel-eyebrow/,
    /\.preview-mode-toggle/,
    /\.conversation-header/,
    /\.conversation-panel[^{]*\{[^}]*border-right:/,
    /\.conversation-panel[^{]*\{[^}]*border-top:(?![ \t]*0[ \t]*;)/,
    /\.conversation-panel[^{]*\{[^}]*box-shadow:(?![ \t]*none[ \t]*;)/,
    /\.conversation-scroll[^{]*\{[^}]*border-top:(?![ \t]*0[ \t]*;)/,
    /\.conversation-scroll[^{]*\{[^}]*box-shadow:(?![ \t]*none[ \t]*;)/,
    /\.workspace-tab-panel[^{]*\{[^}]*border-right:/,
    /\.workspace-tab-panel\s*\{[^}]*border-top:/,
    /\.workspace-tab-panel\s*\{[^}]*box-shadow:/,
    /\.workspace-tab-strip\s*\{[^}]*border-bottom:(?![ \t]*0[ \t]*;)/,
  ]);
  assert.doesNotMatch(
    cssBlock(stylesSource, ".compact-row::before,", ".compact-icon"),
    /(?:top:|height:|background: linear-gradient)/,
  );
  assertDoesNotMatchAny(stylesSource, [
    /\.workspace-main \{[^}]*border-right:/,
    /\.composer-shell \{[^}]*border-top:/,
    /\.composer-shell \{[^}]*box-shadow:/,
    /scrollbar[^{}]*(?::focus|:focus-within|:active)[^{]*\{/,
    /(?::focus|:focus-within|:active)[^{]*::-[^{]*scrollbar/,
  ]);
});

test("right panel terminal rail click is the explicit terminal panel focus source", () => {
  const source = readSource("rightPanel");
  const tokenStateIndex = source.indexOf(
    "const [terminalPanelFocusRequestToken, setTerminalPanelFocusRequestToken]",
  );
  const propIndex = source.indexOf(
    "focusPanelRequestToken={terminalPanelFocusRequestToken}",
  );
  const railClickIndex = source.indexOf('if (item.view === "terminal") {');
  const railClickSource = source.slice(
    railClickIndex,
    source.indexOf("onSetActiveView(next.nextView);", railClickIndex),
  );

  assert.notEqual(tokenStateIndex, -1);
  assert.notEqual(propIndex, -1);
  assert.notEqual(railClickIndex, -1);
  assert.match(
    railClickSource,
    /setTerminalPanelFocusRequestToken\(\s*\(current\) => current \+ 1\s*\);/,
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
  const markup = renderScheduleAgendaDateGroup(0, false);

  assert.match(markup, /aria-expanded="true"/);
  assert.match(markup, /aria-controls="schedule-agenda-items-2026-07-13"/);
  assert.match(markup, /Today/);
  assert.match(markup, /standup ping/);
  assert.match(markup, /Every 6 hours/);
});

test("renders runtime restart progress in thread analysis", () => {
  const markup = renderRightPanel(makeThread([]), "skills", null, {
    runtimeRestartProgress: makeRuntimeRestartProgress(),
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
    runtimeRestartProgress: makeRuntimeRestartProgress({
      status: "failed",
      requestId: "restart-failed",
      stage: "failed",
      stageLabel: "Failed",
      message: "Build failed",
      reason: "Build failed",
      activationId: null,
      releaseId: null,
    }),
  });

  assert.match(failedMarkup, /Runtime Restart/);
  assert.match(failedMarkup, /Failed/);
  assert.match(failedMarkup, /restart-failed/);
  assert.match(failedMarkup, /Build failed/);
});

test("renders schedule agenda with an overall disclosure header", () => {
  const markup = renderScheduleAgendaLayout(false);

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
  const markup = renderScheduleAgendaLayout(true);

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
    group: makeScheduleAgendaGroup(0),
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
  const collapsedMarkup = renderScheduleAgendaDateGroup(0, true);
  const expandedMarkup = renderScheduleAgendaDateGroup(1, false);

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
  const markup = renderRightPanel(makeThread([]), "skills", planUpdate);

  assertMatches(markup, [
    /Thread Analysis/,
    /context-section-card current-plan-card/,
    /Keep the change scoped\./,
    /Filter direct child tasks/,
    /Render current thread plan/,
    /Validate parallel owner/,
    /Wait for release approval/,
    /Run validation/,
    /In progress/,
    /Blocked/,
    /plan-status-label blocked/,
  ]);
  assert.equal(markup.match(/plan-status-label inProgress/g)?.length, 2);
  assertDoesNotMatchAny(markup, [/Plan Work/, /Execution Queue/, /Todo List/]);
});

test("keeps plan and monitor activity on compact right panel layout rules", () => {
  const { rightPanelSource, stylesSource } = readSources([
    "rightPanel",
    "styles",
  ]);

  assert.match(
    rightPanelSource,
    /className="context-section-card current-plan-card"/,
  );
  assertMatches(stylesSource, [
    /\.context-section-card\.current-plan-card\s*\{[\s\S]*padding: 10px 12px;/,
    /\.plan-status-dot\.blocked/,
    /\.plan-status-label\.blocked/,
    /\.monitor-section\s*\{[\s\S]*padding-top: 10px;/,
    /\.right-panel \.monitor-section,[\s\S]*border-top: 0;/,
    /\.monitor-kind-dot\.command,\s*\.monitor-kind-dot\.process/,
  ]);
  assertDoesNotMatchAny(stylesSource, [
    /\.current-plan-card\s*\{[^}]*margin:/,
    /\.monitor-empty\s*\{[^}]*border-top:/,
  ]);
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
  assertDoesNotMatchAny(markup, [
    /Wire plan into analysis/,
    /Move the existing work queue into the analysis view\./,
    /\/my_codex\/owner_dev/,
    /No tasks for this filter/,
    /Todo List/,
    /Todo Board/,
  ]);
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
        changes: [{ path: "/tmp/src/app.tsx", kind: "deleted" }],
      },
    ]),
    "git",
  );

  assertMatches(markup, [
    /Git graph/,
    /Graph/,
    /graph-toolbar/,
    />Auto</,
    /Changes/,
    /aria-expanded="true"/,
    /Collapse Changes/,
    /Select Git branch or ref/,
    /Refresh Git view/,
    /Resize Git graph and changes panes/,
    /panel-rail-badge">2/,
  ]);
  assertDoesNotMatchAny(markup, [
    /Focus current Git ref/,
    /Fetch Git refs/,
    /Pull Git refs/,
    /More Git actions/,
    /Thread File Deltas/,
  ]);
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
    "className": string;
    "onClick": (event: { stopPropagation: () => void }) => void;
    "onKeyDown": (event: { stopPropagation: () => void }) => void;
    "title": string;
    "type": string;
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
  const markup = renderGitDiffPreview(
    makeGitDiffPreview({
      available: false,
      error: "Binary files cannot be previewed as side-by-side text.",
      binary: true,
    }),
  );

  assert.match(markup, />DIFF</);
  assert.match(markup, />unstaged</);
  assert.doesNotMatch(markup, /preview-edit-action/);
});

test("git diff previews label commit file diffs as commit scope", () => {
  const markup = renderGitDiffPreview(
    makeGitDiffPreview({
      path: "src/thread.ts",
      oldLabel: "abc1234^",
      newLabel: "abc1234",
      oldContent: "before\n",
      newContent: "after\n",
      modeLabel: "commit",
      commit: "abc1234",
      parent: "abc1234^",
    }),
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
  const source = readSource("rightPanel");

  assertDoesNotMatchAny(source, [
    /readGitStatusSnapshot/,
    /resolveGitTreeFileOpen/,
    /gitDiffTargetForTreePath/,
    /onGitSnapshotChange/,
  ]);
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
    expandedVisualModel.commits.map((commit) => ({
      hash: commit.commit.hash,
      y: commit.y,
    })),
    [
      { hash: "merge-a", y: 21 },
      { hash: "side-a", y: 125 },
      { hash: "merge-b", y: 167 },
    ],
  );
});

test("git graph styles keep a light theme and full-size visible rail overlay", () => {
  const { rightPanelSource, stylesSource } = readSources([
    "rightPanel",
    "styles",
  ]);

  assertMatches(stylesSource, [
    /\.git-panel \{[\s\S]*--git-surface: rgba\(252, 251, 249, 0\.58\);[\s\S]*background: var\(--git-surface\);/,
    /\.git-graph-section \{[\s\S]*background: var\(--git-surface\);/,
    /\.git-graph-overlay \{[\s\S]*width: var\(--git-graph-visual-width, 58px\);/,
    /\.git-graph-overlay \{[\s\S]*height: var\(--git-graph-visual-height, 42px\);/,
    /\.git-graph-row-main \{[\s\S]*min-height: 42px;/,
    /\.git-graph-list,[\s\S]*\.git-changes-list \{[\s\S]*overflow-x: hidden;/,
    /\.git-graph-visual-stack \{[\s\S]*width: 100%;[\s\S]*max-width: 100%;/,
    /\.git-graph-row-main \{[\s\S]*grid-template-columns: var\(--git-graph-visual-width, 58px\) minmax\(0, 1fr\) 28px;/,
    /\.git-graph-copy \{[\s\S]*overflow: hidden;/,
    /\.git-commit-file-row \{[\s\S]*grid-template-columns: 20px minmax\(0, 1fr\) 22px;/,
    /\.git-change-row \{[\s\S]*grid-template-columns: 22px minmax\(0, 1fr\) 20px;/,
    /\.git-graph-dot\.main \{[\s\S]*fill: #fbfaf8;[\s\S]*stroke-width: 3\.4;/,
    /\.git-graph-dot\.branch \{[\s\S]*fill: currentColor;/,
    /\.git-head-ref \{[\s\S]*background: #2563eb;/,
  ]);
  assert.doesNotMatch(
    stylesSource,
    /\.git-panel \{[\s\S]*background: #0f1419;/,
  );
  assert.match(
    rightPanelSource,
    /"--git-graph-visual-height": `\$\{visualModel\.height\}px`/,
  );
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
      "/tmp/src": [{ path: "/tmp/src/App.tsx", name: "App.tsx", kind: "file" }],
    },
  });

  assertDoesNotMatchAny(markup, [
    /aria-label="Show current file"/,
    /aria-label="Show file tree"/,
    /CWD Tree/,
  ]);
  assertMatches(markup, [
    /Thread cwd file tree/,
    /README\.md/,
    /App\.tsx/,
    /title="\/tmp\/src"/,
    /title="\/tmp\/src\/App\.tsx"/,
  ]);
});

test("renders markdown file previews as markdown content", () => {
  const markup = renderPreviewPanel({
    preview: makePreview({
      content: "# Title\n\nThis is **bold**.\n\n[Other](./other.md)",
      language: "markdown",
    }),
  });

  assertMatches(markup, [
    /<h1>Title<\/h1>/,
    /This is <strong>bold<\/strong>\./,
    /href="#"/,
  ]);
  assert.doesNotMatch(markup, /Loading editor/);
});

test("keeps non-markdown file previews on the editor render path", () => {
  assert.equal(filePreviewRenderMode(makeTypescriptPreview()), "editor");
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
  const editorPreview = makeTypescriptPreview();
  const markdownPreview = makeMarkdownPreview();
  const imagePreview = makeImagePreview();
  const pdfPreview = makePdfPreview();

  const markdownMarkup = renderPreviewPanel({
    preview: markdownPreview,
  });
  const imageMarkup = renderPreviewPanel({
    preview: imagePreview,
  });
  const pdfMarkup = renderPreviewPanel({
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
  assert.doesNotMatch(
    markdownMarkup,
    /preview-utility-strip[\s\S]*preview-edit-action/,
  );
  assert.doesNotMatch(imageMarkup, /preview-edit-action/);
  assert.doesNotMatch(pdfMarkup, /preview-edit-action/);
});

test("header edit controls appear only for loaded editable previews", () => {
  const editorPreview = makeTypescriptPreview();
  const imagePreview = makeImagePreview();

  assert.equal(previewHeaderControlsVisible(), true);
  assert.equal(previewHeaderControlsVisible({ preview: editorPreview }), true);
  assert.equal(previewHeaderControlsVisible({ filePanelView: "tree" }), false);
  assert.equal(previewHeaderControlsVisible({ previewLoading: true }), false);
  assert.equal(previewHeaderControlsVisible({ previewError: "Failed" }), false);
  assert.equal(previewHeaderControlsVisible({ preview: imagePreview }), false);
  assert.equal(previewHeaderControlsVisible({ preview: null }), false);

  const loadingMarkup = renderPreviewPanel({
    preview: makeMarkdownPreview(),
    previewLoading: true,
  });
  const errorMarkup = renderPreviewPanel({
    preview: makeMarkdownPreview(),
    previewError: "Failed to load",
  });
  const emptyMarkup = renderPreviewPanel();

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
  const nonHtmlMarkup = renderPreviewPanel({
    preview: makePreview({
      path: "/tmp/docs/share.md",
      displayPath: "docs/share.md",
      language: "markdown",
      content: "# Share",
    }),
  });
  const loadingMarkup = renderPreviewPanel({
    preview: makePreview({
      path: "/tmp/docs/share.html",
      displayPath: "docs/share.html",
      language: "html",
    }),
    previewLoading: true,
  });
  const errorMarkup = renderPreviewPanel({
    preview: makePreview({
      path: "/tmp/docs/share.html",
      displayPath: "docs/share.html",
      language: "html",
    }),
    previewError: "Failed to load",
  });
  const treeMarkup = renderPreviewPanel({
    filePanelView: "tree",
    preview: makePreview({
      path: "/tmp/docs/share.html",
      displayPath: "docs/share.html",
      language: "html",
    }),
  });

  assert.equal(previewOpenInBrowserVisible({ preview: htmlPreview }), true);
  assert.equal(previewOpenInBrowserVisible({ preview: htmPreview }), true);
  assert.doesNotMatch(nonHtmlMarkup, /Open preview in Browser/);
  assert.match(nonHtmlMarkup, /aria-label="Open preview in system editor"/);
  assert.doesNotMatch(loadingMarkup, /Open preview in Browser/);
  assert.doesNotMatch(errorMarkup, /Open preview in Browser/);
  assert.doesNotMatch(treeMarkup, /Open preview in Browser/);
  assert.equal(previewOpenInBrowserVisible(), false);
});

test("markdown previews keep rendered readonly mode until editing or saving", () => {
  const markdownPreview = makeMarkdownPreview();
  const editorPreview = makeTypescriptPreview();
  const imagePreview = makeImagePreview();
  const markup = renderPreviewPanel({
    preview: markdownPreview,
  });

  assert.match(markup, /<h1>Title<\/h1>/);
  assert.doesNotMatch(markup, /Loading editor/);
  assert.equal(
    filePreviewSourceEditorVisible(markdownPreview, "readonly"),
    false,
  );
  assert.equal(
    filePreviewSourceEditorVisible(markdownPreview, "editing"),
    true,
  );
  assert.equal(filePreviewSourceEditorVisible(markdownPreview, "saving"), true);
  assert.equal(filePreviewSourceEditorVisible(editorPreview, "readonly"), true);
  assert.equal(filePreviewSourceEditorVisible(imagePreview, "editing"), false);
});

test("file preview edit state saves, cancels, keeps failures, and resets on file switch", () => {
  const preview = makeTypescriptPreview({
    content: "const value = 1;",
  });
  const nextPreview = makeTypescriptPreview({
    path: "/tmp/src/Other.tsx",
    displayPath: "src/Other.tsx",
    content: "const other = 1;",
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
  const source = readSource("rightPanel");

  assertMatches(source, [
    /onClick=\{\(\) => void savePreviewDraft\(\)\}/,
    /monaco\.KeyMod\.CtrlCmd \| monaco\.KeyCode\.KeyS/,
    /savePreviewDraftRef\.current\(\)/,
  ]);
});

test("renders PDF file previews with an embedded PDF object", () => {
  const markup = renderPreviewPanel({
    preview: makePdfPreview({
      path: "/tmp/Project Docs/spec.PDF",
      displayPath: "Project Docs/spec.PDF",
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
      makePdfPreview({
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
  assert.match(
    markup,
    /data="morpheus-file-preview:\/\/pdf\/token-1\/spec\.PDF"/,
  );
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
  const markup = renderPreviewPanel({
    preview: makeImagePreview(),
  });

  assertMatches(markup, [/IMAGE/, /image\/png/, /diagram\.png/]);
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
      [thread.cwd]: [
        {
          path: `${thread.cwd}/scratch.txt`,
          name: "scratch.txt",
          kind: "file",
        },
      ],
    },
  });

  assert.match(markup, /This chat has no project cwd to browse\./);
  assertDoesNotMatchAny(markup, [
    /aria-label="Show file tree"/,
    /CWD Tree/,
    /Thread cwd file tree/,
    /scratch\.txt/,
  ]);
});

test("renders directory-specific cwd tree errors instead of empty state", () => {
  const thread = makeThread([]);
  const markup = renderRightPanel(thread, "preview", null, {
    expandedTreeDirectories: ["/tmp/src"],
    filePanelView: "tree",
    fileTreeEntriesByPath: {
      "/tmp": [{ path: "/tmp/src", name: "src", kind: "directory" }],
    },
    fileTreeErrorsByPath: { "/tmp/src": "Permission denied" },
  });

  assert.match(markup, /Permission denied/);
  assert.doesNotMatch(markup, /Empty directory/);
});
