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

import { ConversationPanel, SidebarPanel, TreeContextMenu } from "./components/Panels";
import { RightPanel } from "./components/RightPanel";
import { buildConversationCells, buildConversationEntries } from "./lib/conversation";
import { readImageBlob, readImageFile } from "./lib/images";
import { isThreadNotFoundError, toErrorMessage } from "./lib/shared";
import {
  appendAgentDelta,
  buildAgentTree,
  buildTodoItems,
  getThreadSubtreeIds,
  getTreeRootThreadId,
  getThreadDepth,
  isRootThread,
  isSubagentThread,
  pickInitialRootThread,
  pickInitialThread,
  updateThreadItem,
  updateThreadSkills,
  updateThreadTurn,
  upsertThread,
} from "./lib/thread";
import type {
  BootstrapResponse,
  ComposerImage,
  DraftSkill,
  FilePreview,
  FileLocation,
  NotificationEnvelope,
  RightPanelView,
  TaskFilter,
  Thread,
  ThreadItem,
  ThreadSkill,
  TreeMenuState,
  Turn,
} from "./types";

let initialRootThreadPromise: Promise<Thread> | null = null;

const LEFT_PANEL_WIDTH_RATIO = 0.17;
const RIGHT_PANEL_WIDTH_RATIO = 0.31;
const LEFT_PANEL_MIN_RATIO = 0.13;
const LEFT_PANEL_MAX_RATIO = 0.34;
const RIGHT_PANEL_MIN_RATIO = 0.22;
const RIGHT_PANEL_MAX_RATIO = 0.46;

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
  const [selectedThread, setSelectedThread] = useState<Thread | null>(null);
  const [availableSkills, setAvailableSkills] = useState<ThreadSkill[]>([]);
  const [draft, setDraft] = useState("");
  const [draftSkills, setDraftSkills] = useState<DraftSkill[]>([]);
  const [isSending, setIsSending] = useState(false);
  const [isStoppingTurn, setIsStoppingTurn] = useState(false);
  const [isLoadingThread, setIsLoadingThread] = useState(false);
  const [newRootName, setNewRootName] = useState("root");
  const [error, setError] = useState<string | null>(null);
  const [draftImages, setDraftImages] = useState<ComposerImage[]>([]);
  const [taskFilter, setTaskFilter] = useState<TaskFilter>("all");
  const [collapsedPaths, setCollapsedPaths] = useState<string[]>([]);
  const [treeMenu, setTreeMenu] = useState<TreeMenuState | null>(null);
  const [rightPanelView, setRightPanelView] = useState<RightPanelView>("todo");
  const [filePreview, setFilePreview] = useState<FilePreview | null>(null);
  const [isLoadingPreview, setIsLoadingPreview] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const imageInputRef = useRef<HTMLInputElement | null>(null);
  const conversationScrollRef = useRef<HTMLDivElement | null>(null);
  const shouldStickConversationToBottomRef = useRef(true);
  const filePreviewRef = useRef<FilePreview | null>(null);
  const symbolBackStackRef = useRef<FileLocation[]>([]);
  const symbolForwardStackRef = useRef<FileLocation[]>([]);
  const resizeStateRef = useRef<{
    startX: number;
    startWidth: number;
    panel: "left" | "right";
  } | null>(null);

  useEffect(() => {
    void loadBootstrap();
  }, []);

  useEffect(() => {
    if (!selectedThreadId) {
      return;
    }
    void loadThread(selectedThreadId);
  }, [selectedThreadId]);

  useEffect(() => {
    setAvailableSkills([]);
    setDraftSkills([]);
  }, [selectedThreadId]);

  async function loadAvailableSkills(cwd: string) {
    const payload = (await window.codexDesktop.listSkills(cwd)) as {
      skills: ThreadSkill[];
      errors: string[];
    };
    return payload.skills;
  }

  useEffect(() => {
    const cwd = selectedThread?.cwd;
    if (!cwd) {
      setAvailableSkills([]);
      return;
    }

    let cancelled = false;

    async function refreshAvailableSkills() {
      try {
        const skills = await loadAvailableSkills(cwd);
        if (cancelled) {
          return;
        }
        setAvailableSkills(skills);
      } catch (loadError) {
        if (cancelled) {
          return;
        }
        setAvailableSkills([]);
        setError(toErrorMessage(loadError));
      }
    }

    void refreshAvailableSkills();

    return () => {
      cancelled = true;
    };
  }, [selectedThread?.cwd]);

  useEffect(() => {
    const unsubscribe = window.codexDesktop.subscribe((payload) => {
      handleStreamEvent(payload as NotificationEnvelope);
    });
    return unsubscribe;
  }, [selectedThread?.cwd, selectedThreadId]);

  useEffect(() => {
    shouldStickConversationToBottomRef.current = true;
  }, [selectedThreadId]);

  useEffect(() => {
    filePreviewRef.current = filePreview;
  }, [filePreview]);

  useEffect(() => {
    function handlePointerMove(event: PointerEvent) {
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
    setSidebarWidth((current) => clampPanelWidth(current, viewportWidth, "left"));
    setRightPanelWidth((current) => clampPanelWidth(current, viewportWidth, "right"));
  }, [viewportWidth]);

  useEffect(() => {
    if (!filePreview?.lsp.workspaceRoot) {
      return;
    }

    let cancelled = false;

    async function refreshLspStatus() {
      try {
        const status = await window.codexDesktop.lspStatus(filePreview.path);
        if (cancelled) {
          return;
        }

        setFilePreview((current) =>
          current?.path === filePreview.path
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

  const conversationEntries = useMemo(
    () => buildConversationEntries(selectedThread),
    [selectedThread],
  );
  const conversationCells = useMemo(
    () => buildConversationCells(conversationEntries),
    [conversationEntries],
  );

  useLayoutEffect(() => {
    const container = conversationScrollRef.current;
    if (!container || !shouldStickConversationToBottomRef.current) {
      return;
    }
    container.scrollTop = container.scrollHeight;
  }, [conversationCells, isLoadingThread, selectedThreadId]);

  const selectedTreeRootId = useMemo(() => {
    const seedThread = threads.find((thread) => thread.id === selectedThreadId) ?? pickInitialThread(threads);
    if (!seedThread) {
      return null;
    }
    return getTreeRootThreadId(threads, seedThread.id);
  }, [selectedThreadId, threads]);
  const sessionThreads = useMemo(() => {
    if (!selectedTreeRootId) {
      return [];
    }
    return threads.filter((thread) => getTreeRootThreadId(threads, thread.id) === selectedTreeRootId);
  }, [selectedTreeRootId, threads]);
  const agentTree = useMemo(
    () => buildAgentTree(sessionThreads, selectedTreeRootId),
    [selectedTreeRootId, sessionThreads],
  );
  const todoItems = useMemo(
    () => buildTodoItems(sessionThreads, taskFilter),
    [sessionThreads, taskFilter],
  );
  const collapsedSet = useMemo(() => new Set(collapsedPaths), [collapsedPaths]);

  async function loadBootstrap() {
    try {
      const payload = (await window.codexDesktop.bootstrap()) as BootstrapResponse;
      setWorkspace(payload.workspace);
      setThreads(payload.threads);
      const preferredRoot = pickInitialRootThread(payload.threads);
      if (preferredRoot) {
        setSelectedThreadId(preferredRoot.id);
        return;
      }
      const rootThread = await ensureInitialRootThread(payload.workspace);
      setThreads((current) => upsertThread(current, rootThread));
      setSelectedThreadId(rootThread.id);
      setSelectedThread(rootThread);
    } catch (loadError) {
      setError(toErrorMessage(loadError));
    }
  }

  async function subscribeThread(threadId: string) {
    const payload = (await window.codexDesktop.readThread(threadId, true)) as {
      thread: Thread;
    };
    setThreads((current) => upsertThread(current, payload.thread));
  }

  function removeThreadLocally(threadIds: Iterable<string>) {
    const threadIdSet = new Set(threadIds);
    setThreads((current) => {
      const next = current.filter((thread) => !threadIdSet.has(thread.id));
      setSelectedThreadId((selected) =>
        selected && threadIdSet.has(selected) ? pickInitialRootThread(next)?.id ?? null : selected,
      );
      return next;
    });
    setSelectedThread((current) => (current?.id && threadIdSet.has(current.id) ? null : current));
  }

  function updateThreadStatusLocally(threadId: string, status: Thread["status"]) {
    setThreads((current) =>
      current.map((thread) => (thread.id === threadId ? { ...thread, status } : thread)),
    );
    setSelectedThread((current) =>
      current?.id === threadId ? { ...current, status } : current,
    );
  }

  function updateThreadNameLocally(threadId: string, name: Thread["name"]) {
    setThreads((current) =>
      current.map((thread) => (thread.id === threadId ? { ...thread, name } : thread)),
    );
    setSelectedThread((current) =>
      current?.id === threadId ? { ...current, name } : current,
    );
  }

  function updateThreadSkillsLocally(threadId: string, skills: ThreadSkill[]) {
    setThreads((current) =>
      current.map((thread) => (thread.id === threadId ? updateThreadSkills(thread, skills) : thread)),
    );
    setSelectedThread((current) =>
      current?.id === threadId ? updateThreadSkills(current, skills) : current,
    );
  }

  async function loadThread(threadId: string) {
    setIsLoadingThread(true);
    setError(null);
    try {
      const payload = (await window.codexDesktop.readThread(threadId, true)) as {
        thread: Thread;
      };
      setSelectedThread(payload.thread);
      setThreads((current) => upsertThread(current, payload.thread));
    } catch (loadError) {
      const message = toErrorMessage(loadError);
      if (isThreadNotFoundError(message)) {
        removeThreadLocally([threadId]);
      }
      setError(message);
    } finally {
      setIsLoadingThread(false);
    }
  }

  async function createRootThread(name = newRootName.trim() || "root", cwd = workspace) {
    setError(null);
    try {
      const payload = (await window.codexDesktop.createThread({ cwd, name })) as { thread: Thread };
      setThreads((current) => upsertThread(current, payload.thread));
      setSelectedThreadId(payload.thread.id);
      setSelectedThread(payload.thread);
    } catch (createError) {
      setError(toErrorMessage(createError));
    }
  }

  async function ensureInitialRootThread(cwd: string) {
    if (!initialRootThreadPromise) {
      initialRootThreadPromise = window.codexDesktop
        .createThread({
          cwd,
          name: "root",
        })
        .then((payload) => (payload as { thread: Thread }).thread)
        .finally(() => {
          initialRootThreadPromise = null;
        });
    }

    return initialRootThreadPromise;
  }

  async function clearCurrentRootSession() {
    if (!selectedTreeRootId) {
      return;
    }

    const rootThread = threads.find((thread) => thread.id === selectedTreeRootId) ?? null;
    const replacementName = rootThread?.name ?? rootThread?.agentNickname ?? "root";
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
      setThreads((current) => current.filter((thread) => !threadIdsToArchive.includes(thread.id)));
      setSelectedThread(null);
      setSelectedThreadId(null);
      await createRootThread(replacementName);
      setDraft("");
      setDraftImages([]);
    } catch (clearError) {
      setError(toErrorMessage(clearError));
    } finally {
      setIsSending(false);
    }
  }

  async function sendMessage() {
    if (!selectedThreadId || (!draft.trim() && draftImages.length === 0 && draftSkills.length === 0)) {
      return;
    }
    if (draft.trim() === "/clear" && draftImages.length === 0 && draftSkills.length === 0) {
      await clearCurrentRootSession();
      return;
    }
    setIsSending(true);
    setError(null);
    try {
      await window.codexDesktop.sendMessage({
        threadId: selectedThreadId,
        text: draft.trim(),
        skills: draftSkills,
        images: draftImages.map(({ name, dataUrl }) => ({ name, dataUrl })),
      });
      setDraft("");
      setDraftSkills([]);
      setDraftImages([]);
    } catch (sendError) {
      setError(toErrorMessage(sendError));
    } finally {
      setIsSending(false);
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
      setDraftImages((current) => [...current, ...images]);
    } catch (loadError) {
      setError(toErrorMessage(loadError));
    } finally {
      event.target.value = "";
    }
  }

  function removeDraftImage(imageId: string) {
    setDraftImages((current) => current.filter((image) => image.id !== imageId));
  }

  function addDraftSkill(skill: DraftSkill) {
    setDraftSkills((current) =>
      current.some((candidate) => candidate.path === skill.path) ? current : [...current, skill],
    );
  }

  function removeDraftSkill(path: string) {
    setDraftSkills((current) => current.filter((skill) => skill.path !== path));
  }

  async function handleComposerPaste(event: ClipboardEvent<HTMLTextAreaElement>) {
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
      setDraftImages((current) => [...current, ...images]);
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
        throw new Error("Root agent cannot be deleted.");
      }
      const archive = window.codexDesktop.archiveThread;
      if (typeof archive !== "function") {
        throw new Error("This build does not expose archiveThread. Please reload Electron.");
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

  function handleStreamEvent(payload: NotificationEnvelope) {
    try {
      if (payload.type === "status" && payload.status) {
        return;
      }

      if (payload.type !== "notification" || !payload.notification) {
        return;
      }

      const { method, params } = payload.notification;

      switch (method) {
        case "thread/started": {
          const thread = (params as { thread: Thread }).thread;
          setThreads((current) => upsertThread(current, thread));
          if (isSubagentThread(thread)) {
            void subscribeThread(thread.id);
          }
          if (!selectedThreadId) {
            setSelectedThreadId(thread.id);
          }
          break;
        }
        case "thread/skills/updated": {
          const notification = params as { threadId: string; skills: ThreadSkill[] };
          updateThreadSkillsLocally(notification.threadId, notification.skills);
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
        case "thread/name/updated":
        case "thread/archived":
        case "thread/closed": {
          if (method === "thread/name/updated") {
            const notification = params as { threadId: string; threadName?: string | null };
            updateThreadNameLocally(notification.threadId, notification.threadName ?? null);
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
          const notification = params as { threadId: string; status: Thread["status"] };
          updateThreadStatusLocally(notification.threadId, notification.status);
          break;
        }
        case "turn/started":
        case "turn/completed": {
          const notification = params as { threadId: string; turn: Turn };
          if (method === "turn/completed" && notification.threadId === selectedThreadId) {
            setIsStoppingTurn(false);
          }
          if (notification.threadId === selectedThreadId) {
            setSelectedThread((current) =>
              current ? updateThreadTurn(current, notification.turn) : current,
            );
          }
          break;
        }
        case "item/started":
        case "item/completed": {
          const notification = params as {
            threadId: string;
            turnId: string;
            item: ThreadItem;
          };
          if (notification.threadId === selectedThreadId) {
            setSelectedThread((current) =>
              current
                ? updateThreadItem(current, notification.turnId, notification.item)
                : current,
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
          if (notification.threadId === selectedThreadId) {
            setSelectedThread((current) =>
              current
                ? appendAgentDelta(
                    current,
                    notification.turnId,
                    notification.itemId,
                    notification.delta,
                  )
                : current,
            );
          }
          break;
        }
        default:
          break;
      }
    } catch (streamError) {
      setError(`Failed to render app-server event: ${toErrorMessage(streamError)}`);
    }
  }

  function handleConversationScroll() {
    const container = conversationScrollRef.current;
    if (!container) {
      return;
    }
    const distanceFromBottom =
      container.scrollHeight - container.clientHeight - container.scrollTop;
    shouldStickConversationToBottomRef.current = distanceFromBottom <= 24;
  }

  async function loadFilePreview(target: string) {
    setRightPanelView("preview");
    setIsLoadingPreview(true);
    setPreviewError(null);

    try {
      const preview = (await window.codexDesktop.readLocalFile(target)) as FilePreview;
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
      direction === "back" ? symbolBackStackRef.current : symbolForwardStackRef.current;
    const targetStack =
      direction === "back" ? symbolForwardStackRef.current : symbolBackStackRef.current;
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
          agentTree={agentTree}
          collapsedSet={collapsedSet}
          newRootName={newRootName}
          onCreateRootThread={() => void createRootThread()}
          onOpenMenu={setTreeMenu}
          onSelectThread={setSelectedThreadId}
          onSetNewRootName={setNewRootName}
          onToggleTreeNode={toggleTreeNode}
          selectedThreadId={selectedThreadId}
        />
        <div
          className="panel-resizer"
          role="separator"
          aria-label="Resize sidebar"
          onPointerDown={(event) => beginResize("left", event.clientX)}
        />
        <ConversationPanel
          availableSkills={availableSkills}
          conversationCells={conversationCells}
          conversationScrollRef={conversationScrollRef}
          draft={draft}
          draftImages={draftImages}
          draftSkills={draftSkills}
          imageInputRef={imageInputRef}
          isLoadingThread={isLoadingThread}
          isSending={isSending}
          isStoppingTurn={isStoppingTurn}
          onAddDraftSkill={addDraftSkill}
          onConversationScroll={handleConversationScroll}
          onDraftChange={setDraft}
          onHandleComposerPaste={(event) => void handleComposerPaste(event)}
          onHandleImageSelection={(event) => void handleImageSelection(event)}
          onOpenLocalFile={(target) => void handleOpenLocalFile(target)}
          onRemoveDraftImage={removeDraftImage}
          onRemoveDraftSkill={removeDraftSkill}
          onSendMessage={() => void sendMessage()}
          onStopTurn={() => void interruptCurrentTurn()}
          selectedThread={selectedThread}
          selectedThreadId={selectedThreadId}
        />
        <div
          className="panel-resizer"
          role="separator"
          aria-label="Resize right panel"
          onPointerDown={(event) => beginResize("right", event.clientX)}
        />
        <RightPanel
          activeView={rightPanelView}
          onCreateRootThread={() => void createRootThread()}
          onNavigateToSymbol={(destination, sourceLocation) =>
            void handleNavigateToSymbol(destination, sourceLocation)}
          onOpenPreviewExternally={() => void openPreviewExternally()}
          onSelectTaskThread={setSelectedThreadId}
          onSetActiveView={setRightPanelView}
          onSetTaskFilter={setTaskFilter}
          preview={filePreview}
          previewError={previewError}
          previewLoading={isLoadingPreview}
          skills={selectedThread?.skills ?? []}
          selectedThreadId={selectedThreadId}
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

function clampPanelWidth(value: number, viewportWidth: number, panel: "left" | "right") {
  const min = panel === "left"
    ? widthFromRatio(viewportWidth, LEFT_PANEL_MIN_RATIO)
    : widthFromRatio(viewportWidth, RIGHT_PANEL_MIN_RATIO);
  const max = panel === "left"
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
