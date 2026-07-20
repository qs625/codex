import type {
  TaskFilter,
  Thread,
  ThreadLifecycleActiveFlag,
  ThreadItem,
  ProjectAgentSidebar,
  SidebarProjectNode,
  ThreadSkill,
  ThreadLifecycleStatus,
  TodoCardItem,
  TreeNode,
  Turn,
} from "../types";
import { isChatCompatCwd } from "./chatCompat";

export function pickInitialThread(threads: Thread[]) {
  return (
    [...threads].sort((left, right) => right.updatedAt - left.updatedAt)[0] ??
    null
  );
}

export function pickInitialRootThread(threads: Thread[]) {
  return pickInitialThread(
    threads.filter(
      (thread) => isRootThread(thread) && getThreadPath(thread) === "/root",
    ),
  );
}

export function buildAgentTree(
  threads: Thread[],
  selectedTreeRootId: string | null,
): TreeNode[] {
  if (threads.length === 0 && !selectedTreeRootId) {
    return [];
  }

  const threadById = new Map(threads.map((thread) => [thread.id, thread]));
  const roots = threads.filter((thread) => !getParentThreadId(thread));
  const byParent = new Map<string, string[]>();
  for (const thread of threads) {
    const parentId = getParentThreadId(thread);
    if (!parentId) {
      continue;
    }
    const list = byParent.get(parentId) ?? [];
    list.push(thread.id);
    byParent.set(parentId, list);
  }

  const buildNode = (threadId: string): TreeNode => {
    const thread = threadById.get(threadId) ?? null;
    const childIds = [...(byParent.get(threadId) ?? [])].sort((left, right) => {
      const leftThread = threadById.get(left);
      const rightThread = threadById.get(right);
      if (leftThread && rightThread) {
        return leftThread.createdAt - rightThread.createdAt;
      }
      if (leftThread) {
        return -1;
      }
      if (rightThread) {
        return 1;
      }
      return left.localeCompare(right);
    });

    return {
      key: threadId,
      label: thread ? getTreeNodeLabel(thread) : trimThreadId(threadId),
      path: thread ? getTreeNodeSubtitle(thread) : "Starting worker…",
      thread,
      threadId,
      isPlaceholder: thread === null,
      children: childIds.map(buildNode),
    };
  };

  const rootIds = [
    ...new Set([
      ...roots.map((thread) => thread.id),
      ...(selectedTreeRootId ? [selectedTreeRootId] : []),
    ]),
  ];

  return rootIds
    .sort((left, right) => {
      const leftThread = threadById.get(left);
      const rightThread = threadById.get(right);
      if (leftThread && rightThread) {
        return leftThread.createdAt - rightThread.createdAt;
      }
      if (leftThread) {
        return -1;
      }
      if (rightThread) {
        return 1;
      }
      return left.localeCompare(right);
    })
    .map(buildNode);
}

export function buildProjectAgentSidebar(threads: Thread[]): ProjectAgentSidebar {
  const parentlessThreads = threads.filter(isRootThread);
  const projectRootCandidates = new Map<string, Thread[]>();
  const chatThreads: Thread[] = [];

  for (const thread of parentlessThreads) {
    const projectCwd = normalizeProjectCwd(thread.cwd);
    if (!projectCwd || isChatCompatCwd(projectCwd)) {
      chatThreads.push(thread);
      continue;
    }
    const candidates = projectRootCandidates.get(projectCwd) ?? [];
    candidates.push(thread);
    projectRootCandidates.set(projectCwd, candidates);
  }

  const projects = [...projectRootCandidates.entries()]
    .map(([cwd, candidates]) => {
      const sortedCandidates = [...candidates].sort(compareCanonicalProjectRoot);
      const rootThread = sortedCandidates[0];
      const duplicateRootThreadIds = sortedCandidates
        .slice(1)
        .map((thread) => thread.id);
      const projectTree = buildSidebarRootTree(
        threads,
        rootThread,
        (node) => withProjectRootLabel(node, projectLabelFromCwd(cwd)),
      );
      const projectThreadList = collectTreeThreads(projectTree);
      const counts = countSidebarStatuses(projectThreadList);

      return {
        id: `project:${cwd}`,
        label: projectLabelFromCwd(cwd),
        subtitle: cwd,
        cwd,
        statusClass: selfTreeThreadLifecycleStatusClass(rootThread),
        updatedAt: Math.max(...projectThreadList.map((thread) => thread.updatedAt)),
        tree: projectTree,
        descendantCount: countDescendants(projectTree),
        activeCount: counts.activeCount,
        waitingCount: counts.waitingCount,
        failedCount: counts.failedCount,
        duplicateRootThreadIds,
      } satisfies SidebarProjectNode;
    })
    .sort((left, right) => right.updatedAt - left.updatedAt);

  const chatConversations = [...chatThreads]
    .sort((left, right) => right.updatedAt - left.updatedAt)
    .map((thread) =>
      buildSidebarRootTree(threads, thread, withChatConversationLabel),
    );
  const chatThreadsForStatus = chatConversations.flatMap(collectTreeThreads);

  return {
    projects,
    chat: {
      id: "chat",
      statusClass: aggregateSidebarStatus(chatThreadsForStatus),
      updatedAt:
        chatThreadsForStatus.length > 0
          ? Math.max(...chatThreadsForStatus.map((thread) => thread.updatedAt))
          : 0,
      conversations: chatConversations,
    },
  };
}

export function pickInitialProjectThread(threads: Thread[]) {
  const sidebar = buildProjectAgentSidebar(threads);
  return sidebar.projects[0]?.tree.thread ?? pickInitialThread(threads);
}

export function buildTodoItems(
  threads: Thread[],
  filter: TaskFilter,
): TodoCardItem[] {
  return filterTodoItems(buildTodoCardItems(threads), filter);
}

