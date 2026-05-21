import Editor from "@monaco-editor/react";

import { BranchIcon, ClockIcon, DocumentIcon, FilterIcon, OpenIcon, PlusIcon } from "./icons";
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
  return (
    <aside className="right-panel">
      <div className="panel-switcher">
        <button
          type="button"
          className={activeView === "todo" ? "active" : ""}
          onClick={() => onSetActiveView("todo")}
        >
          <FilterIcon />
          <span>Todo List</span>
        </button>
        <button
          type="button"
          className={activeView === "preview" ? "active" : ""}
          onClick={() => onSetActiveView("preview")}
        >
          <DocumentIcon />
          <span>File Preview</span>
        </button>
      </div>

      {activeView === "todo" ? (
        <TodoPanel
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
    </aside>
  );
}

function TodoPanel({
  onCreateRootThread,
  onSelectTaskThread,
  onSetTaskFilter,
  selectedThreadId,
  taskFilter,
  todoItems,
}: {
  onCreateRootThread: () => void;
  onSelectTaskThread: (threadId: string) => void;
  onSetTaskFilter: (value: TaskFilter) => void;
  selectedThreadId: string | null;
  taskFilter: TaskFilter;
  todoItems: TodoCardItem[];
}) {
  return (
    <div className="todo-panel">
      <header className="todo-header">
        <div>
          <h2>Todo List</h2>
        </div>
        <button type="button" className="icon-button subtle" aria-label="Todo settings">
          <FilterIcon />
        </button>
      </header>

      <div className="todo-filters">
        {(
          [
            ["all", "All"],
            ["todo", "Todo"],
            ["doing", "Doing"],
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
                <span className="todo-radio" />
                <strong>{task.title}</strong>
                <span className={`todo-status ${task.status}`}>{task.statusLabel}</span>
              </div>
              <div className="todo-card-meta">
                <BranchIcon />
                <span>{task.ownerPath}</span>
              </div>
              <div className="todo-card-meta">
                <ClockIcon />
                <span>{task.updatedLabel}</span>
              </div>
              {task.summary ? <p>{task.summary}</p> : null}
            </button>
          ))
        ) : (
          <div className="empty-card todo-empty">
            <p>No tasks for this filter.</p>
          </div>
        )}
      </div>

      <button type="button" className="new-task-button" onClick={onCreateRootThread}>
        <PlusIcon />
        <span>New Task</span>
      </button>
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
      <header className="todo-header preview-header">
        <div>
          <h2>File Preview</h2>
          <p>
            {preview ? preview.displayPath : "Click a local file link in the conversation to preview it here."}
          </p>
        </div>
        <button
          type="button"
          className="icon-button subtle"
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
          <p>Support is focused on local file hyperlinks such as absolute paths and `file://` links.</p>
        </div>
      ) : null}
      {!previewLoading && !previewError && preview ? (
        <div className="preview-editor-shell">
          <div className="preview-lsp-status-row">
            <span className={`preview-lsp-badge ${previewLspBadgeClass(preview)}`}>
              {previewLspBadgeLabel(preview)}
            </span>
            <span className="preview-lsp-copy">
              {preview.lsp.enabled
                ? `Left click a symbol to jump with ${preview.lsp.serverLabel}.`
                : preview.lsp.reason ?? `Plain ${preview.language} preview`}
            </span>
          </div>
          <div className="preview-meta">
            <span>{preview.line ? `Line ${preview.line}` : "Full file"}</span>
            <span>{preview.lsp.enabled ? preview.lsp.serverLabel : preview.language}</span>
          </div>
          {preview.lsp.workspaceRoot ? (
            <div className="preview-meta preview-meta-secondary">
              <span>LSP Root</span>
              <span>{preview.lsp.workspaceRoot}</span>
            </div>
          ) : null}
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
              fontSize: 13,
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
      ) : null}
    </div>
  );
}

function previewLspBadgeClass(preview: FilePreview) {
  if (preview.lsp.enabled) {
    return "enabled";
  }

  if (preview.lsp.workspaceRoot) {
    return "unavailable";
  }

  return "plain";
}

function previewLspBadgeLabel(preview: FilePreview) {
  if (preview.lsp.enabled) {
    return "LSP On";
  }

  if (preview.lsp.workspaceRoot) {
    return "Server Missing";
  }

  return "No Root";
}
