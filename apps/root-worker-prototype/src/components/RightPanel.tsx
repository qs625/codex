import React, { useEffect, useRef, useState, type ReactNode } from "react";
import Editor from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";

import {
  BranchIcon,
  DocumentIcon,
  GridIcon,
  GearIcon,
  OpenIcon,
  SearchIcon,
} from "./icons";
import { LocalImagePreview } from "./Conversation";
import { isChatCompatCwd } from "../lib/chatCompat";
import {
  getContextUsageCategoryColor,
  type ContextUsageToolBreakdownSummary,
} from "../lib/contextUsage";
import {
  buildThreadAnalysis,
  type MonitorSummary,
  type ScheduleAgendaGroup,
  type ThreadAnalysis,
} from "../lib/threadAnalysis";
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
} from "../types";

type GoalActionKind = "set" | "pause" | "resume" | "clear";

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
  onNavigateToSymbol,
  onOpenPreviewExternally,
  onOpenTreeFile,
  onSelectCommandMonitor,
  onSetActiveView,
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
  onNavigateToSymbol: (destination: FileLocation, sourceLocation: FileLocation) => void;
  onOpenPreviewExternally: () => void;
  onOpenTreeFile: (path: string) => void;
  onSelectCommandMonitor?: (commandItemId: string) => void;
  onSetActiveView: (value: RightPanelView) => void;
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
  const { contextUsage } = threadAnalysis;

  return (
    <aside className="right-panel">
      <div className="right-panel-body">
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
                  onSetActiveView(item.view);
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
          <ToolBreakdownRows rows={contextUsage.toolBreakdown} />
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

function ToolBreakdownRows({
  rows,
}: {
  rows: ContextUsageToolBreakdownSummary[];
}) {
  if (rows.length === 0) {
    return null;
  }

  return (
    <div className="context-tool-breakdown" aria-label="Estimated tool I/O mix">
      <div className="context-tool-breakdown-title">
        <span>Tool I/O Detail</span>
        <span>estimated</span>
      </div>
      <div className="context-tool-breakdown-list">
        {rows.map((row) => (
          <div key={row.id} className="context-tool-breakdown-row">
            <div className="context-tool-breakdown-main">
              <strong>{row.label}</strong>
              <span title={row.description}>
                in {formatByteSize(row.inputUnits)} / out {formatByteSize(row.outputUnits)}
              </span>
            </div>
            <span className="context-tool-breakdown-share">{row.sharePercent}%</span>
          </div>
        ))}
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
      {!previewLoading && !previewError && preview?.image ? (
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
      {!previewLoading && !previewError && preview && !preview.image ? (
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