export function buildCurrentThreadTodoItems(
  threads: Thread[],
  selectedThreadId: string | null,
  filter: TaskFilter,
): TodoCardItem[] {
  if (!selectedThreadId) {
    return [];
  }
  return filterTodoItems(
    buildTodoCardItems(
      threads.filter(
        (thread) => getParentThreadId(thread) === selectedThreadId,
      ),
    ),
    filter,
  );
}

function buildTodoCardItems(threads: Thread[]): TodoCardItem[] {
  return [...threads]
    .sort((left, right) => right.updatedAt - left.updatedAt)
    .map((thread) => {
      const status = mapTaskStatus(thread.lifecycleStatus);
      return {
        id: thread.id,
        title: getThreadLabel(thread),
        ownerPath: getThreadPath(thread),
        status,
        statusLabel: capitalize(status),
        updatedLabel: formatUpdatedLabel(thread.updatedAt),
        summary: thread.preview,
        threadId: thread.id,
      };
    });
}

function filterTodoItems(items: TodoCardItem[], filter: TaskFilter) {
  if (filter === "all") {
    return items;
  }
  return items.filter((item) => item.status === filter);
}

export function getThreadLabel(thread: Thread) {
  const path = getThreadPath(thread);
  const last = path.split("/").filter(Boolean).at(-1);
  return thread.name ?? thread.agentNickname ?? last ?? "root";
}

export function getTreeNodeLabel(thread: Thread) {
  if (isRootThread(thread)) {
    return getThreadPath(thread);
  }
  return (
    getThreadPath(thread).split("/").filter(Boolean).at(-1) ??
    getThreadLabel(thread)
  );
}

export function getTreeNodeSubtitle(thread: Thread) {
  return thread.agentRole ?? getAgentRoleLabel(thread);
}

export function getThreadPath(thread: Thread): string {
  if (thread.agentPath) {
    return thread.agentPath;
  }
  const threadSpawn = getThreadSpawnSource(thread);
  if (threadSpawn) {
    return (
      threadSpawn.agentPath ??
      threadSpawn.agent_path ??
      `/root/${thread.agentNickname ?? thread.id.slice(0, 6)}`
    );
  }
  return "/root";
}

export function isSubagentThread(thread: Thread) {
  return (
    getThreadSpawnSource(thread) !== null || thread.threadSource === "subagent"
  );
}

export function getParentThreadId(thread: Thread): string | null {
  const threadSpawn = getThreadSpawnSource(thread);
  if (threadSpawn) {
    return threadSpawn.parentThreadId ?? threadSpawn.parent_thread_id ?? null;
  }
  return null;
}

export function isRootThread(thread: Thread) {
  return !getParentThreadId(thread);
}

export function getTreeRootThreadId(threads: Thread[], threadId: string) {
  const byId = new Map(threads.map((thread) => [thread.id, thread]));
  let current = byId.get(threadId) ?? null;
  let currentId = current?.id ?? threadId;
  let parentId = current ? getParentThreadId(current) : null;

  while (parentId) {
    const parent = byId.get(parentId);
    if (!parent) {
      return currentId;
    }
    current = parent;
    currentId = parent.id;
    parentId = getParentThreadId(parent);
  }

  return current?.id ?? currentId;
}

export function getThreadDepth(threads: Thread[], threadId: string) {
  const byId = new Map(threads.map((thread) => [thread.id, thread]));
  let depth = 0;
  let current = byId.get(threadId) ?? null;
  let parentId = current ? getParentThreadId(current) : null;

  while (parentId) {
    depth += 1;
    current = byId.get(parentId) ?? null;
    parentId = current ? getParentThreadId(current) : null;
  }

  return depth;
}

export function getThreadSubtreeIds(threads: Thread[], rootThreadId: string) {
  const byParent = new Map<string, string[]>();
  for (const thread of threads) {
    const parentId = getParentThreadId(thread);
    if (!parentId) {
      continue;
    }
    const children = byParent.get(parentId) ?? [];
    children.push(thread.id);
    byParent.set(parentId, children);
  }

  const subtreeIds = new Set<string>();
  const pending = [rootThreadId];
  while (pending.length > 0) {
    const threadId = pending.pop();
    if (!threadId || subtreeIds.has(threadId)) {
      continue;
    }
    subtreeIds.add(threadId);
    pending.push(...(byParent.get(threadId) ?? []));
  }

  return subtreeIds;
}

export function getThreadAncestorIds(threads: Thread[], threadId: string) {
  const byId = new Map(threads.map((thread) => [thread.id, thread]));
  const ancestorIds: string[] = [];
  let current = byId.get(threadId) ?? null;
  let parentId = current ? getParentThreadId(current) : null;

  while (parentId) {
    ancestorIds.push(parentId);
    current = byId.get(parentId) ?? null;
    parentId = current ? getParentThreadId(current) : null;
  }

  return ancestorIds;
}

export function getAgentRoleLabel(thread: Thread) {
  const source = thread.source;
  if (typeof source === "object" && "subAgent" in source) {
    return "Worker Agent";
  }
  if (isProjectRootThread(thread)) {
    return getThreadPath(thread);
  }
  return "Chat";
}

export function getRootThreadConversationTitle(thread: Thread) {
  return isProjectRootThread(thread) ? getThreadPath(thread) : getThreadLabel(thread);
}

export function isProjectRootThread(thread: Thread) {
  const projectCwd = normalizeProjectCwd(thread.cwd);
  return isRootThread(thread) && projectCwd !== null && !isChatCompatCwd(projectCwd);
}

export function isCompletedFinalLifecycleStatus(
  lifecycleStatus: ThreadLifecycleStatus,
) {
  return (
    lifecycleStatus.type === "final" &&
    lifecycleStatus.result.type === "completed"
  );
}

