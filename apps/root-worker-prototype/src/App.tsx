import {
  type ChangeEvent,
  type ClipboardEvent,
  type PointerEvent,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {
  ConversationPanel,
  SidebarPanel,
  TreeContextMenu,
} from "./components/Panels";
import { RightPanel } from "./components/RightPanel";
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
import {
  buildConversationEntries,
  buildConversationState,
} from "./lib/conversation";
import {
  extractCompactConversationDetails,
  type LoadedCompactConversationDetails,
} from "./lib/conversationCompact";
import { isConversationNearBottom } from "./lib/conversationScroll";
import {
  readImageBlob,
  readImageFile,
  revokeComposerImage,
} from "./lib/images";
import {
  readStoredRightPanelView,
  storeRightPanelView,
} from "./lib/rightPanelView";
import { advanceCompactHistoryRequestToken } from "./lib/compactHistoryRequest";
import type { RunConfigSelection } from "./lib/runConfig";
import {
  applyRunConfigOverride,
  buildSendMessagePayload,
} from "./lib/sendMessagePayload";
import type { ComposerSlashCommandId } from "./lib/slashMenu";
import { isThreadNotFoundError, toErrorMessage } from "./lib/shared";
import {
  decideThreadSelectionAction,
  nextThreadReadRequestId,
  shouldApplyThreadReadSnapshot,
} from "./lib/threadSelectionPolicy";
import {
  appendAgentDelta,
  appendCommandExecutionDelta,
  applyInitializedThreadUpdate,
  applyPendingThreadUpdates,
  buildCurrentThreadTodoItems,
  buildProjectAgentSidebar,
  getThreadAncestorIds,
  getThreadItemNotificationSyntheticTurnStatus,
  getThreadSubtreeIds,
  getThreadItemNotificationTargetThreadIds,
  getTreeRootThreadId,
  getThreadDepth,
  isRootThread,
  isSubagentThread,
  normalizeProjectCwd,
  normalizeThreadSnapshot,
  pickInitialProjectThread,
  pickInitialThread,
  queuePendingThreadUpdate,
  updateThreadItem,
  updateThreadSkills,
  updateThreadTurnLifecycle,
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
import type {
  BootstrapResponse,
  AppServerErrorNotification,
  ComposerImage,
  DraftSkill,
  FilePanelView,
  FileLocation,
  FilePreview,
  FileTreeEntry,
  NewThreadDraft,
  NotificationEnvelope,
  RightPanelView,
  TaskFilter,
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

type GoalActionKind = "set" | "pause" | "resume" | "clear";
type CompactHistoryViewState = {
  isExpanded: boolean;
  isLoading: boolean;
  error: string | null;
  details: LoadedCompactConversationDetails | null;
};

function getViewportWidth() {
  return window.innerWidth;
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
  const [compactHistoryByThreadId, setCompactHistoryByThreadId] = useState<
    Record<string, Record<string, CompactHistoryViewState>>
  >({});
  const [composerDraftsByThreadId, setComposerDraftsByThreadId] =
    useState<ComposerDraftsByThreadId>({});
  const [isSending, setIsSending] = useState(false);
  const [isStoppingTurn, setIsStoppingTurn] = useState(false);
  const [isLoadingThread, setIsLoadingThread] = useState(false);
  const [newProjectName, setNewProjectName] = useState("Project chat");
  const [error, setError] = useState<string | null>(null);
  const [taskFilter, setTaskFilter] = useState<TaskFilter>("all");
  const [collapsedPaths, setCollapsedPaths] = useState<string[]>([]);
  const [collapsedProjectIds, setCollapsedProjectIds] = useState<string[]>([]);
  const [isChatCollapsed, setIsChatCollapsed] = useState(false);
  const [treeMenu, setTreeMenu] = useState<TreeMenuState | null>(null);
  const [rightPanelView, setRightPanelView] = useState<RightPanelView>(
    readStoredRightPanelView,
  );
  const [focusedConversationItem, setFocusedConversationItem] = useState<{
    itemId: string;
    token: number;
  } | null>(null);
  const [filePreview, setFilePreview] = useState<FilePreview | null>(null);
  const [filePanelView, setFilePanelView] = useState<FilePanelView>("preview");
  const [fileTreeEntriesByPath, setFileTreeEntriesByPath] = useState<
    Record<string, FileTreeEntry[]>
  >({});
  const [fileTreeLoadingPath, setFileTreeLoadingPath] = useState<string | null>(null);
  const [fileTreeErrorsByPath, setFileTreeErrorsByPath] = useState<
    Record<string, string>
  >({});
  const [expandedTreeDirectories, setExpandedTreeDirectories] = useState<string[]>([]);
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
  const composerDraftsRef = useRef<ComposerDraftsByThreadId>({});
  const shouldStickConversationToBottomRef = useRef(true);
  const filePreviewRef = useRef<FilePreview | null>(null);
  const symbolBackStackRef = useRef<FileLocation[]>([]);
  const symbolForwardStackRef = useRef<FileLocation[]>([]);
  const selectedThreadIdRef = useRef<string | null>(null);
  const selectedThreadCwdRef = useRef<string | null>(null);
  const fileTreeSessionTokenRef = useRef(0);
  const liveThreadIdsRef = useRef<Set<string>>(new Set());
  const runConfigOverrideByThreadIdRef = useRef<
    Map<string, RunConfigSelection>
  >(new Map());
  const loadedThreadIdsRef = useRef<Set<string>>(new Set());
  const subscribedThreadIdsRef = useRef<Set<string>>(new Set());
  const subscribeThreadPromisesRef = useRef<Map<string, Promise<boolean>>>(
    new Map(),
  );
  const loadingThreadIdsRef = useRef<Set<string>>(new Set());
  const loadThreadRequestIdsByThreadIdRef = useRef<Map<string, number>>(
    new Map(),
  );
  const compactHistoryRequestIdsRef = useRef<Map<string, number>>(new Map());
  const pendingThreadUpdatesRef = useRef(new Map<string, ThreadUpdate[]>());
  const voiceSessionRef = useRef<ActiveVoiceSession | null>(null);
  const voiceDraftStateRef = useRef<VoiceDraftState | null>(null);
  const voicePeerConnectionRef = useRef<RTCPeerConnection | null>(null);
  const voiceMediaStreamRef = useRef<MediaStream | null>(null);
  const voiceEventsChannelRef = useRef<RTCDataChannel | null>(null);
  const voiceFinalTranscriptWaitersRef = useRef(
    new Map<string, Set<() => void>>(),
  );
  const resizeStateRef = useRef<{
    startX: number;
    startWidth: number;
    panel: "left" | "right";
  } | null>(null);

  useEffect(() => {
    void loadBootstrap();
  }, []);

  useEffect(() => {
    storeRightPanelView(rightPanelView);
  }, [rightPanelView]);

  const selectedComposerDraft = getComposerDraft(
    composerDraftsByThreadId,
    selectedThreadId,
  );
  const selectedThreadGoal = selectedThreadId
    ? goalsByThreadId[selectedThreadId] ?? null
    : null;
  const selectedThreadGoalAction = selectedThreadId
    ? goalActionByThreadId[selectedThreadId] ?? null
    : null;
  const selectedThreadGoalError = selectedThreadId
    ? goalActionErrorsByThreadId[selectedThreadId] ?? null
    : null;
  const selectedThread = useMemo(
    () =>
      selectedThreadId
        ? (threads.find((thread) => thread.id === selectedThreadId) ?? null)
        : null,
    [selectedThreadId, threads],
  );
  const draft = selectedComposerDraft.text;
  const draftSkills = selectedComposerDraft.skills;
  const draftImages = selectedComposerDraft.images;

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

  useEffect(() => {
    if (!selectedThreadId) {
      return;
    }
    const action = decideThreadSelectionAction({
      selectedThreadId,
      hasLocalThread: threads.some((thread) => thread.id === selectedThreadId),
      isLoaded: loadedThreadIdsRef.current.has(selectedThreadId),
      isSubscribed: subscribedThreadIdsRef.current.has(selectedThreadId),
      isLoading: loadingThreadIdsRef.current.has(selectedThreadId),
      hasLiveCache: liveThreadIdsRef.current.has(selectedThreadId),
    });
    if (action === "readAndSubscribe") {
      void loadThread(selectedThreadId);
      return;
    }
    if (action === "subscribeOnly") {
      void ensureThreadSubscribed(selectedThreadId);
    }
  }, [selectedThreadId, threads]);

  useEffect(() => {
    selectedThreadIdRef.current = selectedThreadId;
    selectedThreadCwdRef.current = selectedThread?.cwd ?? null;
    if (!selectedThreadId) {
      setIsLoadingThread(false);
    }
  }, [selectedThread?.cwd, selectedThreadId]);

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

  const selectedThreadPlan = selectedThreadId
    ? (latestPlansByThreadId[selectedThreadId] ??
      selectedThread?.latestPlan ??
      null)
    : null;
  const selectedCompactHistory = selectedThreadId
    ? compactHistoryByThreadId[selectedThreadId] ?? {}
    : {};
  const selectedLoadedCompactDetails = useMemo(
    () =>
      Object.fromEntries(
        Object.entries(selectedCompactHistory)
          .filter(([, state]) => state.isExpanded && state.details)
          .map(([entryId, state]) => [entryId, state.details]),
      ),
    [selectedCompactHistory],
  );
  const selectedRunConfigOverride = selectedThreadId
    ? (runConfigOverrideByThreadIdRef.current.get(selectedThreadId) ?? null)
    : null;

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
    setFilePanelView("preview");
    setFileTreeEntriesByPath({});
    setFileTreeLoadingPath(null);
    setFileTreeErrorsByPath({});
    setExpandedTreeDirectories([]);
    fileTreeSessionTokenRef.current += 1;
  }, [selectedThread?.cwd, selectedThreadId]);

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

    function handlePointerUp() {
      resizeStateRef.current = null;
      document.body.classList.remove("is-resizing-panels");
    }

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);

    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
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
      {
        compactDetailsById: selectedLoadedCompactDetails,
      },
    );
    conversationStateRef.current = nextConversationState;
    return nextConversationState.cells;
  }, [selectedLoadedCompactDetails, selectedThread]);

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
  const sessionThreads = useMemo(() => {
    if (!selectedTreeRootId) {
      return [];
    }
    return threads.filter(
      (thread) =>
        getTreeRootThreadId(threads, thread.id) === selectedTreeRootId,
    );
  }, [selectedTreeRootId, threads]);
  const projectSidebar = useMemo(
    () => buildProjectAgentSidebar(threads),
    [threads],
  );
  const todoItems = useMemo(
    () =>
      buildCurrentThreadTodoItems(sessionThreads, selectedThreadId, taskFilter),
    [selectedThreadId, sessionThreads, taskFilter],
  );
  const collapsedSet = useMemo(() => new Set(collapsedPaths), [collapsedPaths]);
  const collapsedProjectSet = useMemo(
    () => new Set(collapsedProjectIds),
    [collapsedProjectIds],
  );

  useEffect(() => {
    if (!selectedThreadId) {
      return;
    }
    const selectedAncestorIds = new Set(
      getThreadAncestorIds(threads, selectedThreadId),
    );
    const expandSelectedAncestors = () => {
      setCollapsedPaths((current) =>
        current.filter((threadId) => !selectedAncestorIds.has(threadId)),
      );
    };
    const selectedProject = projectSidebar.projects.find((project) =>
      getThreadSubtreeIds(threads, project.tree.threadId).has(selectedThreadId),
    );
    if (selectedProject) {
      setCollapsedProjectIds((current) =>
        current.filter((projectId) => projectId !== selectedProject.id),
      );
      expandSelectedAncestors();
      return;
    }
    if (
      projectSidebar.chat.conversations.some((conversation) =>
        getThreadSubtreeIds(threads, conversation.threadId).has(selectedThreadId),
      )
    ) {
      setIsChatCollapsed(false);
      expandSelectedAncestors();
    }
  }, [projectSidebar, selectedThreadId, threads]);

  async function loadBootstrap() {
    try {
      const payload =
        (await window.codexDesktop.bootstrap()) as BootstrapResponse;
      setWorkspace(payload.workspace);
      const normalizedThreads = payload.threads.map(normalizeThreadSnapshot);
      setThreads(normalizedThreads.map(applyQueuedThreadUpdates));
      const preferredProjectThread = pickInitialProjectThread(normalizedThreads);
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
      applyInitializedThreadUpdate(
        current,
        loadedThreadIdsRef.current,
        threadId,
        update,
      ),
    );
  }

  function upsertThreadWithPending(
    current: Thread[],
    thread: Thread,
  ) {
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
      loadingThreadIdsRef.current.delete(threadId);
      loadThreadRequestIdsByThreadIdRef.current.delete(threadId);
      runConfigOverrideByThreadIdRef.current.delete(threadId);
      for (const requestKey of compactHistoryRequestIdsRef.current.keys()) {
        if (requestKey.startsWith(`${threadId}:`)) {
          compactHistoryRequestIdsRef.current.delete(requestKey);
        }
      }
    }
    clearComposerDraftsForThreads(threadIdSet);
    setCompactHistoryByThreadId((current) => {
      const next = { ...current };
      for (const threadId of threadIdSet) {
        delete next[threadId];
      }
      return next;
    });
    setLatestPlansByThreadId((current) => {
      const next = { ...current };
      for (const threadId of threadIdSet) {
        delete next[threadId];
        pendingThreadUpdatesRef.current.delete(threadId);
      }
      return next;
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

  function updateThreadStatusLocally(
    threadId: string,
    status: Thread["status"],
  ) {
    updateThreadLocally(threadId, (thread) => ({ ...thread, status }));
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

  function updateCompactHistoryState(
    threadId: string,
    entryId: string,
    nextState: CompactHistoryViewState | null,
  ) {
    setCompactHistoryByThreadId((current) => {
      const threadState = current[threadId] ?? {};
      if (nextState === null) {
        if (!(entryId in threadState)) {
          return current;
        }
        const nextThreadState = { ...threadState };
        delete nextThreadState[entryId];
        if (Object.keys(nextThreadState).length === 0) {
          const next = { ...current };
          delete next[threadId];
          return next;
        }
        return {
          ...current,
          [threadId]: nextThreadState,
        };
      }

      return {
        ...current,
        [threadId]: {
          ...threadState,
          [entryId]: nextState,
        },
      };
    });
  }

  async function loadCompactHistory(threadId: string, entryId: string) {
    const requestKey = `${threadId}:${entryId}`;
    const requestId = advanceCompactHistoryRequestToken(
      compactHistoryRequestIdsRef.current,
      requestKey,
    );
    compactHistoryRequestIdsRef.current.set(requestKey, requestId);
    updateCompactHistoryState(threadId, entryId, {
      isExpanded: true,
      isLoading: true,
      error: null,
      details: null,
    });

    try {
      const payload = (await window.codexDesktop.readCompactHistory(threadId)) as {
        thread: Thread;
      };
      const entries = buildConversationEntries(payload.thread);
      const details = extractCompactConversationDetails(entries, entryId);
      const latestRequestId =
        compactHistoryRequestIdsRef.current.get(requestKey) ?? null;
      if (latestRequestId !== requestId) {
        return;
      }
      if (!details) {
        throw new Error("Compact history could not be loaded for this item.");
      }
      updateCompactHistoryState(threadId, entryId, {
        isExpanded: true,
        isLoading: false,
        error: null,
        details,
      });
    } catch (loadError) {
      const latestRequestId =
        compactHistoryRequestIdsRef.current.get(requestKey) ?? null;
      if (latestRequestId !== requestId) {
        return;
      }
      updateCompactHistoryState(threadId, entryId, {
        isExpanded: true,
        isLoading: false,
        error: toErrorMessage(loadError),
        details: null,
      });
    }
  }

  function toggleCompactHistory(entryId: string) {
    const threadId = selectedThreadIdRef.current;
    if (!threadId) {
      return;
    }
    const existing = compactHistoryByThreadId[threadId]?.[entryId] ?? null;
    if (existing?.isExpanded) {
      const requestKey = `${threadId}:${entryId}`;
      compactHistoryRequestIdsRef.current.set(
        requestKey,
        advanceCompactHistoryRequestToken(
          compactHistoryRequestIdsRef.current,
          requestKey,
        ),
      );
      updateCompactHistoryState(threadId, entryId, null);
      return;
    }
    void loadCompactHistory(threadId, entryId);
  }

  async function loadThread(threadId: string) {
    const requestId = nextThreadReadRequestId(
      loadThreadRequestIdsByThreadIdRef.current,
      threadId,
    );
    loadThreadRequestIdsByThreadIdRef.current.set(threadId, requestId);
    loadingThreadIdsRef.current.add(threadId);
    setIsLoadingThread(true);
    setError(null);
    try {
      if (!(await ensureThreadSubscribed(threadId))) {
        return;
      }
      const payload = (await window.codexDesktop.readThread(threadId)) as {
        thread: Thread;
      };
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
      if (
        selectedThreadIdRef.current === threadId &&
        latestRequestId === requestId
      ) {
        setIsLoadingThread(false);
      }
      if (latestRequestId === requestId) {
        loadingThreadIdsRef.current.delete(threadId);
        loadThreadRequestIdsByThreadIdRef.current.delete(threadId);
      }
    }
  }

  async function openWorkspaceProject() {
    const projectCwd = normalizeProjectCwd(workspace);
    if (!projectCwd) {
      setError("Project chat needs a workspace path from the app server.");
      return;
    }
    const existingProject = projectCwd
      ? projectSidebar.projects.find((project) => project.cwd === projectCwd)
      : null;
    if (existingProject) {
      setCollapsedProjectIds((current) =>
        current.filter((projectId) => projectId !== existingProject.id),
      );
      setSelectedThreadId(existingProject.tree.threadId);
      return;
    }
    await createProjectThread(newProjectName.trim() || "Project chat", projectCwd);
  }

  async function submitNewThreadDraft(draft: NewThreadDraft) {
    if (draft.mode === "chat") {
      setError(
        "Chat without project is unavailable in this build because createThread defaults missing cwd to the workspace.",
      );
      return;
    }
    const projectCwd = normalizeProjectCwd(draft.projectPath);
    if (!projectCwd) {
      setError("Project chat needs a project path.");
      return;
    }
    const existingProject = projectSidebar.projects.find(
      (project) => normalizeProjectCwd(project.cwd) === projectCwd,
    );
    if (existingProject) {
      setError(null);
      setCollapsedProjectIds((current) =>
        current.filter((projectId) => projectId !== existingProject.id),
      );
      setSelectedThreadId(existingProject.tree.threadId);
      return;
    }
    await createProjectThread(draft.title || "Project chat", projectCwd, draft);
  }

  async function createProjectThread(
    name = "Project chat",
    cwd = workspace,
    draft?: NewThreadDraft,
  ) {
    const projectCwd = normalizeProjectCwd(cwd);
    if (!projectCwd) {
      setError("Project chat needs a workspace path from the app server.");
      return;
    }
    setError(null);
    try {
      const payload = (await window.codexDesktop.createThread({
        cwd: projectCwd,
        name,
        taskName: draft?.taskName,
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
      await createProjectThread(replacementName);
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
    const runConfigOverride = runConfigOverrideByThreadIdRef.current.get(threadId);
    const threadForSend = applyRunConfigOverride(
      selectedThread,
      runConfigOverride ?? null,
    );
    try {
      await window.codexDesktop.sendMessage({
        ...buildSendMessagePayload({
          draft: draftToSend,
          thread: threadForSend,
          threadId,
        }),
      });
      for (const image of draftToSend.images) {
        revokeComposerImage(image);
      }
      clearComposerDraftForThread(threadId);
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

    const currentTurn = selectedThread?.turns.at(-1) ?? null;
    const turnInProgress =
      currentTurn != null &&
      (currentTurn.status === "inProgress" || currentTurn.completedAt == null);
    if (!turnInProgress) {
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
      setError(toErrorMessage(interruptError));
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
        throw new Error("Project chat cannot be deleted from the subagent menu.");
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

  function toggleTreeNode(threadId: string) {
    setCollapsedPaths((current) =>
      current.includes(threadId)
        ? current.filter((value) => value !== threadId)
        : [...current, threadId],
    );
  }

  function toggleProject(projectId: string) {
    setCollapsedProjectIds((current) =>
      current.includes(projectId)
        ? current.filter((value) => value !== projectId)
        : [...current, projectId],
    );
  }

  function selectProject(projectId: string, threadId: string) {
    setCollapsedProjectIds((current) =>
      current.filter((value) => value !== projectId),
    );
    setSelectedThreadId(threadId);
  }

  function handleStreamEvent(payload: NotificationEnvelope) {
    try {
      if (payload.type === "status" && payload.status) {
        if (!payload.status.connected) {
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
        });
        if (action === "readAndSubscribe") {
          void loadThread(selectedThreadId);
        } else if (action === "subscribeOnly") {
          void ensureThreadSubscribed(selectedThreadId);
        }
        return;
      }

      if (payload.type !== "notification" || !payload.notification) {
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
            status: Thread["status"];
          };
          updateThreadStatusLocally(notification.threadId, notification.status);
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
            updateThreadTurnLifecycle(thread, notification.turn),
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
              updateThreadItem(thread, notification.turnId, notification.item, {
                startedAtMs: notification.startedAtMs,
                completedAtMs: notification.completedAtMs,
                syntheticTurnStatus:
                  getThreadItemNotificationSyntheticTurnStatus(
                    method,
                    notification.item,
                  ),
              }),
            );
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
              thread,
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

  async function loadFilePreview(target: string) {
    setRightPanelView("preview");
    setFilePanelView("preview");
    setIsLoadingPreview(true);
    setPreviewError(null);

    try {
      const preview = (await window.codexDesktop.readLocalFile(
        target,
      )) as FilePreview;
      setFilePreview(preview);
    } catch (previewLoadError) {
      setFilePreview(null);
      setPreviewError(toErrorMessage(previewLoadError));
    } finally {
      setIsLoadingPreview(false);
    }
  }

  async function handleOpenLocalFile(target: string) {
    await loadFilePreview(target);
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
      const payload = (await window.codexDesktop.listLocalDirectory(target)) as {
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
        setFileTreeLoadingPath((current) => (current === target ? null : current));
      }
    }
  }

  function ensureFileTreeDirectoryLoaded(target: string) {
    if (fileTreeEntriesByPath[target]) {
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
    void loadFilePreview(target);
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

  function beginResize(panel: "left" | "right", clientX: number) {
    resizeStateRef.current = {
      panel,
      startX: clientX,
      startWidth: panel === "left" ? sidebarWidth : rightPanelWidth,
    };
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

  return (
    <div className="app-shell" onPointerDown={dismissTreeMenu}>
      {error ? <div className="error-banner">{error}</div> : null}

      <main
        className="workspace"
        style={{
          gridTemplateColumns: `${sidebarWidth}px 1px minmax(0, 1fr) 1px ${rightPanelWidth}px`,
        }}
      >
        <SidebarPanel
          collapsedSet={collapsedSet}
          collapsedProjectSet={collapsedProjectSet}
          isChatCollapsed={isChatCollapsed}
          newProjectName={newProjectName}
          onCreateProjectThread={() => void openWorkspaceProject()}
          onOpenMenu={setTreeMenu}
          onSelectProject={selectProject}
          onSelectThread={setSelectedThreadId}
          onSetNewProjectName={setNewProjectName}
          onSubmitNewThreadDraft={(draft) => void submitNewThreadDraft(draft)}
          onToggleChat={() => setIsChatCollapsed((current) => !current)}
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
          onPointerDown={(event) => beginResize("left", event.clientX)}
        />
        <ConversationPanel
          availableSkills={availableSkills}
          availableWorkflows={availableWorkflows}
          compactHistoryById={selectedCompactHistory}
          conversationCells={conversationCells}
          conversationScrollRef={conversationScrollRef}
          draft={draft}
          draftImages={draftImages}
          draftSkills={draftSkills}
          focusedConversationItem={focusedConversationItem}
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
          onHandleComposerPaste={(event) => void handleComposerPaste(event)}
          onHandleImageSelection={(event) => void handleImageSelection(event)}
          onToggleCompactHistory={toggleCompactHistory}
          onOpenLocalFile={(target) => void handleOpenLocalFile(target)}
          onPauseGoal={pauseCurrentThreadGoal}
          onRemoveDraftImage={removeDraftImage}
          onRemoveDraftSkill={removeDraftSkill}
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
        <div
          className="panel-resizer"
          role="separator"
          aria-label="Resize right panel"
          onPointerDown={(event) => beginResize("right", event.clientX)}
        />
        <RightPanel
          activeView={rightPanelView}
          availableSkillCount={availableSkills.length}
          expandedTreeDirectories={expandedTreeDirectories}
          filePanelView={filePanelView}
          fileTreeEntriesByPath={fileTreeEntriesByPath}
          fileTreeErrorsByPath={fileTreeErrorsByPath}
          fileTreeLoadingPath={fileTreeLoadingPath}
          onOpenProjectChat={() => void openWorkspaceProject()}
          onNavigateToSymbol={(destination, sourceLocation) =>
            void handleNavigateToSymbol(destination, sourceLocation)
          }
          onOpenPreviewExternally={() => void openPreviewExternally()}
          onOpenTreeFile={handleOpenTreeFile}
          onSelectTaskThread={setSelectedThreadId}
          onSelectCommandMonitor={(commandItemId) =>
            setFocusedConversationItem((current) => ({
              itemId: commandItemId,
              token: (current?.token ?? 0) + 1,
            }))
          }
          onSetActiveView={setRightPanelView}
          onSetFilePanelView={handleSetFilePanelView}
          onSetTaskFilter={setTaskFilter}
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
          skills={selectedThread?.skills ?? []}
          selectedThreadId={selectedThreadId}
          thread={selectedThread}
          modelContextWindowOverride={
            selectedRunConfigOverride?.contextWindow ?? null
          }
          taskFilter={taskFilter}
          todoItems={todoItems}
        />
      </main>

      <TreeContextMenu
        threads={threads}
        treeMenu={treeMenu}
        onArchiveThread={(threadId) => void archiveThread(threadId)}
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
