import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type RefObject,
} from "react";

import { AgentTreeNode } from "./AgentTree";
import { ThinkingIndicator } from "./Conversation";
import { ConversationVirtualList } from "./ConversationVirtualList";
import { RunConfigPicker } from "./RunConfigPicker";
import type { RunConfigSelection } from "../lib/runConfig";
import {
  ChevronDownIcon,
  CodeIcon,
  GearIcon,
  ImageIcon,
  MicrophoneIcon,
  MoreIcon,
  OpenIcon,
  PaperclipIcon,
  PlusIcon,
  SearchIcon,
  SendIcon,
  StopIcon,
} from "./icons";
import {
  getThreadSubtreeIds,
  getAgentRoleLabel,
  getRootThreadConversationTitle,
  getThreadPresenceLabel,
  getThreadPath,
  isThreadThinking,
  isTurnInFlight,
  isRootThread,
  threadDisplayStatusClass,
  trimPath,
} from "../lib/thread";
import {
  buildConversationSearchResults,
  getNextConversationSearchIndex,
  type ConversationSearchResult,
} from "../lib/conversationSearch";
import { isVoiceCaptureToggleDisabled } from "../lib/voiceCaptureState";
import {
  buildComposerSlashSuggestions,
  getActiveComposerSlashQuery,
  type ComposerSlashCommandId,
  type ComposerSlashSuggestion,
} from "../lib/slashMenu";
import type {
  ComposerImage,
  ConversationCell,
  DraftSkill,
  ProjectAgentSidebar,
  SidebarProjectNode,
  Thread,
  ThreadGoal,
  ThreadSkill,
  TreeMenuState,
  TreeNode,
  VoiceCaptureStatus,
  WorkflowSummary,
} from "../types";

type GoalActionKind = "set" | "pause" | "resume" | "clear";