export function shouldNotifyProjectThreadCompleted(
  thread: Thread,
  nextLifecycleStatus: ThreadLifecycleStatus,
) {
  return (
    isProjectRootThread(thread) &&
    !isCompletedFinalLifecycleStatus(thread.lifecycleStatus) &&
    isCompletedFinalLifecycleStatus(nextLifecycleStatus)
  );
}

export function getPresenceLabel(lifecycleStatus: ThreadLifecycleStatus) {
  if (lifecycleStatus.type === "active") {
    return getActivePresenceLabel(lifecycleStatus.activeFlags);
  }

  switch (lifecycleStatus.type) {
    case "systemError":
      return "System Error";
    case "notLoaded":
      return "Not Loaded";
    case "waiting":
      if (lifecycleStatus.reason === "command") {
        return "Waiting on Event Tool";
      }
      if (lifecycleStatus.reason === "child") {
        return "Waiting on Subagent";
      }
      if (lifecycleStatus.reason === "eventSubscription") {
        return "Waiting on Subscription";
      }
      return "Complete";
    case "final":
      return "Complete";
    default:
      return "Idle";
  }
}

export function getThreadPresenceLabel(thread: Thread | null) {
  if (!thread) {
    return "Idle";
  }
  return getPresenceLabel(thread.lifecycleStatus);
}

export function threadDisplayStatusClass(thread: Thread | null) {
  if (!thread) {
    return threadStatusClass({ type: "notLoaded" });
  }
  return threadStatusClass(thread.lifecycleStatus);
}

export function getThreadModelLabel(thread: Thread | null) {
  if (!thread) {
    return "unknown";
  }
  if (thread.model && thread.modelProvider) {
    return `${thread.model} · ${thread.modelProvider}`;
  }
  return thread.model ?? thread.modelProvider ?? "unknown";
}

export function getThreadReasoningLabel(thread: Thread | null) {
  if (!thread) {
    return "default";
  }
  return thread.reasoningEffort ?? "default";
}

export function updateThreadTurn(thread: Thread, turn: Turn) {
  const normalizedTurn = normalizeTurnSnapshot(turn);
  const hasExistingTurn = thread.turns.some(
    (existing) => existing.id === normalizedTurn.id,
  );
  if (hasExistingTurn) {
    const turns = thread.turns.map((existing) =>
      existing.id === normalizedTurn.id
        ? mergeTurn(existing, normalizedTurn)
        : existing,
    );
    return { ...thread, turns };
  }

  const turnMatcher = createTurnItemMatcher(
    buildTurnItemIndex([{ turn: normalizedTurn, items: normalizedTurn.items }]),
  );
  const turns = [
    ...thread.turns.flatMap((existing) =>
      getRetainedUnmatchedTurn(existing, turnMatcher),
    ),
    normalizedTurn,
  ];
  return { ...thread, turns };
}

export function updateThreadTurnLifecycle(thread: Thread, turn: Turn) {
  const normalizedTurn = normalizeTurnSnapshot(turn);
  const hasExistingTurn = thread.turns.some(
    (existing) => existing.id === normalizedTurn.id,
  );
  if (hasExistingTurn) {
    const turns = thread.turns.map((existing) =>
      existing.id === normalizedTurn.id
        ? mergeTurnLifecycle(existing, normalizedTurn)
        : existing,
    );
    return { ...thread, turns };
  }

  return {
    ...thread,
    turns: [...thread.turns, { ...normalizedTurn, items: [] }],
  };
}

function mergeTurnLifecycle(existing: Turn, next: Turn): Turn {
  return {
    ...existing,
    ...next,
    items: existing.items,
  };
}

export function updateThreadItem(
  thread: Thread,
  turnId: string,
  item: ThreadItem,
  timestamps?: {
    startedAtMs?: number | null;
    completedAtMs?: number | null;
    syntheticTurnStatus?: "running" | "completed";
  },
) {
  const nextItem = normalizeThreadItemSnapshot(
    applyItemTimestamps(item, timestamps),
  );
  const completedCollabSyntheticTurns = thread.turns.filter(
    (turn) =>
      turn.id !== turnId &&
      isLiveDerivedCompletedAgentTurn(turn) &&
      turn.items.every(isCollabCompletionNotificationItem),
  );
  const completedCollabSyntheticTurnIds = new Set(
    completedCollabSyntheticTurns.map((turn) => turn.id),
  );
  const completedCollabSyntheticItems = completedCollabSyntheticTurns.flatMap(
    (turn) => turn.items,
  );
  let foundTurn = false;
  const updatedTurns = thread.turns.map((turn) => {
    if (completedCollabSyntheticTurnIds.has(turn.id)) {
      return null;
    }
    if (turn.id !== turnId) {
      return turn;
    }
    foundTurn = true;
    const items = [...completedCollabSyntheticItems, nextItem].reduce(
      appendOrMergeThreadItem,
      turn.items,
    );
    if (timestamps?.syntheticTurnStatus === "completed") {
      const startedAt =
        turn.startedAt ?? itemNotificationStartTimeSeconds(timestamps);
      const completedAt =
        itemNotificationCompletedTimeSeconds(timestamps) ??
        turn.completedAt ??
        startedAt;
      const durationMs =
        syntheticTurnDurationMs(timestamps) ??
        (startedAt !== null && completedAt !== null
          ? (completedAt - startedAt) * 1000
          : turn.durationMs);
      return {
        ...turn,
        items,
        status: "completed",
        startedAt,
        completedAt,
        durationMs,
      };
    }
    return { ...turn, items };
  }).filter((turn): turn is Turn => turn !== null);
  if (foundTurn) {
    return {
      ...thread,
      turns: updatedTurns,
    };
  }

  const activeTurn = isCollabCompletionNotificationItem(nextItem)
    ? [...thread.turns].reverse().find(isTurnInFlight)
    : undefined;
  if (activeTurn) {
    const turns = thread.turns.map((turn) =>
      turn.id === activeTurn.id
        ? { ...turn, items: appendOrMergeThreadItem(turn.items, nextItem) }
        : turn,
    );
    return {
      ...thread,
      turns,
    };
  }

  return {
    ...thread,
    turns: [...thread.turns, createSyntheticTurn(turnId, nextItem, timestamps)],
  };
}

