import type { ChangeEvent, ClipboardEvent, RefObject } from "react";

import { AgentTreeNode } from "./AgentTree";
import { EventRow, MessageRow, ToolRow } from "./Conversation";
import {
  BranchIcon,
  ClockIcon,
  CodeIcon,
  FilterIcon,
  GearIcon,
  ImageIcon,
  MoreIcon,
  OpenIcon,
  PaperclipIcon,
  PlusIcon,
  SendIcon,
  SparkleIcon,
} from "./icons";
import { getAgentRoleLabel, getPresenceLabel, getThreadPath, isRootThread, threadStatusClass } from "../lib/thread";
import type {
  ComposerImage,
  ConversationCell,
  TaskFilter,
  Thread,
  TodoCardItem,
  TreeMenuState,
  TreeNode,
} from "../types";

export function SidebarPanel({
  agentTree,
  collapsedSet,
  newRootName,
  onCreateRootThread,
  onOpenMenu,
  onSelectThread,
  onSetNewRootName,
  onToggleTreeNode,
  selectedThreadId,
}: {
  agentTree: TreeNode[];
  collapsedSet: Set<string>;
  newRootName: string;
  onCreateRootThread: () => void;
  onOpenMenu: (menu: TreeMenuState | null) => void;
  onSelectThread: (threadId: string) => void;
  onSetNewRootName: (value: string) => void;
  onToggleTreeNode: (threadId: string) => void;
  selectedThreadId: string | null;
}) {
  return (
    <aside className="sidebar">
      <div className="sidebar-section-header">
        <h2>Agent Tree</h2>
        <button
          type="button"
          className="icon-button subtle"
          aria-label="Create root agent"
          onClick={onCreateRootThread}
        >
          <PlusIcon />
        </button>
      </div>

      <div className="tree-scroll">
        {agentTree.length > 0 ? (
          agentTree.map((node) => (
            <AgentTreeNode
              key={node.key}
              collapsedSet={collapsedSet}
              depth={0}
              node={node}
              onSelect={onSelectThread}
              onToggle={onToggleTreeNode}
              onOpenMenu={onOpenMenu}
              selectedThreadId={selectedThreadId}
            />
          ))
        ) : (
          <div className="empty-card">
            <p>No root session yet.</p>
            <div className="empty-card-actions">
              <input
                value={newRootName}
                onChange={(event) => onSetNewRootName(event.target.value)}
                placeholder="root"
              />
              <button type="button" onClick={onCreateRootThread}>
                New Root
              </button>
            </div>
          </div>
        )}
      </div>

      <button type="button" className="sidebar-footer">
        <div className="sidebar-footer-left">
          <GearIcon />
          <span>Settings</span>
        </div>
        <SparkleIcon />
      </button>
    </aside>
  );
}

