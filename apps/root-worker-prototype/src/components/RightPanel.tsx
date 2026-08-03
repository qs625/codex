import React, { useEffect, useRef, useState, type ReactNode } from "react";
import Editor from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";

import {
  ArrowLeftIcon,
  ArrowRightIcon,
  BranchIcon,
  BrowserIcon,
  DocumentIcon,
  GridIcon,
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
    <div className="schedule-agenda" aria-label="Upcoming schedule events">
      <div className="schedule-agenda-title">Upcoming</div>
      {groups.map((group) => (
        <ScheduleAgendaDateGroup
          key={group.dateKey}
          group={group}
          collapsed={collapsedDateKeys.has(group.dateKey)}
          onToggle={() => toggleDateKey(group.dateKey)}
        />
      ))}
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
    <section className="current-plan-card" aria-label="Current thread plan">
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
  return (
    <div className="preview-panel git-panel">
      <header className="panel-content-header">
        <div className="panel-content-copy">
          <span className="panel-eyebrow">Git Changes</span>
          <h2>{thread ? "Thread File Deltas" : "Select a Thread"}</h2>
          <p>Unique files touched by this thread's recorded file change events.</p>
        </div>
      </header>

      <div className="git-summary-strip">
        <span className="git-summary-label">
          {thread ? (hasProjectCwd ? trimPath(thread.cwd) : "No project") : "No thread selected"}
        </span>
        <span className="git-summary-count">
          {changedFiles.length} file{changedFiles.length === 1 ? "" : "s"}
        </span>
      </div>

      {thread ? (
        changedFiles.length > 0 ? (
          <div className="git-file-list">
            {changedFiles.map((file) => (
              <article key={file.path} className="git-file-row">
                <div className="git-file-copy">
                  <strong title={file.path}>{file.displayPath}</strong>
                  <span title={file.path}>{file.path}</span>
                </div>
                <div className="git-file-meta">
                  <span className={`git-kind-badge ${gitKindClassName(file.kind)}`}>
                    {formatGitKind(file.kind)}
                  </span>
                  <span className="git-file-count">
                    {file.updateCount} update{file.updateCount === 1 ? "" : "s"}
                  </span>
                </div>
              </article>
            ))}
          </div>
        ) : (
          <div className="preview-empty">
            <p>No file changes recorded for this thread.</p>
          </div>
        )
      ) : (
        <div className="preview-empty">
          <p>Select a thread to inspect its changed files.</p>
        </div>
      )}
    </div>
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
                    event.event.browserEvent.preventDefault();
                    const sourcePosition = event.target.position;

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

function formatGitKind(kind: string) {
  switch (kind) {
    case "added":
      return "Added";
    case "deleted":
      return "Deleted";
    case "renamed":
      return "Renamed";
    case "modified":
    case "edited":
      return "Modified";
    default:
      return kind;
  }
}

function gitKindClassName(kind: string) {
  switch (kind) {
    case "added":
    case "deleted":
    case "renamed":
      return kind;
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