function appendOrMergeThreadItem(items: ThreadItem[], nextItem: ThreadItem) {
  const existingItemIndex = items.findIndex((item) => item.id === nextItem.id);
  return existingItemIndex === -1
    ? [...items, nextItem]
    : items.map((existing, index) =>
        index === existingItemIndex
          ? // The server can refine an in-flight item into eventDrivenToolCall
            // once it recognizes a generic function call as an event-driven tool.
            mergeThreadItem(existing, nextItem)
          : existing,
      );
}

export function getThreadItemNotificationTargetThreadIds(
  notificationThreadId: string,
  item: ThreadItem,
) {
  const recipientThreadId =
    item.type === "collabAgentStatusUpdate" ||
    (item.type === "collabAgentMessage" && item.operation === "childCompletion")
      ? item.recipientThreadId
      : null;
  const senderThreadId =
    item.type === "collabAgentStatusUpdate" ||
    (item.type === "collabAgentMessage" && item.operation === "childCompletion")
      ? item.senderThreadId
      : null;
  return recipientThreadId &&
    recipientThreadId !== notificationThreadId &&
    senderThreadId === notificationThreadId
    ? [recipientThreadId]
    : [notificationThreadId];
}

export function getThreadItemNotificationSyntheticTurnStatus(
  method: "item/started" | "item/completed",
  item: ThreadItem,
): "completed" | undefined {
  return method === "item/completed" && isCollabCompletionNotificationItem(item)
    ? "completed"
    : undefined;
}

function isCollabCompletionNotificationItem(item: ThreadItem) {
  return (
    item.type === "collabAgentStatusUpdate" ||
    (item.type === "collabAgentMessage" && item.operation === "childCompletion")
  );
}

function createSyntheticTurn(
  turnId: string,
  item: ThreadItem,
  timestamps?: {
    startedAtMs?: number | null;
    completedAtMs?: number | null;
    syntheticTurnStatus?: "running" | "completed";
  },
): Turn {
  const status = timestamps?.syntheticTurnStatus ?? "running";
  const startedAt = itemNotificationStartTimeSeconds(timestamps);
  const completedAt =
    status === "completed"
      ? (itemNotificationCompletedTimeSeconds(timestamps) ?? startedAt)
      : null;
  return {
    id: turnId,
    items: [item],
    itemsView: "full",
    status,
    error: null,
    startedAt,
    completedAt,
    durationMs:
      status === "completed" ? syntheticTurnDurationMs(timestamps) : null,
  };
}

export function updateThreadSkills(thread: Thread, skills: ThreadSkill[]) {
  return { ...thread, skills };
}

export function appendAgentDelta(
  thread: Thread,
  turnId: string,
  itemId: string,
  delta: string,
) {
  const appendDelta = (item: Extract<ThreadItem, { type: "agentMessage" }>) => {
    return markStreamingAgentMessage({
      ...item,
      text: item.text + delta,
    });
  };
  const turns = thread.turns.some((turn) => turn.id === turnId)
    ? thread.turns.map((turn) => {
        if (turn.id !== turnId) {
          return turn;
        }
        const hasItem = turn.items.some((item) => item.id === itemId);
        const items = hasItem
          ? turn.items.flatMap((item) => {
              if (item.id !== itemId || item.type !== "agentMessage") {
                return [item];
              }
              return [appendDelta(item)];
            })
          : [
              ...turn.items,
              markStreamingAgentMessage({
                type: "agentMessage",
                id: itemId,
                text: delta,
                phase: null,
                memoryCitation: null,
              } satisfies ThreadItem),
            ];
        return { ...turn, items };
      })
    : [
        ...thread.turns,
        {
          id: turnId,
          items: [
            markStreamingAgentMessage({
              type: "agentMessage",
              id: itemId,
              text: delta,
              phase: null,
              memoryCitation: null,
            } satisfies ThreadItem),
          ],
          itemsView: "full" as const,
          status: "running" as const,
          error: null,
          startedAt: null,
          completedAt: null,
          durationMs: null,
        } satisfies Turn,
      ];
  return { ...thread, turns };
}

export function appendCommandExecutionDelta(
  thread: Thread,
  turnId: string,
  itemId: string,
  delta: string,
) {
  const appendDelta = (
    item: Extract<ThreadItem, { type: "commandExecution" }>,
  ): ThreadItem => ({
    ...item,
    aggregatedOutput: `${item.aggregatedOutput ?? ""}${delta}`,
  });
  const turns = thread.turns.some((turn) => turn.id === turnId)
    ? thread.turns.map((turn) => {
        if (turn.id !== turnId) {
          return turn;
        }
        return {
          ...turn,
          items: turn.items.map((item) =>
            item.id === itemId && item.type === "commandExecution"
              ? appendDelta(item)
              : item,
          ),
        };
      })
    : thread.turns;
  return { ...thread, turns };
}

export function mergeTurn(existing: Turn, next: Turn): Turn {
  const existingItems = existing.items.map(normalizeThreadItemSnapshot);
  const nextItems = next.items.map(normalizeThreadItemSnapshot);
  const mergedItems = mergeTurnItemsFromSnapshot(
    existing,
    next,
    existingItems,
    nextItems,
  );

  return {
    ...existing,
    ...next,
    items: mergedItems,
  };
}

