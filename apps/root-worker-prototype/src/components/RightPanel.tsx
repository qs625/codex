import Editor from "@monaco-editor/react";

import {
  BranchIcon,
  ClockIcon,
  DocumentIcon,
  FilterIcon,
  GridIcon,
  OpenIcon,
  PlusIcon,
  SearchIcon,
} from "./icons";
import type { FileLocation, FilePreview, RightPanelView, TaskFilter, TodoCardItem } from "../types";

export function RightPanel({
  activeView,
  onCreateRootThread,
  onNavigateToFile,
  onOpenPreviewExternally,
  onSelectTaskThread,
  onSetActiveView,
  onSetTaskFilter,
  preview,
  previewError,
  previewLoading,
  selectedThreadId,
  taskFilter,
  todoItems,
}: {
  activeView: RightPanelView;
  onCreateRootThread: () => void;
  onNavigateToFile: (location: FileLocation) => void;
  onOpenPreviewExternally: () => void;
  onSelectTaskThread: (threadId: string) => void;
  onSetActiveView: (value: RightPanelView) => void;
  onSetTaskFilter: (value: TaskFilter) => void;
  preview: FilePreview | null;
  previewError: string | null;
  previewLoading: boolean;
  selectedThreadId: string | null;
  taskFilter: TaskFilter;
  todoItems: TodoCardItem[];
}) {
  const todoStats = buildTodoStats(todoItems);

  return (
    <aside className="right-panel">
      <div className="right-panel-body">
        <div className="right-panel-content">
          {activeView === "todo" ? (
            <TodoPanel
              stats={todoStats}
              onCreateRootThread={onCreateRootThread}
              onSelectTaskThread={onSelectTaskThread}
              onSetTaskFilter={onSetTaskFilter}
              selectedThreadId={selectedThreadId}
              taskFilter={taskFilter}
              todoItems={todoItems}
            />
          ) : (
            <FilePreviewPanel
              onNavigateToFile={onNavigateToFile}
              onOpenPreviewExternally={onOpenPreviewExternally}
              preview={preview}
              previewError={previewError}
              previewLoading={previewLoading}
            />
          )}
        </div>

        <nav className="panel-rail" aria-label="Right panel views">
          {[
            {
              view: "todo",
              label: "Todo Board",
              icon: <FilterIcon />,
              badge: String(todoStats.openCount),
            },
            {
              view: "preview",
              label: "File Preview",
              icon: <DocumentIcon />,
              badge: preview ? "1" : "",
            },
            {
              view: null,
              label: "Search",
              icon: <SearchIcon />,
              badge: "",
            },
            {
              view: null,
              label: "Graph",
              icon: <BranchIcon />,
              badge: "",
            },
            {
              view: null,
              label: "Artifacts",
              icon: <GridIcon />,
              badge: "",
            },
          ].map((item) => (
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

function TodoPanel({
  stats,
  onCreateRootThread,
  onSelectTaskThread,
  onSetTaskFilter,
  selectedThreadId,
  taskFilter,
  todoItems,
}: {
  stats: ReturnType<typeof buildTodoStats>;
  onCreateRootThread: () => void;
  onSelectTaskThread: (threadId: string) => void;
  onSetTaskFilter: (value: TaskFilter) => void;
  selectedThreadId: string | null;
  taskFilter: TaskFilter;
  todoItems: TodoCardItem[];
}) {
  return (
    <div className="todo-panel">
      <header className="panel-content-header">
        <div className="panel-content-copy">
          <span className="panel-eyebrow">Todo List</span>
          <h2>Execution Queue</h2>
          <p>Track and manage active threads for this run.</p>
        </div>
        <button type="button" className="panel-inline-action" onClick={onCreateRootThread}>
          <PlusIcon />
          <span>New Task</span>
        </button>
      </header>

      <div className="todo-overview-grid">
        <OverviewMetric label="Open" value={stats.openCount} tone="open" />
        <OverviewMetric label="Running" value={stats.doing} tone="doing" />
        <OverviewMetric label="Blocked" value={stats.blocked} tone="blocked" />
      </div>

      <div className="todo-filters">
        {(
          [
            ["all", "All"],
            ["todo", "Open"],
            ["doing", "Running"],
            ["blocked", "Blocked"],
            ["done", "Done"],
          ] satisfies Array<[TaskFilter, string]>
        ).map(([value, label]) => (
          <button
            key={value}
            type="button"
            className={taskFilter === value ? "active" : ""}
            onClick={() => onSetTaskFilter(value)}
          >
            {label}
          </button>
        ))}
      </div>

      <div className="todo-scroll">
        {todoItems.length > 0 ? (
          todoItems.map((task) => (
            <button
              key={task.id}
              type="button"
              className={`todo-card ${task.threadId === selectedThreadId ? "selected" : ""}`}
              onClick={() => onSelectTaskThread(task.threadId)}
            >
              <div className="todo-card-top">
                <strong>{task.title}</strong>
                <span className={`todo-status ${task.status}`}>{task.statusLabel}</span>
              </div>
              {task.summary ? <p>{task.summary}</p> : null}
              <div className="todo-card-footer">
                <div className="todo-card-meta">
                  <BranchIcon />
                  <span>{task.ownerPath}</span>
                </div>
                <div className="todo-card-meta">
                  <ClockIcon />
                  <span>{task.updatedLabel}</span>
                </div>
              </div>
            </button>
          ))
        ) : (
          <div className="empty-card todo-empty">
            <p>No tasks for this filter.</p>
            <span>Switch filters or create a new root task to seed the queue.</span>
          </div>
        )}
      </div>
    </div>
  );
}

function FilePreviewPanel({
  onNavigateToFile,
  onOpenPreviewExternally,
  preview,
  previewError,
  previewLoading,
}: {
  onNavigateToFile: (location: FileLocation) => void;
  onOpenPreviewExternally: () => void;
  preview: FilePreview | null;
  previewError: string | null;
  previewLoading: boolean;
}) {
  return (
    <div className="preview-panel">
      <header className="panel-content-header preview-header">
        <div className="panel-content-copy">
          <span className="panel-eyebrow">File Preview</span>
          <h2>{preview ? preview.displayPath : "Linked Context"}</h2>
        </div>
        <button
          type="button"
          className="panel-inline-action preview-open-button"
          aria-label="Open preview in system editor"
          onClick={onOpenPreviewExternally}
          disabled={!preview}
        >
          <OpenIcon />
        </button>
      </header>

      {previewLoading ? <div className="preview-empty">Loading file…</div> : null}
      {!previewLoading && previewError ? <div className="preview-empty">{previewError}</div> : null}
      {!previewLoading && !previewError && !preview ? (
        <div className="preview-empty">
          <p>Open a local file link in the conversation to pin code context here.</p>
        </div>
      ) : null}
      {!previewLoading && !previewError && preview ? (
        <div className="preview-editor-shell">
          <div className="preview-utility-strip">
            <div className="preview-utility-primary">
              <span className={`preview-signal ${previewLspState(preview)}`} />
              <button
                type="button"
                className={`preview-lsp-button ${previewLspState(preview)}`}
                disabled
              >
                LSP
              </button>
            </div>
            <div className="preview-utility-secondary">
              <span>{preview.language}</span>
              <span className="preview-utility-separator">•</span>
              <span className="preview-utility-cwd">
                {preview.lsp.workspaceRoot ?? "No workspace root"}
              </span>
            </div>
          </div>
          <div className="preview-editor-pad">
            <Editor
              key={`${preview.path}:${preview.line ?? 0}:${preview.lsp.enabled ? "lsp" : "plain"}`}
              height="100%"
              onMount={(editor) => {
                if (preview.line) {
                  editor.revealLineInCenter(preview.line);
                  editor.setPosition({ lineNumber: preview.line, column: 1 });
                }

                editor.onMouseDown((event) => {
                  if (
                    !preview.lsp.enabled ||
                    !event.target.position ||
                    event.event.browserEvent.button !== 0
                  ) {
                    return;
                  }

                  const model = editor.getModel();
                  if (!model) {
                    return;
                  }

                  const word = model.getWordAtPosition(event.target.position);
                  if (!word) {
                    return;
                  }

                  void window.codexDesktop
                    .lspDefinition({
                      path: preview.path,
                      line: event.target.position.lineNumber,
                      column: event.target.position.column,
                    })
                    .then((response) => {
                      const destination = response.locations[0];
                      if (response.enabled && destination) {
                        onNavigateToFile(destination);
                      }
                    })
                    .catch((error) => {
                      console.error("Failed to resolve definition", error);
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
    </div>
  );
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
  if (preview.lsp.enabled) {
    return "enabled";
  }
  if (preview.lsp.workspaceRoot) {
    return "unavailable";
  }
  return "plain";
}
