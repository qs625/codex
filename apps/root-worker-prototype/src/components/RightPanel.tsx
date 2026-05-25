import { useEffect, useRef } from "react";
import Editor from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";

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
import { ZoomableImage } from "./Conversation";
import type { FileLocation, FilePreview, RightPanelView, TaskFilter, TodoCardItem } from "../types";

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

export function RightPanel({
  activeView,
  onCreateRootThread,
  onNavigateToSymbol,
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
  onNavigateToSymbol: (destination: FileLocation, sourceLocation: FileLocation) => void;
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
              onNavigateToSymbol={onNavigateToSymbol}
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
  onNavigateToSymbol,
  onOpenPreviewExternally,
  preview,
  previewError,
  previewLoading,
}: {
  onNavigateToSymbol: (destination: FileLocation, sourceLocation: FileLocation) => void;
  onOpenPreviewExternally: () => void;
  preview: FilePreview | null;
  previewError: string | null;
  previewLoading: boolean;
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
            <ZoomableImage
              src={preview.image.dataUrl}
              alt={preview.image.name}
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

                  void window.codexDesktop
                    .lspDefinition({
                      path: preview.path,
                      line: event.target.position.lineNumber,
                      column: event.target.position.column,
                    })
                    .then((response) => {
                      const destination = response.locations[0];
                      if (response.enabled && destination) {
                        onNavigateToSymbol(destination, {
                          path: preview.path,
                          line: event.target.position.lineNumber,
                          column: event.target.position.column,
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
  return preview.lsp.lspStatus.phase;
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