function mergeTurnItemsFromSnapshot(
  existing: Turn,
  next: Turn,
  existingItems: ThreadItem[],
  nextItems: ThreadItem[],
) {
  const existingItemsById = new Map(
    existingItems.map((item) => [item.id, item]),
  );
  const mergedItems = nextItems.map((item) => {
    const existingItem = existingItemsById.get(item.id);
    return existingItem ? mergeThreadItem(existingItem, item) : item;
  });

  if (isTurnInFlight(existing) || isTurnInFlight(next)) {
    const mergedItemsMatcher = createTurnItemMatcher(
      buildTurnItemIndex([{ turn: next, items: mergedItems }]),
    );
    for (const item of existingItems) {
      if (!consumeMatchingTurnItem(mergedItemsMatcher, existing, item)) {
        mergedItems.push(item);
      }
    }
  }

  return mergedItems;
}

export function mergeThreadSnapshot(
  existing: Thread | null,
  next: Thread,
) {
  const normalizedNext = normalizeThreadSnapshot(next);
  if (!existing || existing.id !== normalizedNext.id) {
    return normalizedNext;
  }

  const nextTurnIds = new Set(normalizedNext.turns.map((turn) => turn.id));
  const turns = normalizedNext.turns.map((turn) => {
    const existingTurn = existing.turns.find(
      (candidate) => candidate.id === turn.id,
    );
    return existingTurn ? mergeTurn(existingTurn, turn) : turn;
  });
  const nextItemsMatcher = createTurnItemMatcher(
    buildTurnItemIndex(turns.map((turn) => ({ turn, items: turn.items }))),
  );

  for (const turn of existing.turns) {
    if (!nextTurnIds.has(turn.id)) {
      turns.push(...getRetainedUnmatchedTurn(turn, nextItemsMatcher));
    }
  }

  const tokenUsage =
    normalizedNext.tokenUsage ??
    existing.tokenUsage ??
    existing.threadUsage?.tokenUsage ??
    null;
  const contextUsage =
    normalizedNext.contextUsage ??
    existing.contextUsage ??
    existing.threadUsage?.contextUsage ??
    null;
  const threadUsage = normalizedNext.threadUsage
    ? {
        tokenUsage: normalizedNext.threadUsage.tokenUsage ?? tokenUsage,
        contextUsage: normalizedNext.threadUsage.contextUsage ?? contextUsage,
      }
    : (existing.threadUsage ?? {
        tokenUsage,
        contextUsage,
      });

  return {
    ...existing,
    ...normalizedNext,
    threadUsage,
    tokenUsage,
    contextUsage,
    turns,
  };
}

export function normalizeThreadSnapshot(thread: Thread): Thread {
  const turns = thread.turns.reduce<Turn[]>((normalizedTurns, turn) => {
    const normalizedTurn = normalizeTurnSnapshot(turn);
    const existingIndex = normalizedTurns.findIndex(
      (candidate) => candidate.id === normalizedTurn.id,
    );
    if (existingIndex !== -1) {
      return normalizedTurns.map((existing, index) =>
        index === existingIndex
          ? mergeTurn(existing, normalizedTurn)
          : existing,
      );
    }

    const incomingMatcher = createTurnItemMatcher(
      buildTurnItemIndex([
        { turn: normalizedTurn, items: normalizedTurn.items },
      ]),
    );
    const retainedExistingTurns = normalizedTurns.flatMap((existing) =>
      getRetainedUnmatchedTurn(existing, incomingMatcher),
    );
    const existingMatcher = createTurnItemMatcher(
      buildTurnItemIndex(
        retainedExistingTurns.map((existing) => ({
          turn: existing,
          items: existing.items,
        })),
      ),
    );
    return [
      ...retainedExistingTurns,
      ...getRetainedUnmatchedTurn(normalizedTurn, existingMatcher),
    ];
  }, []);

  return turns.length === thread.turns.length &&
    turns.every((turn, index) => turn === thread.turns[index])
    ? thread
    : { ...thread, turns };
}

function normalizeTurnSnapshot(turn: Turn): Turn {
  const items = turn.items.reduce<ThreadItem[]>((normalizedItems, item) => {
    const normalizedItem = normalizeThreadItemSnapshot(item);
    const existingIndex = normalizedItems.findIndex(
      (item) => item.id === normalizedItem.id,
    );
    if (existingIndex === -1) {
      return [...normalizedItems, normalizedItem];
    }
    return normalizedItems.map((existing, index) =>
      index === existingIndex
        ? mergeThreadItem(existing, normalizedItem)
        : existing,
    );
  }, []);

  return items.length === turn.items.length &&
    items.every((item, index) => item === turn.items[index])
    ? turn
    : { ...turn, items };
}

export function upsertThread(threads: Thread[], next: Thread) {
  const existing = threads.find((thread) => thread.id === next.id);
  if (!existing) {
    return [...threads, normalizeThreadSnapshot(next)];
  }
  return threads.map((thread) =>
    thread.id === next.id ? mergeThreadSnapshot(thread, next) : thread,
  );
}

export function upsertThreadMetadataPreservingTurns(
  threads: Thread[],
  next: Thread,
) {
  const existing = threads.find((thread) => thread.id === next.id);
  if (!existing) {
    return [...threads, normalizeThreadSnapshot(next)];
  }
  return threads.map((thread) =>
    thread.id === next.id ? { ...next, turns: existing.turns } : thread,
  );
}

export type ThreadUpdate = (thread: Thread) => Thread;

export function queuePendingThreadUpdate(
  pendingUpdates: Map<string, ThreadUpdate[]>,
  threadId: string,
  update: ThreadUpdate,
) {
  const updates = pendingUpdates.get(threadId) ?? [];
  updates.push(update);
  pendingUpdates.set(threadId, updates);
}

