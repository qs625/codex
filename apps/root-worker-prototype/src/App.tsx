import {
  type ChangeEvent,
  type ClipboardEvent,
  type ComponentType,
  type DragEvent,
  type PointerEvent,
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {
  buildBlankChatThreadDraft,
  ConversationPanel,
  SidebarPanel,
  TreeContextMenu,
} from "./components/Panels";
import { type GitDiffPreviewState } from "./components/RightPanel";
import {
  BrowserPanel,
  type BrowserWorkspaceTabDescriptor,
} from "./components/BrowserPanel";
import {
  isSelfCommandShortcut,
  normalizeSelfCommandText,
  SelfCommandDialog,
  type SelfCommandProject,
} from "./components/SelfCommandDialog";
import { SettingsPanel } from "./components/SettingsPanel";
import { TerminalPanel } from "./components/TerminalPanel";
import {
  BrowserIcon,
  PlusIcon,
  RobotIcon,
  TerminalIcon,
  XIcon,
} from "./components/icons";
import {
  clearComposerDraft,
  getComposerDraft,
  isClearComposerCommand,
  parseGoalComposerCommand,
  updateComposerDraft,
  type ComposerDraft,
  type ComposerDraftsByThreadId,
  type GoalComposerCommand,
} from "./lib/composerDraft";
import { buildConversationState } from "./lib/conversation";
import { filterConversationCellsForDisplay } from "./lib/conversationPresentation";
import { clientLifecycleFailureReason } from "./lib/clientLifecycleStatus";
import { isConversationNearBottom } from "./lib/conversationScroll";
import {
  getProjectFilePreview,
  rememberProjectFilePreview,
  rememberSavedProjectFilePreview,
  shouldRestoreProjectFilePreview,
  type FilePreviewMemoryByRootId,
} from "./lib/filePreviewMemory";
import {
  isHtmlFilePreview,
  localPathToFileUrl,
} from "./lib/filePreviewBrowser";
import {
  readImageBlob,
  readImageFile,
  revokeComposerImage,
} from "./lib/images";
import {
  readStoredRightPanelView,
  storeRightPanelView,
} from "./lib/rightPanelView";
import { resolveThreadAnalysisCommandFocus } from "./lib/threadAnalysisCommandFocus";
import {
  hasWorkspaceObjectDragData,
  readWorkspaceObjectDragData,
  writeWorkspaceObjectDragData,
  type WorkspaceObjectDragPayload,
} from "./lib/workspaceObjectDrag";
import {
  runtimeRestartProgressFromBootstrap,
  runtimeRestartProgressFromStatus,
  type RuntimeRestartProgress,
} from "./lib/runtimeRestartProgress";
import type { RunConfigSelection } from "./lib/runConfig";
import { applyRunConfigOverride } from "./lib/sendMessagePayload";
import { submitThreadMessage } from "./lib/sendMessageFlow";
import type { ComposerSlashCommandId } from "./lib/slashMenu";
import { isThreadNotFoundError, toErrorMessage } from "./lib/shared";
import { maybeNotifyProjectThreadCompleted } from "./lib/systemNotification";
import { isChatCompatCwd } from "./lib/chatCompat";
import type { TerminalCommandFocusRequest } from "./lib/terminalCommandFocus";
import {
  decideThreadSelectionAction,
  isSelectedThreadLoading,
  nextThreadReadRequestId,
  shouldApplyThreadReadSnapshot,
} from "./lib/threadSelectionPolicy";
import {
  appendAgentDelta,
  appendCommandExecutionDelta,
  applyOrQueueInitializedThreadUpdate,
  applyPendingThreadUpdates,
  buildCurrentThreadTodoItems,
  buildProjectAgentSidebar,
  findProjectByRootIdentity,
  getAgentRoleLabel,
  getThreadItemNotificationSyntheticTurnStatus,
  getThreadSubtreeIds,
  getThreadSubtreeIdsChildrenFirst,
  getThreadItemNotificationTargetThreadIds,
  getTreeRootThreadId,
  getThreadDepth,
  getInterruptibleTurn,
  getRootThreadConversationTitle,
  getThreadPath,
  getThreadPresenceLabel,
  isCompletedFinalLifecycleStatus,
  isActiveTurnMismatchError,
  isRootThread,
  isSubagentThread,
  markThreadCommandExecutionRunning,
  mergeDefaultCollapsedProjectIds,
  normalizeProjectCwd,
  normalizeThreadSnapshot,
  orderSidebarProjectsStable,
  pickBootstrapInitialProjectThread,
  pickInitialProjectThread,
  pickInitialThread,
  preserveTerminalLifecycleStatus,
  queuePendingThreadUpdate,
  revealThreadInSidebarState,
  rootAgentPathFromTaskName,
  shouldRefreshThreadAfterItemNotification,
  threadDisplayStatusClass,
  updateThreadItem,
  updateThreadLifecycleStatusFromNotification,
  updateThreadSkills,
  updateThreadTurnLifecycle,
  updateThreadTurnNotification,
  updateThreadTurnSnapshot,
  upsertThread,
  upsertThreadMetadataPreservingTurns,
  type ThreadUpdate,
} from "./lib/thread";
import {
  appendVoiceTranscriptDelta,
  buildVoiceDraft,
  finalizeVoiceTranscriptSegment,
  type VoiceDraftState,
} from "./lib/voiceInput";
import {
  beginVoiceCaptureStop,
  type ActiveVoiceSession,
} from "./lib/voiceCaptureState";
import {
  applyStoredWorkspaceTabOrder,
  closeWorkspaceTabById,
  readStoredWorkspaceTabOrder,
  reorderWorkspaceTabs,
  resolveActiveWorkspaceTabId,
  storeWorkspaceTabOrder,
  upsertWorkspaceTab,
  type WorkspaceObjectTab,
  type WorkspaceTabDropPlacement,
} from "./lib/workspaceTabs";
import {
  approvalRequestKey,
  buildApprovalResponse,
  normalizeApprovalRequest,
} from "./lib/approvalRequests";
import type {
  BootstrapResponse,
  AppServerErrorNotification,
  ApprovalDecision,
  ApprovalRequest,
  ComposerImage,
  DraftSkill,
  FilePanelView,
  FileLocation,
  FilePreview,
  FileTreeEntry,
  NewThreadDraft,
  NotificationEnvelope,
  RightPanelView,
  Thread,
  ThreadContextUsage,
  ThreadGoal,
  ThreadItem,
  ThreadPlanUpdate,
  ThreadSkill,
  ThreadTokenUsage,
  ThreadUsage,
  ThreadRealtimeClosedNotification,
  ThreadRealtimeErrorNotification,
  ThreadRealtimeSdpNotification,
  ThreadRealtimeStartedNotification,
  ThreadRealtimeTranscriptDeltaNotification,
  ThreadRealtimeTranscriptDoneNotification,
  TreeMenuState,
  Turn,
  VoiceCaptureStatus,
  WorkflowSummary,
} from "./types";

const LEFT_PANEL_WIDTH_RATIO = 0.17;
const RIGHT_PANEL_WIDTH_RATIO = 0.31;
const LEFT_PANEL_MIN_RATIO = 0.13;
const LEFT_PANEL_MAX_RATIO = 0.34;
const RIGHT_PANEL_MIN_RATIO = 0.22;
const RIGHT_PANEL_MAX_RATIO = 0.46;
const RIGHT_PANEL_COLLAPSED_WIDTH = 46;
const THREAD_SUBSCRIPTION_IDLE_UNSUBSCRIBE_MS = 10 * 60 * 1000;
const SELECTED_THREAD_STORAGE_KEY = "morpheus.rootWorker.selectedThreadId";
const EMPTY_GIT_DIFF_PREVIEW: GitDiffPreviewState = {
  loading: false,
  diff: null,
  error: null,
};
const PANEL_RESIZER_WIDTH = 4;

type LazyRightPanelExport =
  | "FilePreviewPanel"
  | "GitDiffPreviewPanel"
  | "RightPanel";

function lazyRightPanelComponent(exportName: LazyRightPanelExport) {
  return lazy(async () => {
    const module = await import("./components/RightPanel");
    return {
      default: module[exportName] as ComponentType<Record<string, unknown>>,
    };
  });
}

const FilePreviewPanel = lazyRightPanelComponent("FilePreviewPanel");
const GitDiffPreviewPanel = lazyRightPanelComponent("GitDiffPreviewPanel");
const RightPanel = lazyRightPanelComponent("RightPanel");

type GoalActionKind = "set" | "pause" | "resume" | "clear";

function getViewportWidth() {
  return window.innerWidth;
}

function conversationWorkspaceTabId(threadId: string) {
  return `conversation:${threadId}`;
}

function fileWorkspaceTabId(rootId: string | null, path: string) {
  return `file:${rootId ?? "workspace"}:${path}`;
}

function browserWorkspaceTabId(tabId: string | null = null) {
  return `browser:${tabId ?? "active"}`;
}

function terminalWorkspaceTabId(tabId: string | null = null) {
  return `terminal:${tabId ?? "active"}`;
}

function gitDiffWorkspaceTabId(targetId: string) {
  return `diff:${targetId}`;
}

function workspaceTabForThread(thread: Thread): WorkspaceObjectTab {
  const title = isRootThread(thread)
    ? getRootThreadConversationTitle(thread)
    : getThreadPath(thread);
  return {
    id: conversationWorkspaceTabId(thread.id),
    kind: "conversation",
    title,
    subtitle: isRootThread(thread)
      ? getAgentRoleLabel(thread)
      : getThreadPresenceLabel(thread),
    threadId: thread.id,
  };
}

function workspaceTabForFile(
  preview: FilePreview,
  rootId: string | null,
): WorkspaceObjectTab {
  const displayPath = preview.displayPath || preview.path;
  return {
    id: fileWorkspaceTabId(rootId, preview.path),
    kind: "file",
    title: displayPath.split("/").filter(Boolean).at(-1) ?? displayPath,
    subtitle: displayPath,
    rootId,
    path: preview.path,
  };
}

function workspaceTabForBrowser(
  tab: BrowserWorkspaceTabDescriptor | null = null,
): WorkspaceObjectTab {
  return {
    id: browserWorkspaceTabId(tab?.browserTabId ?? null),
    kind: "browser",
    title: tab?.title?.trim() || "Browser",
    subtitle: tab?.url ?? null,
    browserTabId: tab?.browserTabId ?? null,
    url: tab?.url ?? null,
  };
}

function workspaceTabForTerminal(
  tab: Extract<WorkspaceObjectDragPayload, { kind: "terminal" }> | null,
  thread: Thread | null,
): WorkspaceObjectTab {
  return {
    id: terminalWorkspaceTabId(tab?.terminalTabId ?? null),
    kind: "terminal",
    title: "Terminal",
    subtitle: tab?.cwd ?? (thread ? getThreadPath(thread) : null),
    terminalTabId: tab?.terminalTabId ?? null,
    terminalSessionId: tab?.sessionId ?? null,
    threadId: tab?.threadId ?? thread?.id ?? null,
    cwd: tab?.cwd ?? null,
    status: tab?.status ?? null,
  };
}

function workspaceTabForGitDiff(
  state: GitDiffPreviewState,
): WorkspaceObjectTab | null {
  if (!state.targetId) {
    return null;
  }
  const path = state.diff?.path ?? null;
  const fallbackTitle =
    path?.split("/").filter(Boolean).at(-1) ?? state.title ?? "Diff";
  return {
    id: gitDiffWorkspaceTabId(state.targetId),
    kind: "diff",
    title: (state.title ?? fallbackTitle).trim() || "Diff",
    subtitle: state.subtitle ?? path ?? null,
    gitDiffTargetId: state.targetId,
    path,
    status: state.diff?.status ?? null,
  };
}

function workspaceObjectDragPayloadForTab(
  tab: WorkspaceObjectTab,
): WorkspaceObjectDragPayload | null {
  if (tab.kind === "browser" && tab.browserTabId) {
    return {
      kind: "browser",
      browserTabId: tab.browserTabId,
      title: tab.title,
      url: tab.url ?? tab.subtitle ?? null,
    };
  }
  if (tab.kind === "terminal" && tab.terminalTabId) {
    return {
      kind: "terminal",
      terminalTabId: tab.terminalTabId,
      sessionId: tab.terminalSessionId ?? null,
      threadId: tab.threadId ?? null,
      title: tab.title,
      cwd: tab.cwd ?? tab.subtitle ?? null,
      status: tab.status ?? null,
    };
  }
  return null;
}

function getWorkspaceTabThread(
  tab: WorkspaceObjectTab,
  threads: readonly Thread[],
) {
  if (tab.kind !== "conversation" || !tab.threadId) {
    return null;
  }
  return threads.find((thread) => thread.id === tab.threadId) ?? null;
}

function App() {
  const [viewportWidth, setViewportWidth] = useState(getViewportWidth);
  const [sidebarWidth, setSidebarWidth] = useState(() =>
    widthFromRatio(getViewportWidth(), LEFT_PANEL_WIDTH_RATIO),
  );
  const [rightPanelWidth, setRightPanelWidth] = useState(() =>
    widthFromRatio(getViewportWidth(), RIGHT_PANEL_WIDTH_RATIO),
  );
  const [workspace, setWorkspace] = useState("");
  const [threads, setThreads] = useState<Thread[]>([]);
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(null);
  const [latestPlansByThreadId, setLatestPlansByThreadId] = useState<
    Record<string, ThreadPlanUpdate>
  >({});
  const [availableSkills, setAvailableSkills] = useState<ThreadSkill[]>([]);
  const [availableWorkflows, setAvailableWorkflows] = useState<
    WorkflowSummary[]
  >([]);
  const [goalsByThreadId, setGoalsByThreadId] = useState<
    Record<string, ThreadGoal | null>
  >({});
  const [goalActionByThreadId, setGoalActionByThreadId] = useState<
    Record<string, GoalActionKind | null>
  >({});
  const [goalActionErrorsByThreadId, setGoalActionErrorsByThreadId] = useState<
    Record<string, string | null>
  >({});
  const [approvalRequestsById, setApprovalRequestsById] = useState<
    Record<string, ApprovalRequest>
  >({});
  const [composerDraftsByThreadId, setComposerDraftsByThreadId] =
    useState<ComposerDraftsByThreadId>({});
  const [isSending, setIsSending] = useState(false);
  const [isStoppingTurn, setIsStoppingTurn] = useState(false);
  const [isLoadingThread, setIsLoadingThread] = useState(false);
  const [isCreatingChatThread, setIsCreatingChatThread] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isSelfCommandOpen, setIsSelfCommandOpen] = useState(false);
  const [isSelfCommandSubmitting, setIsSelfCommandSubmitting] = useState(false);
  const [selfCommandText, setSelfCommandText] = useState("");
  const [selfCommandProject, setSelfCommandProject] =
    useState<SelfCommandProject | null>(null);
  const [selfCommandError, setSelfCommandError] = useState<string | null>(null);
  const [selfCommandUnavailableMessage, setSelfCommandUnavailableMessage] =
    useState<string | null>(null);
  const [newProjectName, setNewProjectName] = useState("Project chat");
  const [error, setError] = useState<string | null>(null);
  const [collapsedPaths, setCollapsedPaths] = useState<string[]>([]);
  const [collapsedProjectIds, setCollapsedProjectIds] = useState<string[]>([]);
  const [treeMenu, setTreeMenu] = useState<TreeMenuState | null>(null);
  const [rightPanelView, setRightPanelView] = useState<RightPanelView>(
    readStoredRightPanelView,
  );
  const [activeWorkspaceTabId, setActiveWorkspaceTabId] = useState<
    string | null
  >(null);
  const [workspaceTabs, setWorkspaceTabs] = useState<WorkspaceObjectTab[]>([]);
  const [workspaceAddMenuOpen, setWorkspaceAddMenuOpen] = useState(false);
  const [workspaceAddMenuPosition, setWorkspaceAddMenuPosition] = useState<{
    left: number;
    top: number;
  } | null>(null);
  const [draggedWorkspaceTab, setDraggedWorkspaceTab] = useState<string | null>(
    null,
  );
  const [
    rightPanelBrowserTabFocusRequest,
    setRightPanelBrowserTabFocusRequest,
  ] = useState<{ tabId: string; token: number } | null>(null);
  const [
    rightPanelTerminalTabFocusRequest,
    setRightPanelTerminalTabFocusRequest,
  ] = useState<{ tabId: string; token: number } | null>(null);
  const workspaceTabsRef = useRef<WorkspaceObjectTab[]>([]);
  const storedWorkspaceTabOrderRef = useRef(readStoredWorkspaceTabOrder());
  const [runtimeRestartProgress, setRuntimeRestartProgress] =
    useState<RuntimeRestartProgress | null>(null);
  const [isRightPanelCollapsed, setIsRightPanelCollapsed] = useState(false);
  const [isRightPanelResizing, setIsRightPanelResizing] = useState(false);
  const [browserNavigationRequest, setBrowserNavigationRequest] = useState<{
    url: string;
    token: number;
  } | null>(null);
  const [terminalCommandFocusRequest, setTerminalCommandFocusRequest] =
    useState<TerminalCommandFocusRequest | null>(null);
  const [terminalPanelFocusRequestToken, setTerminalPanelFocusRequestToken] =
    useState(0);
  const [filePreview, setFilePreview] = useState<FilePreview | null>(null);
  const [gitDiffPreview, setGitDiffPreview] = useState<GitDiffPreviewState>(
    EMPTY_GIT_DIFF_PREVIEW,
  );
  const [gitDiffWorkspaceStateById, setGitDiffWorkspaceStateById] = useState<
    Record<string, GitDiffPreviewState>
  >({});
  const [filePreviewByRootId, setFilePreviewByRootId] =
    useState<FilePreviewMemoryByRootId>({});
  const [filePanelView, setFilePanelView] = useState<FilePanelView>("preview");
  const [fileTreeEntriesByPath, setFileTreeEntriesByPath] = useState<
    Record<string, FileTreeEntry[]>
  >({});
  const [fileTreeLoadingPath, setFileTreeLoadingPath] = useState<string | null>(
    null,
  );
  const [fileTreeErrorsByPath, setFileTreeErrorsByPath] = useState<
    Record<string, string>
  >({});
  const [expandedTreeDirectories, setExpandedTreeDirectories] = useState<
    string[]
  >([]);
  const [isLoadingPreview, setIsLoadingPreview] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [voiceCaptureStatus, setVoiceCaptureStatus] =
    useState<VoiceCaptureStatus>("idle");
  const [voiceCaptureMessage, setVoiceCaptureMessage] = useState<string | null>(
    null,
  );
  const imageInputRef = useRef<HTMLInputElement | null>(null);
  const conversationScrollRef = useRef<HTMLDivElement | null>(null);
  const conversationStateRef = useRef<ReturnType<
    typeof buildConversationState
  > | null>(null);
  const threadsRef = useRef<Thread[]>([]);
  const composerDraftsRef = useRef<ComposerDraftsByThreadId>({});
  const shouldStickConversationToBottomRef = useRef(true);
  const filePreviewRef = useRef<FilePreview | null>(null);
  const symbolBackStackRef = useRef<FileLocation[]>([]);
  const symbolForwardStackRef = useRef<FileLocation[]>([]);
  const selectedTreeRootIdRef = useRef<string | null>(null);
  const filePreviewRequestTokenRef = useRef(0);
  const selectedThreadIdRef = useRef<string | null>(null);
  const selectedThreadCwdRef = useRef<string | null>(null);
  const fileTreeSessionTokenRef = useRef(0);
  const previousRightPanelViewRef = useRef<RightPanelView>(rightPanelView);
  const liveThreadIdsRef = useRef<Set<string>>(new Set());
  const runConfigOverrideByThreadIdRef = useRef<
    Map<string, RunConfigSelection>
  >(new Map());
  const loadedThreadIdsRef = useRef<Set<string>>(new Set());
  const subscribedThreadIdsRef = useRef<Set<string>>(new Set());
  const subscribeThreadPromisesRef = useRef<Map<string, Promise<boolean>>>(
    new Map(),
  );
  const unsubscribeThreadTimersRef = useRef<
    Map<string, ReturnType<typeof setTimeout>>
  >(new Map());
  const previousSelectedThreadIdRef = useRef<string | null>(null);
  const selectionReadThreadIdRef = useRef<string | null>(null);
  const approvalRequestsByIdRef = useRef<Record<string, ApprovalRequest>>({});
  const loadingThreadIdsRef = useRef<Set<string>>(new Set());
  const loadThreadRequestIdsByThreadIdRef = useRef<Map<string, number>>(
    new Map(),
  );
  const pendingThreadUpdatesRef = useRef(new Map<string, ThreadUpdate[]>());
  const projectCompletionNotifiedThreadIdsRef = useRef<Set<string>>(new Set());
  const voiceSessionRef = useRef<ActiveVoiceSession | null>(null);
  const voiceDraftStateRef = useRef<VoiceDraftState | null>(null);
  const voicePeerConnectionRef = useRef<RTCPeerConnection | null>(null);
  const voiceMediaStreamRef = useRef<MediaStream | null>(null);
  const voiceEventsChannelRef = useRef<RTCDataChannel | null>(null);
  const workspaceAddButtonRef = useRef<HTMLButtonElement | null>(null);
  const workspaceAddMenuRef = useRef<HTMLDivElement | null>(null);
  const voiceFinalTranscriptWaitersRef = useRef(
    new Map<string, Set<() => void>>(),
  );
  const touchedProjectCollapseIdsRef = useRef<Set<string>>(new Set());
  const projectOrderIdsRef = useRef<string[]>([]);
  const resizeStateRef = useRef<{
    startX: number;
    startWidth: number;
    panel: "left" | "right";
  } | null>(null);
  const resizePointerCaptureRef = useRef<{
    element: HTMLDivElement;
    pointerId: number;
  } | null>(null);
  threadsRef.current = threads;
  workspaceTabsRef.current = workspaceTabs;

  useEffect(() => {
    void loadBootstrap();
  }, []);

  useEffect(() => {
    const isMac = navigator.platform.toLowerCase().includes("mac");
    document.body.classList.toggle("macos-window-chrome", isMac);
    return () => {
      document.body.classList.remove("macos-window-chrome");
    };
  }, []);

  useEffect(() => {
    storeRightPanelView(rightPanelView);
  }, [rightPanelView]);

  const selectedComposerDraft = getComposerDraft(
    composerDraftsByThreadId,
    selectedThreadId,
  );
  const selectedThreadGoal = selectedThreadId
    ? (goalsByThreadId[selectedThreadId] ?? null)
    : null;
  const selectedThreadGoalAction = selectedThreadId
    ? (goalActionByThreadId[selectedThreadId] ?? null)
    : null;
  const selectedThreadGoalError = selectedThreadId
    ? (goalActionErrorsByThreadId[selectedThreadId] ?? null)
    : null;
  const selectedThread = useMemo(
    () =>
      selectedThreadId
        ? (threads.find((thread) => thread.id === selectedThreadId) ?? null)
        : null,
    [selectedThreadId, threads],
  );
  const selectedThreadWorkspaceTabId = selectedThread
    ? conversationWorkspaceTabId(selectedThread.id)
    : null;
  const draft = selectedComposerDraft.text;
  const draftSkills = selectedComposerDraft.skills;
  const draftImages = selectedComposerDraft.images;

  useEffect(() => {
    if (!selectedThread) {
      return;
    }
    const tab = workspaceTabForThread(selectedThread);
    setWorkspaceTabs((current) =>
      current.map((item) => (item.id === tab.id ? tab : item)),
    );
  }, [selectedThread]);

  useEffect(() => {
    if (selectedThreadId) {
      openConversationWorkspaceTab(selectedThreadId);
    }
  }, [selectedThreadId]);

  useEffect(() => {
    if (!workspaceAddMenuOpen) {
      return;
    }

    function closeWorkspaceAddMenuOnPointerDown(
      event: globalThis.PointerEvent,
    ) {
      const target = event.target;
      if (!(target instanceof Node)) {
        return;
      }
      if (
        workspaceAddButtonRef.current?.contains(target) ||
        workspaceAddMenuRef.current?.contains(target)
      ) {
        return;
      }
      setWorkspaceAddMenuOpen(false);
    }

    function closeWorkspaceAddMenuOnKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setWorkspaceAddMenuOpen(false);
      }
    }

    function closeWorkspaceAddMenuOnViewportChange() {
      setWorkspaceAddMenuOpen(false);
    }

    document.addEventListener(
      "pointerdown",
      closeWorkspaceAddMenuOnPointerDown,
      true,
    );
    document.addEventListener("keydown", closeWorkspaceAddMenuOnKeyDown);
    window.addEventListener("resize", closeWorkspaceAddMenuOnViewportChange);
    document.addEventListener(
      "scroll",
      closeWorkspaceAddMenuOnViewportChange,
      true,
    );
    return () => {
      document.removeEventListener(
        "pointerdown",
        closeWorkspaceAddMenuOnPointerDown,
        true,
      );
      document.removeEventListener("keydown", closeWorkspaceAddMenuOnKeyDown);
      window.removeEventListener(
        "resize",
        closeWorkspaceAddMenuOnViewportChange,
      );
      document.removeEventListener(
        "scroll",
        closeWorkspaceAddMenuOnViewportChange,
        true,
      );
    };
  }, [workspaceAddMenuOpen]);

  useEffect(() => {
    const threadsById = new Map(threads.map((thread) => [thread.id, thread]));
    setWorkspaceTabs((current) =>
      current.flatMap((tab) => {
        if (tab.kind !== "conversation" || !tab.threadId) {
          return [tab];
        }
        const thread = threadsById.get(tab.threadId);
        return thread ? [workspaceTabForThread(thread)] : [];
      }),
    );
  }, [threads]);

  useEffect(() => {
    setActiveWorkspaceTabId((current) => {
      return resolveActiveWorkspaceTabId(
        workspaceTabs,
        current,
        selectedThreadWorkspaceTabId,
      );
    });
  }, [selectedThreadWorkspaceTabId, workspaceTabs]);

  useEffect(() => {
    composerDraftsRef.current = composerDraftsByThreadId;
  }, [composerDraftsByThreadId]);

  useEffect(() => {
    return () => {
      for (const draft of Object.values(composerDraftsRef.current)) {
        for (const image of draft.images) {
          revokeComposerImage(image);
        }
      }
    };
  }, []);

  function syncSelectedThreadLoading() {
    setIsLoadingThread(
      isSelectedThreadLoading(
        selectedThreadIdRef.current,
        loadingThreadIdsRef.current,
      ),
    );
  }

  useEffect(() => {
    const previousSelectedThreadId = previousSelectedThreadIdRef.current;
    previousSelectedThreadIdRef.current = selectedThreadId;
    selectedThreadIdRef.current = selectedThreadId;
    selectedThreadCwdRef.current = selectedThread?.cwd ?? null;
    if (
      previousSelectedThreadId &&
      previousSelectedThreadId !== selectedThreadId
    ) {
      scheduleThreadIdleUnsubscribe(previousSelectedThreadId);
    }
    if (selectedThreadId) {
      clearThreadUnsubscribeTimer(selectedThreadId);
    }
    syncSelectedThreadLoading();
  }, [selectedThread?.cwd, selectedThreadId]);

  useEffect(() => {
    approvalRequestsByIdRef.current = approvalRequestsById;
    for (const threadId of subscribedThreadIdsRef.current) {
      if (threadId === selectedThreadIdRef.current) {
        continue;
      }
      if (hasPendingApprovalRequestForThread(threadId)) {
        clearThreadUnsubscribeTimer(threadId);
      } else {
        scheduleThreadIdleUnsubscribe(threadId);
      }
    }
  }, [approvalRequestsById]);

  useEffect(() => {
    return () => {
      for (const timer of unsubscribeThreadTimersRef.current.values()) {
        clearTimeout(timer);
      }
      unsubscribeThreadTimersRef.current.clear();
    };
  }, []);

  useEffect(() => {
    if (!selectedThreadId) {
      selectionReadThreadIdRef.current = null;
      return;
    }
    const selectionChanged =
      selectionReadThreadIdRef.current !== selectedThreadId;
    const action = decideThreadSelectionAction({
      selectedThreadId,
      hasLocalThread: threads.some((thread) => thread.id === selectedThreadId),
      isLoaded: loadedThreadIdsRef.current.has(selectedThreadId),
      isSubscribed: subscribedThreadIdsRef.current.has(selectedThreadId),
      isLoading: loadingThreadIdsRef.current.has(selectedThreadId),
      hasLiveCache: liveThreadIdsRef.current.has(selectedThreadId),
      selectionChanged,
    });
    if (selectionChanged) {
      selectionReadThreadIdRef.current = selectedThreadId;
    }
    if (action === "readAndSubscribe") {
      void loadThread(selectedThreadId);
      return;
    }
    syncSelectedThreadLoading();
  }, [selectedThreadId, threads]);

  useEffect(() => {
    if (!selectedThreadId) {
      return;
    }
    void refreshThreadGoal(selectedThreadId);
  }, [selectedThreadId]);

  useEffect(() => {
    return () => {
      cleanupVoiceTransport();
    };
  }, []);

  useEffect(() => {
    if (!selectedThreadId) {
      return;
    }
    writeStoredSelectedThreadId(selectedThreadId);
  }, [selectedThreadId]);

  const selectedThreadPlan = selectedThreadId
    ? (latestPlansByThreadId[selectedThreadId] ??
      selectedThread?.latestPlan ??
      null)
    : null;
  const selectedRunConfigOverride = selectedThreadId
    ? (runConfigOverrideByThreadIdRef.current.get(selectedThreadId) ?? null)
    : null;
  const selectedApprovalRequests = useMemo(
    () =>
      Object.values(approvalRequestsById)
        .filter((request) => request.threadId === selectedThreadId)
        .sort((left, right) => left.startedAtMs - right.startedAtMs),
    [approvalRequestsById, selectedThreadId],
  );

  function cleanupVoiceTransport() {
    voiceEventsChannelRef.current = null;
    voicePeerConnectionRef.current?.close();
    voicePeerConnectionRef.current = null;

    const mediaStream = voiceMediaStreamRef.current;
    if (mediaStream) {
      for (const track of mediaStream.getTracks()) {
        track.stop();
      }
    }
    voiceMediaStreamRef.current = null;
  }

  function clearVoiceSession(
    nextStatus: VoiceCaptureStatus,
    nextMessage: string | null,
  ) {
    const threadId = voiceSessionRef.current?.threadId;
    cleanupVoiceTransport();
    voiceSessionRef.current = null;
    voiceDraftStateRef.current = null;
    if (threadId) {
      resolveVoiceFinalTranscriptWaiters(threadId);
    }
    setVoiceCaptureStatus(nextStatus);
    setVoiceCaptureMessage(nextMessage);
  }

  function syncVoiceDraftState(nextState: VoiceDraftState) {
    voiceDraftStateRef.current = nextState;
    const threadId = voiceSessionRef.current?.threadId;
    if (threadId) {
      updateComposerDraftForThread(threadId, (draft) => ({
        ...draft,
        text: buildVoiceDraft(nextState),
      }));
    }
  }

  function resolveVoiceFinalTranscriptWaiters(threadId: string) {
    const waiters = voiceFinalTranscriptWaitersRef.current.get(threadId);
    if (!waiters) {
      return;
    }
    voiceFinalTranscriptWaitersRef.current.delete(threadId);
    for (const resolve of waiters) {
      resolve();
    }
  }

  function waitForVoiceFinalTranscript(threadId: string, timeoutMs: number) {
    if (!voiceDraftStateRef.current?.liveSegment.trim()) {
      return Promise.resolve();
    }

    return new Promise<void>((resolve) => {
      let settled = false;
      const finish = () => {
        if (settled) {
          return;
        }
        settled = true;
        window.clearTimeout(timeout);
        const waiters = voiceFinalTranscriptWaitersRef.current.get(threadId);
        waiters?.delete(finish);
        if (waiters?.size === 0) {
          voiceFinalTranscriptWaitersRef.current.delete(threadId);
        }
        resolve();
      };
      const timeout = window.setTimeout(finish, timeoutMs);
      const waiters =
        voiceFinalTranscriptWaitersRef.current.get(threadId) ??
        new Set<() => void>();
      waiters.add(finish);
      voiceFinalTranscriptWaitersRef.current.set(threadId, waiters);
    });
  }

  useEffect(() => {
    setAvailableSkills([]);
    setAvailableWorkflows([]);
  }, [selectedThreadId]);

  useEffect(() => {
    const activeVoiceSession = voiceSessionRef.current;
    if (!activeVoiceSession) {
      return;
    }
    if (selectedThreadId === activeVoiceSession.threadId) {
      return;
    }
    void stopVoiceCapture(activeVoiceSession.threadId, true);
  }, [selectedThreadId]);

  async function loadAvailableSkills(cwd: string) {
    const payload = (await window.codexDesktop.listSkills(cwd)) as {
      skills: ThreadSkill[];
      errors: string[];
    };
    return payload.skills;
  }

  async function loadAvailableWorkflows(cwd: string) {
    const payload = (await window.codexDesktop.listWorkflows(cwd)) as {
      workflows: WorkflowSummary[];
      diagnostics: unknown[];
    };
    return payload.workflows;
  }

  useEffect(() => {
    const cwd = selectedThread?.cwd ?? null;
    if (!cwd) {
      setAvailableSkills([]);
      setAvailableWorkflows([]);
      return;
    }
    const threadCwd = cwd;

    let cancelled = false;

    async function refreshAvailableSkills() {
      const [skillsResult, workflowsResult] = await Promise.allSettled([
        loadAvailableSkills(threadCwd),
        loadAvailableWorkflows(threadCwd),
      ]);
      if (cancelled) {
        return;
      }

      if (skillsResult.status === "fulfilled") {
        setAvailableSkills(skillsResult.value);
      } else {
        setAvailableSkills([]);
        setError(toErrorMessage(skillsResult.reason));
      }

      if (workflowsResult.status === "fulfilled") {
        setAvailableWorkflows(workflowsResult.value);
      } else {
        setAvailableWorkflows([]);
        setError(toErrorMessage(workflowsResult.reason));
      }
    }

    void refreshAvailableSkills();

    return () => {
      cancelled = true;
    };
  }, [selectedThread?.cwd, selectedThreadId]);

  useEffect(() => {
    const unsubscribe = window.codexDesktop.subscribe((payload) => {
      handleStreamEvent(payload as NotificationEnvelope);
    });
    return unsubscribe;
  }, [selectedThread?.cwd, selectedThreadId]);

  useLayoutEffect(() => {
    shouldStickConversationToBottomRef.current = true;
  }, [selectedThreadId]);

  useEffect(() => {
    filePreviewRef.current = filePreview;
  }, [filePreview]);

  useEffect(() => {
    setFileTreeEntriesByPath({});
    setFileTreeLoadingPath(null);
    setFileTreeErrorsByPath({});
    setExpandedTreeDirectories([]);
    fileTreeSessionTokenRef.current += 1;
    if (rightPanelView === "preview" && selectedThread?.cwd) {
      void loadFileTreeDirectory(selectedThread.cwd);
    }
  }, [selectedThread?.cwd, selectedThreadId]);

  useEffect(() => {
    const previousRightPanelView = previousRightPanelViewRef.current;
    previousRightPanelViewRef.current = rightPanelView;
    if (
      previousRightPanelView !== "preview" &&
      rightPanelView === "preview" &&
      selectedThread?.cwd
    ) {
      ensureFileTreeDirectoryLoaded(selectedThread.cwd);
    }
  }, [rightPanelView, selectedThread?.cwd, selectedThreadId]);

  useEffect(() => {
    function handlePointerMove(event: globalThis.PointerEvent) {
      const resizeState = resizeStateRef.current;
      if (!resizeState) {
        return;
      }

      if (resizeState.panel === "left") {
        setSidebarWidth(
          clampPanelWidth(
            resizeState.startWidth + (event.clientX - resizeState.startX),
            viewportWidth,
            "left",
          ),
        );
        return;
      }

      setRightPanelWidth(
        clampPanelWidth(
          resizeState.startWidth - (event.clientX - resizeState.startX),
          viewportWidth,
          "right",
        ),
      );
    }

    function finishResize() {
      resizeStateRef.current = null;
      const pointerCapture = resizePointerCaptureRef.current;
      resizePointerCaptureRef.current = null;
      if (pointerCapture?.element.hasPointerCapture(pointerCapture.pointerId)) {
        pointerCapture.element.releasePointerCapture(pointerCapture.pointerId);
      }
      setIsRightPanelResizing(false);
      document.body.classList.remove("is-resizing-panels");
    }

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    window.addEventListener("blur", finishResize);

    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
      window.removeEventListener("blur", finishResize);
      finishResize();
    };
  }, [viewportWidth]);

  useEffect(() => {
    function handleResize() {
      setViewportWidth(window.innerWidth);
    }

    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  useEffect(() => {
    setSidebarWidth((current) =>
      clampPanelWidth(current, viewportWidth, "left"),
    );
    setRightPanelWidth((current) =>
      clampPanelWidth(current, viewportWidth, "right"),
    );
  }, [viewportWidth]);

  useEffect(() => {
    if (!filePreview?.lsp.workspaceRoot) {
      return;
    }

    const previewPath = filePreview.path;
    let cancelled = false;

    async function refreshLspStatus() {
      try {
        const status = await window.codexDesktop.lspStatus(previewPath);
        if (cancelled) {
          return;
        }

        setFilePreview((current) =>
          current?.path === previewPath
            ? {
                ...current,
                lsp: {
                  ...current.lsp,
                  enabled: status.enabled,
                  lspStatus: status.lspStatus,
                  reason: status.reason,
                  workspaceRoot: status.workspaceRoot,
                },
              }
            : current,
        );
      } catch {
        // Keep the last known status if the poll fails.
      }
    }

    void refreshLspStatus();
    const intervalId = window.setInterval(() => {
      void refreshLspStatus();
    }, 1000);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [filePreview?.path, filePreview?.lsp.workspaceRoot]);

  useEffect(() => {
    function handleWindowKeyDown(event: KeyboardEvent) {
      if (event.defaultPrevented) {
        return;
      }

      if (isSelfCommandShortcut(event)) {
        event.preventDefault();
        void openSelfCommand();
        return;
      }

      if (event.metaKey && event.key === "[") {
        event.preventDefault();
        void navigateSymbolHistory("back");
        return;
      }

      if (event.metaKey && event.key === "]") {
        event.preventDefault();
        void navigateSymbolHistory("forward");
      }
    }

    function handleWindowMouseDown(event: MouseEvent) {
      if (event.button === 3) {
        event.preventDefault();
        void navigateSymbolHistory("back");
        return;
      }

      if (event.button === 4) {
        event.preventDefault();
        void navigateSymbolHistory("forward");
      }
    }

    window.addEventListener("keydown", handleWindowKeyDown);
    window.addEventListener("mousedown", handleWindowMouseDown);

    return () => {
      window.removeEventListener("keydown", handleWindowKeyDown);
      window.removeEventListener("mousedown", handleWindowMouseDown);
    };
  });

  const conversationCells = useMemo(() => {
    const nextConversationState = buildConversationState(
      selectedThread,
      conversationStateRef.current,
    );
    conversationStateRef.current = nextConversationState;
    return filterConversationCellsForDisplay(nextConversationState.cells);
  }, [selectedThread]);

  useLayoutEffect(() => {
    const container = conversationScrollRef.current;
    if (!container || !shouldStickConversationToBottomRef.current) {
      return;
    }
    container.scrollTop = container.scrollHeight;
  }, [conversationCells, isLoadingThread, selectedThreadId]);

  const selectedTreeRootId = useMemo(() => {
    const seedThread =
      threads.find((thread) => thread.id === selectedThreadId) ??
      pickInitialThread(threads);
    if (!seedThread) {
      return null;
    }
    return getTreeRootThreadId(threads, seedThread.id);
  }, [selectedThreadId, threads]);

  useEffect(() => {
    if (selectedTreeRootIdRef.current === selectedTreeRootId) {
      return;
    }
    selectedTreeRootIdRef.current = selectedTreeRootId;
    filePreviewRequestTokenRef.current += 1;
    symbolBackStackRef.current = [];
    symbolForwardStackRef.current = [];
    setIsLoadingPreview(false);
  }, [selectedTreeRootId]);

  useEffect(() => {
    if (!shouldRestoreProjectFilePreview(rightPanelView, filePanelView)) {
      return;
    }
    if (isLoadingPreview) {
      return;
    }

    setFilePreview(
      getProjectFilePreview(filePreviewByRootId, selectedTreeRootId),
    );
    setPreviewError(null);
    setIsLoadingPreview(false);
  }, [
    filePanelView,
    filePreviewByRootId,
    isLoadingPreview,
    rightPanelView,
    selectedTreeRootId,
  ]);

  const sessionThreads = useMemo(() => {
    if (!selectedTreeRootId) {
      return [];
    }
    return threads.filter(
      (thread) =>
        getTreeRootThreadId(threads, thread.id) === selectedTreeRootId,
    );
  }, [selectedTreeRootId, threads]);
  const rawProjectSidebar = useMemo(
    () => buildProjectAgentSidebar(threads),
    [threads],
  );
  const projectSidebar = useMemo(() => {
    const orderedProjects = orderSidebarProjectsStable(
      rawProjectSidebar.projects,
      projectOrderIdsRef.current,
    );
    return {
      ...rawProjectSidebar,
      projects: orderedProjects,
    };
  }, [rawProjectSidebar]);
  useEffect(() => {
    projectOrderIdsRef.current = projectSidebar.projects.map(
      (project) => project.id,
    );
  }, [projectSidebar.projects]);
  const projectIds = useMemo(
    () => projectSidebar.projects.map((project) => project.id),
    [projectSidebar],
  );
  const todoItems = useMemo(
    () => buildCurrentThreadTodoItems(sessionThreads, selectedThreadId, "all"),
    [selectedThreadId, sessionThreads],
  );
  const collapsedSet = useMemo(() => new Set(collapsedPaths), [collapsedPaths]);
  const collapsedProjectSet = useMemo(
    () => new Set(collapsedProjectIds),
    [collapsedProjectIds],
  );

  useEffect(() => {
    setCollapsedProjectIds((current) =>
      mergeDefaultCollapsedProjectIds(
        current,
        projectIds,
        touchedProjectCollapseIdsRef.current,
      ),
    );
  }, [projectIds]);

  async function loadBootstrap() {
    try {
      const payload =
        (await window.codexDesktop.bootstrap()) as BootstrapResponse;
      setWorkspace(payload.workspace);
      setRuntimeRestartProgress(
        runtimeRestartProgressFromBootstrap(payload.expectedRestart),
      );
      const normalizedThreads = payload.threads.map(normalizeThreadSnapshot);
      touchedProjectCollapseIdsRef.current.clear();
      setCollapsedProjectIds(
        buildProjectAgentSidebar(normalizedThreads).projects.map(
          (project) => project.id,
        ),
      );
      setThreads(normalizedThreads.map(applyQueuedThreadUpdates));
      const excludedInitialThreadIds = payload.materializedSelfThreadId
        ? new Set([payload.materializedSelfThreadId])
        : undefined;
      const preferredProjectThread = pickBootstrapInitialProjectThread(
        normalizedThreads,
        {
          focusThreadId:
            payload.expectedRestart?.focusThreadId ??
            payload.autoResume?.focusThreadId,
          rememberedThreadId: readStoredSelectedThreadId(),
          excludedThreadIds: excludedInitialThreadIds,
        },
      );
      if (preferredProjectThread) {
        setSelectedThreadId(preferredProjectThread.id);
        return;
      }
      setSelectedThreadId(null);
    } catch (loadError) {
      setError(toErrorMessage(loadError));
    }
  }

  function markThreadLoaded(threadId: string) {
    loadedThreadIdsRef.current.add(threadId);
  }

  function markThreadSubscribed(threadId: string) {
    subscribedThreadIdsRef.current.add(threadId);
    clearThreadUnsubscribeTimer(threadId);
    if (threadId !== selectedThreadIdRef.current) {
      scheduleThreadIdleUnsubscribe(threadId);
    }
  }

  function hasPendingApprovalRequestForThread(threadId: string) {
    return Object.values(approvalRequestsByIdRef.current).some(
      (request) => request.threadId === threadId,
    );
  }

  function clearThreadUnsubscribeTimer(threadId: string) {
    const timer = unsubscribeThreadTimersRef.current.get(threadId);
    if (!timer) {
      return;
    }
    clearTimeout(timer);
    unsubscribeThreadTimersRef.current.delete(threadId);
  }

  function clearAllThreadUnsubscribeTimers() {
    for (const timer of unsubscribeThreadTimersRef.current.values()) {
      clearTimeout(timer);
    }
    unsubscribeThreadTimersRef.current.clear();
  }

  function scheduleThreadIdleUnsubscribe(threadId: string) {
    if (
      threadId === selectedThreadIdRef.current ||
      !subscribedThreadIdsRef.current.has(threadId) ||
      hasPendingApprovalRequestForThread(threadId) ||
      unsubscribeThreadTimersRef.current.has(threadId)
    ) {
      return;
    }
    const timer = setTimeout(() => {
      unsubscribeThreadTimersRef.current.delete(threadId);
      if (
        threadId === selectedThreadIdRef.current ||
        !subscribedThreadIdsRef.current.has(threadId) ||
        hasPendingApprovalRequestForThread(threadId)
      ) {
        return;
      }
      void window.codexDesktop
        .unsubscribeThread(threadId)
        .then(() => {
          subscribedThreadIdsRef.current.delete(threadId);
        })
        .catch((unsubscribeError) => {
          setError(toErrorMessage(unsubscribeError));
          scheduleThreadIdleUnsubscribe(threadId);
        });
    }, THREAD_SUBSCRIPTION_IDLE_UNSUBSCRIBE_MS);
    unsubscribeThreadTimersRef.current.set(threadId, timer);
  }

  async function ensureThreadSubscribed(threadId: string) {
    if (subscribedThreadIdsRef.current.has(threadId)) {
      return true;
    }
    const existingPromise = subscribeThreadPromisesRef.current.get(threadId);
    if (existingPromise) {
      return existingPromise;
    }
    const subscribePromise = window.codexDesktop
      .subscribeThread(threadId)
      .then((payload) => {
        const response = payload as { thread?: Thread | null };
        markThreadSubscribed(threadId);
        if (response.thread) {
          const thread = response.thread;
          setThreads((current) => upsertThreadMetadata(current, thread));
        }
        return true;
      })
      .catch((subscribeError) => {
        setError(toErrorMessage(subscribeError));
        return false;
      })
      .finally(() => {
        subscribeThreadPromisesRef.current.delete(threadId);
      });
    subscribeThreadPromisesRef.current.set(threadId, subscribePromise);
    return subscribePromise;
  }

  function updateThreadLocally(
    threadId: string,
    update: (thread: Thread) => Thread,
  ) {
    setThreads((current) => {
      let foundThread = false;
      const next = current.map((thread) => {
        if (thread.id !== threadId) {
          return thread;
        }
        foundThread = true;
        return update(thread);
      });
      if (!foundThread) {
        queuePendingThreadUpdate(
          pendingThreadUpdatesRef.current,
          threadId,
          update,
        );
        return current;
      }
      return next;
    });
  }

  function updateInitializedThreadLocally(
    threadId: string,
    update: (thread: Thread) => Thread,
  ) {
    setThreads((current) =>
      applyOrQueueInitializedThreadUpdate(
        current,
        loadedThreadIdsRef.current,
        pendingThreadUpdatesRef.current,
        threadId,
        update,
      ),
    );
  }

  function upsertThreadWithPending(current: Thread[], thread: Thread) {
    return upsertThread(current, applyQueuedThreadUpdates(thread));
  }

  function upsertThreadMetadata(current: Thread[], thread: Thread) {
    const existing = current.find((candidate) => candidate.id === thread.id);
    if (!existing) {
      return upsertThreadWithPending(current, thread);
    }
    return upsertThreadMetadataPreservingTurns(current, thread);
  }

  function markThreadLive(threadId: string) {
    liveThreadIdsRef.current.add(threadId);
  }

  function applyQueuedThreadUpdates(thread: Thread) {
    return applyPendingThreadUpdates(thread, pendingThreadUpdatesRef.current);
  }

  function updateSelectedComposerDraft(
    update: (draft: ComposerDraft) => ComposerDraft,
  ) {
    setComposerDraftsByThreadId((current) =>
      updateComposerDraft(current, selectedThreadId, update),
    );
  }

  function updateComposerDraftForThread(
    threadId: string,
    update: (draft: ComposerDraft) => ComposerDraft,
  ) {
    setComposerDraftsByThreadId((current) =>
      updateComposerDraft(current, threadId, update),
    );
  }

  function clearComposerDraftForThread(threadId: string | null) {
    setComposerDraftsByThreadId((current) =>
      clearComposerDraft(current, threadId),
    );
  }

  function clearComposerDraftsForThreads(threadIds: Iterable<string>) {
    setComposerDraftsByThreadId((current) => {
      let next = current;
      for (const threadId of threadIds) {
        const draft = next[threadId];
        if (draft) {
          for (const image of draft.images) {
            revokeComposerImage(image);
          }
        }
        next = clearComposerDraft(next, threadId);
      }
      return next;
    });
  }

  function removeThreadLocally(threadIds: Iterable<string>) {
    const threadIdSet = new Set(threadIds);
    for (const threadId of threadIdSet) {
      loadedThreadIdsRef.current.delete(threadId);
      subscribedThreadIdsRef.current.delete(threadId);
      liveThreadIdsRef.current.delete(threadId);
      subscribeThreadPromisesRef.current.delete(threadId);
      clearThreadUnsubscribeTimer(threadId);
      loadingThreadIdsRef.current.delete(threadId);
      loadThreadRequestIdsByThreadIdRef.current.delete(threadId);
      runConfigOverrideByThreadIdRef.current.delete(threadId);
      projectCompletionNotifiedThreadIdsRef.current.delete(threadId);
    }
    clearComposerDraftsForThreads(threadIdSet);
    setLatestPlansByThreadId((current) => {
      const next = { ...current };
      for (const threadId of threadIdSet) {
        delete next[threadId];
        pendingThreadUpdatesRef.current.delete(threadId);
      }
      return next;
    });
    setApprovalRequestsById((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(
          ([, request]) => !threadIdSet.has(request.threadId),
        ),
      );
      return Object.keys(next).length === Object.keys(current).length
        ? current
        : next;
    });
    setThreads((current) => {
      const next = current.filter((thread) => !threadIdSet.has(thread.id));
      setSelectedThreadId((selected) =>
        selected && threadIdSet.has(selected)
          ? (pickInitialProjectThread(next)?.id ?? null)
          : selected,
      );
      return next;
    });
  }

  function updateThreadLifecycleStatusLocally(
    threadId: string,
    lifecycleStatus: Thread["lifecycleStatus"],
  ) {
    updateThreadLocally(threadId, (thread) =>
      updateThreadLifecycleStatusFromNotification(thread, lifecycleStatus),
    );
  }

  function updateThreadNameLocally(threadId: string, name: Thread["name"]) {
    updateThreadLocally(threadId, (thread) => ({ ...thread, name }));
  }

  function updateThreadRunConfigLocally(
    threadId: string,
    selection: {
      model: string | null;
      modelProvider: string | null;
      reasoningEffort: string | null;
    },
  ) {
    const existingThread =
      threads.find((thread) => thread.id === threadId) ?? null;
    const previousSelection = existingThread
      ? {
          model: existingThread.model,
          modelProvider: existingThread.modelProvider,
          reasoningEffort: existingThread.reasoningEffort,
        }
      : null;
    updateThreadLocally(threadId, (thread) => {
      return {
        ...thread,
        model: selection.model,
        modelProvider: selection.modelProvider ?? thread.modelProvider,
        reasoningEffort: selection.reasoningEffort,
      };
    });
    return previousSelection;
  }

  async function updateSelectedThreadRunConfig(selection: RunConfigSelection) {
    const threadId = selectedThreadId;
    if (!threadId) {
      return;
    }
    setError(null);
    runConfigOverrideByThreadIdRef.current.set(threadId, selection);
    const previousSelection = updateThreadRunConfigLocally(threadId, selection);
    try {
      await window.codexDesktop.setThreadRunConfig({
        threadId,
        model: selection.model,
        modelProvider: selection.modelProvider,
        reasoningEffort: selection.reasoningEffort,
      });
    } catch (runConfigError) {
      if (previousSelection) {
        updateThreadRunConfigLocally(threadId, previousSelection);
        if (
          previousSelection.model != null &&
          previousSelection.reasoningEffort != null
        ) {
          runConfigOverrideByThreadIdRef.current.set(threadId, {
            model: previousSelection.model,
            modelProvider: previousSelection.modelProvider,
            reasoningEffort: previousSelection.reasoningEffort,
            contextWindow: null,
            maxContextWindow: null,
            autoCompactTokenLimit: null,
          });
        } else {
          runConfigOverrideByThreadIdRef.current.delete(threadId);
        }
      } else {
        runConfigOverrideByThreadIdRef.current.delete(threadId);
      }
      setError(toErrorMessage(runConfigError));
    }
  }

  function updateThreadSkillsLocally(threadId: string, skills: ThreadSkill[]) {
    updateThreadLocally(threadId, (thread) =>
      updateThreadSkills(thread, skills),
    );
  }

  function updateThreadPlanLocally(planUpdate: ThreadPlanUpdate) {
    setLatestPlansByThreadId((current) => ({
      ...current,
      [planUpdate.threadId]: planUpdate,
    }));
    updateThreadLocally(planUpdate.threadId, (thread) => ({
      ...thread,
      latestPlan: planUpdate,
    }));
  }

  function updateThreadUsageLocally(
    threadId: string,
    threadUsage: ThreadUsage,
  ) {
    updateThreadLocally(threadId, (thread) => ({
      ...thread,
      threadUsage: {
        tokenUsage:
          threadUsage.tokenUsage ??
          thread.threadUsage?.tokenUsage ??
          thread.tokenUsage ??
          null,
        contextUsage:
          threadUsage.contextUsage ??
          thread.threadUsage?.contextUsage ??
          thread.contextUsage ??
          null,
      },
      tokenUsage:
        threadUsage.tokenUsage ??
        thread.threadUsage?.tokenUsage ??
        thread.tokenUsage ??
        null,
      contextUsage:
        threadUsage.contextUsage ??
        thread.threadUsage?.contextUsage ??
        thread.contextUsage ??
        null,
    }));
  }

  function updateThreadGoalLocally(threadId: string, goal: ThreadGoal | null) {
    setGoalsByThreadId((current) => ({
      ...current,
      [threadId]: goal,
    }));
    setGoalActionErrorsByThreadId((current) => ({
      ...current,
      [threadId]: null,
    }));
    setGoalActionByThreadId((current) => ({
      ...current,
      [threadId]: null,
    }));
  }

  async function refreshThreadGoal(threadId: string) {
    try {
      const response = await window.codexDesktop.getThreadGoal(threadId);
      if (selectedThreadIdRef.current !== threadId) {
        return;
      }
      updateThreadGoalLocally(threadId, response.goal as ThreadGoal | null);
    } catch {
      updateThreadGoalLocally(threadId, null);
    }
  }

  async function loadThread(threadId: string) {
    const requestId = nextThreadReadRequestId(
      loadThreadRequestIdsByThreadIdRef.current,
      threadId,
    );
    loadThreadRequestIdsByThreadIdRef.current.set(threadId, requestId);
    loadingThreadIdsRef.current.add(threadId);
    syncSelectedThreadLoading();
    setError(null);
    try {
      const payload = (await window.codexDesktop.readThread(threadId)) as {
        thread: Thread;
      };
      markThreadSubscribed(threadId);
      if (
        !shouldApplyThreadReadSnapshot({
          threadId,
          selectedThreadId: selectedThreadIdRef.current,
          requestId,
          latestRequestId:
            loadThreadRequestIdsByThreadIdRef.current.get(threadId) ?? null,
          isLoaded: loadedThreadIdsRef.current.has(threadId),
        })
      ) {
        return;
      }
      markThreadLoaded(threadId);
      setThreads((current) => upsertThreadWithPending(current, payload.thread));
    } catch (loadError) {
      const latestRequestId =
        loadThreadRequestIdsByThreadIdRef.current.get(threadId) ?? null;
      if (
        selectedThreadIdRef.current !== threadId ||
        latestRequestId !== requestId
      ) {
        return;
      }
      const message = toErrorMessage(loadError);
      if (isThreadNotFoundError(message)) {
        removeThreadLocally([threadId]);
      }
      setError(message);
    } finally {
      const latestRequestId =
        loadThreadRequestIdsByThreadIdRef.current.get(threadId) ?? null;
      if (latestRequestId === requestId) {
        loadingThreadIdsRef.current.delete(threadId);
        loadThreadRequestIdsByThreadIdRef.current.delete(threadId);
      }
      syncSelectedThreadLoading();
    }
  }

  async function openWorkspaceProject() {
    const projectCwd = normalizeProjectCwd(workspace);
    if (!projectCwd) {
      setError("Project chat needs a workspace path from the app server.");
      return;
    }
    const existingProject = findProjectByRootIdentity(
      projectSidebar.projects,
      projectCwd,
      null,
    );
    if (existingProject) {
      expandProjectSection(existingProject.id);
      setSelectedThreadId(existingProject.tree.threadId);
      return;
    }
    await createProjectThread(
      newProjectName.trim() || "Project chat",
      projectCwd,
    );
  }

  async function submitNewThreadDraft(draft: NewThreadDraft) {
    if (draft.mode === "chat") {
      await createProjectThread(draft.taskName || "Chat", undefined, draft);
      return;
    }
    const projectCwd = normalizeProjectCwd(draft.projectPath);
    if (!projectCwd) {
      setError("Project chat needs a project path.");
      return;
    }
    const existingProject = findProjectByRootIdentity(
      projectSidebar.projects,
      projectCwd,
      rootAgentPathFromTaskName(draft.taskName),
    );
    if (existingProject) {
      setError(null);
      expandProjectSection(existingProject.id);
      setSelectedThreadId(existingProject.tree.threadId);
      return;
    }
    await createProjectThread(
      draft.taskName || "Project chat",
      projectCwd,
      draft,
    );
  }

  async function createBlankChatThread() {
    if (isCreatingChatThread) {
      return;
    }
    setIsCreatingChatThread(true);
    try {
      await createProjectThread("Chat", undefined, buildBlankChatThreadDraft());
    } finally {
      setIsCreatingChatThread(false);
    }
  }

  async function createProjectThread(
    name = "Project chat",
    cwd?: string,
    draft?: NewThreadDraft,
  ) {
    const projectCwd = normalizeProjectCwd(cwd);
    if (draft?.mode !== "chat" && !projectCwd) {
      setError("Project chat needs a workspace path from the app server.");
      return;
    }
    setError(null);
    try {
      const payload = (await window.codexDesktop.createThread({
        ...(projectCwd ? { cwd: projectCwd } : {}),
        threadMode: draft?.mode === "chat" ? "chat" : "project",
        name,
        taskName: draft?.taskName,
        threadProvider: draft?.threadProvider,
        agentType: draft?.agentType,
        model: draft?.model,
        modelProvider: draft?.modelProvider,
        reasoningEffort: draft?.reasoningEffort,
        serviceTier: draft?.serviceTier,
      })) as { thread: Thread };
      markThreadLoaded(payload.thread.id);
      markThreadSubscribed(payload.thread.id);
      setThreads((current) => upsertThreadWithPending(current, payload.thread));
      setSelectedThreadId(payload.thread.id);
    } catch (createError) {
      setError(toErrorMessage(createError));
    }
  }

  async function openSelfCommand() {
    setIsSelfCommandOpen(true);
    setSelfCommandText("");
    setSelfCommandError(null);
    setSelfCommandUnavailableMessage(null);
    setSelfCommandProject(null);

    const desktopApi = (
      window as Window & { codexDesktop?: Window["codexDesktop"] }
    ).codexDesktop;
    if (!desktopApi?.getSelfProject || !desktopApi.startSelfCommand) {
      setSelfCommandUnavailableMessage(
        "Self command is only available in the desktop app.",
      );
      return;
    }

    try {
      const payload = await desktopApi.getSelfProject();
      if (!payload.project) {
        setSelfCommandUnavailableMessage(
          "Self command is available after installing the packaged app.",
        );
        return;
      }
      setSelfCommandProject(payload.project);
    } catch (selfProjectError) {
      setSelfCommandUnavailableMessage("Self command is unavailable.");
      setSelfCommandError(toErrorMessage(selfProjectError));
    }
  }

  function closeSelfCommand() {
    if (isSelfCommandSubmitting) {
      return;
    }
    setIsSelfCommandOpen(false);
    setSelfCommandText("");
    setSelfCommandError(null);
  }

  async function submitSelfCommand() {
    const text = normalizeSelfCommandText(selfCommandText);
    if (!selfCommandProject || !text || isSelfCommandSubmitting) {
      return;
    }
    const desktopApi = (
      window as Window & { codexDesktop?: Window["codexDesktop"] }
    ).codexDesktop;
    if (!desktopApi?.startSelfCommand) {
      setSelfCommandError("Self command is only available in the desktop app.");
      return;
    }

    setIsSelfCommandSubmitting(true);
    setSelfCommandError(null);
    try {
      const payload = await desktopApi.startSelfCommand({ text });
      const thread = payload.thread as Thread;
      markThreadLoaded(thread.id);
      markThreadSubscribed(thread.id);
      setThreads((current) => upsertThreadWithPending(current, thread));
      setSelectedThreadId(thread.id);
      setIsSelfCommandOpen(false);
      setSelfCommandText("");
    } catch (selfCommandError) {
      setSelfCommandError(toErrorMessage(selfCommandError));
    } finally {
      setIsSelfCommandSubmitting(false);
    }
  }

  async function clearCurrentRootSession() {
    if (!selectedTreeRootId) {
      return;
    }

    const rootThread =
      threads.find((thread) => thread.id === selectedTreeRootId) ?? null;
    const replacementName =
      rootThread?.name ?? rootThread?.agentNickname ?? "root";
    const threadIdsToArchive = [...sessionThreads]
      .sort((left, right) => {
        const leftDepth = getThreadDepth(threads, left.id);
        const rightDepth = getThreadDepth(threads, right.id);
        return rightDepth - leftDepth;
      })
      .map((thread) => thread.id);

    setError(null);
    setIsSending(true);
    try {
      for (const threadId of threadIdsToArchive) {
        await window.codexDesktop.archiveThread(threadId);
      }
      setThreads((current) =>
        current.filter((thread) => !threadIdsToArchive.includes(thread.id)),
      );
      setSelectedThreadId(null);
      clearComposerDraftsForThreads(threadIdsToArchive);
      const replacementProjectCwd = isChatCompatCwd(rootThread?.cwd)
        ? null
        : normalizeProjectCwd(rootThread?.cwd);
      await createProjectThread(
        replacementName,
        replacementProjectCwd ?? undefined,
        replacementProjectCwd
          ? undefined
          : {
              mode: "chat",
              projectPath: "",
              title: replacementName,
              taskName: "",
              agentType: null,
              model: null,
              modelProvider: null,
              reasoningEffort: null,
              serviceTier: null,
            },
      );
    } catch (clearError) {
      setError(toErrorMessage(clearError));
    } finally {
      setIsSending(false);
    }
  }

  async function sendMessage() {
    const threadId = selectedThreadId;
    const draftToSend = getComposerDraft(composerDraftsByThreadId, threadId);
    if (
      !threadId ||
      (!draftToSend.text.trim() &&
        draftToSend.images.length === 0 &&
        draftToSend.skills.length === 0)
    ) {
      return;
    }
    if (isClearComposerCommand(draftToSend)) {
      await clearCurrentRootSession();
      return;
    }
    const goalCommand = parseGoalComposerCommand(draftToSend);
    if (goalCommand) {
      await runCurrentThreadGoalCommand(goalCommand);
      return;
    }
    setIsSending(true);
    setError(null);
    const runConfigOverride =
      runConfigOverrideByThreadIdRef.current.get(threadId);
    const threadForSend = applyRunConfigOverride(
      selectedThread,
      runConfigOverride ?? null,
    );
    try {
      await submitThreadMessage({
        draft: draftToSend,
        thread: threadForSend,
        threadId,
        sendMessage: async (payload) =>
          (await window.codexDesktop.sendMessage(payload)) as {
            turn?: Turn | null;
          },
        applyTurn: (targetThreadId, turn) => {
          updateInitializedThreadLocally(targetThreadId, (thread) =>
            updateThreadTurnSnapshot(thread, turn),
          );
        },
        clearDraft: clearComposerDraftForThread,
        revokeImage: revokeComposerImage,
      });
    } catch (sendError) {
      setError(toErrorMessage(sendError));
    } finally {
      setIsSending(false);
    }
  }

  function runComposerSlashCommand(commandId: ComposerSlashCommandId) {
    switch (commandId) {
      case "clear":
        void clearCurrentRootSession();
        return;
      case "goalCreate":
      case "goalPause":
      case "goalResume":
      case "goalCancel":
        return;
    }
  }

  async function runCurrentThreadGoalCommand(command: GoalComposerCommand) {
    switch (command.type) {
      case "set":
        await setCurrentThreadGoal({
          objective: command.objective,
          status: command.status,
          action: "set",
        });
        return;
      case "status":
        await setCurrentThreadGoal({
          status: command.status,
          action: command.status === "active" ? "resume" : "pause",
        });
        return;
      case "clear":
        await clearCurrentThreadGoal();
        return;
      case "invalid":
        showCurrentThreadGoalError(command.message);
        return;
    }
  }

  function showCurrentThreadGoalError(message: string) {
    if (!selectedThreadId) {
      return;
    }
    setGoalActionErrorsByThreadId((current) => ({
      ...current,
      [selectedThreadId]: message,
    }));
  }

  async function setCurrentThreadGoal({
    action,
    objective,
    status,
  }: {
    action: Exclude<GoalActionKind, "clear">;
    objective?: string;
    status: ThreadGoal["status"];
  }) {
    if (!selectedThreadId) {
      return;
    }

    const threadId = selectedThreadId;
    if (!objective && !goalsByThreadId[threadId]) {
      setGoalActionErrorsByThreadId((current) => ({
        ...current,
        [threadId]: `No active goal to ${action}.`,
      }));
      clearComposerDraftForThread(threadId);
      return;
    }

    setGoalActionByThreadId((current) => ({
      ...current,
      [threadId]: action,
    }));
    setGoalActionErrorsByThreadId((current) => ({
      ...current,
      [threadId]: null,
    }));
    setError(null);
    try {
      const response = await window.codexDesktop.setThreadGoal({
        threadId,
        objective,
        status,
      });
      updateThreadGoalLocally(threadId, response.goal as ThreadGoal);
      clearComposerDraftForThread(threadId);
    } catch (goalError) {
      setGoalActionErrorsByThreadId((current) => ({
        ...current,
        [threadId]: toErrorMessage(goalError),
      }));
    } finally {
      setGoalActionByThreadId((current) => ({
        ...current,
        [threadId]: null,
      }));
    }
  }

  function pauseCurrentThreadGoal() {
    void setCurrentThreadGoal({
      action: "pause",
      status: "paused",
    });
  }

  function resumeCurrentThreadGoal() {
    void setCurrentThreadGoal({
      action: "resume",
      status: "active",
    });
  }

  function clearCurrentThreadGoalFromUi() {
    void clearCurrentThreadGoal();
  }

  async function clearCurrentThreadGoal() {
    if (!selectedThreadId) {
      return;
    }
    const threadId = selectedThreadId;
    setGoalActionByThreadId((current) => ({
      ...current,
      [threadId]: "clear",
    }));
    setGoalActionErrorsByThreadId((current) => ({
      ...current,
      [threadId]: null,
    }));
    setError(null);
    try {
      const response = await window.codexDesktop.clearThreadGoal(threadId);
      updateThreadGoalLocally(threadId, null);
      if (!response.cleared) {
        setGoalActionErrorsByThreadId((current) => ({
          ...current,
          [threadId]: "No active goal to cancel.",
        }));
      }
      clearComposerDraftForThread(threadId);
    } catch (goalError) {
      setGoalActionErrorsByThreadId((current) => ({
        ...current,
        [threadId]: toErrorMessage(goalError),
      }));
    } finally {
      setGoalActionByThreadId((current) => ({
        ...current,
        [threadId]: null,
      }));
    }
  }

  async function interruptCurrentTurn() {
    if (!selectedThreadId || isStoppingTurn) {
      return;
    }

    const thread =
      threadsRef.current.find(
        (candidate) => candidate.id === selectedThreadId,
      ) ?? selectedThread;
    const currentTurn = getInterruptibleTurn(thread);
    if (!currentTurn) {
      return;
    }

    setIsStoppingTurn(true);
    setError(null);
    try {
      await window.codexDesktop.interruptTurn({
        threadId: selectedThreadId,
        turnId: currentTurn.id,
      });
    } catch (interruptError) {
      const message = toErrorMessage(interruptError);
      if (isActiveTurnMismatchError(message)) {
        void loadThread(selectedThreadId);
        setError("The running turn changed. Refreshed the thread status.");
      } else {
        setError(message);
      }
      setIsStoppingTurn(false);
    }
  }

  async function handleImageSelection(event: ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? []);
    if (files.length === 0) {
      return;
    }

    try {
      const images = await Promise.all(files.map(readImageFile));
      updateSelectedComposerDraft((draft) => ({
        ...draft,
        images: [...draft.images, ...images],
      }));
    } catch (loadError) {
      setError(toErrorMessage(loadError));
    } finally {
      event.target.value = "";
    }
  }

  function removeDraftImage(imageId: string) {
    updateSelectedComposerDraft((draft) => {
      const next = draft.images.filter((image) => image.id !== imageId);
      const removed = draft.images.find((image) => image.id === imageId);
      if (removed) {
        revokeComposerImage(removed);
      }
      return { ...draft, images: next };
    });
  }

  function addDraftSkill(skill: DraftSkill) {
    updateSelectedComposerDraft((draft) =>
      draft.skills.some((candidate) => candidate.path === skill.path)
        ? draft
        : { ...draft, skills: [...draft.skills, skill] },
    );
  }

  function handleDraftChange(value: string) {
    updateSelectedComposerDraft((draft) => ({ ...draft, text: value }));
    if (voiceSessionRef.current?.threadId === selectedThreadId) {
      voiceDraftStateRef.current = {
        baseDraft: value,
        committedSegments: [],
        liveSegment: "",
      };
    }
  }

  function removeDraftSkill(path: string) {
    updateSelectedComposerDraft((draft) => ({
      ...draft,
      skills: draft.skills.filter((skill) => skill.path !== path),
    }));
  }

  async function startVoiceCapture() {
    if (
      !selectedThreadId ||
      voiceSessionRef.current ||
      !navigator.mediaDevices?.getUserMedia ||
      typeof RTCPeerConnection === "undefined"
    ) {
      if (
        !navigator.mediaDevices?.getUserMedia ||
        typeof RTCPeerConnection === "undefined"
      ) {
        setVoiceCaptureStatus("error");
        setVoiceCaptureMessage(
          "Voice input is not supported in this renderer.",
        );
      }
      return;
    }

    setError(null);
    setVoiceCaptureStatus("requesting");
    setVoiceCaptureMessage("Requesting microphone access…");
    const threadId = selectedThreadId;

    try {
      const microphoneAccess =
        await window.codexDesktop.requestMicrophoneAccess();
      if (!microphoneAccess.granted) {
        setVoiceCaptureStatus("error");
        setVoiceCaptureMessage(
          `Microphone access is ${microphoneAccess.status}. Enable it in System Settings.`,
        );
        return;
      }

      const mediaStream = await navigator.mediaDevices.getUserMedia({
        audio: true,
      });
      const peerConnection = new RTCPeerConnection();

      voiceMediaStreamRef.current = mediaStream;
      voicePeerConnectionRef.current = peerConnection;
      voiceSessionRef.current = {
        threadId,
        status: "connecting",
      };
      voiceDraftStateRef.current = {
        baseDraft: draft,
        committedSegments: [],
        liveSegment: "",
      };

      peerConnection.onconnectionstatechange = () => {
        if (voiceSessionRef.current?.threadId !== threadId) {
          return;
        }
        if (peerConnection.connectionState === "connected") {
          setVoiceCaptureStatus("listening");
          setVoiceCaptureMessage("Listening… tap stop when finished.");
          return;
        }
        if (
          peerConnection.connectionState === "failed" ||
          peerConnection.connectionState === "disconnected"
        ) {
          clearVoiceSession(
            "error",
            `Voice connection ${peerConnection.connectionState}.`,
          );
        }
      };

      for (const track of mediaStream.getAudioTracks()) {
        peerConnection.addTrack(track, mediaStream);
      }

      const eventsChannel = peerConnection.createDataChannel("oai-events");
      voiceEventsChannelRef.current = eventsChannel;

      const offer = await peerConnection.createOffer();
      await peerConnection.setLocalDescription(offer);

      const sdp = peerConnection.localDescription?.sdp;
      if (!sdp) {
        throw new Error("Failed to prepare a realtime voice session.");
      }

      setVoiceCaptureStatus("connecting");
      setVoiceCaptureMessage("Connecting voice input…");

      await window.codexDesktop.startRealtime({
        threadId,
        outputModality: "text",
        transport: {
          type: "webrtc",
          sdp,
        },
      });
    } catch (voiceError) {
      clearVoiceSession("error", toErrorMessage(voiceError));
    }
  }

  async function stopVoiceCapture(
    threadId = voiceSessionRef.current?.threadId,
    silent = false,
  ) {
    const pendingStop = beginVoiceCaptureStop(threadId, silent);
    if (!pendingStop) {
      return;
    }

    voiceSessionRef.current = pendingStop.nextSession;
    setVoiceCaptureStatus(pendingStop.nextStatus);
    setVoiceCaptureMessage(pendingStop.nextMessage);

    const eventsChannel = voiceEventsChannelRef.current;
    if (eventsChannel?.readyState === "open") {
      try {
        const finalTranscript = waitForVoiceFinalTranscript(
          pendingStop.nextSession.threadId,
          3500,
        );
        eventsChannel.send(
          JSON.stringify({ type: "input_audio_buffer.commit" }),
        );
        await finalTranscript;
      } catch (commitError) {
        clearVoiceSession("error", toErrorMessage(commitError));
        return;
      }
    }

    try {
      await window.codexDesktop.stopRealtime({
        threadId: pendingStop.nextSession.threadId,
      });
      cleanupVoiceTransport();
    } catch (stopError) {
      clearVoiceSession("error", toErrorMessage(stopError));
    }
  }

  function toggleVoiceCapture() {
    if (voiceSessionRef.current) {
      void stopVoiceCapture();
      return;
    }
    void startVoiceCapture();
  }

  async function handleComposerPaste(
    event: ClipboardEvent<HTMLTextAreaElement>,
  ) {
    const imageFiles = Array.from(event.clipboardData.items)
      .filter((item) => item.type.startsWith("image/"))
      .map((item) => item.getAsFile())
      .filter((file): file is File => file !== null);

    if (imageFiles.length === 0) {
      return;
    }

    event.preventDefault();

    try {
      const images = await Promise.all(
        imageFiles.map((file, index) =>
          readImageBlob(file, file.name || `pasted-image-${index + 1}.png`),
        ),
      );
      updateSelectedComposerDraft((draft) => ({
        ...draft,
        images: [...draft.images, ...images],
      }));
    } catch (loadError) {
      setError(toErrorMessage(loadError));
    }
  }

  async function archiveThread(threadId: string) {
    setError(null);
    setTreeMenu(null);
    try {
      const thread = threads.find((candidate) => candidate.id === threadId);
      if (thread && isRootThread(thread)) {
        throw new Error(
          "Project chat cannot be deleted from the subagent menu.",
        );
      }
      const archive = window.codexDesktop.archiveThread;
      if (typeof archive !== "function") {
        throw new Error(
          "This build does not expose archiveThread. Please reload Electron.",
        );
      }
      await archive(threadId);
      removeThreadLocally(getThreadSubtreeIds(threads, threadId));
    } catch (archiveError) {
      setError(toErrorMessage(archiveError));
    }
  }

  async function archiveChatThread(threadId: string) {
    setError(null);
    setTreeMenu(null);
    try {
      const chatThreadIds = new Set(
        projectSidebar.chat.conversations.map((node) => node.threadId),
      );
      if (!chatThreadIds.has(threadId)) {
        throw new Error("Only chat conversations can be deleted from Chat.");
      }
      const archive = window.codexDesktop.archiveThread;
      if (typeof archive !== "function") {
        throw new Error(
          "This build does not expose archiveThread. Please reload Electron.",
        );
      }
      await archive(threadId);
      removeThreadLocally(getThreadSubtreeIds(threads, threadId));
    } catch (archiveError) {
      setError(toErrorMessage(archiveError));
    }
  }

  async function archiveProjectThread(threadId: string) {
    setError(null);
    setTreeMenu(null);
    try {
      const projectThreadIds = new Set(
        projectSidebar.projects.map((project) => project.tree.threadId),
      );
      if (!projectThreadIds.has(threadId)) {
        throw new Error("Only project chats can be deleted from Projects.");
      }
      const archive = window.codexDesktop.archiveThread;
      if (typeof archive !== "function") {
        throw new Error(
          "This build does not expose archiveThread. Please reload Electron.",
        );
      }
      const threadIdsToArchive = getThreadSubtreeIdsChildrenFirst(
        threads,
        threadId,
      );
      for (const archivedThreadId of threadIdsToArchive) {
        await archive(archivedThreadId);
      }
      removeThreadLocally(threadIdsToArchive);
    } catch (archiveError) {
      setError(toErrorMessage(archiveError));
    }
  }

  function toggleTreeNode(threadId: string) {
    setCollapsedPaths((current) =>
      current.includes(threadId)
        ? current.filter((value) => value !== threadId)
        : [...current, threadId],
    );
  }

  function toggleProject(projectId: string) {
    touchedProjectCollapseIdsRef.current.add(projectId);
    setCollapsedProjectIds((current) =>
      current.includes(projectId)
        ? current.filter((value) => value !== projectId)
        : [...current, projectId],
    );
  }

  function expandProjectSection(projectId: string) {
    touchedProjectCollapseIdsRef.current.add(projectId);
    setCollapsedProjectIds((current) =>
      current.filter((collapsedProjectId) => collapsedProjectId !== projectId),
    );
  }

  function revealThreadInSidebar(threadId: string) {
    const next = revealThreadInSidebarState({
      collapsedProjectIds,
      collapsedThreadIds: collapsedPaths,
      projectSidebar,
      threadId,
      threads,
    });
    if (next.expandedProjectId) {
      touchedProjectCollapseIdsRef.current.add(next.expandedProjectId);
    }
    setCollapsedPaths(next.collapsedThreadIds);
    setCollapsedProjectIds(next.collapsedProjectIds);
  }

  function selectThread(threadId: string) {
    revealThreadInSidebar(threadId);
    openConversationWorkspaceTab(threadId);
    setSelectedThreadId(threadId);
  }

  function selectProject(_projectId: string, threadId: string) {
    openConversationWorkspaceTab(threadId);
    setSelectedThreadId(threadId);
  }

  function upsertApprovalRequest(request: ApprovalRequest) {
    setApprovalRequestsById((current) => ({
      ...current,
      [approvalRequestKey(request.requestId)]: request,
    }));
  }

  function removeApprovalRequest(threadId: string, requestId: string | number) {
    setApprovalRequestsById((current) => {
      const key = approvalRequestKey(requestId);
      if (current[key]?.threadId !== threadId) {
        return current;
      }
      const next = { ...current };
      delete next[key];
      return next;
    });
  }

  function clearThreadApprovalRequests(threadId: string) {
    setApprovalRequestsById((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(
          ([, request]) => request.threadId !== threadId,
        ),
      );
      return Object.keys(next).length === Object.keys(current).length
        ? current
        : next;
    });
  }

  async function respondToApprovalRequest(
    request: ApprovalRequest,
    decision: ApprovalDecision,
  ) {
    const key = approvalRequestKey(request.requestId);
    setApprovalRequestsById((current) =>
      current[key]
        ? {
            ...current,
            [key]: {
              ...current[key],
              status: "submitting",
              error: null,
            },
          }
        : current,
    );
    setError(null);
    try {
      await window.codexDesktop.respondServerRequest({
        requestId: request.requestId,
        result: buildApprovalResponse(request, decision),
      });
      setApprovalRequestsById((current) => {
        if (!current[key]) {
          return current;
        }
        const next = { ...current };
        delete next[key];
        return next;
      });
    } catch (approvalError) {
      setApprovalRequestsById((current) =>
        current[key]
          ? {
              ...current,
              [key]: {
                ...current[key],
                status: "failed",
                error: toErrorMessage(approvalError),
              },
            }
          : current,
      );
    }
  }

  function handleServerRequest(request: {
    id: string | number;
    method: string;
    params?: unknown;
  }) {
    const approvalRequest = normalizeApprovalRequest(request);
    if (!approvalRequest) {
      const message = `Unsupported app-server request: ${request.method}`;
      setError(message);
      void window.codexDesktop
        .rejectServerRequest({
          requestId: request.id,
          message,
        })
        .catch((requestError) => {
          setError(toErrorMessage(requestError));
        });
      return;
    }
    upsertApprovalRequest(approvalRequest);
    markThreadLive(approvalRequest.threadId);
    if (!selectedThreadIdRef.current) {
      setSelectedThreadId(approvalRequest.threadId);
    }
  }

  function handleStreamEvent(payload: NotificationEnvelope) {
    try {
      if (payload.type === "status" && payload.status) {
        setRuntimeRestartProgress((current) =>
          runtimeRestartProgressFromStatus(payload.status, current),
        );
        const lifecycleFailure = clientLifecycleFailureReason(payload.status);
        if (lifecycleFailure) {
          setError(lifecycleFailure);
        }
        if (!payload.status.connected) {
          clearAllThreadUnsubscribeTimers();
          subscribedThreadIdsRef.current.clear();
          return;
        }
        if (!selectedThreadId) {
          return;
        }
        const action = decideThreadSelectionAction({
          selectedThreadId,
          hasLocalThread: Boolean(selectedThread),
          isLoaded: loadedThreadIdsRef.current.has(selectedThreadId),
          isSubscribed: subscribedThreadIdsRef.current.has(selectedThreadId),
          isLoading: loadingThreadIdsRef.current.has(selectedThreadId),
          hasLiveCache: liveThreadIdsRef.current.has(selectedThreadId),
          selectionChanged: false,
        });
        if (action === "readAndSubscribe") {
          void loadThread(selectedThreadId);
        }
        return;
      }

      if (payload.type !== "notification" || !payload.notification) {
        if (payload.type === "request" && payload.request) {
          handleServerRequest(payload.request);
        }
        return;
      }

      const { method, params } = payload.notification;

      switch (method) {
        case "thread/started": {
          const thread = (params as { thread: Thread }).thread;
          markThreadLive(thread.id);
          setThreads((current) => upsertThreadWithPending(current, thread));
          if (isSubagentThread(thread)) {
            void ensureThreadSubscribed(thread.id);
          } else {
            markThreadSubscribed(thread.id);
          }
          if (!selectedThreadId) {
            setSelectedThreadId(thread.id);
          }
          break;
        }
        case "thread/skills/updated": {
          const notification = params as {
            threadId: string;
            skills: ThreadSkill[];
          };
          updateThreadSkillsLocally(notification.threadId, notification.skills);
          break;
        }
        case "thread/contextUsage/updated": {
          const notification = params as {
            threadId: string;
            tokenUsage: ThreadTokenUsage;
            contextUsage: ThreadContextUsage;
          };
          updateThreadUsageLocally(notification.threadId, {
            tokenUsage: notification.tokenUsage,
            contextUsage: notification.contextUsage,
          });
          break;
        }
        case "thread/tokenUsage/updated": {
          const notification = params as {
            threadId: string;
            tokenUsage: ThreadTokenUsage;
          };
          updateThreadUsageLocally(notification.threadId, {
            tokenUsage: notification.tokenUsage,
            contextUsage: null,
          });
          break;
        }
        case "turn/plan/updated": {
          updateThreadPlanLocally(params as ThreadPlanUpdate);
          break;
        }
        case "skills/changed": {
          if (!selectedThread?.cwd) {
            break;
          }
          void loadAvailableSkills(selectedThread.cwd)
            .then((skills) => {
              setAvailableSkills(skills);
            })
            .catch(() => {
              // Keep the current list if the background refresh fails.
            });
          break;
        }
        case "workflow/run/updated": {
          break;
        }
        case "thread/name/updated":
        case "thread/archived":
        case "thread/closed": {
          if (method === "thread/name/updated") {
            const notification = params as {
              threadId: string;
              threadName?: string | null;
            };
            updateThreadNameLocally(
              notification.threadId,
              notification.threadName ?? null,
            );
            break;
          }
          if (method === "thread/archived") {
            const notification = params as { threadId: string };
            removeThreadLocally([notification.threadId]);
            break;
          }
          break;
        }
        case "thread/status/changed": {
          const notification = params as {
            threadId: string;
            lifecycleStatus: Thread["lifecycleStatus"];
          };
          if (!isCompletedFinalLifecycleStatus(notification.lifecycleStatus)) {
            projectCompletionNotifiedThreadIdsRef.current.delete(
              notification.threadId,
            );
          } else if (
            !projectCompletionNotifiedThreadIdsRef.current.has(
              notification.threadId,
            ) &&
            maybeNotifyProjectThreadCompleted(
              threadsRef.current.find(
                (thread) => thread.id === notification.threadId,
              ) ?? null,
              notification.lifecycleStatus,
            )
          ) {
            projectCompletionNotifiedThreadIdsRef.current.add(
              notification.threadId,
            );
          }
          updateThreadLifecycleStatusLocally(
            notification.threadId,
            notification.lifecycleStatus,
          );
          break;
        }
        case "serverRequest/resolved": {
          const notification = params as {
            threadId: string;
            requestId: string | number;
          };
          removeApprovalRequest(notification.threadId, notification.requestId);
          break;
        }
        case "thread/goal/updated": {
          const notification = params as {
            threadId: string;
            goal: ThreadGoal;
          };
          updateThreadGoalLocally(notification.threadId, notification.goal);
          break;
        }
        case "thread/goal/cleared": {
          const notification = params as { threadId: string };
          updateThreadGoalLocally(notification.threadId, null);
          break;
        }
        case "error": {
          const notification = params as AppServerErrorNotification;
          const message =
            notification.error?.message ?? "App-server reported an error.";
          const details = notification.error?.additionalDetails;
          setError(details ? `${message} ${details}` : message);
          break;
        }
        case "turn/started":
        case "turn/completed": {
          const notification = params as { threadId: string; turn: Turn };
          markThreadLive(notification.threadId);
          clearThreadApprovalRequests(notification.threadId);
          if (
            method === "turn/started" &&
            notification.threadId === selectedThreadId
          ) {
            setError(null);
          }
          if (
            method === "turn/completed" &&
            notification.threadId === selectedThreadId
          ) {
            setIsStoppingTurn(false);
          }
          updateInitializedThreadLocally(notification.threadId, (thread) =>
            updateThreadTurnNotification(thread, method, notification.turn),
          );
          break;
        }
        case "item/started":
        case "item/completed": {
          const notification = params as {
            threadId: string;
            turnId: string;
            item: ThreadItem;
            startedAtMs?: number | null;
            completedAtMs?: number | null;
          };
          for (const threadId of getThreadItemNotificationTargetThreadIds(
            notification.threadId,
            notification.item,
          )) {
            markThreadLive(threadId);
            updateInitializedThreadLocally(threadId, (thread) =>
              updateThreadItem(
                method === "item/started" &&
                  notification.item.type === "commandExecution"
                  ? markThreadCommandExecutionRunning(thread)
                  : thread,
                notification.turnId,
                notification.item,
                {
                  startedAtMs: notification.startedAtMs,
                  completedAtMs: notification.completedAtMs,
                  syntheticTurnStatus:
                    getThreadItemNotificationSyntheticTurnStatus(
                      method,
                      notification.item,
                    ),
                },
              ),
            );
          }
          if (
            shouldRefreshThreadAfterItemNotification(
              method,
              notification.threadId,
              selectedThreadIdRef.current,
              notification.item,
            )
          ) {
            void loadThread(notification.threadId);
          }
          break;
        }
        case "item/agentMessage/delta": {
          const notification = params as {
            threadId: string;
            turnId: string;
            itemId: string;
            delta: string;
          };
          markThreadLive(notification.threadId);
          updateInitializedThreadLocally(notification.threadId, (thread) =>
            appendAgentDelta(
              thread,
              notification.turnId,
              notification.itemId,
              notification.delta,
            ),
          );
          break;
        }
        case "item/commandExecution/outputDelta": {
          const notification = params as {
            threadId: string;
            turnId: string;
            itemId: string;
            delta: string;
          };
          markThreadLive(notification.threadId);
          updateInitializedThreadLocally(notification.threadId, (thread) =>
            appendCommandExecutionDelta(
              markThreadCommandExecutionRunning(thread),
              notification.turnId,
              notification.itemId,
              notification.delta,
            ),
          );
          break;
        }
        case "thread/realtime/started": {
          const notification = params as ThreadRealtimeStartedNotification;
          if (voiceSessionRef.current?.threadId !== notification.threadId) {
            break;
          }
          voiceSessionRef.current = {
            threadId: notification.threadId,
            status: "connecting",
          };
          setVoiceCaptureStatus("connecting");
          setVoiceCaptureMessage(
            "Voice session started. Finalizing connection…",
          );
          break;
        }
        case "thread/realtime/sdp": {
          const notification = params as ThreadRealtimeSdpNotification;
          if (voiceSessionRef.current?.threadId !== notification.threadId) {
            break;
          }
          const peerConnection = voicePeerConnectionRef.current;
          if (!peerConnection) {
            break;
          }
          void peerConnection
            .setRemoteDescription({
              type: "answer",
              sdp: notification.sdp,
            })
            .then(() => {
              if (voiceSessionRef.current?.threadId !== notification.threadId) {
                return;
              }
              voiceSessionRef.current = {
                threadId: notification.threadId,
                status: "listening",
              };
              setVoiceCaptureStatus("listening");
              setVoiceCaptureMessage("Listening… tap stop when finished.");
            })
            .catch((voiceError) => {
              clearVoiceSession("error", toErrorMessage(voiceError));
            });
          break;
        }
        case "thread/realtime/transcript/delta": {
          const notification =
            params as ThreadRealtimeTranscriptDeltaNotification;
          if (
            notification.role !== "user" ||
            voiceSessionRef.current?.threadId !== notification.threadId ||
            !voiceDraftStateRef.current
          ) {
            break;
          }
          syncVoiceDraftState(
            appendVoiceTranscriptDelta(
              voiceDraftStateRef.current,
              notification.delta,
            ),
          );
          break;
        }
        case "thread/realtime/transcript/done": {
          const notification =
            params as ThreadRealtimeTranscriptDoneNotification;
          if (
            notification.role !== "user" ||
            voiceSessionRef.current?.threadId !== notification.threadId ||
            !voiceDraftStateRef.current
          ) {
            break;
          }
          syncVoiceDraftState(
            finalizeVoiceTranscriptSegment(
              voiceDraftStateRef.current,
              notification.text,
            ),
          );
          resolveVoiceFinalTranscriptWaiters(notification.threadId);
          break;
        }
        case "thread/realtime/error": {
          const notification = params as ThreadRealtimeErrorNotification;
          if (voiceSessionRef.current?.threadId !== notification.threadId) {
            break;
          }
          clearVoiceSession("error", notification.message);
          break;
        }
        case "thread/realtime/closed": {
          const notification = params as ThreadRealtimeClosedNotification;
          if (voiceSessionRef.current?.threadId !== notification.threadId) {
            break;
          }
          clearVoiceSession(
            "idle",
            notification.reason
              ? `Voice input ended: ${notification.reason}`
              : null,
          );
          break;
        }
        default:
          break;
      }
    } catch (streamError) {
      setError(
        `Failed to render app-server event: ${toErrorMessage(streamError)}`,
      );
    }
  }

  function handleConversationScroll() {
    const container = conversationScrollRef.current;
    if (!container) {
      return;
    }
    shouldStickConversationToBottomRef.current = isConversationNearBottom({
      scrollHeight: container.scrollHeight,
      clientHeight: container.clientHeight,
      scrollTop: container.scrollTop,
    });
  }

  async function loadFilePreview(
    target: string,
    options: { preserveRightPanel?: boolean } = {},
  ) {
    const requestRootId = selectedTreeRootId;
    const requestToken = filePreviewRequestTokenRef.current + 1;
    filePreviewRequestTokenRef.current = requestToken;
    if (!options.preserveRightPanel) {
      setRightPanelView("preview");
      setIsRightPanelCollapsed(false);
      setFilePanelView("preview");
    }
    setIsLoadingPreview(true);
    setPreviewError(null);

    try {
      const preview = (await window.codexDesktop.readLocalFile(
        target,
      )) as FilePreview;
      if (
        filePreviewRequestTokenRef.current !== requestToken ||
        selectedTreeRootIdRef.current !== requestRootId
      ) {
        return;
      }
      setFilePreview(preview);
      setFilePreviewByRootId((current) =>
        rememberProjectFilePreview(current, requestRootId, preview),
      );
      return { preview, rootId: requestRootId };
    } catch (previewLoadError) {
      if (
        filePreviewRequestTokenRef.current !== requestToken ||
        selectedTreeRootIdRef.current !== requestRootId
      ) {
        return;
      }
      setFilePreview(null);
      setPreviewError(toErrorMessage(previewLoadError));
      return null;
    } finally {
      if (
        filePreviewRequestTokenRef.current === requestToken &&
        selectedTreeRootIdRef.current === requestRootId
      ) {
        setIsLoadingPreview(false);
      }
    }
  }

  async function handleOpenLocalFile(target: string) {
    await openFilePathInWorkspace(target);
  }

  function handleOpenArtifactUrl(url: string) {
    setBrowserNavigationRequest((current) => ({
      url,
      token: (current?.token ?? 0) + 1,
    }));
    setRightPanelView("browser");
    setIsRightPanelCollapsed(false);
  }

  function handleBrowserNavigationRequestHandled(token: number) {
    setBrowserNavigationRequest((current) =>
      current?.token === token ? null : current,
    );
  }

  async function loadFileTreeDirectory(target: string) {
    const requestThreadId = selectedThreadIdRef.current;
    const requestCwd = selectedThreadCwdRef.current;
    const sessionToken = fileTreeSessionTokenRef.current;
    setFileTreeLoadingPath(target);
    setFileTreeErrorsByPath((current) => {
      if (!(target in current)) {
        return current;
      }
      const next = { ...current };
      delete next[target];
      return next;
    });

    try {
      const payload = (await window.codexDesktop.listLocalDirectory(
        target,
      )) as {
        path: string;
        entries: FileTreeEntry[];
      };
      if (
        fileTreeSessionTokenRef.current !== sessionToken ||
        selectedThreadIdRef.current !== requestThreadId ||
        selectedThreadCwdRef.current !== requestCwd
      ) {
        return;
      }
      setFileTreeEntriesByPath((current) => ({
        ...current,
        [target]: payload.entries,
        [payload.path]: payload.entries,
      }));
    } catch (treeLoadError) {
      if (
        fileTreeSessionTokenRef.current !== sessionToken ||
        selectedThreadIdRef.current !== requestThreadId ||
        selectedThreadCwdRef.current !== requestCwd
      ) {
        return;
      }
      setFileTreeErrorsByPath((current) => ({
        ...current,
        [target]: toErrorMessage(treeLoadError),
      }));
    } finally {
      if (
        fileTreeSessionTokenRef.current === sessionToken &&
        selectedThreadIdRef.current === requestThreadId &&
        selectedThreadCwdRef.current === requestCwd
      ) {
        setFileTreeLoadingPath((current) =>
          current === target ? null : current,
        );
      }
    }
  }

  function ensureFileTreeDirectoryLoaded(target: string) {
    if (fileTreeEntriesByPath[target] || fileTreeLoadingPath === target) {
      return;
    }
    void loadFileTreeDirectory(target);
  }

  function handleSetFilePanelView(nextView: FilePanelView) {
    setFilePanelView(nextView);
    if (nextView === "tree" && selectedThread?.cwd) {
      ensureFileTreeDirectoryLoaded(selectedThread.cwd);
    }
  }

  function handleToggleTreeDirectory(target: string) {
    const isExpanded = expandedTreeDirectories.includes(target);
    setExpandedTreeDirectories((current) =>
      isExpanded
        ? current.filter((entry) => entry !== target)
        : [...current, target],
    );

    if (!isExpanded) {
      ensureFileTreeDirectoryLoaded(target);
    }
  }

  function handleOpenTreeFile(target: string) {
    void openFilePathInWorkspace(target);
  }

  async function handleNavigateToSymbol(
    destination: FileLocation,
    sourceLocation: FileLocation,
  ) {
    const currentLocation = normalizeFileLocation(sourceLocation);
    const nextLocation = normalizeFileLocation(destination);
    if (!nextLocation) {
      return;
    }

    if (currentLocation) {
      symbolBackStackRef.current.push(currentLocation);
    }
    symbolForwardStackRef.current = [];
    await navigateToPreviewLocation(nextLocation);
  }

  async function navigateSymbolHistory(direction: "back" | "forward") {
    const currentLocation = normalizeFileLocation(filePreviewRef.current);
    const sourceStack =
      direction === "back"
        ? symbolBackStackRef.current
        : symbolForwardStackRef.current;
    const targetStack =
      direction === "back"
        ? symbolForwardStackRef.current
        : symbolBackStackRef.current;
    const destination = sourceStack.pop();

    if (!destination) {
      return;
    }

    if (currentLocation) {
      targetStack.push(currentLocation);
    }

    await navigateToPreviewLocation(destination);
  }

  async function navigateToPreviewLocation(location: FileLocation) {
    await loadFilePreview(formatFileTarget(location));
  }

  async function openPreviewExternally() {
    if (!filePreview) {
      return;
    }

    try {
      await window.codexDesktop.openLink(filePreview.path);
    } catch (openError) {
      setError(toErrorMessage(openError));
    }
  }

  function openPreviewInBrowser() {
    if (!filePreview || !isHtmlFilePreview(filePreview)) {
      return;
    }

    handleOpenArtifactUrl(localPathToFileUrl(filePreview.path));
  }

  function updateFilePreviewAfterSave(
    preview: FilePreview,
    rootId: string | null,
  ) {
    const currentRootId = selectedTreeRootIdRef.current;
    const currentPreview = filePreviewRef.current;
    if (currentRootId !== rootId || currentPreview?.path !== preview.path) {
      return;
    }
    setFilePreview(preview);
    setFilePreviewByRootId((current) =>
      rememberSavedProjectFilePreview(
        current,
        currentRootId,
        rootId,
        currentPreview,
        preview,
      ),
    );
  }

  function beginResize(
    panel: "left" | "right",
    clientX: number,
    pointerTarget: HTMLDivElement,
    pointerId: number,
  ) {
    resizeStateRef.current = {
      panel,
      startX: clientX,
      startWidth: panel === "left" ? sidebarWidth : rightPanelWidth,
    };
    pointerTarget.setPointerCapture(pointerId);
    resizePointerCaptureRef.current = {
      element: pointerTarget,
      pointerId,
    };
    setIsRightPanelResizing(panel === "right");
    document.body.classList.add("is-resizing-panels");
  }

  function dismissTreeMenu(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) {
      return;
    }
    const target = event.target;
    if (target instanceof Element && target.closest(".tree-context-menu")) {
      return;
    }
    setTreeMenu(null);
  }

  function selectWorkspaceTab(tabId: string) {
    const tab = workspaceTabs.find((item) => item.id === tabId);
    if (!tab) {
      return;
    }
    activateWorkspaceTab(tab);
  }

  function activateWorkspaceTab(tab: WorkspaceObjectTab) {
    if (tab.kind === "conversation" && tab.threadId) {
      if (!threads.some((thread) => thread.id === tab.threadId)) {
        return;
      }
      selectThread(tab.threadId);
    }
    if (tab.kind === "terminal") {
      setTerminalPanelFocusRequestToken((current) => current + 1);
    }
    if (tab.kind === "file" && tab.path) {
      setFilePanelView("preview");
      if (filePreview?.path !== tab.path) {
        void loadFilePreview(tab.path, { preserveRightPanel: true });
      }
    }
    setActiveWorkspaceTabId(tab.id);
  }

  function upsertWorkspaceObjectTab(
    tab: WorkspaceObjectTab,
    options: { activate?: boolean } = {},
  ) {
    const next = applyStoredWorkspaceTabOrder(
      upsertWorkspaceTab(workspaceTabsRef.current, tab),
      storedWorkspaceTabOrderRef.current,
    );
    workspaceTabsRef.current = next;
    setWorkspaceTabs(next);
    storedWorkspaceTabOrderRef.current = storeWorkspaceTabOrder(
      next,
      undefined,
      storedWorkspaceTabOrderRef.current,
    );
    if (options.activate ?? true) {
      activateWorkspaceTab(tab);
    }
  }

  function openConversationWorkspaceTab(threadId: string) {
    const thread = threads.find((item) => item.id === threadId);
    if (!thread) {
      return;
    }
    const tab = workspaceTabForThread(thread);
    upsertWorkspaceObjectTab(tab, { activate: false });
    setActiveWorkspaceTabId(tab.id);
  }

  function openCurrentFileInWorkspace() {
    if (!filePreview) {
      return;
    }
    if (rightPanelView === "preview") {
      setRightPanelView("skills");
    }
    upsertWorkspaceObjectTab(
      workspaceTabForFile(filePreview, selectedTreeRootIdRef.current),
    );
  }

  async function openFilePathInWorkspace(target: string) {
    const result = await loadFilePreview(target, { preserveRightPanel: true });
    if (!result) {
      return;
    }
    upsertWorkspaceObjectTab(
      workspaceTabForFile(result.preview, result.rootId),
    );
  }

  function openBrowserInWorkspace(
    tab: BrowserWorkspaceTabDescriptor | null = null,
  ) {
    upsertWorkspaceObjectTab(workspaceTabForBrowser(tab));
  }

  function openTerminalInWorkspace(
    tab: Extract<
      WorkspaceObjectDragPayload,
      { kind: "terminal" }
    > | null = null,
  ) {
    upsertWorkspaceObjectTab(workspaceTabForTerminal(tab, selectedThread));
  }

  function toggleWorkspaceAddMenu() {
    if (workspaceAddMenuOpen) {
      setWorkspaceAddMenuOpen(false);
      return;
    }
    const button = workspaceAddButtonRef.current;
    if (!button) {
      return;
    }
    const rect = button.getBoundingClientRect();
    setWorkspaceAddMenuPosition({
      left: Math.max(8, Math.min(rect.left, window.innerWidth - 232)),
      top: rect.bottom + 6,
    });
    setWorkspaceAddMenuOpen(true);
  }

  function selectWorkspaceAddMenuItem(kind: "terminal" | "browser" | "chat") {
    setWorkspaceAddMenuOpen(false);
    if (kind === "terminal") {
      openTerminalInWorkspace();
      return;
    }
    if (kind === "browser") {
      openBrowserInWorkspace();
      return;
    }
    void createBlankChatThread();
  }

  function handleGitDiffPreviewChange(state: GitDiffPreviewState) {
    setGitDiffPreview(state);
    if (!state.targetId) {
      return;
    }
    setGitDiffWorkspaceStateById((current) => ({
      ...current,
      [state.targetId!]: state,
    }));
    const tab = workspaceTabForGitDiff(state);
    if (tab) {
      if (
        state.loading ||
        workspaceTabsRef.current.some((item) => item.id === tab.id)
      ) {
        upsertWorkspaceObjectTab(tab, { activate: state.loading });
      }
    }
  }

  function openRightPanelObjectInWorkspace(
    payload: WorkspaceObjectDragPayload | WorkspaceObjectDragPayload["kind"],
  ) {
    const normalizedPayload =
      typeof payload === "string" ? { kind: payload } : payload;
    const kind = normalizedPayload.kind;
    if (kind === "file") {
      openCurrentFileInWorkspace();
      return;
    }
    if (kind === "browser") {
      openBrowserInWorkspace(
        "browserTabId" in normalizedPayload ? normalizedPayload : null,
      );
      return;
    }
    openTerminalInWorkspace(
      "terminalTabId" in normalizedPayload ? normalizedPayload : null,
    );
  }

  function handleSetRightPanelView(view: RightPanelView) {
    setRightPanelView(view);
  }

  function closeWorkspaceTab(
    tabId: string,
    options: { closeOwnedBrowserTab?: boolean } = {},
  ) {
    const currentTabs = workspaceTabsRef.current;
    const closingIndex = currentTabs.findIndex((tab) => tab.id === tabId);
    if (closingIndex === -1) {
      return;
    }
    const closingTab = currentTabs[closingIndex];
    const next = closeWorkspaceTabById(currentTabs, tabId);
    workspaceTabsRef.current = next;
    setWorkspaceTabs(next);
    storedWorkspaceTabOrderRef.current =
      storedWorkspaceTabOrderRef.current.filter((id) => id !== tabId);
    storedWorkspaceTabOrderRef.current = storeWorkspaceTabOrder(
      next,
      undefined,
      storedWorkspaceTabOrderRef.current,
    );
    if (
      options.closeOwnedBrowserTab &&
      closingTab?.kind === "browser" &&
      closingTab.browserTabId
    ) {
      void window.codexDesktop
        .closeBrowserTab(closingTab.browserTabId)
        .catch((error) => setError(toErrorMessage(error)));
    }
    if (visibleWorkspaceTabId !== tabId) {
      return;
    }
    const fallback =
      next[closingIndex] ?? next[closingIndex - 1] ?? next[0] ?? null;
    if (fallback) {
      activateWorkspaceTab(fallback);
      return;
    }
    setActiveWorkspaceTabId(null);
  }

  function handleWorkspaceObjectDragOver(event: DragEvent<HTMLElement>) {
    if (hasWorkspaceObjectDragData(event.dataTransfer)) {
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
    }
  }

  function handleWorkspaceObjectDrop(event: DragEvent<HTMLElement>) {
    const payload = readWorkspaceObjectDragData(event.dataTransfer);
    if (!payload) {
      return;
    }
    event.preventDefault();
    openRightPanelObjectInWorkspace(payload);
  }

  function handleReturnWorkspaceObjectToRightPanel(
    payload: WorkspaceObjectDragPayload,
  ) {
    if (payload.kind === "browser") {
      const tab = workspaceTabsRef.current.find(
        (item) =>
          item.kind === "browser" && item.browserTabId === payload.browserTabId,
      );
      if (!tab) {
        return;
      }
      closeWorkspaceTab(tab.id);
      setRightPanelView("browser");
      setIsRightPanelCollapsed(false);
      setRightPanelBrowserTabFocusRequest((current) => ({
        tabId: payload.browserTabId,
        token: (current?.token ?? 0) + 1,
      }));
      return;
    }
    if (payload.kind === "terminal") {
      const tab = workspaceTabsRef.current.find(
        (item) =>
          item.kind === "terminal" &&
          item.terminalTabId === payload.terminalTabId,
      );
      if (!tab) {
        return;
      }
      closeWorkspaceTab(tab.id);
      setRightPanelView("terminal");
      setIsRightPanelCollapsed(false);
      setRightPanelTerminalTabFocusRequest((current) => ({
        tabId: payload.terminalTabId,
        token: (current?.token ?? 0) + 1,
      }));
    }
  }

  function handleWorkspaceTabDragStart(
    event: DragEvent<HTMLButtonElement>,
    tabId: string,
  ) {
    setDraggedWorkspaceTab(tabId);
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", tabId);
    const tab = workspaceTabsRef.current.find((item) => item.id === tabId);
    const payload = tab ? workspaceObjectDragPayloadForTab(tab) : null;
    if (payload) {
      writeWorkspaceObjectDragData(event.dataTransfer, payload);
    }
  }

  function handleWorkspaceTabDragOver(
    event: DragEvent<HTMLButtonElement>,
    targetTabId: string,
  ) {
    if (!draggedWorkspaceTab || draggedWorkspaceTab === targetTabId) {
      return;
    }
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    const placement = getWorkspaceTabDropPlacement(event);
    const next = reorderWorkspaceTabs(
      workspaceTabsRef.current,
      draggedWorkspaceTab,
      targetTabId,
      placement,
    );
    workspaceTabsRef.current = next;
    setWorkspaceTabs(next);
    storedWorkspaceTabOrderRef.current = storeWorkspaceTabOrder(
      next,
      undefined,
      storedWorkspaceTabOrderRef.current,
    );
  }

  function handleWorkspaceTabDrop(
    event: DragEvent<HTMLButtonElement>,
    targetTabId: string,
  ) {
    if (
      !draggedWorkspaceTab &&
      hasWorkspaceObjectDragData(event.dataTransfer)
    ) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const draggedTab =
      draggedWorkspaceTab ?? event.dataTransfer.getData("text/plain");
    const placement = getWorkspaceTabDropPlacement(event);
    const next = reorderWorkspaceTabs(
      workspaceTabsRef.current,
      draggedTab,
      targetTabId,
      placement,
    );
    workspaceTabsRef.current = next;
    setWorkspaceTabs(next);
    storedWorkspaceTabOrderRef.current = storeWorkspaceTabOrder(
      next,
      undefined,
      storedWorkspaceTabOrderRef.current,
    );
    setDraggedWorkspaceTab(null);
  }

  function getWorkspaceTabDropPlacement(
    event: DragEvent<HTMLButtonElement>,
  ): WorkspaceTabDropPlacement {
    const bounds = event.currentTarget.getBoundingClientRect();
    return event.clientX > bounds.left + bounds.width / 2 ? "after" : "before";
  }

  function handleThreadAnalysisCommandFocus(
    monitor: Parameters<typeof resolveThreadAnalysisCommandFocus>[1],
  ) {
    const target = resolveThreadAnalysisCommandFocus(selectedThread, monitor);
    if (!target) {
      return;
    }
    setTerminalCommandFocusRequest((current) => ({
      ...target,
      token: (current?.token ?? 0) + 1,
    }));
    setTerminalPanelFocusRequestToken((current) => current + 1);
    setRightPanelView("terminal");
    setIsRightPanelCollapsed(false);
  }

  const visibleWorkspaceTabId = resolveActiveWorkspaceTabId(
    workspaceTabs,
    activeWorkspaceTabId,
    selectedThreadWorkspaceTabId,
  );
  const activeWorkspaceTab =
    workspaceTabs.find((tab) => tab.id === visibleWorkspaceTabId) ?? null;
  const activeTerminalThread =
    activeWorkspaceTab?.kind === "terminal" && activeWorkspaceTab.threadId
      ? (threads.find((thread) => thread.id === activeWorkspaceTab.threadId) ??
        null)
      : selectedThread;
  const activeWorkspaceDiffState =
    activeWorkspaceTab?.kind === "diff" && activeWorkspaceTab.gitDiffTargetId
      ? (gitDiffWorkspaceStateById[activeWorkspaceTab.gitDiffTargetId] ??
        EMPTY_GIT_DIFF_PREVIEW)
      : EMPTY_GIT_DIFF_PREVIEW;
  const detachedWorkspaceBrowserTabIds = useMemo(
    () =>
      workspaceTabs
        .filter((tab) => tab.kind === "browser" && tab.browserTabId)
        .map((tab) => tab.browserTabId as string),
    [workspaceTabs],
  );
  const detachedWorkspaceTerminalTabIds = useMemo(
    () =>
      workspaceTabs
        .filter((tab) => tab.kind === "terminal" && tab.terminalTabId)
        .map((tab) => tab.terminalTabId as string),
    [workspaceTabs],
  );

  return (
    <div className="app-shell" onPointerDown={dismissTreeMenu}>
      {error ? <div className="error-banner">{error}</div> : null}

      <main
        className="workspace"
        style={{
          gridTemplateColumns: `${sidebarWidth}px ${PANEL_RESIZER_WIDTH}px minmax(0, 1fr) ${PANEL_RESIZER_WIDTH}px ${
            isRightPanelCollapsed
              ? RIGHT_PANEL_COLLAPSED_WIDTH
              : rightPanelWidth
          }px`,
        }}
      >
        <SidebarPanel
          collapsedSet={collapsedSet}
          collapsedProjectSet={collapsedProjectSet}
          isCreatingChatThread={isCreatingChatThread}
          newProjectName={newProjectName}
          onArchiveChatThread={(threadId) => void archiveChatThread(threadId)}
          onArchiveProjectThread={(threadId) =>
            void archiveProjectThread(threadId)
          }
          onCreateChatThread={() => void createBlankChatThread()}
          onCreateProjectThread={() => void openWorkspaceProject()}
          onOpenMenu={setTreeMenu}
          onSelectProject={selectProject}
          onSelectThread={selectThread}
          onSetNewProjectName={setNewProjectName}
          onOpenSettings={() => setIsSettingsOpen(true)}
          onSubmitNewThreadDraft={(draft) => void submitNewThreadDraft(draft)}
          onToggleProject={toggleProject}
          onToggleTreeNode={toggleTreeNode}
          projectSidebar={projectSidebar}
          selectedThreadId={selectedThreadId}
          workspacePath={workspace}
        />
        <div
          className="panel-resizer"
          role="separator"
          aria-label="Resize sidebar"
          onPointerDown={(event) =>
            beginResize(
              "left",
              event.clientX,
              event.currentTarget,
              event.pointerId,
            )
          }
        />
        <section
          className="workspace-main"
          aria-label="Workspace"
          onDragOver={handleWorkspaceObjectDragOver}
          onDrop={handleWorkspaceObjectDrop}
        >
          <div className="workspace-tab-strip">
            <div
              className="workspace-tab-list"
              role="tablist"
              aria-label="Workspace object tabs"
            >
              {workspaceTabs.map((tab) => {
                const active = tab.id === visibleWorkspaceTabId;
                const tabThread = getWorkspaceTabThread(tab, threads);
                return (
                  <button
                    key={tab.id}
                    type="button"
                    className={`workspace-tab ${active ? "active" : ""} ${
                      draggedWorkspaceTab === tab.id ? "dragging" : ""
                    }`}
                    role="tab"
                    aria-selected={active}
                    draggable
                    title={`${tab.title}${tab.subtitle ? ` · ${tab.subtitle}` : ""}`}
                    onClick={() => selectWorkspaceTab(tab.id)}
                    onDragEnd={() => setDraggedWorkspaceTab(null)}
                    onDragOver={(event) =>
                      handleWorkspaceTabDragOver(event, tab.id)
                    }
                    onDragStart={(event) =>
                      handleWorkspaceTabDragStart(event, tab.id)
                    }
                    onDrop={(event) => handleWorkspaceTabDrop(event, tab.id)}
                  >
                    <span
                      className={`workspace-tab-dot ${threadDisplayStatusClass(tabThread)}`}
                    />
                    <span className="workspace-tab-label">{tab.title}</span>
                    <span
                      aria-label={`Close ${tab.title}`}
                      className="workspace-tab-close"
                      role="button"
                      tabIndex={0}
                      onClick={(event) => {
                        event.stopPropagation();
                        closeWorkspaceTab(tab.id, {
                          closeOwnedBrowserTab: true,
                        });
                      }}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          event.stopPropagation();
                          closeWorkspaceTab(tab.id, {
                            closeOwnedBrowserTab: true,
                          });
                        }
                      }}
                    >
                      <XIcon />
                    </span>
                  </button>
                );
              })}
            </div>
            <button
              ref={workspaceAddButtonRef}
              type="button"
              className={`workspace-tab-add-button ${
                workspaceAddMenuOpen ? "active" : ""
              }`}
              aria-label="Add workspace tab"
              aria-haspopup="menu"
              aria-expanded={workspaceAddMenuOpen}
              onClick={toggleWorkspaceAddMenu}
            >
              <PlusIcon />
            </button>
          </div>
          {workspaceAddMenuOpen && workspaceAddMenuPosition ? (
            <div
              ref={workspaceAddMenuRef}
              className="workspace-tab-add-menu"
              role="menu"
              aria-label="Add workspace tab"
              style={{
                left: workspaceAddMenuPosition.left,
                top: workspaceAddMenuPosition.top,
              }}
            >
              <button
                type="button"
                role="menuitem"
                className="workspace-tab-add-menu-item"
                onClick={() => selectWorkspaceAddMenuItem("terminal")}
              >
                <span className="workspace-tab-add-menu-icon">
                  <TerminalIcon />
                </span>
                <span className="workspace-tab-add-menu-copy">
                  <span className="workspace-tab-add-menu-label">Terminal</span>
                  <span className="workspace-tab-add-menu-description">
                    Open a workspace terminal
                  </span>
                </span>
              </button>
              <button
                type="button"
                role="menuitem"
                className="workspace-tab-add-menu-item"
                onClick={() => selectWorkspaceAddMenuItem("browser")}
              >
                <span className="workspace-tab-add-menu-icon">
                  <BrowserIcon />
                </span>
                <span className="workspace-tab-add-menu-copy">
                  <span className="workspace-tab-add-menu-label">Browser</span>
                  <span className="workspace-tab-add-menu-description">
                    Open a workspace browser
                  </span>
                </span>
              </button>
              <button
                type="button"
                role="menuitem"
                className="workspace-tab-add-menu-item"
                disabled={isCreatingChatThread}
                onClick={() => selectWorkspaceAddMenuItem("chat")}
              >
                <span className="workspace-tab-add-menu-icon">
                  <RobotIcon />
                </span>
                <span className="workspace-tab-add-menu-copy">
                  <span className="workspace-tab-add-menu-label">Chat</span>
                  <span className="workspace-tab-add-menu-description">
                    Start a blank chat
                  </span>
                </span>
              </button>
            </div>
          ) : null}
          <div className="workspace-tab-content">
            <div
              className="workspace-tab-panel"
              hidden={activeWorkspaceTab?.kind !== "conversation"}
            >
              <ConversationPanel
                availableSkills={availableSkills}
                availableWorkflows={availableWorkflows}
                approvalRequests={selectedApprovalRequests}
                conversationCells={conversationCells}
                conversationScrollRef={conversationScrollRef}
                draft={draft}
                draftImages={draftImages}
                draftSkills={draftSkills}
                focusedConversationItem={null}
                imageInputRef={imageInputRef}
                isLoadingThread={isLoadingThread}
                isSending={isSending}
                isStoppingTurn={isStoppingTurn}
                goal={selectedThreadGoal}
                goalAction={selectedThreadGoalAction}
                goalActionError={selectedThreadGoalError}
                onAddDraftSkill={addDraftSkill}
                onCancelGoal={clearCurrentThreadGoalFromUi}
                onConversationScroll={handleConversationScroll}
                onDraftChange={handleDraftChange}
                onHandleComposerPaste={(event) =>
                  void handleComposerPaste(event)
                }
                onHandleImageSelection={(event) =>
                  void handleImageSelection(event)
                }
                onOpenLocalFile={(target) => void handleOpenLocalFile(target)}
                onOpenArtifactUrl={handleOpenArtifactUrl}
                onPauseGoal={pauseCurrentThreadGoal}
                onRemoveDraftImage={removeDraftImage}
                onRemoveDraftSkill={removeDraftSkill}
                onRespondApproval={(request, decision) =>
                  void respondToApprovalRequest(request, decision)
                }
                onResumeGoal={resumeCurrentThreadGoal}
                onRunSlashCommand={runComposerSlashCommand}
                onUpdateRunConfig={(selection) =>
                  void updateSelectedThreadRunConfig(selection)
                }
                onSendMessage={() => void sendMessage()}
                onStopTurn={() => void interruptCurrentTurn()}
                onToggleVoiceCapture={toggleVoiceCapture}
                selectedThread={selectedThread}
                selectedThreadId={selectedThreadId}
                voiceCaptureMessage={voiceCaptureMessage}
                voiceCaptureStatus={voiceCaptureStatus}
              />
            </div>
            <div
              className="workspace-tab-panel"
              hidden={activeWorkspaceTab?.kind !== "file"}
            >
              <Suspense
                fallback={
                  <div className="preview-panel preview-empty">
                    Loading preview...
                  </div>
                }
              >
                <FilePreviewPanel
                  variant="workspace"
                  expandedTreeDirectories={expandedTreeDirectories}
                  filePanelView="preview"
                  fileTreeEntriesByPath={fileTreeEntriesByPath}
                  fileTreeErrorsByPath={fileTreeErrorsByPath}
                  fileTreeLoadingPath={fileTreeLoadingPath}
                  gitDiffPreview={null}
                  gitDiffPreviewError={null}
                  gitDiffPreviewLoading={false}
                  onNavigateToSymbol={handleNavigateToSymbol}
                  onOpenPreviewExternally={() => void openPreviewExternally()}
                  onOpenPreviewInBrowser={openPreviewInBrowser}
                  onOpenTreeFile={handleOpenTreeFile}
                  onPreviewUpdated={updateFilePreviewAfterSave}
                  onToggleTreeDirectory={handleToggleTreeDirectory}
                  preview={filePreview}
                  previewError={previewError}
                  previewLoading={isLoadingPreview}
                  previewRootId={selectedTreeRootId}
                  thread={selectedThread}
                />
              </Suspense>
            </div>
            <div
              className="workspace-tab-panel"
              hidden={activeWorkspaceTab?.kind !== "diff"}
            >
              {activeWorkspaceTab?.kind === "diff" ? (
                <div className="preview-panel diff-panel-workspace">
                  <Suspense
                    fallback={
                      <div className="preview-empty">Loading Git diff...</div>
                    }
                  >
                    <GitDiffPreviewPanel
                      diff={activeWorkspaceDiffState.diff}
                      error={activeWorkspaceDiffState.error}
                      loading={activeWorkspaceDiffState.loading}
                    />
                  </Suspense>
                </div>
              ) : null}
            </div>
            <div
              className="workspace-tab-panel"
              hidden={activeWorkspaceTab?.kind !== "browser"}
            >
              {activeWorkspaceTab?.kind === "browser" ? (
                <BrowserPanel
                  active
                  variant="workspace"
                  nativeOverlayActive={
                    isSelfCommandOpen || isSettingsOpen || isCreatingChatThread
                  }
                  resizing={isRightPanelResizing}
                  navigationRequest={browserNavigationRequest}
                  onNavigationRequestHandled={
                    handleBrowserNavigationRequestHandled
                  }
                  onOpenBrowserTabInWorkspace={openBrowserInWorkspace}
                  activeBrowserTabId={activeWorkspaceTab.browserTabId ?? null}
                />
              ) : null}
            </div>
            <div
              className="workspace-tab-panel"
              hidden={activeWorkspaceTab?.kind !== "terminal"}
            >
              {activeWorkspaceTab?.kind === "terminal" ? (
                <TerminalPanel
                  variant="workspace"
                  thread={activeTerminalThread}
                  focusCommandRequest={terminalCommandFocusRequest}
                  focusPanelRequestToken={terminalPanelFocusRequestToken}
                  onOpenTerminalTabInWorkspace={openTerminalInWorkspace}
                  activeTerminalTabId={activeWorkspaceTab.terminalTabId ?? null}
                />
              ) : null}
            </div>
          </div>
        </section>
        <div
          className="panel-resizer"
          role="separator"
          aria-label="Resize right panel"
          onPointerDown={(event) => {
            if (isRightPanelCollapsed) {
              setIsRightPanelCollapsed(false);
            }
            beginResize(
              "right",
              event.clientX,
              event.currentTarget,
              event.pointerId,
            );
          }}
        />
        <Suspense
          fallback={
            <aside
              className={`right-panel ${isRightPanelCollapsed ? "collapsed" : ""}`}
            >
              <div className="right-panel-body">
                <div className="right-panel-content">
                  <div className="preview-empty">Loading panel...</div>
                </div>
              </div>
            </aside>
          }
        >
          <RightPanel
            activeView={rightPanelView}
            browserNativeOverlayActive={
              isSelfCommandOpen || isSettingsOpen || isCreatingChatThread
            }
            browserPanelResizing={isRightPanelResizing}
            browserNavigationRequest={browserNavigationRequest}
            onBrowserNavigationRequestHandled={
              handleBrowserNavigationRequestHandled
            }
            workspaceTabsEnabled
            onGitDiffPreviewChange={handleGitDiffPreviewChange}
            onFocusCommandMonitor={handleThreadAnalysisCommandFocus}
            availableSkillCount={availableSkills.length}
            availableWorkflows={availableWorkflows}
            isCollapsed={isRightPanelCollapsed}
            expandedTreeDirectories={expandedTreeDirectories}
            filePanelView={filePanelView}
            fileTreeEntriesByPath={fileTreeEntriesByPath}
            fileTreeErrorsByPath={fileTreeErrorsByPath}
            fileTreeLoadingPath={fileTreeLoadingPath}
            onNavigateToSymbol={(destination, sourceLocation) =>
              void handleNavigateToSymbol(destination, sourceLocation)
            }
            onOpenPreviewExternally={() => void openPreviewExternally()}
            onOpenPreviewInBrowser={openPreviewInBrowser}
            onOpenBrowserTabInWorkspace={openBrowserInWorkspace}
            onOpenTerminalTabInWorkspace={openTerminalInWorkspace}
            onOpenWorkspaceObject={openRightPanelObjectInWorkspace}
            onReturnWorkspaceObject={handleReturnWorkspaceObjectToRightPanel}
            browserTabFocusRequest={rightPanelBrowserTabFocusRequest}
            terminalTabFocusRequest={rightPanelTerminalTabFocusRequest}
            detachedBrowserTabIds={detachedWorkspaceBrowserTabIds}
            detachedTerminalTabIds={detachedWorkspaceTerminalTabIds}
            onOpenTreeFile={handleOpenTreeFile}
            onPreviewUpdated={updateFilePreviewAfterSave}
            previewRootId={selectedTreeRootId}
            onSetActiveView={handleSetRightPanelView}
            onSetCollapsed={setIsRightPanelCollapsed}
            onSetFilePanelView={handleSetFilePanelView}
            onToggleTreeDirectory={handleToggleTreeDirectory}
            preview={filePreview}
            previewError={previewError}
            previewLoading={isLoadingPreview}
            planUpdate={selectedThreadPlan}
            goal={selectedThreadGoal}
            goalAction={selectedThreadGoalAction}
            goalActionError={selectedThreadGoalError}
            onCancelGoal={clearCurrentThreadGoalFromUi}
            onPauseGoal={pauseCurrentThreadGoal}
            onResumeGoal={resumeCurrentThreadGoal}
            runtimeRestartProgress={runtimeRestartProgress}
            skills={selectedThread?.skills ?? []}
            thread={selectedThread}
            modelContextWindowOverride={
              selectedRunConfigOverride?.contextWindow ?? null
            }
            todoItems={todoItems}
          />
        </Suspense>
      </main>

      <TreeContextMenu
        threads={threads}
        treeMenu={treeMenu}
        onArchiveProjectThread={(threadId) =>
          void archiveProjectThread(threadId)
        }
        onArchiveThread={(threadId) => void archiveThread(threadId)}
      />
      {isSettingsOpen ? (
        <SettingsPanel
          onClose={() => setIsSettingsOpen(false)}
          workspacePath={workspace}
        />
      ) : null}
      <SelfCommandDialog
        error={selfCommandError}
        isOpen={isSelfCommandOpen}
        isSubmitting={isSelfCommandSubmitting}
        onClose={closeSelfCommand}
        onSubmit={() => void submitSelfCommand()}
        onTextChange={setSelfCommandText}
        project={selfCommandProject}
        text={selfCommandText}
        unavailableMessage={selfCommandUnavailableMessage}
      />
    </div>
  );
}

