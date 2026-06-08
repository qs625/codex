import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type RefObject,
} from "react";

import { AgentTreeNode } from "./AgentTree";
import { ThinkingIndicator } from "./Conversation";
import { ConversationVirtualList } from "./ConversationVirtualList";
import {
  CodeIcon,
  GearIcon,
  ImageIcon,
  MicrophoneIcon,
  MoreIcon,
  OpenIcon,
  PaperclipIcon,
  PlusIcon,
  SendIcon,
  StopIcon,
} from "./icons";
import {
  getThreadSubtreeIds,
  getAgentRoleLabel,
  getThreadPresenceLabel,
  getThreadModelLabel,
  getThreadPath,
  getThreadReasoningLabel,
  isThreadThinking,
  isTurnInFlight,
  isRootThread,
  threadDisplayStatusClass,
  trimPath,
} from "../lib/thread";
import { isVoiceCaptureToggleDisabled } from "../lib/voiceCaptureState";
import type {
  ComposerImage,
  ConversationCell,
  DraftSkill,
  Thread,
  ThreadSkill,
  TreeMenuState,
  TreeNode,
  VoiceCaptureStatus,
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
        <div className="section-heading">
          <h2>Agent Tree</h2>
          <span>{agentTree.length} roots</span>
        </div>
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
        <span className="sidebar-footer-shortcut">⌘,</span>
      </button>
    </aside>
  );
}