export function applyPendingThreadUpdates(
  thread: Thread,
  pendingUpdates: Map<string, ThreadUpdate[]>,
) {
  const updates = pendingUpdates.get(thread.id);
  if (!updates || updates.length === 0) {
    return thread;
  }
  pendingUpdates.delete(thread.id);
  return dropDuplicatePendingAgentTurns(
    thread,
    updates.reduce((updated, update) => update(updated), thread),
  );
}

export function applyInitializedThreadUpdate(
  threads: Thread[],
  initializedThreadIds: ReadonlySet<string>,
  threadId: string,
  update: ThreadUpdate,
) {
  if (!initializedThreadIds.has(threadId)) {
    return threads;
  }

  let foundThread = false;
  const next = threads.map((thread) => {
    if (thread.id !== threadId) {
      return thread;
    }
    foundThread = true;
    return update(thread);
  });

  return foundThread ? next : threads;
}

export function isTurnInFlight(turn: Turn) {
  return turn.status === "running" || turn.status === "inProgress";
}

export function isThreadThinking(
  thread: Thread | null,
  {
    isLoadingThread,
    isSending,
  }: {
    isLoadingThread: boolean;
    isSending: boolean;
  },
) {
  if (isLoadingThread) {
    return false;
  }
  if (isSending) {
    return true;
  }

  return (
    thread?.lifecycleStatus.type === "active" &&
    thread.lifecycleStatus.activeFlags.includes("running")
  );
}

function mergeThreadItem(existing: ThreadItem, next: ThreadItem): ThreadItem {
  const timestamps = mergeItemTimestamps(existing, next);

  if (existing.type === "agentMessage" && next.type === "agentMessage") {
    return {
      ...existing,
      ...next,
      ...timestamps,
      text: preferMoreCompleteText(existing.text, next.text),
    };
  }

  return {
    ...next,
    ...timestamps,
  };
}

function normalizeThreadItemSnapshot(item: ThreadItem): ThreadItem {
  return item;
}

function markStreamingAgentMessage<
  T extends Extract<ThreadItem, { type: "agentMessage" }>,
>(item: T) {
  return item;
}

function mergeItemTimestamps(
  existing: ThreadItem,
  next: ThreadItem,
): Pick<ThreadItem, "startedAtMs" | "completedAtMs"> {
  const timestamps: Pick<ThreadItem, "startedAtMs" | "completedAtMs"> = {};
  const startedAtMs = existing.startedAtMs ?? next.startedAtMs;
  if (startedAtMs !== null && startedAtMs !== undefined) {
    timestamps.startedAtMs = startedAtMs;
  }
  const completedAtMs = next.completedAtMs ?? existing.completedAtMs;
  if (completedAtMs !== null && completedAtMs !== undefined) {
    timestamps.completedAtMs = completedAtMs;
  }
  return timestamps;
}

function applyItemTimestamps(
  item: ThreadItem,
  timestamps?: {
    startedAtMs?: number | null;
    completedAtMs?: number | null;
  },
): ThreadItem {
  if (
    !timestamps ||
    ((timestamps.startedAtMs === null ||
      timestamps.startedAtMs === undefined) &&
      (timestamps.completedAtMs === null ||
        timestamps.completedAtMs === undefined))
  ) {
    return item;
  }
  const next = { ...item };
  if (timestamps.startedAtMs !== null && timestamps.startedAtMs !== undefined) {
    next.startedAtMs = timestamps.startedAtMs;
  }
  if (
    timestamps.completedAtMs !== null &&
    timestamps.completedAtMs !== undefined
  ) {
    next.completedAtMs = timestamps.completedAtMs;
  }
  return next;
}

function itemNotificationStartTimeSeconds(timestamps?: {
  startedAtMs?: number | null;
  completedAtMs?: number | null;
}) {
  const timestampMs = timestamps?.startedAtMs ?? timestamps?.completedAtMs;
  return timestampMs === null || timestampMs === undefined
    ? null
    : timestampMs / 1000;
}

function itemNotificationCompletedTimeSeconds(timestamps?: {
  completedAtMs?: number | null;
}) {
  const timestampMs = timestamps?.completedAtMs;
  return timestampMs === null || timestampMs === undefined
    ? null
    : timestampMs / 1000;
}

function syntheticTurnDurationMs(timestamps?: {
  startedAtMs?: number | null;
  completedAtMs?: number | null;
}) {
  const startedAtMs = timestamps?.startedAtMs;
  const completedAtMs = timestamps?.completedAtMs;
  return startedAtMs !== null &&
    startedAtMs !== undefined &&
    completedAtMs !== null &&
    completedAtMs !== undefined
    ? completedAtMs - startedAtMs
    : null;
}

type TurnItemIndex = {
  ids: Set<string>;
};

type TurnItemMatcher = {
  index: TurnItemIndex;
};

function createTurnItemMatcher(index: TurnItemIndex): TurnItemMatcher {
  return {
    index,
  };
}

function buildTurnItemIndex(
  entries: Array<{ turn: Turn; items: ThreadItem[] }>,
): TurnItemIndex {
  const ids = new Set<string>();

  for (const { items } of entries) {
    for (const item of items) {
      ids.add(item.id);
    }
  }

  return { ids };
}

function consumeMatchingTurnItem(
  matcher: TurnItemMatcher,
  turn: Turn,
  item: ThreadItem,
) {
  void turn;
  return matcher.index.ids.has(item.id);
}

function getRetainedUnmatchedTurn(
  turn: Turn,
  matcher: TurnItemMatcher,
): Turn[] {
  const normalizedItems = turn.items.map(normalizeThreadItemSnapshot);
  const normalizedTurn = normalizedItems.every(
    (item, index) => item === turn.items[index],
  )
    ? turn
    : { ...turn, items: normalizedItems };

  if (!isTurnInFlight(turn) && !isLiveDerivedCompletedAgentTurn(turn)) {
    return [normalizedTurn];
  }

  const items = normalizedItems.filter(
    (item) => !consumeMatchingTurnItem(matcher, turn, item),
  );
  if (items.length === 0) {
    return [];
  }
  return [
    items.length === normalizedItems.length
      ? normalizedTurn
      : { ...turn, items },
  ];
}