export function SidebarPanel({
  collapsedSet,
  collapsedProjectSet,
  isChatCollapsed,
  newProjectName,
  onCreateChatThread,
  onCreateProjectThread,
  onOpenMenu,
  onSelectProject,
  onSelectThread,
  onSetNewProjectName,
  onToggleChat,
  onToggleProject,
  onToggleTreeNode,
  projectSidebar,
  selectedThreadId,
}: {
  collapsedSet: Set<string>;
  collapsedProjectSet: Set<string>;
  isChatCollapsed: boolean;
  newProjectName: string;
  onCreateChatThread: () => void;
  onCreateProjectThread: () => void;
  onOpenMenu: (menu: TreeMenuState | null) => void;
  onSelectProject: (projectId: string, threadId: string) => void;
  onSelectThread: (threadId: string) => void;
  onSetNewProjectName: (value: string) => void;
  onToggleChat: () => void;
  onToggleProject: (projectId: string) => void;
  onToggleTreeNode: (threadId: string) => void;
  projectSidebar: ProjectAgentSidebar;
  selectedThreadId: string | null;
}) {
  const [isCreateMenuOpen, setIsCreateMenuOpen] = useState(false);
  const projectCount = projectSidebar.projects.length;
  const chatCount = projectSidebar.chat.conversations.length;
  const chooseProjectChat = () => {
    setIsCreateMenuOpen(false);
    onCreateProjectThread();
  };
  const choosePlainChat = () => {
    setIsCreateMenuOpen(false);
    onCreateChatThread();
  };

  return (
    <aside className="sidebar">
      <div className="sidebar-section-header">
        <div className="section-heading">
          <h2>Projects</h2>
          <span>
            {projectCount} projects · {chatCount} chats
          </span>
        </div>
        <div className="sidebar-actions">
          <button
            type="button"
            className="sidebar-action-button"
            onClick={() => setIsCreateMenuOpen((current) => !current)}
            aria-expanded={isCreateMenuOpen}
          >
            <PlusIcon />
            <span>New</span>
          </button>
          {isCreateMenuOpen ? (
            <div className="sidebar-create-menu">
              <button type="button" onClick={chooseProjectChat}>
                <strong>Current workspace</strong>
                <span>Open project chat</span>
              </button>
              <button type="button" onClick={choosePlainChat}>
                <strong>Chat without project</strong>
                <span>Needs backend support</span>
              </button>
            </div>
          ) : null}
        </div>
      </div>

      <div className="tree-scroll">
        {projectSidebar.projects.length > 0 ? (
          projectSidebar.projects.map((project) => (
            <ProjectSection
              key={project.id}
              collapsedProjectSet={collapsedProjectSet}
              collapsedSet={collapsedSet}
              onOpenMenu={onOpenMenu}
              onSelectProject={onSelectProject}
              onSelectThread={onSelectThread}
              onToggleProject={onToggleProject}
              onToggleTreeNode={onToggleTreeNode}
              project={project}
              selectedThreadId={selectedThreadId}
            />
          ))
        ) : (
          <div className="empty-card">
            <p>No projects yet.</p>
            <div className="empty-card-actions">
              <input
                value={newProjectName}
                onChange={(event) => onSetNewProjectName(event.target.value)}
                placeholder="Project chat"
              />
              <button type="button" onClick={onCreateProjectThread}>
                Open
              </button>
            </div>
          </div>
        )}
        <ChatSection
          chatNodes={projectSidebar.chat.conversations}
          collapsedSet={collapsedSet}
          isCollapsed={isChatCollapsed}
          onOpenMenu={onOpenMenu}
          onSelectThread={onSelectThread}
          onToggleChat={onToggleChat}
          onToggleTreeNode={onToggleTreeNode}
          selectedThreadId={selectedThreadId}
          statusClass={projectSidebar.chat.statusClass}
        />
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

function ProjectSection({
  collapsedProjectSet,
  collapsedSet,
  onOpenMenu,
  onSelectProject,
  onSelectThread,
  onToggleProject,
  onToggleTreeNode,
  project,
  selectedThreadId,
}: {
  collapsedProjectSet: Set<string>;
  collapsedSet: Set<string>;
  onOpenMenu: (menu: TreeMenuState | null) => void;
  onSelectProject: (projectId: string, threadId: string) => void;
  onSelectThread: (threadId: string) => void;
  onToggleProject: (projectId: string) => void;
  onToggleTreeNode: (threadId: string) => void;
  project: SidebarProjectNode;
  selectedThreadId: string | null;
}) {
  const isCollapsed = collapsedProjectSet.has(project.id);
  const containsSelected =
    selectedThreadId != null && treeContainsThread(project.tree, selectedThreadId);
  const counts = [
    project.failedCount > 0 ? `${project.failedCount} failed` : null,
    project.activeCount > 0 ? `${project.activeCount} active` : null,
    project.waitingCount > 0 ? `${project.waitingCount} waiting` : null,
    project.descendantCount > 0 ? `${project.descendantCount} agents` : null,
  ].filter(Boolean);

  return (
    <section className="project-section">
      <button
        type="button"
        className={`project-header ${containsSelected ? "contains-selected" : ""}`}
        onClick={() => onSelectProject(project.id, project.tree.threadId)}
      >
        <span
          className={`tree-toggle ${isCollapsed ? "collapsed" : ""}`}
          onClick={(event) => {
            event.stopPropagation();
            onToggleProject(project.id);
          }}
        >
          <ChevronDownIcon />
        </span>
        <span
          className={`tree-inline-status ${project.statusClass}`}
          title={project.statusClass}
          aria-label={project.statusClass}
        />
        <span className="project-header-copy">
          <strong>{project.label}</strong>
          <span>{project.subtitle}</span>
        </span>
        {counts.length > 0 ? (
          <span className="project-counts">{counts.join(" · ")}</span>
        ) : null}
      </button>
      {!isCollapsed ? (
        project.tree.children.length > 0 ? (
          project.tree.children.map((child) => (
            <AgentTreeNode
              key={child.key}
              collapsedSet={collapsedSet}
              depth={0}
              node={child}
              onSelect={onSelectThread}
              onToggle={onToggleTreeNode}
              onOpenMenu={onOpenMenu}
              selectedThreadId={selectedThreadId}
            />
          ))
        ) : null
      ) : null}
    </section>
  );
}

function treeContainsThread(node: TreeNode, threadId: string): boolean {
  return (
    node.threadId === threadId ||
    node.children.some((child) => treeContainsThread(child, threadId))
  );
}

function ChatSection({
  chatNodes,
  collapsedSet,
  isCollapsed,
  onOpenMenu,
  onSelectThread,
  onToggleChat,
  onToggleTreeNode,
  selectedThreadId,
  statusClass,
}: {
  chatNodes: TreeNode[];
  collapsedSet: Set<string>;
  isCollapsed: boolean;
  onOpenMenu: (menu: TreeMenuState | null) => void;
  onSelectThread: (threadId: string) => void;
  onToggleChat: () => void;
  onToggleTreeNode: (threadId: string) => void;
  selectedThreadId: string | null;
  statusClass: string;
}) {
  return (
    <section className="project-section chat-section">
      <button type="button" className="project-header" onClick={onToggleChat}>
        <span className={`tree-toggle ${isCollapsed ? "collapsed" : ""}`}>
          <ChevronDownIcon />
        </span>
        <span
          className={`tree-inline-status ${statusClass}`}
          title={statusClass}
          aria-label={statusClass}
        />
        <span className="project-header-copy">
          <strong>Chat</strong>
          <span>Conversations without a project</span>
        </span>
        <span className="project-counts">{chatNodes.length}</span>
      </button>
      {!isCollapsed ? (
        chatNodes.length > 0 ? (
          chatNodes.map((node) => (
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
          <div className="chat-empty">No chat conversations.</div>
        )
      ) : null}
    </section>
  );
}

export function ConversationPanel({
  availableSkills,
  availableWorkflows,
  compactHistoryById,
  conversationCells,
  conversationScrollRef,
  draft,
  draftImages,
  draftSkills,
  focusedConversationItem,
  imageInputRef,
  isLoadingThread,
  isSending,
  isStoppingTurn,
  goal,
  goalAction,
  goalActionError,
  onAddDraftSkill,
  onCancelGoal,
  onConversationScroll,
  onDraftChange,
  onHandleComposerPaste,
  onHandleImageSelection,
  onToggleCompactHistory,
  onOpenLocalFile,
  onPauseGoal,
  onRemoveDraftImage,
  onRemoveDraftSkill,
  onResumeGoal,
  onRunSlashCommand,
  onUpdateRunConfig,
  onSendMessage,
  onStopTurn,
  onToggleVoiceCapture,
  selectedThread,
  selectedThreadId,
  voiceCaptureMessage,
  voiceCaptureStatus,
}: {
  availableSkills: ThreadSkill[];
  availableWorkflows: WorkflowSummary[];
  compactHistoryById: Readonly<
    Record<string, { isLoading: boolean; isExpanded: boolean; error: string | null }>
  >;
  conversationCells: ConversationCell[];
  conversationScrollRef: RefObject<HTMLDivElement | null>;
  draft: string;
  draftImages: ComposerImage[];
  draftSkills: DraftSkill[];
  focusedConversationItem: { itemId: string; token: number } | null;
  imageInputRef: RefObject<HTMLInputElement | null>;
  isLoadingThread: boolean;
  isSending: boolean;
  isStoppingTurn: boolean;
  goal: ThreadGoal | null;
  goalAction: GoalActionKind | null;
  goalActionError: string | null;
  onAddDraftSkill: (skill: DraftSkill) => void;
  onCancelGoal: () => void;
  onConversationScroll: () => void;
  onDraftChange: (value: string) => void;
  onHandleComposerPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
  onHandleImageSelection: (event: ChangeEvent<HTMLInputElement>) => void;
  onToggleCompactHistory: (entryId: string) => void;
  onOpenLocalFile: (target: string) => void;
  onPauseGoal: () => void;
  onRemoveDraftImage: (imageId: string) => void;
  onRemoveDraftSkill: (path: string) => void;
  onResumeGoal: () => void;
  onRunSlashCommand: (commandId: ComposerSlashCommandId) => void;
  onUpdateRunConfig: (selection: RunConfigSelection) => void;
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
  const [selectedSlashIndex, setSelectedSlashIndex] = useState(0);
  const [dismissedSlashQuery, setDismissedSlashQuery] = useState<string | null>(
    null,
  );
  const [conversationSearchOpen, setConversationSearchOpen] = useState(false);
  const [conversationSearchQuery, setConversationSearchQuery] = useState("");
  const [activeSearchIndex, setActiveSearchIndex] = useState(0);
  const [searchFocusToken, setSearchFocusToken] = useState(0);
  const [conversationFocusSource, setConversationFocusSource] = useState<
    "external" | "search"
  >("external");
  const selectedSlashOptionRef = useRef<HTMLButtonElement | null>(null);
  const conversationSearchInputRef = useRef<HTMLInputElement | null>(null);
  const slashQuery = getActiveComposerSlashQuery(draft);
  const slashSuggestions = useMemo(
    () =>
      buildComposerSlashSuggestions({
        availableSkills,
        availableWorkflows,
        commandsEnabled: draftImages.length === 0 && draftSkills.length === 0,
        draftSkills,
        query: slashQuery,
      }),
    [
      availableSkills,
      availableWorkflows,
      draftImages.length,
      draftSkills,
      slashQuery,
    ],
  );
  const conversationSearchResults = useMemo(
    () =>
      buildConversationSearchResults(
        conversationCells,
        conversationSearchQuery,
      ),
    [conversationCells, conversationSearchQuery],
  );
  const safeActiveSearchIndex =
    conversationSearchResults.length > 0
      ? Math.min(activeSearchIndex, conversationSearchResults.length - 1)
      : 0;
  const activeSearchResult =
    conversationSearchResults[safeActiveSearchIndex] ?? null;
  const conversationSearchMatchingCellIds = useMemo(
    () => new Set(conversationSearchResults.map((result) => result.cellId)),
    [conversationSearchResults],
  );
  const focusedSearchItem = useMemo(
    () =>
      activeSearchResult
        ? { itemId: activeSearchResult.entryId, token: searchFocusToken }
        : null,
    [activeSearchResult?.entryId, searchFocusToken],
  );
  const focusedConversationListItem =
    conversationFocusSource === "search" && focusedSearchItem
      ? focusedSearchItem
      : focusedConversationItem;
  const slashMenuVisible =
    slashQuery !== null && dismissedSlashQuery !== slashQuery;
  const commandSlashSuggestions = slashSuggestions.filter(
    (suggestion) => suggestion.type === "command",
  );
  const skillSlashSuggestions = slashSuggestions.filter(
    (suggestion) => suggestion.type === "skill",
  );
  const workflowSlashSuggestions = slashSuggestions.filter(
    (suggestion) => suggestion.type === "workflow",
  );
  const voiceCaptureActive =
    voiceCaptureStatus === "requesting" ||
    voiceCaptureStatus === "connecting" ||
    voiceCaptureStatus === "listening" ||
    voiceCaptureStatus === "stopping";

  useEffect(() => {
    setSelectedSlashIndex(0);
  }, [slashQuery]);

  useEffect(() => {
    if (slashQuery !== dismissedSlashQuery) {
      setDismissedSlashQuery(null);
    }
  }, [dismissedSlashQuery, slashQuery]);

  useEffect(() => {
    if (slashSuggestions.length === 0) {
      setSelectedSlashIndex(0);
      return;
    }
    setSelectedSlashIndex((current) =>
      Math.min(current, slashSuggestions.length - 1),
    );
  }, [slashSuggestions]);

  useEffect(() => {
    if (!slashMenuVisible) {
      return;
    }
    selectedSlashOptionRef.current?.scrollIntoView({
      block: "nearest",
    });
  }, [selectedSlashIndex, slashMenuVisible]);

  useEffect(() => {
    setConversationSearchQuery("");
    setActiveSearchIndex(0);
    setConversationFocusSource("external");
  }, [selectedThreadId]);

  useEffect(() => {
    if (!focusedConversationItem) {
      return;
    }
    setConversationFocusSource("external");
  }, [focusedConversationItem?.itemId, focusedConversationItem?.token]);

  useEffect(() => {
    if (!conversationSearchOpen) {
      return;
    }
    window.requestAnimationFrame(() => {
      conversationSearchInputRef.current?.focus();
      conversationSearchInputRef.current?.select();
    });
  }, [conversationSearchOpen]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        setConversationSearchOpen(true);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  useEffect(() => {
    if (conversationSearchResults.length === 0) {
      setActiveSearchIndex(0);
      return;
    }
    setActiveSearchIndex((current) =>
      Math.min(current, conversationSearchResults.length - 1),
    );
  }, [conversationSearchResults.length]);

  useEffect(() => {
    if (!activeSearchResult) {
      return;
    }
    setSearchFocusToken((current) => current + 1);
    setConversationFocusSource("search");
  }, [activeSearchResult?.id]);

  function selectSkill(skill: ThreadSkill) {
    onAddDraftSkill({
      name: skill.name,
      path: skill.path,
    });
    onDraftChange("");
    setDismissedSlashQuery(null);
    setSelectedSlashIndex(0);
  }

  function selectSlashSuggestion(suggestion: ComposerSlashSuggestion) {
    switch (suggestion.type) {
      case "command":
        onDraftChange(suggestion.draftText ?? "");
        if (suggestion.draftText) {
          setDismissedSlashQuery(null);
          setSelectedSlashIndex(0);
          return;
        }
        onRunSlashCommand(suggestion.commandId);
        setDismissedSlashQuery(null);
        setSelectedSlashIndex(0);
        return;
      case "skill":
        selectSkill(suggestion.skill);
        return;
      case "workflow":
        onDraftChange(suggestion.draftText);
        setDismissedSlashQuery(null);
        setSelectedSlashIndex(0);
        return;
    }
  }

  function openConversationSearch() {
    setConversationSearchOpen(true);
  }

  function clearConversationSearch() {
    if (conversationSearchQuery) {
      setConversationSearchQuery("");
      setActiveSearchIndex(0);
      return;
    }
    setConversationSearchOpen(false);
  }

  function moveConversationSearchResult(direction: -1 | 1) {
    if (conversationSearchResults.length === 0) {
      return;
    }
    setActiveSearchIndex((current) =>
      getNextConversationSearchIndex(
        current,
        conversationSearchResults.length,
        direction,
      ),
    );
  }

  function handleConversationSearchKeyDown(
    event: ReactKeyboardEvent<HTMLInputElement>,
  ) {
    if (event.key === "Enter") {
      event.preventDefault();
      moveConversationSearchResult(event.shiftKey ? -1 : 1);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      clearConversationSearch();
    }
  }

  return (
    <section className="conversation-panel">
      <header className="conversation-header">
        <div className="conversation-heading">
          <div className="conversation-title-row">
            <h1>
              {selectedThread
                ? isRootThread(selectedThread)
                  ? getRootThreadConversationTitle(selectedThread)
                  : getThreadPath(selectedThread)
                : "Select a project"}
            </h1>
            <span
              className={`status-dot ${threadDisplayStatusClass(selectedThread)}`}
            />
            <span>
              {selectedThread
                ? getAgentRoleLabel(selectedThread)
                : "No thread selected"}
            </span>
            <span className="subtitle-separator">•</span>
            <span>{getThreadPresenceLabel(selectedThread)}</span>
            <span className="subtitle-separator">•</span>
            <RunConfigPicker
              disabled={isSending || lastTurnInProgress}
              onApply={onUpdateRunConfig}
              selectedThread={selectedThread}
            />
            {selectedThread?.cwd.trim() ? (
              <span
                className="thread-chip thread-chip-cwd"
                title={selectedThread.cwd}
              >
                cwd: {trimPath(selectedThread.cwd)}
              </span>
            ) : null}
          </div>
        </div>
        <div className="conversation-actions">
          {conversationSearchOpen ? (
            <ConversationSearchControls
              activeResult={activeSearchResult}
              disabled={!selectedThread}
              inputRef={conversationSearchInputRef}
              onClear={clearConversationSearch}
              onKeyDown={handleConversationSearchKeyDown}
              onMoveNext={() => moveConversationSearchResult(1)}
              onMovePrevious={() => moveConversationSearchResult(-1)}
              onQueryChange={(value) => {
                setConversationSearchQuery(value);
                setActiveSearchIndex(0);
              }}
              query={conversationSearchQuery}
              resultCount={conversationSearchResults.length}
              resultIndex={safeActiveSearchIndex}
            />
          ) : (
            <button
              type="button"
              className="icon-button subtle"
              aria-label="Search conversation"
              disabled={!selectedThread}
              onClick={openConversationSearch}
            >
              <SearchIcon />
            </button>
          )}
          <button
            type="button"
            className="icon-button subtle"
            aria-label="Open thread details"
          >
            <OpenIcon />
          </button>
          <button
            type="button"
            className="icon-button subtle"
            aria-label="More thread actions"
          >
            <MoreIcon />
          </button>
        </div>
      </header>

      <GoalStrip
        goal={goal}
        action={goalAction}
        actionError={goalActionError}
        onCancel={onCancelGoal}
        onPause={onPauseGoal}
        onResume={onResumeGoal}
      />

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
              compactHistoryById={compactHistoryById}
              containerRef={conversationScrollRef}
              focusedItem={focusedConversationListItem}
              onToggleCompactHistory={onToggleCompactHistory}
              onOpenLocalFile={onOpenLocalFile}
              searchCurrentCellId={activeSearchResult?.cellId ?? null}
              searchMatchCellIds={conversationSearchMatchingCellIds}
            />
          ) : (
            <div className="conversation-empty">
              <p>
                Select this agent and start the work from the composer below.
              </p>
            </div>
          )
        ) : (
          <div className="conversation-empty">
            <p>Open a project or select a chat to begin.</p>
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
              if (slashMenuVisible) {
                if (event.key === "ArrowDown") {
                  if (slashSuggestions.length === 0) {
                    return;
                  }
                  event.preventDefault();
                  setSelectedSlashIndex((current) =>
                    current + 1 >= slashSuggestions.length ? 0 : current + 1,
                  );
                  return;
                }
                if (event.key === "ArrowUp") {
                  if (slashSuggestions.length === 0) {
                    return;
                  }
                  event.preventDefault();
                  setSelectedSlashIndex((current) =>
                    current === 0 ? slashSuggestions.length - 1 : current - 1,
                  );
                  return;
                }
                if (event.key === "Enter" || event.key === "Tab") {
                  const suggestion = slashSuggestions[selectedSlashIndex];
                  if (suggestion) {
                    event.preventDefault();
                    selectSlashSuggestion(suggestion);
                  }
                  return;
                }
                if (event.key === "Escape") {
                  event.preventDefault();
                  setDismissedSlashQuery(slashQuery);
                  return;
                }
              }
              if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                event.preventDefault();
                onSendMessage();
              }
            }}
          />
          {slashMenuVisible ? (
            <div
              className="composer-slash-menu"
              role="listbox"
              aria-label="Slash commands and skills"
            >
              {slashSuggestions.length > 0 ? (
                <>
                  {commandSlashSuggestions.length > 0 ? (
                    <SlashMenuGroup
                      allSuggestions={slashSuggestions}
                      selectedIndex={selectedSlashIndex}
                      selectedOptionRef={selectedSlashOptionRef}
                      suggestions={commandSlashSuggestions}
                      title="Commands"
                      onSelect={selectSlashSuggestion}
                      onSelectIndex={setSelectedSlashIndex}
                    />
                  ) : null}
                  {workflowSlashSuggestions.length > 0 ? (
                    <SlashMenuGroup
                      allSuggestions={slashSuggestions}
                      selectedIndex={selectedSlashIndex}
                      selectedOptionRef={selectedSlashOptionRef}
                      suggestions={workflowSlashSuggestions}
                      title="Workflows"
                      onSelect={selectSlashSuggestion}
                      onSelectIndex={setSelectedSlashIndex}
                    />
                  ) : null}
                  {skillSlashSuggestions.length > 0 ? (
                    <SlashMenuGroup
                      allSuggestions={slashSuggestions}
                      selectedIndex={selectedSlashIndex}
                      selectedOptionRef={selectedSlashOptionRef}
                      suggestions={skillSlashSuggestions}
                      title="Skills"
                      onSelect={selectSlashSuggestion}
                      onSelectIndex={setSelectedSlashIndex}
                    />
                  ) : null}
                </>
              ) : (
                <div className="composer-slash-empty">
                  No commands, workflows, or skills match “/{slashQuery ?? ""}”
                </div>
              )}
            </div>
          ) : null}
          {voiceCaptureMessage ? (
            <div
              className={`composer-status composer-status-${voiceCaptureStatus}`}
            >
              {voiceCaptureMessage}
            </div>
          ) : null}
          <div className="composer-toolbar">
            <div className="composer-tools">
              <button
                type="button"
                className="tool-button"
                aria-label="Attach file"
              >
                <PaperclipIcon />
              </button>
              <button
                type="button"
                className="tool-button"
                aria-label="Insert code"
              >
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
                aria-label={
                  voiceCaptureActive ? "Stop voice input" : "Start voice input"
                }
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
                      (!draft.trim() &&
                        draftImages.length === 0 &&
                        draftSkills.length === 0)
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

function GoalStrip({
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
  if (!goal && !actionError) {
    return null;
  }

  const label = goal ? formatGoalStatus(goal.status) : "Goal";
  const objective = goal?.objective ?? "No active goal.";
  const usage = goal ? formatGoalUsage(goal) : "";
  const primaryAction = getGoalPrimaryAction(goal);

  return (
    <section className="goal-strip" aria-label="Thread goal">
      <div className="goal-strip-main">
        <span className={`goal-status-badge ${goal?.status ?? "none"}`}>
          {action ? formatGoalActionProgress(action) : label}
        </span>
        <span className="goal-strip-objective" title={objective}>
          {objective}
        </span>
        {usage ? <span className="goal-strip-usage">{usage}</span> : null}
      </div>
      {actionError ? (
        <span className="goal-strip-error" role="status">
          {actionError}
        </span>
      ) : null}
      {primaryAction ? (
        <button
          type="button"
          className="goal-strip-cancel goal-strip-primary-action"
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
        className="goal-strip-cancel"
        disabled={!goal || !!action}
        title={goal ? "Cancel goal" : "No active goal"}
        onClick={onCancel}
      >
        {action === "clear" ? "Cancelling" : "Cancel"}
      </button>
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

function formatGoalUsage(goal: ThreadGoal) {
  if (goal.tokenBudget && goal.tokenBudget > 0) {
    return `${formatCompactNumber(goal.tokensUsed)} / ${formatCompactNumber(goal.tokenBudget)} tokens`;
  }
  if (goal.tokensUsed > 0) {
    return `${formatCompactNumber(goal.tokensUsed)} tokens`;
  }
  return "";
}

function formatCompactNumber(value: number) {
  if (value >= 1_000_000) {
    return `${Math.round((value / 1_000_000) * 10) / 10}M`;
  }
  if (value >= 1_000) {
    return `${Math.round(value / 100) / 10}K`;
  }
  return String(value);
}

function ConversationSearchControls({
  activeResult,
  disabled,
  inputRef,
  onClear,
  onKeyDown,
  onMoveNext,
  onMovePrevious,
  onQueryChange,
  query,
  resultCount,
  resultIndex,
}: {
  activeResult: ConversationSearchResult | null;
  disabled: boolean;
  inputRef: RefObject<HTMLInputElement | null>;
  onClear: () => void;
  onKeyDown: (event: ReactKeyboardEvent<HTMLInputElement>) => void;
  onMoveNext: () => void;
  onMovePrevious: () => void;
  onQueryChange: (value: string) => void;
  query: string;
  resultCount: number;
  resultIndex: number;
}) {
  const hasResults = resultCount > 0;
  const countLabel = hasResults
    ? `${resultIndex + 1} / ${resultCount}`
    : "0 / 0";
  const title = activeResult
    ? `${activeResult.sourceLabel}: ${activeResult.preview}`
    : "No matches";

  return (
    <div className="conversation-search-controls" title={title}>
      <SearchIcon />
      <input
        ref={inputRef}
        aria-label="Search current conversation"
        disabled={disabled}
        placeholder="Search conversation"
        value={query}
        onChange={(event) => onQueryChange(event.target.value)}
        onKeyDown={onKeyDown}
      />
      <span className="conversation-search-count">{countLabel}</span>
      <button
        type="button"
        className="conversation-search-step"
        disabled={!hasResults}
        aria-label="Previous conversation search result"
        onClick={onMovePrevious}
      >
        Prev
      </button>
      <button
        type="button"
        className="conversation-search-step"
        disabled={!hasResults}
        aria-label="Next conversation search result"
        onClick={onMoveNext}
      >
        Next
      </button>
      <button
        type="button"
        className="conversation-search-clear"
        aria-label={
          query ? "Clear conversation search" : "Close conversation search"
        }
        onClick={onClear}
      >
        Clear
      </button>
    </div>
  );
}

function formatSkillOptionMeta(skill: ThreadSkill) {
  return `${formatSkillKindLabel(skill.kind)} · ${trimPath(skill.path)}`;
}

function getSlashSuggestionKey(suggestion: ComposerSlashSuggestion) {
  switch (suggestion.type) {
    case "command":
      return `command:${suggestion.commandId}`;
    case "skill":
      return `skill:${suggestion.skill.path}`;
    case "workflow":
      return `workflow:${suggestion.workflow.id}`;
  }
}

function SlashMenuGroup({
  allSuggestions,
  onSelect,
  onSelectIndex,
  selectedIndex,
  selectedOptionRef,
  suggestions,
  title,
}: {
  allSuggestions: ComposerSlashSuggestion[];
  onSelect: (suggestion: ComposerSlashSuggestion) => void;
  onSelectIndex: (index: number) => void;
  selectedIndex: number;
  selectedOptionRef: RefObject<HTMLButtonElement | null>;
  suggestions: ComposerSlashSuggestion[];
  title: string;
}) {
  return (
    <div className="composer-slash-group">
      <div className="composer-slash-group-title">{title}</div>
      {suggestions.map((suggestion) => {
        const index = allSuggestions.indexOf(suggestion);
        return (
          <button
            key={getSlashSuggestionKey(suggestion)}
            ref={index === selectedIndex ? selectedOptionRef : null}
            type="button"
            className={`composer-slash-option ${index === selectedIndex ? "selected" : ""}`}
            role="option"
            aria-selected={index === selectedIndex}
            onMouseDown={(event) => {
              event.preventDefault();
              onSelect(suggestion);
            }}
            onMouseEnter={() => onSelectIndex(index)}
          >
            {renderSlashSuggestion(suggestion)}
          </button>
        );
      })}
    </div>
  );
}

function renderSlashSuggestion(suggestion: ComposerSlashSuggestion) {
  switch (suggestion.type) {
    case "command":
      return (
        <>
          <span className="composer-slash-option-name">
            {suggestion.label}
          </span>
          <span className="composer-slash-option-meta">
            Command · {suggestion.description}
          </span>
        </>
      );
    case "skill":
      return (
        <>
          <span className="composer-slash-option-name">
            ${suggestion.skill.name}
          </span>
          <span className="composer-slash-option-meta">
            Skill · {formatSkillOptionMeta(suggestion.skill)}
          </span>
        </>
      );
    case "workflow":
      return (
        <>
          <span className="composer-slash-option-name">
            /workflow {suggestion.workflow.id}
          </span>
          <span className="composer-slash-option-meta">
            Workflow · {suggestion.workflow.name} ·{" "}
            {formatWorkflowSourceLabel(suggestion.workflow.source)}
          </span>
        </>
      );
  }
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

function formatWorkflowSourceLabel(source: WorkflowSummary["source"]) {
  switch (source) {
    case "home":
      return "home";
    case "project":
      return "project";
    default:
      return "workflow";
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

  const thread = threads.find(
    (candidate) => candidate.id === treeMenu.threadId,
  );
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
        {descendantCount > 0 ? "Delete Subagent Tree" : "Delete Agent"}
      </button>
    </div>
  );
}
