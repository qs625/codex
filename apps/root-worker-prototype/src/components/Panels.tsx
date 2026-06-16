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
  Thread,
  ThreadGoal,
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
  focusedConversationItem,
  imageInputRef,
  isLoadingThread,
  isSending,
  isStoppingTurn,
  goal,
  goalCancelError,
  goalCanceling,
  onAddDraftSkill,
  onCancelGoal,
  onConversationScroll,
  onDraftChange,
  onHandleComposerPaste,
  onHandleImageSelection,
  onOpenLocalFile,
  onRemoveDraftImage,
  onRemoveDraftSkill,
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
  goalCancelError: string | null;
  goalCanceling: boolean;
  onAddDraftSkill: (skill: DraftSkill) => void;
  onCancelGoal: () => void;
  onConversationScroll: () => void;
  onDraftChange: (value: string) => void;
  onHandleComposerPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
  onHandleImageSelection: (event: ChangeEvent<HTMLInputElement>) => void;
  onOpenLocalFile: (target: string) => void;
  onRemoveDraftImage: (imageId: string) => void;
  onRemoveDraftSkill: (path: string) => void;
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
        commandsEnabled: draftImages.length === 0 && draftSkills.length === 0,
        draftSkills,
        query: slashQuery,
      }),
    [availableSkills, draftImages.length, draftSkills, slashQuery],
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
        onDraftChange("");
        onRunSlashCommand(suggestion.commandId);
        setDismissedSlashQuery(null);
        setSelectedSlashIndex(0);
        return;
      case "skill":
        selectSkill(suggestion.skill);
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
            <h1>{selectedThread ? getThreadPath(selectedThread) : "/root"}</h1>
            <span
              className={`status-dot ${threadDisplayStatusClass(selectedThread)}`}
            />
            <span>
              {selectedThread
                ? getAgentRoleLabel(selectedThread)
                : "Root Agent"}
            </span>
            <span className="subtitle-separator">•</span>
            <span>{getThreadPresenceLabel(selectedThread)}</span>
            <span className="subtitle-separator">•</span>
            <RunConfigPicker
              disabled={isSending || lastTurnInProgress}
              onApply={onUpdateRunConfig}
              selectedThread={selectedThread}
            />
            {selectedThread ? (
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
        cancelError={goalCancelError}
        canceling={goalCanceling}
        onCancel={onCancelGoal}
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
              containerRef={conversationScrollRef}
              focusedItem={focusedConversationListItem}
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
                  No commands or skills match “/{slashQuery ?? ""}”
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
  cancelError,
  canceling,
  goal,
  onCancel,
}: {
  cancelError: string | null;
  canceling: boolean;
  goal: ThreadGoal | null;
  onCancel: () => void;
}) {
  if (!goal && !cancelError) {
    return null;
  }

  const label = goal ? formatGoalStatus(goal.status) : "Goal";
  const objective = goal?.objective ?? "No active goal.";
  const usage = goal ? formatGoalUsage(goal) : "";

  return (
    <section className="goal-strip" aria-label="Thread goal">
      <div className="goal-strip-main">
        <span className={`goal-status-badge ${goal?.status ?? "none"}`}>
          {canceling ? "Cancelling" : label}
        </span>
        <span className="goal-strip-objective" title={objective}>
          {objective}
        </span>
        {usage ? <span className="goal-strip-usage">{usage}</span> : null}
      </div>
      {cancelError ? (
        <span className="goal-strip-error" role="status">
          {cancelError}
        </span>
      ) : null}
      <button
        type="button"
        className="goal-strip-cancel"
        disabled={!goal || canceling}
        title={goal ? "Cancel goal" : "No active goal"}
        onClick={onCancel}
      >
        {canceling ? "Cancelling" : "Cancel"}
      </button>
    </section>
  );
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
        {descendantCount > 0 ? "Delete Agent Tree" : "Delete Agent"}
      </button>
    </div>
  );
}
