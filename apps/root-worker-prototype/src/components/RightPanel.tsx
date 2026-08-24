import React, { useEffect, useRef, useState, type ReactNode } from "react";
import Editor from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";

import {
  ArrowLeftIcon,
  ArrowRightIcon,
  BranchIcon,
  BrowserIcon,
  ChevronDownIcon,
  DocumentIcon,
  GridIcon,
  MoreIcon,
  GearIcon,
  OpenIcon,
  RefreshIcon,
  SearchIcon,
  StopIcon,
} from "./icons";
import { LocalImagePreview } from "./Conversation";
import { isChatCompatCwd } from "../lib/chatCompat";
import { normalizeBrowserUrl } from "../lib/browserUrl";
import { getContextUsageCategoryColor } from "../lib/contextUsage";
import { MarkdownContent } from "../lib/markdown";
import { resolveRightPanelTabClick } from "../lib/rightPanelView";
import {
  buildThreadAnalysis,
  type MonitorSummary,
  type ScheduleAgendaGroup,
  type ThreadAnalysis,
} from "../lib/threadAnalysis";
import {
  buildWorkflowPanelViewModel,
  formatWorkflowTimestamp,
  type WorkflowRunView,
  type WorkflowStageView,
  type WorkflowTimelineItem,
} from "../lib/workflowProgress";
import type {
  FilePanelView,
  FileLocation,
  FilePreview,
  FileTreeEntry,
  RightPanelView,
  Thread,
  ThreadGoal,
  ThreadPlanStep,
  ThreadPlanUpdate,
  ThreadSkill,
  TodoCardItem,
  WorkflowSummary,
} from "../types";

type GoalActionKind = "set" | "pause" | "resume" | "clear";

type BrowserViewBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

type BrowserPanelState = {
  url: string | null;
  title: string | null;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  error: string | null;
};

type BrowserPanelApi = Pick<
  Window["codexDesktop"],
  | "browserGoBack"
  | "browserGoForward"
  | "hideBrowserView"
  | "navigateBrowserView"
  | "openLink"
  | "reloadBrowserView"
  | "setBrowserViewBounds"
  | "showBrowserView"
  | "stopBrowserView"
  | "subscribeBrowserState"
>;

type GitSnapshot = Awaited<ReturnType<Window["codexDesktop"]["readGitSnapshot"]>>;
type GitChange = GitSnapshot["changes"][number];
type GitGraphItem = GitSnapshot["graph"][number];
type GitGraphCommit = Extract<GitGraphItem, { type: "commit" }>;
type GitCommitFilesSnapshot = Awaited<ReturnType<Window["codexDesktop"]["readGitCommitFiles"]>>;
type GitCommitFile = GitCommitFilesSnapshot["files"][number];
type GitGraphVisualCommit = {
  commit: GitGraphCommit;
  rowIndex: number;
  lane: number;
  colorLane: number;
  x: number;
  y: number;
};
type GitGraphVisualPath = {
  id: string;
  lane: number;
  colorLane: number;
  kind: "main" | "branch";
  d: string;
};
type GitGraphVisualModel = {
  width: number;
  height: number;
  commits: GitGraphVisualCommit[];
  paths: GitGraphVisualPath[];
};
type GitGraphVisualOptions = {
  expandedHeightsByHash?: Record<string, number>;
  rowHeight?: number;
};

const GIT_GRAPH_SPINE_X = 18;
const GIT_GRAPH_LANE_WIDTH = 23;
const GIT_GRAPH_COMMIT_ROW_HEIGHT = 42;
const GIT_COMMIT_FILE_ROW_HEIGHT = 28;
const GIT_COMMIT_FILES_STATE_HEIGHT = 34;
const GIT_COMMIT_FILES_BLOCK_MARGIN_BOTTOM = 6;
type PreviewDefinitionPositionEditor = {
  getModel(): {
    getWordAtPosition(position: PreviewDefinitionPosition): {
      startColumn: number;
      endColumn: number;
    } | null;
  } | null;
};
type PreviewDefinitionPosition = {
  lineNumber: number;
  column: number;
};

const EMPTY_BROWSER_STATE: BrowserPanelState = {
  url: null,
  title: null,
  loading: false,
  canGoBack: false,
  canGoForward: false,
  error: null,
};