function dropDuplicatePendingAgentTurns(snapshot: Thread, updated: Thread) {
  const snapshotTurns = snapshot.turns.map(normalizeTurnSnapshot);
  const snapshotTurnIds = new Set(snapshotTurns.map((turn) => turn.id));
  const matcher = createTurnItemMatcher(
    buildTurnItemIndex(
      snapshotTurns.map((turn) => ({ turn, items: turn.items })),
    ),
  );
  const turns = updated.turns.flatMap((turn) => {
    const isPendingAgentTurn = turn.items.every(
      (item) => item.type === "agentMessage",
    );
    const normalizedItems = turn.items.map(normalizeThreadItemSnapshot);
    const normalizedTurn = normalizedItems.every(
      (item, index) => item === turn.items[index],
    )
      ? turn
      : { ...turn, items: normalizedItems };
    if (snapshotTurnIds.has(turn.id) || !isPendingAgentTurn) {
      return [normalizedTurn];
    }
    const items = normalizedItems.filter(
      (item) => !consumeMatchingTurnItem(matcher, turn, item),
    );
    if (items.length === 0) {
      return [];
    }
    return [
      items.length === normalizedItems.length
        ? normalizedTurn
        : { ...turn, items },
    ];
  });
  return { ...updated, turns };
}

function isLiveDerivedCompletedAgentTurn(turn: Turn) {
  return (
    turn.status === "completed" &&
    turn.items.length > 0 &&
    turn.items.every(
      (item) =>
        item.type === "agentMessage" || isCollabCompletionNotificationItem(item),
    ) &&
    (turn.itemsView !== "full" ||
      turn.items.every(isCollabCompletionNotificationItem))
  );
}

function preferMoreCompleteText(existing: string, next: string) {
  if (existing === next) {
    return next;
  }
  if (existing.startsWith(next)) {
    return existing;
  }
  return next;
}

export function threadStatusClass(lifecycleStatus: ThreadLifecycleStatus) {
  switch (lifecycleStatus.type) {
    case "active":
      return "doing";
    case "waiting":
      if (lifecycleStatus.reason === "command") {
        return "waiting-eventtool";
      }
      if (lifecycleStatus.reason === "child") {
        return "waiting-subagent";
      }
      if (lifecycleStatus.reason === "eventSubscription") {
        return "waiting-subscription";
      }
      return "todo";
    case "systemError":
      return "blocked";
    default:
      return "todo";
  }
}

export type TreeThreadLifecycleStatusClass =
  | "todo"
  | "doing"
  | "blocked"
  | "done"
  | "waiting-subagent"
  | "waiting-eventtool"
  | "waiting-subscription";

export function treeThreadLifecycleStatusClass(node: TreeNode): TreeThreadLifecycleStatusClass {
  return node.thread ? selfTreeThreadLifecycleStatusClass(node.thread) : "todo";
}

export function treeThreadLifecycleStatusLabel(statusClass: TreeThreadLifecycleStatusClass) {
  switch (statusClass) {
    case "doing":
      return "Active";
    case "waiting-subagent":
      return "Waiting on subagent";
    case "waiting-eventtool":
      return "Waiting on event tool";
    case "waiting-subscription":
      return "Waiting on subscription";
    case "blocked":
      return "System error";
    case "done":
      return "Done";
    case "todo":
      return "Inactive";
  }
}

function selfTreeThreadLifecycleStatusClass(thread: Thread): TreeThreadLifecycleStatusClass {
  if (thread.lifecycleStatus.type === "systemError") {
    return "blocked";
  }
  if (thread.lifecycleStatus.type === "waiting") {
    if (thread.lifecycleStatus.reason === "command") {
      return "waiting-eventtool";
    }
    if (thread.lifecycleStatus.reason === "child") {
      return "waiting-subagent";
    }
    if (thread.lifecycleStatus.reason === "eventSubscription") {
      return "waiting-subscription";
    }
    return "todo";
  }
  if (thread.lifecycleStatus.type === "active") {
    return "doing";
  }
  return "todo";
}

export function countDescendants(node: TreeNode): number {
  return node.children.reduce(
    (count, child) => count + 1 + countDescendants(child),
    0,
  );
}

export function normalizeProjectCwd(cwd: string | null | undefined) {
  const trimmed = cwd?.trim();
  if (!trimmed) {
    return null;
  }
  const normalized = trimmed.replaceAll("\\", "/").replace(/\/+$/, "");
  return normalized || "/";
}

function projectLabelFromCwd(cwd: string) {
  return cwd.split("/").filter(Boolean).at(-1) ?? cwd;
}

function compareCanonicalProjectRoot(left: Thread, right: Thread) {
  const statusDelta =
    canonicalProjectRootPriority(right.lifecycleStatus) -
    canonicalProjectRootPriority(left.lifecycleStatus);
  if (statusDelta !== 0) {
    return statusDelta;
  }
  if (left.updatedAt !== right.updatedAt) {
    return right.updatedAt - left.updatedAt;
  }
  if (left.createdAt !== right.createdAt) {
    return right.createdAt - left.createdAt;
  }
  return left.id.localeCompare(right.id);
}

function canonicalProjectRootPriority(lifecycleStatus: ThreadLifecycleStatus) {
  if (lifecycleStatus.type === "systemError") {
    return 4;
  }
  if (lifecycleStatus.type === "active" || lifecycleStatus.type === "initializing") {
    return 3;
  }
  if (lifecycleStatus.type === "waiting") {
    return 2;
  }
  if (lifecycleStatus.type === "notLoaded") {
    return 1;
  }
  return 0;
}