export function ConversationPanel({
  conversationCells,
  conversationScrollRef,
  draft,
  draftImages,
  imageInputRef,
  isLoadingThread,
  isSending,
  onConversationScroll,
  onDraftChange,
  onHandleComposerPaste,
  onHandleImageSelection,
  onRemoveDraftImage,
  onSendMessage,
  selectedThread,
  selectedThreadId,
}: {
  conversationCells: ConversationCell[];
  conversationScrollRef: RefObject<HTMLDivElement | null>;
  draft: string;
  draftImages: ComposerImage[];
  imageInputRef: RefObject<HTMLInputElement | null>;
  isLoadingThread: boolean;
  isSending: boolean;
  onConversationScroll: () => void;
  onDraftChange: (value: string) => void;
  onHandleComposerPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
  onHandleImageSelection: (event: ChangeEvent<HTMLInputElement>) => void;
  onRemoveDraftImage: (imageId: string) => void;
  onSendMessage: () => void;
  selectedThread: Thread | null;
  selectedThreadId: string | null;
}) {
  return (
    <section className="conversation-panel">
      <header className="conversation-header">
        <div>
          <h1>{selectedThread ? getThreadPath(selectedThread) : "/root"}</h1>
          <div className="conversation-subtitle">
            <span className={`status-dot ${threadStatusClass(selectedThread?.status ?? "waiting")}`} />
            <span>{selectedThread ? getAgentRoleLabel(selectedThread) : "Root Agent"}</span>
            <span className="subtitle-separator">•</span>
            <span>{selectedThread ? getPresenceLabel(selectedThread.status) : "Idle"}</span>
          </div>
        </div>
        <div className="conversation-actions">
          <button type="button" className="icon-button" aria-label="Open thread details">
            <OpenIcon />
          </button>
          <button type="button" className="icon-button" aria-label="More thread actions">
            <MoreIcon />
          </button>
        </div>
      </header>

      <div
        ref={conversationScrollRef}
        className="conversation-scroll"
        onScroll={onConversationScroll}
      >
        {selectedThread ? (
          conversationCells.length > 0 ? (
            conversationCells.map((cell) =>
              cell.kind === "event" ? (
                <EventRow key={cell.id} entry={cell.entries[0]} />
              ) : cell.kind === "tool" ? (
                <ToolRow key={cell.id} entries={cell.entries} />
              ) : (
                <MessageRow key={cell.id} entries={cell.entries} />
              ),
            )
          ) : (
            <div className="conversation-empty">
              <p>Select this agent and start the work from the composer below.</p>
            </div>
          )
        ) : (
          <div className="conversation-empty">
            <p>Create or select a root session to begin.</p>
          </div>
        )}
        {isLoadingThread ? <div className="inline-note">Loading thread…</div> : null}
      </div>

      <footer className="composer-shell">
        <div className="composer-input-shell">
          <input
            ref={imageInputRef}
            type="file"
            accept="image/*"
            multiple
            className="composer-file-input"
            onChange={onHandleImageSelection}
          />
          {draftImages.length > 0 ? (
            <div className="composer-image-strip">
              {draftImages.map((image) => (
                <div key={image.id} className="composer-image-card">
                  <img src={image.dataUrl} alt={image.name} />
                  <button
                    type="button"
                    className="composer-image-remove"
                    aria-label={`Remove ${image.name}`}
                    onClick={() => onRemoveDraftImage(image.id)}
                  >
                    ×
                  </button>
                  <span>{image.name}</span>
                </div>
              ))}
            </div>
          ) : null}
          <textarea
            className="composer-input"
            placeholder={
              selectedThread
                ? `Message ${getThreadPath(selectedThread)}...`
                : "Select an agent to start messaging..."
            }
            value={draft}
            onChange={(event) => onDraftChange(event.target.value)}
            onPaste={onHandleComposerPaste}
            onKeyDown={(event) => {
              if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                event.preventDefault();
                onSendMessage();
              }
            }}
          />
          <div className="composer-toolbar">
            <div className="composer-tools">
              <button type="button" className="tool-button" aria-label="Attach file">
                <PaperclipIcon />
              </button>
              <button type="button" className="tool-button" aria-label="Insert code">
                <CodeIcon />
              </button>
              <button
                type="button"
                className="tool-button"
                aria-label="Attach image"
                onClick={() => imageInputRef.current?.click()}
              >
                <ImageIcon />
              </button>
            </div>
            <button
              type="button"
              className="send-button"
              disabled={!selectedThreadId || isSending || (!draft.trim() && draftImages.length === 0)}
              onClick={onSendMessage}
            >
              <SendIcon />
            </button>
          </div>
        </div>
      </footer>
    </section>
  );
}

export function TodoPanel({
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
    <aside className="todo-panel">
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
                <MoreIcon />
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
    </aside>
  );
}

export function TreeContextMenu({
  threads,
  treeMenu,
  onArchiveThread,
}: {
  threads: Thread[];
  treeMenu: TreeMenuState | null;
  onArchiveThread: (threadId: string) => void;
}) {
  if (!treeMenu) {
    return null;
  }

  const thread = threads.find((candidate) => candidate.id === treeMenu.threadId);
  if (!thread || isRootThread(thread)) {
    return null;
  }

  return (
    <div
      className="tree-context-menu"
      style={{ left: treeMenu.x, top: treeMenu.y }}
      onClick={(event) => event.stopPropagation()}
    >
      <button
        type="button"
        className="tree-context-menu-item danger"
        onClick={() => onArchiveThread(treeMenu.threadId)}
      >
        Delete Agent
      </button>
    </div>
  );
}