function formatByteSize(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value < 10 && unitIndex > 0 ? value.toFixed(1) : Math.round(value)} ${units[unitIndex]}`;
}

function formatTokenCount(value: number | null) {
  if (!value || value <= 0) {
    return "0";
  }
  if (value >= 1_000_000) {
    return `${Math.round((value / 1_000_000) * 10) / 10}M`;
  }
  if (value >= 1_000) {
    return `${Math.round(value / 100) / 10}K`;
  }
  return String(value);
}

export function RightPanel({
  activeView,
  availableSkillCount,
  availableWorkflows,
  isCollapsed,
  onNavigateToSymbol,
  onOpenPreviewExternally,
  onOpenTreeFile,
  onSelectCommandMonitor,
  onSetActiveView,
  onSetCollapsed,
  onSetFilePanelView,
  onToggleTreeDirectory,
  onCancelGoal,
  onPauseGoal,
  onResumeGoal,
  filePanelView,
  fileTreeEntriesByPath,
  fileTreeErrorsByPath,
  fileTreeLoadingPath,
  expandedTreeDirectories,
  planUpdate,
  goal,
  goalAction,
  goalActionError,
  preview,
  previewError,
  previewLoading,
  skills,
  thread,
  modelContextWindowOverride,
  todoItems,
}: {
  activeView: RightPanelView;
  availableSkillCount: number;
  availableWorkflows: WorkflowSummary[];
  isCollapsed: boolean;
  onNavigateToSymbol: (destination: FileLocation, sourceLocation: FileLocation) => void;
  onOpenPreviewExternally: () => void;
  onOpenTreeFile: (path: string) => void;
  onSelectCommandMonitor?: (commandItemId: string) => void;
  onSetActiveView: (value: RightPanelView) => void;
  onSetCollapsed: (value: boolean) => void;
  onSetFilePanelView: (value: FilePanelView) => void;
  onToggleTreeDirectory: (path: string) => void;
  onCancelGoal: () => void;
  onPauseGoal: () => void;
  onResumeGoal: () => void;
  filePanelView: FilePanelView;
  fileTreeEntriesByPath: Record<string, FileTreeEntry[]>;
  fileTreeErrorsByPath: Record<string, string>;
  fileTreeLoadingPath: string | null;
  expandedTreeDirectories: string[];
  planUpdate: ThreadPlanUpdate | null;
  goal: ThreadGoal | null;
  goalAction: GoalActionKind | null;
  goalActionError: string | null;
  preview: FilePreview | null;
  previewError: string | null;
  previewLoading: boolean;
  skills: ThreadSkill[];
  thread: Thread | null;
  modelContextWindowOverride?: number | null;
  todoItems: TodoCardItem[];
}) {
  const todoStats = buildTodoStats(todoItems);
  const threadAnalysis = buildThreadAnalysis(
    thread,
    availableSkillCount,
    modelContextWindowOverride,
  );
  const workflowPanel = buildWorkflowPanelViewModel(thread, availableWorkflows);
  const { contextUsage } = threadAnalysis;

  return (
    <aside className={`right-panel ${isCollapsed ? "collapsed" : ""}`}>
      <div className="right-panel-body">
        {!isCollapsed ? (
          <div className="right-panel-content">
            {activeView === "skills" ? (
              <ThreadAnalysisPanel
                analysis={threadAnalysis}
                goal={goal}
                goalAction={goalAction}
                goalActionError={goalActionError}
                onCancelGoal={onCancelGoal}
                onPauseGoal={onPauseGoal}
                onResumeGoal={onResumeGoal}
                onSelectCommandMonitor={onSelectCommandMonitor}
                planUpdate={planUpdate}
              />
            ) : activeView === "git" ? (
              <GitPanel changedFiles={threadAnalysis.changedFiles} thread={thread} />
            ) : activeView === "browser" ? (
              <BrowserPanel />
            ) : activeView === "workflow" ? (
              <WorkflowPanel model={workflowPanel} />
            ) : (
              <FilePreviewPanel
                expandedTreeDirectories={expandedTreeDirectories}
                filePanelView={filePanelView}
                fileTreeEntriesByPath={fileTreeEntriesByPath}
                fileTreeErrorsByPath={fileTreeErrorsByPath}
                fileTreeLoadingPath={fileTreeLoadingPath}
                onNavigateToSymbol={onNavigateToSymbol}
                onOpenPreviewExternally={onOpenPreviewExternally}
                onOpenTreeFile={onOpenTreeFile}
                onSetFilePanelView={onSetFilePanelView}
                onToggleTreeDirectory={onToggleTreeDirectory}
                preview={preview}
                previewError={previewError}
                previewLoading={previewLoading}
                thread={thread}
              />
            )}
          </div>
        ) : null}

        <nav className="panel-rail" aria-label="Right panel views">
          {(
            [
              {
                view: "skills",
                label: "Thread Analysis",
                icon: <GearIcon />,
                badge:
                  todoStats.openCount > 0
                    ? String(todoStats.openCount)
                    : threadAnalysis.monitors.totalCount > 0
                      ? String(threadAnalysis.monitors.totalCount)
                      : contextUsage.loadedSkills > 0
                        ? String(contextUsage.loadedSkills)
                        : "",
              },
              {
                view: "preview",
                label: "File Preview",
                icon: <DocumentIcon />,
                badge: preview ? "1" : "",
              },
              {
                view: "git",
                label: "Git Changes",
                icon: <BranchIcon />,
                badge:
                  threadAnalysis.changedFiles.length > 0
                    ? String(threadAnalysis.changedFiles.length)
                    : "",
              },
              {
                view: "browser",
                label: "Browser",
                icon: <BrowserIcon />,
                badge: "",
              },
              {
                view: "workflow",
                label: "Workflow",
                icon: <GridIcon />,
                badge:
                  workflowPanel.selectedRun?.statusTone === "running"
                    ? "1"
                    : workflowPanel.runs.length > 0
                      ? String(workflowPanel.runs.length)
                      : "",
              },
              {
                view: null,
                label: "Search",
                icon: <SearchIcon />,
                badge: "",
              },
              {
                view: null,
                label: "Artifacts",
                icon: <GridIcon />,
                badge: "",
              },
            ] satisfies Array<{
              view: RightPanelView | null;
              label: string;
              icon: ReactNode;
              badge: string;
            }>
          ).map((item) => (
            <button
              key={item.label}
              type="button"
              className={`panel-rail-button ${item.view === activeView ? "active" : ""}`}
              aria-label={item.label}
              disabled={item.view == null}
              onClick={() => {
                if (item.view) {
                  const next = resolveRightPanelTabClick({
                    activeView,
                    clickedView: item.view,
                    isCollapsed,
                  });
                  onSetActiveView(next.nextView);
                  onSetCollapsed(next.nextCollapsed);
                }
              }}
            >
              <span className="panel-rail-icon">{item.icon}</span>
              {item.badge ? <span className="panel-rail-badge">{item.badge}</span> : null}
            </button>
          ))}
        </nav>
      </div>
    </aside>
  );
}

function WorkflowPanel({
  model,
}: {
  model: ReturnType<typeof buildWorkflowPanelViewModel>;
}) {
  const run = model.selectedRun;
  return (
    <div className="skills-panel workflow-panel">
      <header className="panel-content-header">
        <div className="panel-content-copy">
          <span className="panel-eyebrow">Workflow</span>
          <h2>{run ? run.workflowName : "Workflow"}</h2>
          <p>
            {run
              ? `${run.runId} · ${run.statusLabel}`
              : `${model.availableWorkflows.length} available workflow${model.availableWorkflows.length === 1 ? "" : "s"}`}
          </p>
        </div>
        {run ? (
          <span className={`workflow-status-pill ${run.statusTone}`}>
            {run.statusLabel}
          </span>
        ) : null}
      </header>

      <div className="skills-scroll workflow-scroll">
        {run ? (
          <>
            <WorkflowRunSummary run={run} />
            <WorkflowStageRail run={run} />
            <WorkflowTimeline timeline={run.timeline} />
          </>
        ) : (
          <WorkflowEmptyState availableWorkflows={model.availableWorkflows} />
        )}
      </div>
    </div>
  );
}

function WorkflowRunSummary({ run }: { run: WorkflowRunView }) {
  return (
    <section className="workflow-summary-card">
      <div className="workflow-summary-grid">
        <WorkflowSummaryMetric label="Runner" value={run.runnerStatus || "unknown"} />
        <WorkflowSummaryMetric label="Run" value={run.statusLabel} tone={run.statusTone} />
        <WorkflowSummaryMetric label="Updated" value={formatWorkflowTimestamp(run.updatedAt)} />
      </div>
      <div className="workflow-message" title={run.message}>
        {run.message || "No workflow message."}
      </div>
      <div className="workflow-run-meta">
        <span title={run.runId}>{run.runId}</span>
        <span>{run.source}</span>
      </div>
    </section>
  );
}

function WorkflowSummaryMetric({
  label,
  value,
  tone = "unknown",
}: {
  label: string;
  value: string;
  tone?: WorkflowRunView["statusTone"];
}) {
  return (
    <div className={`workflow-summary-metric ${tone}`}>
      <span>{label}</span>
      <strong title={value}>{value}</strong>
    </div>
  );
}

function WorkflowStageRail({ run }: { run: WorkflowRunView }) {
  return (
    <section className="context-section-card workflow-graph-card">
      <div className="context-section-header">
        <div>
          <span className="context-section-eyebrow">Graph</span>
          <strong>Stages</strong>
        </div>
        <span className={`workflow-graph-source ${run.graphSource}`}>
          {run.graphSource}
        </span>
      </div>

      {run.stages.length > 0 ? (
        <ol className="workflow-stage-list" aria-label="Workflow stages">
          {run.stages.map((stage, index) => (
            <WorkflowStageRow
              key={stage.id}
              isLast={index === run.stages.length - 1}
              stage={stage}
            />
          ))}
        </ol>
      ) : (
        <div className="workflow-graph-empty">Graph unavailable</div>
      )}

      {run.graphNote ? <p className="workflow-graph-note">{run.graphNote}</p> : null}
    </section>
  );
}

function WorkflowStageRow({
  isLast,
  stage,
}: {
  isLast: boolean;
  stage: WorkflowStageView;
}) {
  return (
    <li className={`workflow-stage-row ${stage.status}`}>
      <span className="workflow-stage-marker" aria-hidden="true">
        <span className="workflow-stage-dot" />
        {!isLast ? <span className="workflow-stage-line" /> : null}
      </span>
      <span className="workflow-stage-label">{stage.label}</span>
      <span className={`workflow-stage-status ${stage.status}`}>
        {formatWorkflowStageStatus(stage.status)}
      </span>
    </li>
  );
}

function WorkflowTimeline({ timeline }: { timeline: WorkflowTimelineItem[] }) {
  return (
    <section className="context-section-card workflow-timeline-card">
      <div className="context-section-header">
        <div>
          <span className="context-section-eyebrow">Progress</span>
          <strong>Recent Events</strong>
        </div>
        <span className="context-inline-metric">
          {timeline.length} event{timeline.length === 1 ? "" : "s"}
        </span>
      </div>

      <div className="workflow-timeline-list">
        {timeline.map((item) => (
          <article key={item.id} className="workflow-timeline-row">
            <div className="workflow-timeline-main">
              <div className="workflow-timeline-head">
                <strong>{item.label}</strong>
                <span className={`workflow-status-pill ${item.statusTone}`}>
                  {item.statusLabel}
                </span>
              </div>
              <span title={item.message}>{item.message || item.runnerStatus}</span>
              <time dateTime={new Date(item.updatedAt * 1000).toISOString()}>
                {formatWorkflowTimestamp(item.updatedAt)}
              </time>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

function WorkflowEmptyState({
  availableWorkflows,
}: {
  availableWorkflows: WorkflowSummary[];
}) {
  return (
    <div className="workflow-empty-state">
      <div className="empty-card">
        <p>No workflow activity in this thread.</p>
      </div>
      {availableWorkflows.length > 0 ? (
        <section className="context-section-card workflow-available-card">
          <div className="context-section-header">
            <div>
              <span className="context-section-eyebrow">Available</span>
              <strong>Workflows</strong>
            </div>
            <span className="context-inline-metric">{availableWorkflows.length}</span>
          </div>
          <div className="workflow-available-list">
            {availableWorkflows.slice(0, 5).map((workflow) => (
              <article key={workflow.id} className="workflow-available-row">
                <strong>{workflow.name || workflow.id}</strong>
                <span>{workflow.description || workflow.id}</span>
              </article>
            ))}
          </div>
        </section>
      ) : null}
    </div>
  );
}

function formatWorkflowStageStatus(status: WorkflowStageView["status"]) {
  switch (status) {
    case "completed":
      return "Done";
    case "current":
      return "Current";
    case "failed":
      return "Failed";
    case "aborted":
      return "Aborted";
    case "pending":
      return "Waiting";
    default:
      return "Unknown";
  }
}

function BrowserPanel() {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const [address, setAddress] = useState("");
  const [state, setState] = useState<BrowserPanelState>(EMPTY_BROWSER_STATE);
  const [localError, setLocalError] = useState<string | null>(null);
  const hasBrowserApi = currentBrowserPanelApi() !== null;

  useEffect(() => {
    const browserApi = currentBrowserPanelApi();
    const unsubscribe = browserApi?.subscribeBrowserState((nextState) => {
      setState({
        url: nextState.url,
        title: nextState.title,
        loading: nextState.loading,
        canGoBack: nextState.canGoBack,
        canGoForward: nextState.canGoForward,
        error: nextState.error,
      });
      if (nextState.url) {
        setAddress(nextState.url);
      }
    });

    return () => {
      unsubscribe?.();
    };
  }, []);

  useEffect(() => {
    const viewport = viewportRef.current;
    const browserApi = currentBrowserPanelApi();
    if (!viewport || !browserApi) {
      return undefined;
    }

    const updateBounds = () => {
      const bounds = browserBoundsFromElement(viewport);
      void browserApi
        .setBrowserViewBounds(bounds)
        .catch((error) => setLocalError(toBrowserError(error)));
    };

    void browserApi
      .showBrowserView(browserBoundsFromElement(viewport))
      .then((nextState) => {
        setState(nextState);
        if (nextState.url) {
          setAddress(nextState.url);
        }
      })
      .catch((error) => setLocalError(toBrowserError(error)));

    updateBounds();
    const resizeObserver = new ResizeObserver(updateBounds);
    resizeObserver.observe(viewport);
    window.addEventListener("resize", updateBounds);

    return () => {
      resizeObserver.disconnect();
      window.removeEventListener("resize", updateBounds);
      void browserApi.hideBrowserView();
    };
  }, []);

  const displayUrl = state.url ?? "";
  const error = localError ?? state.error;

  const navigate = () => {
    const normalized = normalizeBrowserUrl(address);
    if (!normalized.ok) {
      setLocalError(normalized.reason);
      return;
    }
    const browserApi = currentBrowserPanelApi();
    if (!browserApi) {
      setLocalError("In-app browser is unavailable in this environment.");
      return;
    }
    setLocalError(null);
    void browserApi
      .navigateBrowserView(normalized.url)
      .then(setState)
      .catch((navigationError) => setLocalError(toBrowserError(navigationError)));
  };

  const runCommand = (
    command: (browserApi: BrowserPanelApi) => Promise<BrowserPanelState>,
    fallbackError: string,
  ) => {
    const browserApi = currentBrowserPanelApi();
    if (!browserApi) {
      setLocalError("In-app browser is unavailable in this environment.");
      return;
    }
    setLocalError(null);
    void command(browserApi)
      .then(setState)
      .catch((commandError) =>
        setLocalError(toBrowserError(commandError) || fallbackError),
      );
  };

  return (
    <div className="preview-panel browser-panel">
      <header className="panel-content-header browser-header">
        <div className="panel-content-copy">
          <span className="panel-eyebrow">Browser</span>
          <h2>{state.title || "Browser"}</h2>
        </div>
        <button
          type="button"
          className="panel-inline-action browser-open-external"
          aria-label="Open browser page externally"
          title="Open externally"
          disabled={!displayUrl || !hasBrowserApi}
          onClick={() => {
            if (!displayUrl) {
              return;
            }
            runCommand(
              (browserApi) => browserApi.openLink(displayUrl).then(() => state),
              "Could not open the page externally.",
            );
          }}
        >
          <OpenIcon />
        </button>
      </header>

      <form
        className="browser-toolbar"
        onSubmit={(event) => {
          event.preventDefault();
          navigate();
        }}
      >
        <button
          type="button"
          className="browser-icon-button"
          aria-label="Go back"
          title="Back"
          disabled={!state.canGoBack || !hasBrowserApi}
          onClick={() =>
            runCommand(
              (browserApi) => browserApi.browserGoBack(),
              "Could not go back.",
            )
          }
        >
          <ArrowLeftIcon />
        </button>
        <button
          type="button"
          className="browser-icon-button"
          aria-label="Go forward"
          title="Forward"
          disabled={!state.canGoForward || !hasBrowserApi}
          onClick={() =>
            runCommand(
              (browserApi) => browserApi.browserGoForward(),
              "Could not go forward.",
            )
          }
        >
          <ArrowRightIcon />
        </button>
        <button
          type="button"
          className="browser-icon-button"
          aria-label={state.loading ? "Stop loading" : "Reload"}
          title={state.loading ? "Stop" : "Reload"}
          disabled={!hasBrowserApi}
          onClick={() =>
            runCommand(
              (browserApi) =>
                state.loading
                  ? browserApi.stopBrowserView()
                  : browserApi.reloadBrowserView(),
              "Could not update the page.",
            )
          }
        >
          {state.loading ? <StopIcon /> : <RefreshIcon />}
        </button>
        <input
          aria-label="Browser URL"
          value={address}
          placeholder="https://example.com or localhost:5173"
          onChange={(event) => setAddress(event.target.value)}
        />
        <button
          type="submit"
          className="browser-go-button"
          disabled={!hasBrowserApi}
        >
          Go
        </button>
      </form>

      <div className="browser-status-row" role="status">
        <span className={`browser-status-dot ${state.loading ? "loading" : "idle"}`} />
        <span title={error ?? displayUrl}>
          {error ?? (displayUrl || "Ready")}
        </span>
      </div>

      <div ref={viewportRef} className="browser-native-viewport">
        {!displayUrl ? (
          <div className="browser-empty">
            <BrowserIcon />
            <span>Open a page in the right panel.</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function browserBoundsFromElement(element: HTMLElement): BrowserViewBounds {
  const rect = element.getBoundingClientRect();
  return {
    x: Math.max(0, Math.round(rect.left)),
    y: Math.max(0, Math.round(rect.top)),
    width: Math.max(0, Math.round(rect.width)),
    height: Math.max(0, Math.round(rect.height)),
  };
}

function currentBrowserPanelApi(): BrowserPanelApi | null {
  if (typeof window === "undefined") {
    return null;
  }
  const browserApi = window.codexDesktop;
  if (
    !browserApi?.browserGoBack ||
    !browserApi.browserGoForward ||
    !browserApi.hideBrowserView ||
    !browserApi.navigateBrowserView ||
    !browserApi.openLink ||
    !browserApi.reloadBrowserView ||
    !browserApi.setBrowserViewBounds ||
    !browserApi.showBrowserView ||
    !browserApi.stopBrowserView ||
    !browserApi.subscribeBrowserState
  ) {
    return null;
  }
  return browserApi;
}

function toBrowserError(error: unknown) {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return "Browser action failed.";
}

function ThreadAnalysisPanel({
  analysis,
  goal,
  goalAction,
  goalActionError,
  onCancelGoal,
  onPauseGoal,
  onResumeGoal,
  onSelectCommandMonitor,
  planUpdate,
}: {
  analysis: ThreadAnalysis;
  goal: ThreadGoal | null;
  goalAction: GoalActionKind | null;
  goalActionError: string | null;
  onCancelGoal: () => void;
  onPauseGoal: () => void;
  onResumeGoal: () => void;
  onSelectCommandMonitor?: (commandItemId: string) => void;
  planUpdate: ThreadPlanUpdate | null;
}) {
  const { contextUsage, monitors } = analysis;

  return (
    <div className="skills-panel context-usage-panel">
      <header className="panel-content-header">
        <div className="panel-content-copy">
          <span className="panel-eyebrow">Thread Analysis</span>
          <h2>Thread Analysis</h2>
          <p>Context mix, loaded skills, and live monitor activity.</p>
        </div>
      </header>

      <div className="skills-scroll">
        <section
          className="thread-analysis-summary"
          aria-label="Thread analysis summary"
        >
          <OverviewMetric
            label="Context"
            value={contextUsage.hasBudgetData ? contextUsage.budgetUsedPercent : 0}
            tone="open"
          />
          <OverviewMetric
            label="Monitors"
            value={monitors.totalCount}
            tone="doing"
          />
          <OverviewMetric
            label="Events"
            value={monitors.eventCount}
            tone="blocked"
          />
        </section>

        <GoalDetailPanel
          goal={goal}
          action={goalAction}
          actionError={goalActionError}
          onCancel={onCancelGoal}
          onPause={onPauseGoal}
          onResume={onResumeGoal}
        />

        <CurrentPlanCard planUpdate={planUpdate} />

        <section className="context-budget-card">
          <div className="context-budget-header">
            <div>
              <span className="context-budget-label">Context Window Used</span>
              <strong>
                {contextUsage.hasBudgetData
                  ? `${contextUsage.budgetUsedPercent}%`
                  : "Unavailable"}
              </strong>
            </div>
            <span className="context-budget-note">
              {contextUsage.hasBudgetData
                ? `${formatTokenCount(contextUsage.usedTokens)} / ${formatTokenCount(contextUsage.contextWindowTokens)}`
                : "Waiting for token usage"}
            </span>
          </div>
          <div className="context-budget-track" aria-hidden="true">
            <span
              className="context-budget-fill"
              style={{ width: `${contextUsage.budgetUsedPercent}%` }}
            />
          </div>
          <div className="context-category-row">
            {contextUsage.categories.map((category) => (
              <div key={category.id} className="context-category-pill">
                <span
                  className="context-category-dot"
                  style={{
                    backgroundColor: getContextUsageCategoryColor(category.id),
                  }}
                />
                <span className="context-category-pill-label">{category.label}</span>
                <span className="context-category-pill-value">{category.sharePercent}%</span>
              </div>
            ))}
          </div>
        </section>

        <section className="context-section-card">
          <div className="context-section-header">
            <div>
              <span className="context-section-eyebrow">Monitor Activity</span>
              <strong>Live Index</strong>
            </div>
            <span className="context-inline-metric">
              {monitors.totalCount} item
              {monitors.totalCount === 1 ? "" : "s"}
            </span>
          </div>

          <ScheduleAgenda groups={monitors.scheduleAgenda} />

          <div className="monitor-section-list">
            {monitors.sections.map((section) => (
              <div key={section.kind} className="monitor-section">
                <div className="monitor-section-title">
                  <span className={`monitor-kind-dot ${section.kind}`} />
                  <span>{section.title}</span>
                </div>
                {section.monitors.length > 0 ? (
                  <div className="monitor-list">
                    {section.monitors.map((monitor) => (
                      <article
                        key={monitor.id}
                        className={`monitor-row ${monitor.kind === "command" ? "clickable" : ""}`}
                        tabIndex={monitor.kind === "command" ? 0 : undefined}
                        role={monitor.kind === "command" ? "button" : undefined}
                        onClick={() => {
                          if (monitor.kind === "command") {
                            onSelectCommandMonitor?.(monitor.id);
                          }
                        }}
                        onKeyDown={(event) => {
                          if (
                            monitor.kind === "command" &&
                            (event.key === "Enter" || event.key === " ")
                          ) {
                            event.preventDefault();
                            onSelectCommandMonitor?.(monitor.id);
                          }
                        }}
                      >
                        <div className="monitor-row-main">
                          <strong title={monitor.label}>{monitor.label}</strong>
                          <span title={monitor.detail}>{monitor.detail}</span>
                          {shouldRenderMonitorLatestEvent(monitor) ? (
                            <span title={monitor.latestEvent ?? undefined}>
                              {monitor.latestEvent}
                            </span>
                          ) : null}
                        </div>
                        <span
                          className={`monitor-status ${statusClassName(monitor.status)}`}
                        >
                          {monitor.status}
                        </span>
                      </article>
                    ))}
                  </div>
                ) : (
                  <div className="monitor-empty">{section.emptyLabel}</div>
                )}
              </div>
            ))}
          </div>
        </section>

        <section className="context-section-card">
          <div className="context-section-header">
            <div>
              <span className="context-section-eyebrow">Skills Analysis</span>
              <strong>Loaded Skills</strong>
            </div>
            <span className="context-inline-metric">
              {contextUsage.loadedSkills} / {contextUsage.totalSkills}
            </span>
          </div>

          {contextUsage.loadedConcreteSkills.length > 0 ? (
            <div className="context-skill-list">
              {contextUsage.loadedConcreteSkills.map((skill) => (
                <article key={`${skill.kind}:${skill.path}:${skill.name}`} className="context-skill-row">
                  <div className="context-skill-copy">
                    <strong>{skill.name}</strong>
                    <span>loaded {skill.loadCount} time{skill.loadCount === 1 ? "" : "s"}</span>
                  </div>
                  <span className={`skill-kind-badge ${skill.kind}`}>{formatSkillKind(skill.kind)}</span>
                </article>
              ))}
            </div>
          ) : (
            <div className="empty-card">
              <p>No loaded skills yet.</p>
            </div>
          )}
        </section>
      </div>
    </div>
  );
}

function ScheduleAgenda({ groups }: { groups: ScheduleAgendaGroup[] }) {
  const [collapsed, setCollapsed] = useState(false);
  const [collapsedDateKeys, setCollapsedDateKeys] = useState<Set<string>>(
    () => new Set(),
  );

  if (groups.length === 0) {
    return null;
  }

  const toggleDateKey = (dateKey: string) => {
    setCollapsedDateKeys((current) => {
      const next = new Set(current);
      if (next.has(dateKey)) {
        next.delete(dateKey);
      } else {
        next.add(dateKey);
      }
      return next;
    });
  };

  return (
    <ScheduleAgendaLayout
      groups={groups}
      collapsed={collapsed}
      collapsedDateKeys={collapsedDateKeys}
      onToggleCollapsed={() => setCollapsed((current) => !current)}
      onToggleDateKey={toggleDateKey}
    />
  );
}

export function ScheduleAgendaLayout({
  groups,
  collapsed,
  collapsedDateKeys,
  onToggleCollapsed,
  onToggleDateKey,
}: {
  groups: ScheduleAgendaGroup[];
  collapsed: boolean;
  collapsedDateKeys: Set<string>;
  onToggleCollapsed: () => void;
  onToggleDateKey: (dateKey: string) => void;
}) {
  if (groups.length === 0) {
    return null;
  }

  const itemCount = groups.reduce((total, group) => total + group.items.length, 0);
  const groupsId = "schedule-agenda-groups";

  return (
    <div className="schedule-agenda" aria-label="Upcoming schedule events">
      <button
        type="button"
        className="schedule-agenda-header"
        aria-expanded={!collapsed}
        aria-controls={groupsId}
        onClick={onToggleCollapsed}
      >
        <span className="schedule-agenda-chevron" aria-hidden="true" />
        <span className="schedule-agenda-title">Upcoming</span>
        <span className="schedule-agenda-count">
          {itemCount} item{itemCount === 1 ? "" : "s"}
        </span>
      </button>
      {collapsed ? null : (
        <div id={groupsId} className="schedule-agenda-groups">
          {groups.map((group) => (
            <ScheduleAgendaDateGroup
              key={group.dateKey}
              group={group}
              collapsed={collapsedDateKeys.has(group.dateKey)}
              onToggle={() => onToggleDateKey(group.dateKey)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export function ScheduleAgendaDateGroup({
  group,
  collapsed,
  onToggle,
}: {
  group: ScheduleAgendaGroup;
  collapsed: boolean;
  onToggle: () => void;
}) {
  const itemsId = `schedule-agenda-items-${group.dateKey}`;

  return (
    <div className="schedule-agenda-group">
      <button
        type="button"
        className="schedule-agenda-date"
        aria-expanded={!collapsed}
        aria-controls={itemsId}
        onClick={onToggle}
      >
        <span className="schedule-agenda-chevron" aria-hidden="true" />
        <span>{group.dateLabel}</span>
        <span className="schedule-agenda-count">
          {group.items.length} item{group.items.length === 1 ? "" : "s"}
        </span>
      </button>
      {collapsed ? null : (
        <div id={itemsId} className="schedule-agenda-items">
          {group.items.map((item) => (
            <div key={item.id} className="schedule-agenda-row">
              <time dateTime={item.startsAt}>{item.timeLabel}</time>
              <span className="schedule-agenda-label" title={item.label}>
                {item.label}
              </span>
              <span className="schedule-agenda-rule" title={item.rule}>
                {item.rule}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function shouldRenderMonitorLatestEvent(monitor: MonitorSummary) {
  return monitor.kind !== "command" && Boolean(monitor.latestEvent);
}

function GoalDetailPanel({
  action,
  actionError,
  goal,
  onCancel,
  onPause,
  onResume,
}: {
  action: GoalActionKind | null;
  actionError: string | null;
  goal: ThreadGoal | null;
  onCancel: () => void;
  onPause: () => void;
  onResume: () => void;
}) {
  const primaryAction = getGoalPrimaryAction(goal);

  return (
    <section className="context-section-card goal-detail-card">
      <div className="context-section-header">
        <div>
          <span className="context-section-eyebrow">Thread Goal</span>
          <strong>{goal ? formatGoalStatus(goal.status) : "No active goal"}</strong>
        </div>
        <div className="goal-detail-actions">
          {primaryAction ? (
            <button
              type="button"
              className="goal-detail-cancel goal-detail-primary-action"
              disabled={!!action}
              title={primaryAction.title}
              onClick={primaryAction.kind === "pause" ? onPause : onResume}
            >
              {action === primaryAction.kind
                ? formatGoalActionProgress(action)
                : primaryAction.label}
            </button>
          ) : null}
          <button
            type="button"
            className="goal-detail-cancel"
            disabled={!goal || !!action}
            title={goal ? "Cancel goal" : "No active goal"}
            onClick={onCancel}
          >
            {action === "clear" ? "Cancelling" : "Cancel"}
          </button>
        </div>
      </div>
      {goal ? (
        <>
          <p className="goal-detail-objective">{goal.objective}</p>
          <div className="goal-detail-metrics">
            <span>{formatGoalTokens(goal)}</span>
            <span>{formatGoalDuration(goal.timeUsedSeconds)}</span>
          </div>
        </>
      ) : (
        <p className="goal-detail-empty">
          No active goal.
        </p>
      )}
      {actionError ? (
        <p className="goal-detail-error" role="status">
          Could not update goal: {actionError}
        </p>
      ) : null}
    </section>
  );
}

function getGoalPrimaryAction(goal: ThreadGoal | null) {
  if (!goal) {
    return null;
  }

  switch (goal.status) {
    case "active":
      return {
        kind: "pause" as const,
        label: "Pause",
        title: "Pause goal",
      };
    case "paused":
      return {
        kind: "resume" as const,
        label: "Resume",
        title: "Resume goal",
      };
    case "budgetLimited":
    case "complete":
      return null;
  }
}

function formatGoalActionProgress(action: GoalActionKind) {
  switch (action) {
    case "set":
      return "Setting";
    case "pause":
      return "Pausing";
    case "resume":
      return "Resuming";
    case "clear":
      return "Cancelling";
  }
}

function formatGoalStatus(status: ThreadGoal["status"]) {
  switch (status) {
    case "active":
      return "Goal active";
    case "paused":
      return "Goal paused";
    case "budgetLimited":
      return "Budget limited";
    case "complete":
      return "Goal complete";
  }
}

function formatGoalTokens(goal: ThreadGoal) {
  if (goal.tokenBudget && goal.tokenBudget > 0) {
    return `${formatTokenCount(goal.tokensUsed)} / ${formatTokenCount(goal.tokenBudget)} tokens`;
  }
  return `${formatTokenCount(goal.tokensUsed)} tokens`;
}

function formatGoalDuration(seconds: number) {
  if (!Number.isFinite(seconds) || seconds <= 0) {
    return "0s";
  }
  if (seconds >= 3600) {
    return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
  }
  if (seconds >= 60) {
    return `${Math.floor(seconds / 60)}m`;
  }
  return `${Math.floor(seconds)}s`;
}

function statusClassName(status: string) {
  if (status === "Event received") {
    return "evented";
  }

  return status.toLowerCase().replace(/\s+/g, "-");
}

function CurrentPlanCard({
  planUpdate,
}: {
  planUpdate: ThreadPlanUpdate | null;
}) {
  const plan = planUpdate?.plan ?? [];
  return (
    <section
      className="context-section-card current-plan-card"
      aria-label="Current thread plan"
    >
      <div className="current-plan-header">
        <div>
          <span className="current-plan-eyebrow">Current Thread</span>
          <strong>Plan</strong>
        </div>
        {plan.length > 0 ? (
          <span className="current-plan-count">
            {plan.length} step{plan.length === 1 ? "" : "s"}
          </span>
        ) : null}
      </div>
      {planUpdate?.explanation ? (
        <p className="current-plan-explanation">{planUpdate.explanation}</p>
      ) : null}
      {plan.length > 0 ? (
        <ol className="current-plan-list">
          {plan.map((step, index) => (
            <li key={`${step.status}:${index}:${step.step}`}>
              <span className={`plan-status-dot ${planStatusClass(step)}`} />
              <span className="current-plan-step">{step.step}</span>
              <span className={`plan-status-label ${planStatusClass(step)}`}>
                {formatPlanStatus(step.status)}
              </span>
            </li>
          ))}
        </ol>
      ) : (
        <div className="current-plan-empty">No plan published yet.</div>
      )}
    </section>
  );
}

function planStatusClass(step: ThreadPlanStep) {
  return step.status;
}

function formatPlanStatus(status: ThreadPlanStep["status"]) {
  switch (status) {
    case "completed":
      return "Done";
    case "inProgress":
      return "In progress";
    case "pending":
      return "Pending";
  }
}

function GitPanel({
  changedFiles,
  thread,
}: {
  changedFiles: ThreadAnalysis["changedFiles"];
  thread: Thread | null;
}) {
  const hasProjectCwd = thread ? !isChatCompatCwd(thread.cwd) : false;
  const gitPanelRef = useRef<HTMLDivElement | null>(null);
  const [snapshot, setSnapshot] = useState<GitSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const [selectedGraphRef, setSelectedGraphRef] = useState<string | null>(null);
  const [changesCollapsed, setChangesCollapsed] = useState(false);
  const [graphPanePercent, setGraphPanePercent] = useState(66);
  const [isResizingPanes, setIsResizingPanes] = useState(false);
  const [selectedCommitHash, setSelectedCommitHash] = useState<string | null>(null);
  const [commitFilesByHash, setCommitFilesByHash] = useState<
    Record<string, GitCommitFilesSnapshot | undefined>
  >({});
  const [commitFilesLoadingByHash, setCommitFilesLoadingByHash] = useState<Record<string, boolean>>(
    {},
  );
  const lastGitCwd = useRef<string | null>(null);
  const gitRequestScope = useRef(0);
  const graphScroll = useDragScroll<HTMLDivElement>();
  const changesScroll = useDragScroll<HTMLDivElement>();

  useEffect(() => {
    let cancelled = false;
    const currentCwd = thread?.cwd ?? null;
    if (lastGitCwd.current !== currentCwd) {
      lastGitCwd.current = currentCwd;
      if (selectedGraphRef) {
        setSnapshot(null);
        setSelectedCommitHash(null);
        setCommitFilesByHash({});
        setCommitFilesLoadingByHash({});
        setSelectedGraphRef(null);
        return;
      }
    }
    gitRequestScope.current += 1;
    setSnapshot(null);
    setSelectedCommitHash(null);
    setCommitFilesByHash({});
    setCommitFilesLoadingByHash({});

    if (!thread || !hasProjectCwd) {
      setLoading(false);
      return;
    }

    setLoading(true);
    window.codexDesktop
      .readGitSnapshot(thread.cwd, selectedGraphRef ? { ref: selectedGraphRef } : undefined)
      .then((nextSnapshot) => {
        if (!cancelled) {
          setSnapshot(nextSnapshot);
          if (selectedGraphRef && !nextSnapshot.selectedRef) {
            setSelectedGraphRef(null);
          }
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setSnapshot({
            available: false,
            root: null,
            branch: null,
            selectedRef: null,
            refs: [],
            graph: [],
            changes: [],
            error: error instanceof Error ? error.message : "Failed to read Git status.",
          });
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [hasProjectCwd, refreshKey, selectedGraphRef, thread?.cwd]);

  const stagedChanges = snapshot?.changes.filter((change) => change.staged) ?? [];
  const unstagedChanges = snapshot?.changes.filter((change) => change.unstaged) ?? [];
  const changeCount = snapshot?.changes.length ?? changedFiles.length;
  const graphCommitCount = snapshot?.graph.filter(isGitGraphCommit).length ?? 0;
  const branchLabel = snapshot?.selectedRef ?? snapshot?.branch ?? "Auto";

  function loadCommitFiles(hash: string) {
    if (!thread || !hasProjectCwd || commitFilesByHash[hash] || commitFilesLoadingByHash[hash]) {
      return;
    }

    const scope = gitRequestScope.current;
    setCommitFilesLoadingByHash((current) => ({ ...current, [hash]: true }));
    window.codexDesktop
      .readGitCommitFiles(thread.cwd, hash)
      .then((filesSnapshot) => {
        if (gitRequestScope.current === scope) {
          setCommitFilesByHash((current) => ({ ...current, [hash]: filesSnapshot }));
        }
      })
      .catch((error) => {
        if (gitRequestScope.current === scope) {
          setCommitFilesByHash((current) => ({
            ...current,
            [hash]: {
              available: false,
              files: [],
              error: error instanceof Error ? error.message : "Failed to read commit files.",
            },
          }));
        }
      })
      .finally(() => {
        if (gitRequestScope.current === scope) {
          setCommitFilesLoadingByHash((current) => ({ ...current, [hash]: false }));
        }
      });
  }

  function toggleSelectedCommit(commit: GitGraphCommit) {
    setSelectedCommitHash((current) => {
      if (current === commit.hash) {
        return null;
      }
      loadCommitFiles(commit.hash);
      return commit.hash;
    });
  }

  function updatePaneSplit(clientY: number) {
    const panel = gitPanelRef.current;
    if (!panel) {
      return;
    }
    const rect = panel.getBoundingClientRect();
    const nextPercent = ((clientY - rect.top) / Math.max(1, rect.height)) * 100;
    setGraphPanePercent(Math.min(82, Math.max(35, nextPercent)));
  }

  function handlePaneSplitterPointerDown(event: React.PointerEvent<HTMLDivElement>) {
    if (changesCollapsed) {
      return;
    }
    event.currentTarget.setPointerCapture(event.pointerId);
    setIsResizingPanes(true);
    updatePaneSplit(event.clientY);
    event.preventDefault();
  }

  function handlePaneSplitterPointerMove(event: React.PointerEvent<HTMLDivElement>) {
    if (!isResizingPanes) {
      return;
    }
    updatePaneSplit(event.clientY);
    event.preventDefault();
  }

  function handlePaneSplitterPointerEnd(event: React.PointerEvent<HTMLDivElement>) {
    if (isResizingPanes && event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setIsResizingPanes(false);
  }

  function handlePaneSplitterKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowUp") {
      setGraphPanePercent((value) => Math.max(35, value - 4));
      event.preventDefault();
    } else if (event.key === "ArrowDown") {
      setGraphPanePercent((value) => Math.min(82, value + 4));
      event.preventDefault();
    }
  }

  return (
    <div
      ref={gitPanelRef}
      className={`preview-panel git-panel ${isResizingPanes ? "resizing-panes" : ""}`}
    >
      <section
        className={`git-section git-graph-section ${changesCollapsed ? "changes-collapsed" : ""}`}
        aria-label="Git graph"
        style={!changesCollapsed ? { flexBasis: `${graphPanePercent}%` } : undefined}
      >
        <GitSectionHeader
          graphToolbar
          count={0}
          title="Graph"
          trailing={
            <>
              <label className="git-ref-select-label" title="Select Git ref">
                <BranchIcon />
                <select
                  className="git-ref-select"
                  value={selectedGraphRef ?? ""}
                  disabled={!thread || !hasProjectCwd || loading || !snapshot?.available}
                  onChange={(event) => {
                    setSelectedGraphRef(event.target.value || null);
                  }}
                  aria-label="Select Git branch or ref"
                  title={branchLabel}
                >
                  <option value="">Auto</option>
                  {(snapshot?.refs ?? []).map((ref) => (
                    <option key={ref.name} value={ref.name}>
                      {ref.head ? "* " : ""}
                      {ref.name}
                    </option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                className="git-icon-button"
                disabled
                title="Focus current ref"
                aria-label="Focus current Git ref"
              >
                <BrowserIcon />
              </button>
              <button
                type="button"
                className="git-icon-button"
                disabled
                title="Fetch"
                aria-label="Fetch Git refs"
              >
                <ArrowLeftIcon />
              </button>
              <button
                type="button"
                className="git-icon-button"
                disabled
                title="Pull"
                aria-label="Pull Git refs"
              >
                <ArrowRightIcon />
              </button>
              <button
                type="button"
                className="git-icon-button"
                disabled={!thread || !hasProjectCwd || loading}
                onClick={() => setRefreshKey((value) => value + 1)}
                title="Refresh Git view"
                aria-label="Refresh Git view"
              >
                <RefreshIcon />
              </button>
              <button
                type="button"
                className="git-icon-button"
                disabled
                title="More Git actions"
                aria-label="More Git actions"
              >
                <MoreIcon />
              </button>
            </>
          }
        />
        <div className="git-graph-list drag-scroll-region" {...graphScroll}>
          {!thread ? (
            <GitEmptyState message="Select a thread to inspect its repository." />
          ) : !hasProjectCwd ? (
            <GitEmptyState message="This thread has no project workspace." />
          ) : loading && !snapshot ? (
            <GitEmptyState message="Loading Git graph..." />
          ) : snapshot?.available === false ? (
            <GitEmptyState message={snapshot.error ?? "Git is unavailable for this workspace."} />
          ) : snapshot && graphCommitCount > 0 ? (
            <GitGraphVisualList
              commitFilesByHash={commitFilesByHash}
              commitFilesLoadingByHash={commitFilesLoadingByHash}
              graph={snapshot.graph}
              onToggleCommit={toggleSelectedCommit}
              selectedCommitHash={selectedCommitHash}
            />
          ) : (
            <GitEmptyState message="No commits found in this repository." />
          )}
        </div>
      </section>

      {!changesCollapsed ? (
        <div
          className="git-pane-splitter"
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize Git graph and changes panes"
          aria-valuemin={35}
          aria-valuemax={82}
          aria-valuenow={Math.round(graphPanePercent)}
          tabIndex={0}
          onPointerDown={handlePaneSplitterPointerDown}
          onPointerMove={handlePaneSplitterPointerMove}
          onPointerUp={handlePaneSplitterPointerEnd}
          onPointerCancel={handlePaneSplitterPointerEnd}
          onKeyDown={handlePaneSplitterKeyDown}
        />
      ) : null}

      <section
        className={`git-section git-changes-section ${changesCollapsed ? "collapsed" : ""}`}
        aria-label="Git changes"
        style={!changesCollapsed ? { flexBasis: `${100 - graphPanePercent}%` } : undefined}
      >
        <GitSectionHeader
          collapsed={changesCollapsed}
          count={changeCount}
          onToggle={() => setChangesCollapsed((collapsed) => !collapsed)}
          title="Changes"
          trailing={
            <button
              type="button"
              className="git-icon-button"
              disabled={!thread || !hasProjectCwd || loading}
              onClick={() => setRefreshKey((value) => value + 1)}
              title="Refresh changes"
              aria-label="Refresh changes"
            >
              <RefreshIcon />
            </button>
          }
        />
        {!changesCollapsed ? (
          <div className="git-changes-list drag-scroll-region" {...changesScroll}>
            {loading && !snapshot ? (
              <GitEmptyState message="Loading changes..." />
            ) : snapshot?.available === false ? (
              <GitEmptyState message={snapshot.error ?? "Git is unavailable for this workspace."} />
            ) : snapshot ? (
              <>
                <GitChangeGroup changes={stagedChanges} mode="staged" title="Staged Changes" />
                <GitChangeGroup changes={unstagedChanges} mode="unstaged" title="Changes" />
              </>
            ) : (
              <GitEmptyState message="Select a Git repository to inspect changes." />
            )}
          </div>
        ) : null}
      </section>
    </div>
  );
}

function isGitGraphCommit(item: GitGraphItem): item is GitGraphCommit {
  return item.type === "commit";
}

function GitSectionHeader({
  collapsed,
  count,
  graphToolbar,
  onToggle,
  title,
  trailing,
}: {
  collapsed?: boolean;
  count: number;
  graphToolbar?: boolean;
  onToggle?: () => void;
  title: string;
  trailing?: ReactNode;
}) {
  return (
    <div className={`git-section-header ${graphToolbar ? "graph-toolbar" : ""}`}>
      <div className="git-section-title">
        {onToggle ? (
          <button
            type="button"
            className="git-section-toggle"
            onClick={onToggle}
            aria-expanded={!collapsed}
            title={`${collapsed ? "Expand" : "Collapse"} ${title}`}
          >
            <ChevronDownIcon />
          </button>
        ) : (
          <span className="git-section-marker" aria-hidden="true">
            <ChevronDownIcon />
          </span>
        )}
        <span>{title}</span>
        {count > 0 ? <span className="git-section-count">{count}</span> : null}
      </div>
      {trailing ? <div className="git-section-actions">{trailing}</div> : null}
    </div>
  );
}

function GitGraphVisualList({
  commitFilesByHash,
  commitFilesLoadingByHash,
  graph,
  onToggleCommit,
  selectedCommitHash,
}: {
  commitFilesByHash: Record<string, GitCommitFilesSnapshot | undefined>;
  commitFilesLoadingByHash: Record<string, boolean>;
  graph: GitGraphItem[];
  onToggleCommit: (commit: GitGraphCommit) => void;
  selectedCommitHash: string | null;
}) {
  const selectedCommitExtraHeight = selectedCommitHash
    ? estimateGitCommitFilesBlockHeight(
        commitFilesByHash[selectedCommitHash],
        Boolean(commitFilesLoadingByHash[selectedCommitHash]),
      )
    : 0;
  const visualModel = buildGitGraphVisualModel(graph, {
    expandedHeightsByHash:
      selectedCommitHash && selectedCommitExtraHeight > 0
        ? { [selectedCommitHash]: selectedCommitExtraHeight }
        : undefined,
  });
  return (
    <div
      className="git-graph-visual-stack"
      style={
        {
          "--git-graph-visual-width": `${visualModel.width}px`,
          "--git-graph-visual-height": `${visualModel.height}px`,
        } as React.CSSProperties
      }
    >
      <svg
        className="git-graph-overlay"
        width={visualModel.width}
        height={visualModel.height}
        viewBox={`0 0 ${visualModel.width} ${visualModel.height}`}
        aria-hidden="true"
        data-graph-overlay="true"
      >
        {visualModel.paths.map((path) => (
          <path
            key={path.id}
            className={`git-graph-path ${path.kind} lane-${path.colorLane % 6}`}
            d={path.d}
            data-graph-path-kind={path.kind}
          />
        ))}
        {visualModel.commits.map((visualCommit) => (
          <circle
            key={`dot:${visualCommit.commit.hash}`}
            className={`git-graph-dot lane-${visualCommit.colorLane % 6} ${
              visualCommit.lane === 0 ? "main" : "branch"
            } ${selectedCommitHash === visualCommit.commit.hash ? "selected" : ""}`}
            cx={visualCommit.x}
            cy={visualCommit.y}
            r={visualCommit.lane === 0 ? 4 : 4.25}
            data-graph-dot-lane={visualCommit.lane}
          />
        ))}
      </svg>
      {visualModel.commits.map((visualCommit) => (
        <GitGraphRow
          key={visualCommit.commit.hash}
          commit={visualCommit.commit}
          filesSnapshot={commitFilesByHash[visualCommit.commit.hash]}
          isLoadingFiles={Boolean(commitFilesLoadingByHash[visualCommit.commit.hash])}
          isSelected={selectedCommitHash === visualCommit.commit.hash}
          onToggle={() => onToggleCommit(visualCommit.commit)}
        />
      ))}
    </div>
  );
}

function GitGraphRow({
  commit,
  filesSnapshot,
  isLoadingFiles,
  isSelected,
  onToggle,
}: {
  commit: GitGraphCommit;
  filesSnapshot?: GitCommitFilesSnapshot;
  isLoadingFiles: boolean;
  isSelected: boolean;
  onToggle: () => void;
}) {
  const headRef = commit.refs.find((ref) => ref.startsWith("HEAD -> "));
  const otherRefs = commit.refs.filter((ref) => ref !== headRef).slice(0, 3);
  const extraRefCount = Math.max(0, commit.refs.length - (headRef ? 1 : 0) - otherRefs.length);
  const isMerge = commit.parents.length > 1;
  return (
    <article
      className={`git-graph-row ${isMerge ? "merge" : ""} ${isSelected ? "selected" : ""}`}
      role="button"
      tabIndex={0}
      aria-expanded={isSelected}
      aria-label={`${isSelected ? "Hide" : "Show"} files changed by ${commit.shortHash}`}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onToggle();
        }
      }}
    >
      <div
        className="git-graph-row-main"
        onClick={(event) => {
          if (isSuppressedDragScrollClick(event.target)) {
            return;
          }
          onToggle();
        }}
      >
        <div className="git-graph-lanes" aria-hidden="true" data-drag-scroll-handle="true">
          <span className="git-graph-lane-placeholder" />
        </div>
        <div className="git-graph-copy">
          <div className="git-graph-subject-line">
            <strong title={commit.subject}>{commit.subject}</strong>
            {headRef ? (
              <span className="git-head-ref" title={headRef}>
                <span className="git-head-ref-target" aria-hidden="true" />
                {headRef.replace("HEAD -> ", "")}
              </span>
            ) : null}
            {otherRefs.map((ref) => (
              <span key={ref} className="git-ref-pill" title={ref}>
                {ref}
              </span>
            ))}
            {extraRefCount > 0 ? <span className="git-ref-more">+{extraRefCount}</span> : null}
          </div>
          <span className="git-graph-meta">
            {commit.author} · {commit.relativeTime || "recently"} · {commit.shortHash}
            {isMerge ? ` · merge ${commit.parents.length}` : ""}
          </span>
        </div>
        <span className="git-graph-row-affordance" aria-hidden="true">
          <DocumentIcon />
        </span>
      </div>
      {isSelected ? (
        <GitCommitFileList filesSnapshot={filesSnapshot} isLoading={isLoadingFiles} />
      ) : null}
    </article>
  );
}

function GitCommitFileList({
  filesSnapshot,
  isLoading,
}: {
  filesSnapshot?: GitCommitFilesSnapshot;
  isLoading: boolean;
}) {
  if (isLoading && !filesSnapshot) {
    return <div className="git-commit-files-state">Loading commit files...</div>;
  }
  if (filesSnapshot?.available === false) {
    return (
      <div className="git-commit-files-state error">
        {filesSnapshot.error ?? "Failed to read commit files."}
      </div>
    );
  }
  if (!filesSnapshot || filesSnapshot.files.length === 0) {
    return <div className="git-commit-files-state">No file changes in this commit.</div>;
  }

  return (
    <div className="git-commit-files" aria-label="Commit changed files">
      {filesSnapshot.files.map((file) => (
        <GitCommitFileRow key={`${file.status}:${file.path}:${file.originalPath ?? ""}`} file={file} />
      ))}
    </div>
  );
}

function GitCommitFileRow({ file }: { file: GitCommitFile }) {
  const directory = directoryName(file.path);
  return (
    <article className="git-commit-file-row">
      <span className={`git-file-kind ${gitStatusClass(file.status)}`} aria-hidden="true">
        {fileIconLabel(file.path)}
      </span>
      <div className="git-commit-file-copy">
        <strong title={file.path}>{baseName(file.path)}</strong>
        <span title={file.path}>{directory}</span>
        {file.originalPath ? <span title={file.originalPath}>from {file.originalPath}</span> : null}
      </div>
      <span className={`git-commit-file-status ${gitStatusClass(file.status)}`}>
        {gitStatusLabel(file.status)}
      </span>
    </article>
  );
}

function fileIconLabel(path: string) {
  const extension = path.split(".").pop()?.toUpperCase() ?? "";
  return extension && extension.length <= 3 ? extension.slice(0, 2) : "F";
}

export function buildGitGraphVisualModel(
  graph: GitGraphItem[],
  options: GitGraphVisualOptions | number = {},
): GitGraphVisualModel {
  const rowHeight = typeof options === "number" ? options : options.rowHeight ?? GIT_GRAPH_COMMIT_ROW_HEIGHT;
  const expandedHeightsByHash = typeof options === "number" ? undefined : options.expandedHeightsByHash;
  const commits = graph.filter(isGitGraphCommit);
  let nextRowTop = 0;
  const visualCommits = commits.map((commit, rowIndex): GitGraphVisualCommit => {
    const lane = gitGraphVisualLane(commit);
    const visualCommit = {
      commit,
      rowIndex,
      lane,
      colorLane: lane,
      x: laneCenterX(lane),
      y: nextRowTop + rowHeight / 2,
    };
    nextRowTop += rowHeight + (expandedHeightsByHash?.[commit.hash] ?? 0);
    return visualCommit;
  });
  const maxLane = Math.max(0, ...visualCommits.map((commit) => commit.lane));
  const height = Math.max(rowHeight, nextRowTop);
  const paths: GitGraphVisualPath[] = [];

  if (visualCommits.length > 0) {
    const firstY = visualCommits[0].y;
    const lastY = visualCommits[visualCommits.length - 1].y;
    paths.push({
      id: "main-spine",
      lane: 0,
      colorLane: 0,
      kind: "main",
      d: `M ${laneCenterX(0)} ${firstY} L ${laneCenterX(0)} ${lastY}`,
    });
  }

  let index = 0;
  while (index < visualCommits.length) {
    const current = visualCommits[index];
    if (current.lane === 0) {
      index += 1;
      continue;
    }

    const runStart = index;
    const lane = current.lane;
    while (index < visualCommits.length && visualCommits[index].lane === lane) {
      index += 1;
    }
    const run = visualCommits.slice(runStart, index);
    const startAnchor = findMainlineCommitBefore(visualCommits, runStart);
    const endAnchor = findMainlineCommitAfter(visualCommits, index);
    paths.push({
      id: `branch:${runStart}:${index}:${lane}`,
      lane,
      colorLane: lane,
      kind: "branch",
      d: buildBranchPath(run, startAnchor, endAnchor, rowHeight),
    });
  }

  return {
    width: laneCenterX(maxLane) + GIT_GRAPH_LANE_WIDTH / 2,
    height,
    commits: visualCommits,
    paths,
  };
}

function estimateGitCommitFilesBlockHeight(
  filesSnapshot: GitCommitFilesSnapshot | undefined,
  isLoadingFiles: boolean,
) {
  if (isLoadingFiles && !filesSnapshot) {
    return GIT_COMMIT_FILES_STATE_HEIGHT + GIT_COMMIT_FILES_BLOCK_MARGIN_BOTTOM;
  }
  if (!filesSnapshot || filesSnapshot.available === false || filesSnapshot.files.length === 0) {
    return GIT_COMMIT_FILES_STATE_HEIGHT + GIT_COMMIT_FILES_BLOCK_MARGIN_BOTTOM;
  }
  return filesSnapshot.files.length * GIT_COMMIT_FILE_ROW_HEIGHT + GIT_COMMIT_FILES_BLOCK_MARGIN_BOTTOM;
}

function gitGraphVisualLane(commit: GitGraphCommit) {
  if (commit.parents.length > 1) {
    return 0;
  }
  const markerIndex = [...commit.graph].findIndex((char) => char === "*");
  if (markerIndex <= 0) {
    return 0;
  }
  const occupiedLaneCount = [...commit.graph.slice(0, markerIndex)].filter(
    (char) => char.trim().length > 0,
  ).length;
  return Math.min(Math.max(0, occupiedLaneCount), 3);
}

function findMainlineCommitBefore(commits: GitGraphVisualCommit[], beforeIndex: number) {
  for (let index = beforeIndex - 1; index >= 0; index -= 1) {
    if (commits[index].lane === 0) {
      return commits[index];
    }
  }
  return null;
}

function findMainlineCommitAfter(commits: GitGraphVisualCommit[], afterIndex: number) {
  for (let index = afterIndex; index < commits.length; index += 1) {
    if (commits[index].lane === 0) {
      return commits[index];
    }
  }
  return null;
}

function buildBranchPath(
  run: GitGraphVisualCommit[],
  startAnchor: GitGraphVisualCommit | null,
  endAnchor: GitGraphVisualCommit | null,
  rowHeight: number,
) {
  const first = run[0];
  const last = run[run.length - 1];
  const spineX = laneCenterX(0);
  const branchX = first.x;
  const startY = startAnchor?.y ?? first.y - rowHeight;
  const endY = endAnchor?.y ?? last.y + rowHeight;
  const pull = Math.max(GIT_GRAPH_LANE_WIDTH, (branchX - spineX) * 0.72);
  const commands = [
    `M ${spineX} ${startY}`,
    `C ${spineX + pull} ${startY} ${branchX} ${first.y - rowHeight * 0.34} ${branchX} ${first.y}`,
  ];

  for (const nextCommit of run.slice(1)) {
    commands.push(`L ${branchX} ${nextCommit.y}`);
  }

  commands.push(
    `C ${branchX} ${last.y + rowHeight * 0.34} ${spineX + pull} ${endY} ${spineX} ${endY}`,
  );
  return commands.join(" ");
}

function laneCenterX(lane: number) {
  return lane * GIT_GRAPH_LANE_WIDTH + GIT_GRAPH_SPINE_X;
}

function GitChangeGroup({
  changes,
  mode,
  title,
}: {
  changes: GitChange[];
  mode: "staged" | "unstaged";
  title: string;
}) {
  return (
    <div className="git-change-group">
      <div className="git-change-group-header" data-drag-scroll-handle="true">
        <ChevronDownIcon />
        <span>{title}</span>
        <span className="git-change-count">{changes.length}</span>
      </div>
      {changes.length > 0 ? (
        <div className="git-change-rows">
          {changes.map((change) => (
            <GitChangeRow
              key={`${title}:${change.path}:${change.originalPath ?? ""}`}
              change={change}
              mode={mode}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function GitChangeRow({ change, mode }: { change: GitChange; mode: "staged" | "unstaged" }) {
  const status =
    (mode === "staged" ? change.stagedStatus : change.unstagedStatus) ??
    change.stagedStatus ??
    change.unstagedStatus ??
    "M";
  const directory = directoryName(change.path);
  return (
    <article className="git-change-row">
      <span className={`git-status-letter ${gitStatusClass(status)}`} data-drag-scroll-handle="true">
        {gitStatusLabel(status)}
      </span>
      <div className="git-change-copy">
        <strong title={change.path}>{baseName(change.path)}</strong>
        <span title={change.path}>{directory}</span>
      </div>
      <span className={`git-change-state ${gitStatusClass(status)}`} data-drag-scroll-handle="true">
        {gitStatusLabel(status)}
      </span>
    </article>
  );
}

function GitEmptyState({ message }: { message: string }) {
  return (
    <div className="git-empty-state" role="status">
      {message}
    </div>
  );
}

function useDragScroll<T extends HTMLElement>() {
  const ref = useRef<T | null>(null);
  const dragState = useRef<{
    dragging: boolean;
    startX: number;
    startY: number;
    scrollLeft: number;
    scrollTop: number;
  } | null>(null);

  function endDrag() {
    dragState.current = null;
    ref.current?.classList.remove("is-dragging");
  }

  return {
    ref,
    onMouseDown(event: React.MouseEvent<T>) {
      const element = ref.current;
      if (element) {
        delete element.dataset.dragScrollSuppressed;
      }
      if (
        !element ||
        event.button !== 0 ||
        isInteractiveDragTarget(event.target) ||
        !isDragScrollHandle(event.target, element)
      ) {
        return;
      }
      dragState.current = {
        dragging: false,
        startX: event.clientX,
        startY: event.clientY,
        scrollLeft: element.scrollLeft,
        scrollTop: element.scrollTop,
      };
    },
    onMouseMove(event: React.MouseEvent<T>) {
      const state = dragState.current;
      const element = ref.current;
      if (!state || !element) {
        return;
      }
      const deltaX = event.clientX - state.startX;
      const deltaY = event.clientY - state.startY;
      if (!state.dragging && Math.hypot(deltaX, deltaY) < 4) {
        return;
      }
      if (!state.dragging) {
        state.dragging = true;
        element.classList.add("is-dragging");
        element.dataset.dragScrollSuppressed = "true";
      }
      element.scrollLeft = state.scrollLeft - deltaX;
      element.scrollTop = state.scrollTop - deltaY;
      event.preventDefault();
    },
    onMouseLeave: endDrag,
    onMouseUp: endDrag,
  };
}

function isInteractiveDragTarget(target: EventTarget) {
  return target instanceof Element && Boolean(target.closest("button,a,input,textarea,select"));
}

function isDragScrollHandle(target: EventTarget, scrollRoot: HTMLElement) {
  return (
    target instanceof Element &&
    (target === scrollRoot || Boolean(target.closest("[data-drag-scroll-handle]")))
  );
}

function isSuppressedDragScrollClick(target: EventTarget) {
  return (
    target instanceof Element &&
    target.closest(".drag-scroll-region") instanceof HTMLElement &&
    target.closest(".drag-scroll-region")?.getAttribute("data-drag-scroll-suppressed") === "true"
  );
}

export function resolveMarkdownPreviewLocalFileTarget(
  previewPath: string,
  target: string,
) {
  if (!isRelativeMarkdownPreviewTarget(target)) {
    return target;
  }

  const lastSlash = Math.max(previewPath.lastIndexOf("/"), previewPath.lastIndexOf("\\"));
  if (lastSlash < 0) {
    return target;
  }

  const baseDir = lastSlash === 0 ? previewPath.slice(0, 1) : previewPath.slice(0, lastSlash);
  const separator = previewPath.includes("\\") && !previewPath.includes("/") ? "\\" : "/";
  return normalizeRelativeFileTarget(baseDir, target, separator);
}

function isRelativeMarkdownPreviewTarget(target: string) {
  return /^\.{1,2}[\\/]/.test(target);
}

function normalizeRelativeFileTarget(baseDir: string, target: string, separator: "/" | "\\") {
  const driveMatch = baseDir.match(/^([A-Za-z]:)(.*)$/);
  const isUncPath = baseDir.startsWith("\\\\");
  const isAbsolutePosixPath = baseDir.startsWith("/");
  const prefix = driveMatch
    ? `${driveMatch[1]}${separator}`
    : isUncPath
      ? "\\\\"
      : isAbsolutePosixPath
        ? "/"
        : "";
  const baseWithoutPrefix = driveMatch
    ? driveMatch[2]
    : isUncPath
      ? baseDir.replace(/^\\+/, "")
      : isAbsolutePosixPath
        ? baseDir.slice(1)
        : baseDir;
  const parts = baseWithoutPrefix.split(/[\\/]+/).filter(Boolean);

  for (const part of target.split(/[\\/]+/)) {
    if (!part || part === ".") {
      continue;
    }
    if (part === "..") {
      if (parts.length > 0) {
        parts.pop();
      }
      continue;
    }
    parts.push(part);
  }

  return `${prefix}${parts.join(separator)}`;
}

export function filePreviewRenderMode(preview: FilePreview | null) {
  if (!preview) {
    return "empty";
  }
  if (preview.image) {
    return "image";
  }
  if (preview.language === "markdown") {
    return "markdown";
  }
  return "editor";
}

function FilePreviewPanel({
  expandedTreeDirectories,
  filePanelView,
  fileTreeEntriesByPath,
  fileTreeErrorsByPath,
  fileTreeLoadingPath,
  onNavigateToSymbol,
  onOpenPreviewExternally,
  onOpenTreeFile,
  onSetFilePanelView,
  onToggleTreeDirectory,
  preview,
  previewError,
  previewLoading,
  thread,
}: {
  expandedTreeDirectories: string[];
  filePanelView: FilePanelView;
  fileTreeEntriesByPath: Record<string, FileTreeEntry[]>;
  fileTreeErrorsByPath: Record<string, string>;
  fileTreeLoadingPath: string | null;
  onNavigateToSymbol: (destination: FileLocation, sourceLocation: FileLocation) => void;
  onOpenPreviewExternally: () => void;
  onOpenTreeFile: (path: string) => void;
  onSetFilePanelView: (value: FilePanelView) => void;
  onToggleTreeDirectory: (path: string) => void;
  preview: FilePreview | null;
  previewError: string | null;
  previewLoading: boolean;
  thread: Thread | null;
}) {
  const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
  const hoverPositionRef = useRef<Monaco.Position | null>(null);
  const decorationCollectionRef = useRef<Monaco.editor.IEditorDecorationsCollection | null>(null);
  const pendingDefinitionRef = useRef(false);
  const modifierPressedRef = useRef(false);
  const previewEnabledRef = useRef(preview?.lsp.enabled ?? false);

  useEffect(() => {
    previewEnabledRef.current = preview?.lsp.enabled ?? false;
  }, [preview?.lsp.enabled]);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) {
      return;
    }

    editor.revealPositionInCenter({
      lineNumber: preview?.line ?? 1,
      column: preview?.column ?? 1,
    });
    editor.setPosition({
      lineNumber: preview?.line ?? 1,
      column: preview?.column ?? 1,
    });
  }, [preview?.column, preview?.line, preview?.path]);

  useEffect(() => {
    function handleModifierKey(event: KeyboardEvent) {
      modifierPressedRef.current = event.metaKey || event.ctrlKey;
      updateLinkDecoration(editorRef.current, decorationCollectionRef.current, {
        enabled: previewEnabledRef.current,
        modifierPressed: modifierPressedRef.current,
        position: hoverPositionRef.current,
      });
    }

    function clearModifierState() {
      modifierPressedRef.current = false;
      updateLinkDecoration(editorRef.current, decorationCollectionRef.current, {
        enabled: previewEnabledRef.current,
        modifierPressed: false,
        position: hoverPositionRef.current,
      });
    }

    window.addEventListener("keydown", handleModifierKey);
    window.addEventListener("keyup", handleModifierKey);
    window.addEventListener("blur", clearModifierState);

    return () => {
      window.removeEventListener("keydown", handleModifierKey);
      window.removeEventListener("keyup", handleModifierKey);
      window.removeEventListener("blur", clearModifierState);
    };
  }, []);

  const threadRootPath =
    thread && !isChatCompatCwd(thread.cwd) ? thread.cwd : null;
  const previewRenderMode = filePreviewRenderMode(preview);

  return (
    <div className="preview-panel">
      <header className="panel-content-header preview-header">
        <div className="panel-content-copy">
          <span className="panel-eyebrow">File Preview</span>
          <h2>
            {filePanelView === "tree"
              ? (threadRootPath ? trimPath(threadRootPath) : "Workspace Browser")
              : preview
                ? preview.displayPath
                : "Linked Context"}
          </h2>
        </div>
        <div className="preview-header-actions">
          <div className="preview-mode-toggle" role="tablist" aria-label="File panel mode">
            <button
              type="button"
              role="tab"
              aria-selected={filePanelView === "preview"}
              className={filePanelView === "preview" ? "active" : ""}
              onClick={() => onSetFilePanelView("preview")}
            >
              Preview
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={filePanelView === "tree"}
              className={filePanelView === "tree" ? "active" : ""}
              onClick={() => onSetFilePanelView("tree")}
            >
              CWD Tree
            </button>
          </div>
          <button
            type="button"
            className="panel-inline-action preview-open-button"
            aria-label="Open preview in system editor"
            onClick={onOpenPreviewExternally}
            disabled={!preview || filePanelView !== "preview"}
          >
            <OpenIcon />
          </button>
        </div>
      </header>

      {filePanelView === "tree" ? (
        <CwdFileTreePanel
          entriesByPath={fileTreeEntriesByPath}
          errorsByPath={fileTreeErrorsByPath}
          expandedDirectories={expandedTreeDirectories}
          loadingPath={fileTreeLoadingPath}
          onOpenFile={onOpenTreeFile}
          onToggleDirectory={onToggleTreeDirectory}
          rootPath={threadRootPath}
          hasThread={thread !== null}
        />
      ) : null}
      {filePanelView === "tree" ? null : (
        <>
      {previewLoading ? <div className="preview-empty">Loading file…</div> : null}
      {!previewLoading && previewError ? <div className="preview-empty">{previewError}</div> : null}
      {!previewLoading && !previewError && !preview ? (
        <div className="preview-empty">
          <p>Open a local file link in the conversation to pin code context here.</p>
        </div>
      ) : null}
      {!previewLoading && !previewError && previewRenderMode === "image" && preview?.image ? (
        <div className="preview-editor-shell preview-image-shell">
          <div className="preview-utility-strip">
            <div className="preview-utility-primary">
              <span className="preview-signal plain" />
              <button type="button" className="preview-lsp-button plain" disabled>
                IMAGE
              </button>
            </div>
            <div className="preview-utility-secondary">
              <span>{preview.image.mimeType}</span>
              <span className="preview-utility-separator">•</span>
              <span>{formatByteSize(preview.image.byteSize)}</span>
              <span className="preview-utility-separator">•</span>
              <span className="preview-utility-cwd">{preview.image.name}</span>
            </div>
          </div>
          <div className="preview-image-pad">
            <LocalImagePreview
              path={preview.image.path}
              label={preview.image.name}
              className="preview-image"
            />
          </div>
        </div>
      ) : null}
      {!previewLoading && !previewError && preview && previewRenderMode !== "image" ? (
        <div className="preview-editor-shell">
          <div className="preview-utility-strip">
            <div className="preview-utility-primary">
              <span className={`preview-signal ${previewLspState(preview)}`} />
              <button
                type="button"
                className={`preview-lsp-button ${previewLspState(preview)}`}
                disabled
              >
                {preview.lsp.lspStatus.phase.toUpperCase()}
              </button>
            </div>
            <div className="preview-utility-secondary">
              <span>{preview.language}</span>
              <span className="preview-utility-separator">•</span>
              <span>{preview.lsp.lspStatus.detail ?? preview.lsp.serverLabel ?? "LSP idle"}</span>
              <span className="preview-utility-separator">•</span>
              <span className="preview-utility-cwd">
                {preview.lsp.workspaceRoot ?? "No workspace root"}
              </span>
            </div>
          </div>
          {previewRenderMode === "markdown" ? (
            <div className="preview-markdown-pad">
              <MarkdownContent
                text={preview.content}
                onOpenLocalFile={(target) =>
                  onOpenTreeFile(
                    resolveMarkdownPreviewLocalFileTarget(preview.path, target),
                  )
                }
              />
            </div>
          ) : (
            <div className="preview-editor-pad">
              <Editor
                key={`${preview.path}:${preview.line ?? 0}:${preview.column ?? 0}:${preview.lsp.enabled ? "lsp" : "plain"}`}
                height="100%"
                onMount={(editor, monaco) => {
                  editorRef.current = editor;
                  decorationCollectionRef.current = editor.createDecorationsCollection();
                  editor.revealPositionInCenter({
                    lineNumber: preview.line ?? 1,
                    column: preview.column ?? 1,
                  });
                  editor.setPosition({
                    lineNumber: preview.line ?? 1,
                    column: preview.column ?? 1,
                  });

                  editor.onMouseMove((event) => {
                    hoverPositionRef.current = event.target.position ?? null;
                    modifierPressedRef.current =
                      event.event.browserEvent.metaKey || event.event.browserEvent.ctrlKey;
                    updateLinkDecoration(editor, decorationCollectionRef.current, {
                      modifierPressed: modifierPressedRef.current,
                      enabled: preview.lsp.enabled,
                      position: hoverPositionRef.current,
                    });
                  });

                  editor.onMouseLeave(() => {
                    hoverPositionRef.current = null;
                    updateLinkDecoration(editor, decorationCollectionRef.current, {
                      enabled: preview.lsp.enabled,
                      modifierPressed: false,
                      position: null,
                    });
                  });

                  editor.onMouseDown((event) => {
                    if (
                      !preview.lsp.enabled ||
                      !event.target.position ||
                      event.event.browserEvent.button !== 0 ||
                      !(event.event.browserEvent.metaKey || event.event.browserEvent.ctrlKey) ||
                      pendingDefinitionRef.current
                    ) {
                      return;
                    }

                    pendingDefinitionRef.current = true;
                    event.event.preventDefault();
                    event.event.stopPropagation();
                    event.event.browserEvent.preventDefault();
                    event.event.browserEvent.stopPropagation();
                    const sourcePosition = resolvePreviewDefinitionPosition(
                      editor,
                      event.target.position,
                    );

                    void window.codexDesktop
                      .lspDefinition({
                        path: preview.path,
                        line: sourcePosition.lineNumber,
                        column: sourcePosition.column,
                      })
                      .then((response) => {
                        const destination = response.locations[0];
                        if (response.enabled && destination) {
                          onNavigateToSymbol(destination, {
                            path: preview.path,
                            line: sourcePosition.lineNumber,
                            column: sourcePosition.column,
                          });
                        }
                      })
                      .catch((error) => {
                        console.error("Failed to resolve definition", error);
                      })
                      .finally(() => {
                        pendingDefinitionRef.current = false;
                        updateLinkDecoration(editor, decorationCollectionRef.current, {
                          enabled: preview.lsp.enabled,
                          modifierPressed: modifierPressedRef.current,
                          position: hoverPositionRef.current,
                        });
                      });
                  });
                }}
                language={preview.language}
                loading={<div className="preview-empty">Loading editor…</div>}
                options={{
                  automaticLayout: true,
                  definitionLinkOpensInPeek: false,
                  contextmenu: false,
                  fontSize: 12,
                  lineNumbersMinChars: 3,
                  minimap: { enabled: false },
                  readOnly: true,
                  renderLineHighlight: "all",
                  roundedSelection: false,
                  scrollBeyondLastLine: false,
                  selectionHighlight: false,
                  wordWrap: "on",
                }}
                path={preview.path}
                theme="vs"
                value={preview.content}
              />
            </div>
          )}
        </div>
      ) : null}
        </>
      )}
    </div>
  );
}

function CwdFileTreePanel({
  entriesByPath,
  errorsByPath,
  expandedDirectories,
  loadingPath,
  onOpenFile,
  onToggleDirectory,
  rootPath,
  hasThread,
}: {
  entriesByPath: Record<string, FileTreeEntry[]>;
  errorsByPath: Record<string, string>;
  expandedDirectories: string[];
  loadingPath: string | null;
  onOpenFile: (path: string) => void;
  onToggleDirectory: (path: string) => void;
  rootPath: string | null;
  hasThread: boolean;
}) {
  if (!rootPath) {
    return (
      <div className="preview-empty">
        <p>
          {hasThread
            ? "This chat has no project cwd to browse."
            : "Select a thread to browse its cwd file tree."}
        </p>
      </div>
    );
  }

  const rootEntries = entriesByPath[rootPath] ?? [];
  const isLoadingRoot = loadingPath === rootPath && rootEntries.length === 0;
  const rootError = errorsByPath[rootPath] ?? null;

  if (rootError && rootEntries.length === 0) {
    return <div className="preview-empty">{rootError}</div>;
  }

  if (isLoadingRoot) {
    return <div className="preview-empty">Loading file tree…</div>;
  }

  return (
    <div className="preview-tree-shell">
      <div className="preview-utility-strip">
        <div className="preview-utility-primary">
          <span className="preview-signal ready" />
          <span className="preview-tree-label">cwd</span>
        </div>
        <div className="preview-utility-secondary">
          <span className="preview-utility-cwd">{rootPath}</span>
        </div>
      </div>
      {rootEntries.length > 0 ? (
        <div className="cwd-tree-list" role="tree" aria-label="Thread cwd file tree">
          {rootEntries.map((entry) => (
            <CwdTreeEntryRow
              key={entry.path}
              depth={0}
              entriesByPath={entriesByPath}
              entry={entry}
              errorsByPath={errorsByPath}
              expandedDirectories={expandedDirectories}
              loadingPath={loadingPath}
              onOpenFile={onOpenFile}
              onToggleDirectory={onToggleDirectory}
            />
          ))}
        </div>
      ) : (
        <div className="preview-empty">
          <p>No files found in this cwd.</p>
        </div>
      )}
      {rootError && rootEntries.length > 0 ? (
        <div className="cwd-tree-error" role="status">
          {rootError}
        </div>
      ) : null}
    </div>
  );
}

function CwdTreeEntryRow({
  depth,
  entriesByPath,
  entry,
  errorsByPath,
  expandedDirectories,
  loadingPath,
  onOpenFile,
  onToggleDirectory,
}: {
  depth: number;
  entriesByPath: Record<string, FileTreeEntry[]>;
  entry: FileTreeEntry;
  errorsByPath: Record<string, string>;
  expandedDirectories: string[];
  loadingPath: string | null;
  onOpenFile: (path: string) => void;
  onToggleDirectory: (path: string) => void;
}) {
  const isDirectory = entry.kind === "directory";
  const isExpanded = isDirectory && expandedDirectories.includes(entry.path);
  const childEntries = entriesByPath[entry.path] ?? [];
  const childError = errorsByPath[entry.path] ?? null;
  const isLoading = loadingPath === entry.path;

  return (
    <>
      <div
        className="cwd-tree-row"
        role="treeitem"
        aria-expanded={isDirectory ? isExpanded : undefined}
        style={{ paddingLeft: `${12 + depth * 16}px` }}
      >
        {isDirectory ? (
          <button
            type="button"
            className="cwd-tree-button directory"
            onClick={() => onToggleDirectory(entry.path)}
          >
            <span className={`cwd-tree-caret ${isExpanded ? "expanded" : ""}`}>
              ▸
            </span>
            <span className="cwd-tree-name">{entry.name}</span>
          </button>
        ) : (
          <button
            type="button"
            className="cwd-tree-button file"
            onClick={() => onOpenFile(entry.path)}
          >
            <DocumentIcon />
            <span className="cwd-tree-name">{entry.name}</span>
          </button>
        )}
      </div>
      {isDirectory && isExpanded ? (
        isLoading && childEntries.length === 0 ? (
          <div className="cwd-tree-loading" style={{ paddingLeft: `${28 + depth * 16}px` }}>
            Loading…
          </div>
        ) : childEntries.length > 0 ? (
          childEntries.map((child) => (
            <CwdTreeEntryRow
              key={child.path}
              depth={depth + 1}
              entriesByPath={entriesByPath}
              entry={child}
              errorsByPath={errorsByPath}
              expandedDirectories={expandedDirectories}
              loadingPath={loadingPath}
              onOpenFile={onOpenFile}
              onToggleDirectory={onToggleDirectory}
            />
          ))
        ) : childError ? (
          <div className="cwd-tree-error" style={{ paddingLeft: `${28 + depth * 16}px` }}>
            {childError}
          </div>
        ) : (
          <div className="cwd-tree-loading" style={{ paddingLeft: `${28 + depth * 16}px` }}>
            Empty directory
          </div>
        )
      ) : null}
    </>
  );
}

function formatSkillKind(kind: ThreadSkill["kind"]) {
  switch (kind) {
    case "explicit":
      return "Explicit";
    case "implicit":
      return "Implicit";
    case "all":
      return "All";
    default:
      return kind;
  }
}

function OverviewMetric({
  label,
  tone,
  value,
}: {
  label: string;
  tone: "open" | "doing" | "blocked";
  value: number;
}) {
  return (
    <div className={`overview-metric ${tone}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function buildTodoStats(todoItems: TodoCardItem[]) {
  const todo = todoItems.filter((item) => item.status === "todo").length;
  const doing = todoItems.filter((item) => item.status === "doing").length;
  const blocked = todoItems.filter((item) => item.status === "blocked").length;
  const done = todoItems.filter((item) => item.status === "done").length;
  return {
    todo,
    doing,
    blocked,
    done,
    openCount: todo + doing + blocked,
  };
}