function withProjectRootLabel(node: TreeNode, projectLabel: string): TreeNode {
  const descendantCount = countDescendants(node);
  return {
    ...node,
    label: node.thread ? (node.thread.name ?? projectLabel) : node.label,
    path:
      node.thread?.preview ||
      (descendantCount > 0
        ? `${descendantCount} subagents`
        : getThreadPresenceLabel(node.thread)),
  };
}

function withChatConversationLabel(node: TreeNode): TreeNode {
  return {
    ...node,
    label: node.thread ? getThreadLabel(node.thread) : node.label,
    path: node.thread?.preview || getThreadPresenceLabel(node.thread),
  };
}

function buildSidebarRootTree(
  threads: Thread[],
  rootThread: Thread,
  mapRoot: (node: TreeNode) => TreeNode,
): TreeNode {
  const subtreeIds = getThreadSubtreeIds(threads, rootThread.id);
  const subtreeThreads = threads.filter((thread) => subtreeIds.has(thread.id));
  const rootTree =
    buildAgentTree(subtreeThreads, rootThread.id)[0] ??
    buildSidebarThreadNode(rootThread);
  return mapRoot(rootTree);
}

function buildSidebarThreadNode(thread: Thread): TreeNode {
  return {
    key: thread.id,
    label: getThreadLabel(thread),
    path: thread.preview || getThreadPresenceLabel(thread),
    thread,
    threadId: thread.id,
    isPlaceholder: false,
    children: [],
  };
}

function collectTreeThreads(node: TreeNode): Thread[] {
  return [
    ...(node.thread ? [node.thread] : []),
    ...node.children.flatMap(collectTreeThreads),
  ];
}

function aggregateSidebarStatus(threads: Thread[]): TreeThreadLifecycleStatusClass {
  const statusClasses = threads.map(selfTreeThreadLifecycleStatusClass);
  if (statusClasses.includes("blocked")) {
    return "blocked";
  }
  if (statusClasses.includes("doing")) {
    return "doing";
  }
  if (statusClasses.includes("waiting-subagent")) {
    return "waiting-subagent";
  }
  if (statusClasses.includes("waiting-eventtool")) {
    return "waiting-eventtool";
  }
  if (statusClasses.includes("waiting-subscription")) {
    return "waiting-subscription";
  }
  if (threads.length > 0 && statusClasses.every((status) => status === "done")) {
    return "done";
  }
  return "todo";
}

function countSidebarStatuses(threads: Thread[]) {
  let activeCount = 0;
  let waitingCount = 0;
  let failedCount = 0;
  for (const thread of threads) {
    const statusClass = selfTreeThreadLifecycleStatusClass(thread);
    if (statusClass === "doing") {
      activeCount += 1;
    } else if (statusClass === "blocked") {
      failedCount += 1;
    } else if (statusClass.startsWith("waiting-")) {
      waitingCount += 1;
    }
  }
  return { activeCount, waitingCount, failedCount };
}

export function formatClockTime(unixSeconds: number) {
  return new Intl.DateTimeFormat("en-US", {
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(unixSeconds * 1000));
}

export function formatUpdatedLabel(unixSeconds: number) {
  const target = new Date(unixSeconds * 1000);
  const now = new Date();
  const diffDays = Math.floor((now.getTime() - target.getTime()) / 86_400_000);
  if (diffDays <= 0) {
    return `Updated ${formatClockTime(unixSeconds)}`;
  }
  if (diffDays === 1) {
    return "Updated Yesterday";
  }
  return `Updated ${target.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  })}`;
}

export function capitalize(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

export function trimPath(path: string) {
  return path.split("/").slice(-2).join("/");
}

export function trimThreadId(threadId: string) {
  return threadId.slice(0, 8);
}

function mapTaskStatus(lifecycleStatus: ThreadLifecycleStatus): Exclude<TaskFilter, "all"> {
  const mapped = threadStatusClass(lifecycleStatus);
  if (mapped === "blocked" || mapped === "doing") {
    return mapped;
  }
  return "todo";
}

function getActivePresenceLabel(activeFlags: ThreadLifecycleActiveFlag[]) {
  if (activeFlags.includes("waitingOnApproval")) {
    return "Waiting on Approval";
  }
  if (activeFlags.includes("waitingOnUserInput")) {
    return "Waiting on Input";
  }
  if (activeFlags.includes("running")) {
    return "Running";
  }
  return "Active";
}

type ThreadSpawnSource = {
  parent_thread_id?: string;
  parentThreadId?: string;
  depth?: number;
  agent_path?: string | null;
  agentPath?: string | null;
  agent_nickname?: string | null;
  agentNickname?: string | null;
  agent_role?: string | null;
  agentRole?: string | null;
};

type SubAgentSourceRecord = {
  thread_spawn?: ThreadSpawnSource;
  threadSpawn?: ThreadSpawnSource;
};

type ThreadSourceRecord = {
  subAgent?: SubAgentSourceRecord | string;
  subagent?: SubAgentSourceRecord | string;
};

function getThreadSpawnSource(thread: Thread): ThreadSpawnSource | null {
  const source = thread.source;
  if (!source || typeof source !== "object") {
    return null;
  }

  const sourceRecord = source as ThreadSourceRecord;
  const subAgentSource = sourceRecord.subAgent ?? sourceRecord.subagent;
  if (!subAgentSource || typeof subAgentSource !== "object") {
    return null;
  }

  const subAgentSourceRecord = subAgentSource as SubAgentSourceRecord;
  const threadSpawn =
    subAgentSourceRecord.thread_spawn ?? subAgentSourceRecord.threadSpawn;
  return threadSpawn && typeof threadSpawn === "object" ? threadSpawn : null;
}
