import { type ChangeEvent, type ClipboardEvent, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { ConversationPanel, SidebarPanel, TodoPanel, TreeContextMenu } from "./components/Panels";
import { buildConversationCells, buildConversationEntries } from "./lib/conversation";
import { readImageBlob, readImageFile } from "./lib/images";
import { isThreadNotFoundError, toErrorMessage } from "./lib/shared";
import {
  appendAgentDelta,
  buildAgentTree,
  buildTodoItems,
  getCachedDescendantIds,
  getThreadDepth,
  getTreeRootThreadId,
  isRootThread,
  pickInitialRootThread,
  pickInitialThread,
  pruneCachedThreadParents,
  updateThreadItem,
  updateThreadTurn,
  upsertThread,
} from "./lib/thread";
import type { BootstrapResponse, ComposerImage, NotificationEnvelope, TaskFilter, Thread, ThreadItem, TreeMenuState, Turn } from "./types";

function App() {
  const [workspace, setWorkspace] = useState("");
  const [threads, setThreads] = useState<Thread[]>([]);
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(null);
  const [selectedThread, setSelectedThread] = useState<Thread | null>(null);
  const [draft, setDraft] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [isLoadingThread, setIsLoadingThread] = useState(false);
  const [newRootName, setNewRootName] = useState("root");
  const [error, setError] = useState<string | null>(null);
  const [draftImages, setDraftImages] = useState<ComposerImage[]>([]);
  const [taskFilter, setTaskFilter] = useState<TaskFilter>("all");
  const [collapsedPaths, setCollapsedPaths] = useState<string[]>([]);
  const [treeMenu, setTreeMenu] = useState<TreeMenuState | null>(null);
  const [cachedThreadParents, setCachedThreadParents] = useState<Record<string, string>>({});
  const imageInputRef = useRef<HTMLInputElement | null>(null);
  const conversationScrollRef = useRef<HTMLDivElement | null>(null);
  const shouldStickConversationToBottomRef = useRef(true);

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
    const unsubscribe = window.codexDesktop.subscribe((payload) => {
      handleStreamEvent(payload as NotificationEnvelope);
    });
    return unsubscribe;
  }, [selectedThreadId]);

  useEffect(() => {
    shouldStickConversationToBottomRef.current = true;
  }, [selectedThreadId]);

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
    const seedThreadId = selectedThreadId ?? pickInitialThread(threads)?.id ?? null;
    if (!seedThreadId) {
      return null;
    }
    return getTreeRootThreadId(threads, seedThreadId);
  }, [selectedThreadId, threads]);
  const sessionThreads = useMemo(() => {
    if (!selectedTreeRootId) {
      return [];
    }
    const cachedDescendants = getCachedDescendantIds(cachedThreadParents, selectedTreeRootId);
    return threads.filter(
      (thread) =>
        getTreeRootThreadId(threads, thread.id, cachedThreadParents) === selectedTreeRootId ||
        cachedDescendants.has(thread.id),
    );
  }, [cachedThreadParents, selectedTreeRootId, threads]);
  const agentTree = useMemo(
    () => buildAgentTree(sessionThreads, cachedThreadParents, selectedTreeRootId),
    [cachedThreadParents, selectedTreeRootId, sessionThreads],
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
      await createRootThread("root", payload.workspace);
    } catch (loadError) {
      setError(toErrorMessage(loadError));
    }
  }

  async function refreshThreads() {
    const payload = (await window.codexDesktop.listThreads()) as { data: Thread[] };
    setThreads(payload.data);
    setCachedThreadParents((current) => pruneCachedThreadParents(payload.data, current));
    setSelectedThreadId((current) => {
      if (!current) {
        return pickInitialRootThread(payload.data)?.id ?? null;
      }
      return payload.data.some((thread) => thread.id === current)
        ? current
        : pickInitialRootThread(payload.data)?.id ?? null;
    });
  }

  async function hydrateThread(threadId: string) {
    const payload = (await window.codexDesktop.readThread(threadId, false)) as {
      thread: Thread;
    };
    setThreads((current) => upsertThread(current, payload.thread));
  }

  async function hydrateReceiverThreads(threadIds: string[]) {
    const uniqueIds = [...new Set(threadIds.filter(Boolean))];
    if (uniqueIds.length === 0) {
      return;
    }

    for (const delayMs of [0, 150, 500, 1200]) {
      if (delayMs > 0) {
        await new Promise((resolve) => window.setTimeout(resolve, delayMs));
      }

      await Promise.all(
        uniqueIds.map(async (threadId) => {
          try {
            await hydrateThread(threadId);
          } catch {
            // The child thread can lag briefly behind the tool-call event.
          }
        }),
      );
    }

    await refreshThreads();
  }

  function cacheReceiverThreads(parentThreadId: string, receiverThreadIds: string[]) {
    setCachedThreadParents((current) => {
      const next = { ...current };
      for (const receiverThreadId of receiverThreadIds) {
        if (!receiverThreadId || next[receiverThreadId]) {
          continue;
        }
        next[receiverThreadId] = parentThreadId;
      }
      return next;
    });
  }

  function removeMissingReceiverThreads(
    item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
  ) {
    const missingThreadIds = item.receiverThreadIds.filter((threadId) => {
      const state = item.agentsStates[threadId]?.status;
      return state === "not_found" || state === "notFound";
    });
    if (missingThreadIds.length === 0) {
      return;
    }
    setCachedThreadParents((current) => {
      const next = { ...current };
      for (const threadId of missingThreadIds) {
        delete next[threadId];
      }
      return next;
    });
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
        setSelectedThread(null);
        await refreshThreads();
      }
      setError(message);
    } finally {
      setIsLoadingThread(false);
    }
  }

  async function createRootThread(name = newRootName.trim() || "root", cwd = workspace) {
    setError(null);
    try {
      const payload = (await window.codexDesktop.createThread({
        cwd,
        name,
      })) as { thread: Thread };
      setThreads((current) => upsertThread(current, payload.thread));
      setSelectedThreadId(payload.thread.id);
      setSelectedThread(payload.thread);
    } catch (createError) {
      setError(toErrorMessage(createError));
    }
  }

  async function clearCurrentRootSession() {
    if (!selectedTreeRootId) {
      return;
    }

    const rootThread = threads.find((thread) => thread.id === selectedTreeRootId) ?? null;
    const replacementName = rootThread?.name ?? rootThread?.agentNickname ?? "root";
    const threadIdsToArchive = [...sessionThreads]
      .sort((left, right) => {
        const leftDepth = getThreadDepth(threads, left.id, cachedThreadParents);
        const rightDepth = getThreadDepth(threads, right.id, cachedThreadParents);
        return rightDepth - leftDepth;
      })
      .map((thread) => thread.id);

    setError(null);
    setIsSending(true);
    try {
      for (const threadId of threadIdsToArchive) {
        await window.codexDesktop.archiveThread(threadId);
      }
      setCachedThreadParents((current) => {
        const next = { ...current };
        for (const threadId of threadIdsToArchive) {
          delete next[threadId];
        }
        return next;
      });
      setSelectedThread(null);
      await refreshThreads();
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
    if (!selectedThreadId || (!draft.trim() && draftImages.length === 0)) {
      return;
    }
    if (draft.trim() === "/clear" && draftImages.length === 0) {
      await clearCurrentRootSession();
      return;
    }
    setIsSending(true);
    setError(null);
    try {
      await window.codexDesktop.sendMessage({
        threadId: selectedThreadId,
        text: draft.trim(),
        images: draftImages.map(({ name, dataUrl }) => ({ name, dataUrl })),
      });
      setDraft("");
      setDraftImages([]);
    } catch (sendError) {
      setError(toErrorMessage(sendError));
    } finally {
      setIsSending(false);
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
      await refreshThreads();
      if (selectedThreadId === threadId) {
        setSelectedThread(null);
      }
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
          if (!selectedThreadId) {
            setSelectedThreadId(thread.id);
          }
          break;
        }
        case "thread/name/updated":
        case "thread/status/changed":
        case "thread/archived":
        case "thread/closed": {
          void refreshThreads();
          break;
        }
        case "turn/started":
        case "turn/completed": {
          const notification = params as { threadId: string; turn: Turn };
          if (notification.threadId === selectedThreadId) {
            setSelectedThread((current) =>
              current ? updateThreadTurn(current, notification.turn) : current,
            );
          }
          void refreshThreads();
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
          if (notification.item.type === "collabAgentToolCall") {
            cacheReceiverThreads(
              notification.item.senderThreadId,
              notification.item.receiverThreadIds,
            );
            removeMissingReceiverThreads(notification.item);
            void hydrateReceiverThreads(notification.item.receiverThreadIds);
            void refreshThreads();
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

  return (
    <div className="app-shell" onClick={() => setTreeMenu(null)}>
      {error ? <div className="error-banner">{error}</div> : null}

      <main className="workspace">
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
        <ConversationPanel
          conversationCells={conversationCells}
          conversationScrollRef={conversationScrollRef}
          draft={draft}
          draftImages={draftImages}
          imageInputRef={imageInputRef}
          isLoadingThread={isLoadingThread}
          isSending={isSending}
          onConversationScroll={handleConversationScroll}
          onDraftChange={setDraft}
          onHandleComposerPaste={(event) => void handleComposerPaste(event)}
          onHandleImageSelection={(event) => void handleImageSelection(event)}
          onRemoveDraftImage={removeDraftImage}
          onSendMessage={() => void sendMessage()}
          selectedThread={selectedThread}
          selectedThreadId={selectedThreadId}
        />
        <TodoPanel
          onCreateRootThread={() => void createRootThread()}
          onSelectTaskThread={setSelectedThreadId}
          onSetTaskFilter={setTaskFilter}
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