export function ConversationPanel({
  availableSkills,
  conversationCells,
  conversationScrollRef,
  draft,
  draftImages,
  draftSkills,
  imageInputRef,
  isLoadingThread,
  isSending,
  isStoppingTurn,
  onAddDraftSkill,
  onConversationScroll,
  onDraftChange,
  onHandleComposerPaste,
  onHandleImageSelection,
  onOpenLocalFile,
  onRemoveDraftImage,
  onRemoveDraftSkill,
  onSendMessage,
  onStopTurn,
  onToggleVoiceCapture,
  selectedThread,
  selectedThreadId,
  voiceCaptureMessage,
  voiceCaptureStatus,
}: {
  availableSkills: ThreadSkill[];
  conversationCells: ConversationCell[];
  conversationScrollRef: RefObject<HTMLDivElement | null>;
  draft: string;
  draftImages: ComposerImage[];
  draftSkills: DraftSkill[];
  imageInputRef: RefObject<HTMLInputElement | null>;
  isLoadingThread: boolean;
  isSending: boolean;
  isStoppingTurn: boolean;
  onAddDraftSkill: (skill: DraftSkill) => void;
  onConversationScroll: () => void;
  onDraftChange: (value: string) => void;
  onHandleComposerPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
  onHandleImageSelection: (event: ChangeEvent<HTMLInputElement>) => void;
  onOpenLocalFile: (target: string) => void;
  onRemoveDraftImage: (imageId: string) => void;
  onRemoveDraftSkill: (path: string) => void;
  onSendMessage: () => void;
  onStopTurn: () => void;
  onToggleVoiceCapture: () => void;
  selectedThread: Thread | null;
  selectedThreadId: string | null;
  voiceCaptureMessage: string | null;
  voiceCaptureStatus: VoiceCaptureStatus;
}) {
  const lastTurn = selectedThread?.turns.at(-1) ?? null;
  const lastTurnInProgress = lastTurn != null && isTurnInFlight(lastTurn);
  const activeTurnId = lastTurnInProgress ? lastTurn.id : null;
  const isThinking = isThreadThinking(selectedThread, {
    isLoadingThread,
    isSending,
  });
  const [selectedSkillIndex, setSelectedSkillIndex] = useState(0);
  const [dismissedSkillQuery, setDismissedSkillQuery] = useState<string | null>(null);
  const selectedSkillOptionRef = useRef<HTMLButtonElement | null>(null);
  const skillQuery = getActiveSkillSlashQuery(draft);
  const skillSuggestions = useMemo(
    () => filterSkillSlashSuggestions(availableSkills, draftSkills, skillQuery),
    [availableSkills, draftSkills, skillQuery],
  );
  const skillMenuVisible =
    skillQuery !== null && skillSuggestions.length > 0 && dismissedSkillQuery !== skillQuery;
  const voiceCaptureActive =
    voiceCaptureStatus === "requesting" ||
    voiceCaptureStatus === "connecting" ||
    voiceCaptureStatus === "listening" ||
    voiceCaptureStatus === "stopping";

  useEffect(() => {
    setSelectedSkillIndex(0);
  }, [skillQuery]);

  useEffect(() => {
    if (skillQuery !== dismissedSkillQuery) {
      setDismissedSkillQuery(null);
    }
  }, [dismissedSkillQuery, skillQuery]);

  useEffect(() => {
    if (skillSuggestions.length === 0) {
      setSelectedSkillIndex(0);
      return;
    }
    setSelectedSkillIndex((current) => Math.min(current, skillSuggestions.length - 1));
  }, [skillSuggestions]);

  useEffect(() => {
    if (!skillMenuVisible) {
      return;
    }
    selectedSkillOptionRef.current?.scrollIntoView({
      block: "nearest",
    });
  }, [selectedSkillIndex, skillMenuVisible]);

  function selectSkill(skill: ThreadSkill) {
    onAddDraftSkill({
      name: skill.name,
      path: skill.path,
    });
    onDraftChange("");
    setDismissedSkillQuery(null);
    setSelectedSkillIndex(0);
  }

  return (
    <section className="conversation-panel">
      <header className="conversation-header">
        <div className="conversation-heading">
          <div className="conversation-title-row">
            <h1>{selectedThread ? getThreadPath(selectedThread) : "/root"}</h1>
            <span
              className={`status-dot ${threadDisplayStatusClass(selectedThread)}`}
            />
            <span>{selectedThread ? getAgentRoleLabel(selectedThread) : "Root Agent"}</span>
            <span className="subtitle-separator">•</span>
            <span>{getThreadPresenceLabel(selectedThread)}</span>
            <span className="subtitle-separator">•</span>
            <span className="thread-chip">{getThreadModelLabel(selectedThread)}</span>
            <span className="thread-chip">reasoning: {getThreadReasoningLabel(selectedThread)}</span>
            {selectedThread ? (
              <span className="thread-chip thread-chip-cwd" title={selectedThread.cwd}>
                cwd: {trimPath(selectedThread.cwd)}
              </span>
            ) : null}
          </div>
        </div>
        <div className="conversation-actions">
          <button type="button" className="icon-button subtle" aria-label="Open thread details">
            <OpenIcon />
          </button>
          <button type="button" className="icon-button subtle" aria-label="More thread actions">
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
            <ConversationVirtualList
              key={selectedThreadId}
              cells={conversationCells}
              containerRef={conversationScrollRef}
              onOpenLocalFile={onOpenLocalFile}
            />
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
        {isLoadingThread ? (
          <div className="inline-note is-loading">Loading thread…</div>
        ) : null}
        {isThinking ? <ThinkingIndicator /> : null}
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
                  <img src={image.previewUrl} alt={image.name} />
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
          {draftSkills.length > 0 ? (
            <div className="composer-skill-strip">
              {draftSkills.map((skill) => (
                <span key={skill.path} className="composer-skill-chip">
                  <span>/{skill.name}</span>
                  <button
                    type="button"
                    className="composer-skill-remove"
                    aria-label={`Remove /${skill.name}`}
                    onClick={() => onRemoveDraftSkill(skill.path)}
                  >
                    ×
                  </button>
                </span>
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
              if (skillMenuVisible) {
                if (event.key === "ArrowDown") {
                  event.preventDefault();
                  setSelectedSkillIndex((current) =>
                    current + 1 >= skillSuggestions.length ? 0 : current + 1,
                  );
                  return;
                }
                if (event.key === "ArrowUp") {
                  event.preventDefault();
                  setSelectedSkillIndex((current) =>
                    current === 0 ? skillSuggestions.length - 1 : current - 1,
                  );
                  return;
                }
                if (event.key === "Enter" || event.key === "Tab") {
                  event.preventDefault();
                  const skill = skillSuggestions[selectedSkillIndex];
                  if (skill) {
                    selectSkill(skill);
                  }
                  return;
                }
                if (event.key === "Escape") {
                  event.preventDefault();
                  setDismissedSkillQuery(skillQuery);
                  return;
                }
              }
              if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                event.preventDefault();
                onSendMessage();
              }
            }}
          />
          {skillMenuVisible ? (
            <div className="composer-skill-menu" role="listbox" aria-label="Skill slash commands">
              {skillSuggestions.map((skill, index) => (
                <button
                  key={skill.path}
                  ref={index === selectedSkillIndex ? selectedSkillOptionRef : null}
                  type="button"
                  className={`composer-skill-option ${index === selectedSkillIndex ? "selected" : ""}`}
                  role="option"
                  aria-selected={index === selectedSkillIndex}
                  onMouseDown={(event) => {
                    event.preventDefault();
                    selectSkill(skill);
                  }}
                  onMouseEnter={() => setSelectedSkillIndex(index)}
                >
                  <span className="composer-skill-option-name">/{skill.name}</span>
                  <span className="composer-skill-option-meta">{formatSkillOptionMeta(skill)}</span>
                </button>
              ))}
            </div>
          ) : null}
          {voiceCaptureMessage ? (
            <div className={`composer-status composer-status-${voiceCaptureStatus}`}>
              {voiceCaptureMessage}
            </div>
          ) : null}
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
            <div className="composer-actions">
              <button
                type="button"
                className={`tool-button voice-button ${voiceCaptureActive ? "is-active" : ""} ${
                  voiceCaptureStatus === "error" ? "is-error" : ""
                }`}
                aria-label={voiceCaptureActive ? "Stop voice input" : "Start voice input"}
                disabled={isVoiceCaptureToggleDisabled({
                  selectedThreadId,
                  isSending,
                  isStoppingTurn,
                })}
                onClick={onToggleVoiceCapture}
              >
                {voiceCaptureActive ? <StopIcon /> : <MicrophoneIcon />}
              </button>
            <button
              type="button"
              className={`send-button ${activeTurnId ? "is-stop-button" : ""}`}
              disabled={
                activeTurnId
                  ? isStoppingTurn
                  : !selectedThreadId ||
                    isSending ||
                    voiceCaptureActive ||
                    (!draft.trim() && draftImages.length === 0 && draftSkills.length === 0)
              }
              aria-label={activeTurnId ? "Stop current turn" : "Send message"}
              onClick={activeTurnId ? onStopTurn : onSendMessage}
            >
              {activeTurnId ? <StopIcon /> : <SendIcon />}
            </button>
            </div>
          </div>
        </div>
      </footer>
    </section>
  );
}

function getActiveSkillSlashQuery(draft: string) {
  const firstLine = draft.trimStart().split("\n", 1)[0] ?? "";
  if (!firstLine.startsWith("/") || firstLine.includes(" ")) {
    return null;
  }
  return firstLine.slice(1);
}

function filterSkillSlashSuggestions(
  availableSkills: ThreadSkill[],
  draftSkills: DraftSkill[],
  query: string | null,
) {
  if (query === null) {
    return [];
  }

  const normalizedQuery = query.trim().toLowerCase();
  const selectedPaths = new Set(draftSkills.map((skill) => skill.path));

  return availableSkills.filter((skill) => {
    if (selectedPaths.has(skill.path)) {
      return false;
    }

    if (!normalizedQuery) {
      return true;
    }

    const searchable = [skill.name, skill.kind, skill.path].join(" ").toLowerCase();
    return searchable.includes(normalizedQuery);
  });
}

function formatSkillOptionMeta(skill: ThreadSkill) {
  return `${formatSkillKindLabel(skill.kind)} · ${trimPath(skill.path)}`;
}

function formatSkillKindLabel(kind: ThreadSkill["kind"]) {
  switch (kind) {
    case "explicit":
      return "explicit";
    case "implicit":
      return "implicit";
    case "all":
      return "all";
    default:
      return "skill";
  }
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

  const descendantCount = getThreadSubtreeIds(threads, thread.id).size - 1;

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
        {descendantCount > 0 ? "Delete Agent Tree" : "Delete Agent"}
      </button>
    </div>
  );
}