export default App;

function widthFromRatio(viewportWidth: number, ratio: number) {
  return Math.round(viewportWidth * ratio);
}

function clampPanelWidth(
  value: number,
  viewportWidth: number,
  panel: "left" | "right",
) {
  const min =
    panel === "left"
      ? widthFromRatio(viewportWidth, LEFT_PANEL_MIN_RATIO)
      : widthFromRatio(viewportWidth, RIGHT_PANEL_MIN_RATIO);
  const max =
    panel === "left"
      ? widthFromRatio(viewportWidth, LEFT_PANEL_MAX_RATIO)
      : widthFromRatio(viewportWidth, RIGHT_PANEL_MAX_RATIO);
  return clampWidth(value, min, max);
}

function clampWidth(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function formatFileTarget(location: FileLocation) {
  if (location.line == null) {
    return location.path;
  }

  if (location.column == null) {
    return `${location.path}:${location.line}`;
  }

  return `${location.path}:${location.line}:${location.column}`;
}

function normalizeFileLocation(location: FileLocation | FilePreview | null) {
  if (!location || location.line == null) {
    return null;
  }

  return {
    path: location.path,
    line: location.line,
    column: location.column ?? 1,
  };
}

function readStoredSelectedThreadId() {
  try {
    return window.localStorage.getItem(SELECTED_THREAD_STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeStoredSelectedThreadId(threadId: string) {
  try {
    window.localStorage.setItem(SELECTED_THREAD_STORAGE_KEY, threadId);
  } catch {
    // Selection persistence is best-effort. The client should still work in
    // restricted storage environments.
  }
}