function previewLspState(preview: FilePreview) {
  return preview.lsp.lspStatus.phase;
}

function baseName(value: string) {
  const parts = value.split("/");
  return parts[parts.length - 1] || value;
}

function directoryName(value: string) {
  const index = value.lastIndexOf("/");
  return index > 0 ? value.slice(0, index) : ".";
}

function gitStatusLabel(status: string) {
  return status === "?" ? "U" : status;
}

function gitStatusClass(status: string) {
  switch (status) {
    case "A":
      return "added";
    case "D":
      return "deleted";
    case "R":
    case "C":
      return "renamed";
    case "?":
      return "untracked";
    default:
      return "modified";
  }
}

function trimPath(value: string) {
  return value.length > 48 ? `…${value.slice(-47)}` : value;
}

function updateLinkDecoration(
  editor: Monaco.editor.IStandaloneCodeEditor | null,
  decorations: Monaco.editor.IEditorDecorationsCollection | null,
  options: {
    enabled: boolean;
    modifierPressed: boolean;
    position: Monaco.Position | null;
  },
) {
  const domNode = editor?.getDomNode();
  const model = editor?.getModel();
  if (
    !editor ||
    !decorations ||
    !domNode ||
    !model ||
    !options.enabled ||
    !options.modifierPressed ||
    !options.position
  ) {
    domNode?.classList.remove("preview-editor-link-mode");
    decorations?.set([]);
    return;
  }

  const word = model.getWordAtPosition(options.position);
  if (!word) {
    domNode.classList.remove("preview-editor-link-mode");
    decorations.set([]);
    return;
  }

  domNode.classList.add("preview-editor-link-mode");
  decorations.set([
    {
      range: {
        startLineNumber: options.position.lineNumber,
        startColumn: word.startColumn,
        endLineNumber: options.position.lineNumber,
        endColumn: word.endColumn,
      },
      options: {
        inlineClassName: "preview-symbol-link",
      },
    },
  ]);
}

export function resolvePreviewDefinitionPosition(
  editor: PreviewDefinitionPositionEditor,
  position: PreviewDefinitionPosition,
) {
  const word = editor.getModel()?.getWordAtPosition(position);
  if (!word) {
    return position;
  }

  return {
    lineNumber: position.lineNumber,
    column: Math.min(
      Math.max(position.column, word.startColumn),
      Math.max(word.startColumn, word.endColumn - 1),
    ),
  };
}
