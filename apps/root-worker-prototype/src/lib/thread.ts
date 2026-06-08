import type {
  TaskFilter,
  Thread,
  ThreadActiveFlag,
  ThreadItem,
  ThreadSkill,
  ThreadStatus,
  TodoCardItem,
  TreeNode,
  Turn,
} from "../types";
import { hasActiveMonitors } from "./threadAnalysis";

const STREAMING_AGENT_MESSAGE = Symbol("streamingAgentMessage");

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
      const status = mapTaskStatus(thread.status);
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

export function getAgentRoleLabel(thread: Thread) {
  const source = thread.source;
  if (typeof source === "object" && "subAgent" in source) {
    return "Worker Agent";
  }
  return "Root Agent";
}

export function getPresenceLabel(status: ThreadStatus) {
  if (status.type === "active") {
    return getActivePresenceLabel(status.activeFlags);
  }

  switch (status.type) {
    case "systemError":
      return "System Error";
    case "notLoaded":
      return "Not Loaded";
    case "idle":
    default:
      return "Idle";
  }
}

export function getThreadPresenceLabel(thread: Thread | null) {
  if (!thread) {
    return "Idle";
  }
  if (!isEffectivelyActiveThread(thread)) {
    return getPresenceLabel({ type: "idle" });
  }
  return getPresenceLabel(thread.status);
}

export function threadDisplayStatusClass(thread: Thread | null) {
  if (!thread) {
    return threadStatusClass({ type: "notLoaded" });
  }
  if (!isEffectivelyActiveThread(thread)) {
    return threadStatusClass({ type: "idle" });
  }
  return threadStatusClass(thread.status);
}

export function getThreadModelLabel(thread: Thread | null) {
  if (!thread) {
    return "unknown";
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
  let foundTurn = false;
  const updatedTurns = thread.turns.map((turn) => {
    if (turn.id !== turnId) {
      return turn;
    }
    foundTurn = true;
    const items = appendOrMergeThreadItem(turn.items, nextItem);
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
  });
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
  const existingItemIndex = findMatchingThreadItemIndex(items, nextItem);
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
  const turns = thread.turns.some((turn) => turn.id === turnId)
    ? thread.turns.map((turn) => {
        if (turn.id !== turnId) {
          return turn;
        }
        const hasItem = turn.items.some((item) => item.id === itemId);
        const items = hasItem
          ? turn.items.map((item) =>
              item.id === itemId && item.type === "agentMessage"
                ? markStreamingAgentMessage({
                    ...item,
                    text: item.text + delta,
                  })
                : item,
            )
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

export function mergeTurn(existing: Turn, next: Turn): Turn {
  const existingItems = existing.items.map(normalizeThreadItemSnapshot);
  const nextItems = next.items.map(normalizeThreadItemSnapshot);
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

  return {
    ...existing,
    ...next,
    items: mergedItems,
  };
}

export function mergeThreadSnapshot(existing: Thread | null, next: Thread) {
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
    const existingIndex = findMatchingSnapshotItemIndex(
      normalizedItems,
      normalizedItem,
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

  const lastTurn = thread?.turns.at(-1) ?? null;
  if (!lastTurn || !isTurnInFlight(lastTurn)) {
    return false;
  }
  if (lastTurn.items.length === 0) {
    return true;
  }

  return lastTurn.items.some(
    (item) => item.type !== "userMessage" && item.type !== "injectedContext",
  );
}

function mergeThreadItem(existing: ThreadItem, next: ThreadItem): ThreadItem {
  const normalizedExisting = normalizeThreadItemSnapshot(existing);
  const normalizedNext = normalizeThreadItemSnapshot(next);
  if (normalizedExisting !== existing || normalizedNext !== next) {
    return mergeThreadItem(normalizedExisting, normalizedNext);
  }

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
  if (item.type !== "agentMessage") {
    return item;
  }

  const trigger = parseEventDrivenToolTrigger(item.text);
  if (!trigger) {
    return item;
  }

  const normalized: ThreadItem = {
    type: "eventDrivenTool",
    id: item.id,
    tool: trigger.tool,
    title: trigger.title,
    text: trigger.text,
  };
  if (item.startedAtMs !== null && item.startedAtMs !== undefined) {
    normalized.startedAtMs = item.startedAtMs;
  }
  if (item.completedAtMs !== null && item.completedAtMs !== undefined) {
    normalized.completedAtMs = item.completedAtMs;
  }
  return normalized;
}

function parseEventDrivenToolTrigger(text: string) {
  const trimmed = text.trim();
  const startMarker = "<event_driven_tool>";
  const endMarker = "</event_driven_tool>";
  if (!trimmed.startsWith(startMarker) || !trimmed.endsWith(endMarker)) {
    return null;
  }

  const body = trimmed
    .slice(startMarker.length, trimmed.length - endMarker.length)
    .trim();
  try {
    const parsed = JSON.parse(body);
    if (
      parsed &&
      typeof parsed === "object" &&
      !Array.isArray(parsed) &&
      typeof parsed.tool === "string" &&
      typeof parsed.title === "string" &&
      typeof parsed.text === "string"
    ) {
      return parsed as { tool: string; title: string; text: string };
    }
  } catch {
    return null;
  }
  return null;
}

function markStreamingAgentMessage<
  T extends Extract<ThreadItem, { type: "agentMessage" }>,
>(item: T) {
  Object.defineProperty(item, STREAMING_AGENT_MESSAGE, {
    value: true,
    enumerable: false,
  });
  return item;
}

function isStreamingAgentMessage(
  item: Extract<ThreadItem, { type: "agentMessage" }>,
) {
  return Boolean(
    (
      item as Extract<ThreadItem, { type: "agentMessage" }> & {
        [STREAMING_AGENT_MESSAGE]?: true;
      }
    )[STREAMING_AGENT_MESSAGE],
  );
}

function findMatchingThreadItemIndex(
  items: ThreadItem[],
  nextItem: ThreadItem,
) {
  const idIndex = items.findIndex((item) => item.id === nextItem.id);
  if (idIndex !== -1) {
    return idIndex;
  }

  if (nextItem.type === "agentMessage") {
    for (let index = items.length - 1; index >= 0; index -= 1) {
      const item = items[index];
      if (
        item?.type === "agentMessage" &&
        canMergeSameTurnThreadItems(item, nextItem)
      ) {
        return index;
      }
    }
    return -1;
  }

  return items.findIndex((item) => canMergeSameTurnThreadItems(item, nextItem));
}

function findMatchingSnapshotItemIndex(
  items: ThreadItem[],
  nextItem: ThreadItem,
) {
  const idIndex = items.findIndex((item) => item.id === nextItem.id);
  if (idIndex !== -1) {
    return idIndex;
  }

  return items.findIndex((item) =>
    canMergeSameSnapshotThreadItems(item, nextItem),
  );
}

function canMergeSameTurnThreadItems(existing: ThreadItem, next: ThreadItem) {
  if (
    existing.type !== next.type ||
    !canMatchThreadItemSemantically(existing)
  ) {
    return false;
  }

  if (existing.type === "agentMessage" && next.type === "agentMessage") {
    return haveCompatibleAgentMessages(existing, next);
  }

  return getThreadItemSemanticKey(existing) === getThreadItemSemanticKey(next);
}

function canMergeSameSnapshotThreadItems(
  existing: ThreadItem,
  next: ThreadItem,
) {
  if (
    existing.type !== next.type ||
    !canMatchThreadItemSemantically(existing)
  ) {
    return false;
  }

  if (existing.type === "agentMessage" && next.type === "agentMessage") {
    return haveCompatibleSnapshotAgentMessages(existing, next);
  }

  return getThreadItemSemanticKey(existing) === getThreadItemSemanticKey(next);
}

function haveCompatibleAgentMessages(
  existing: Extract<ThreadItem, { type: "agentMessage" }>,
  next: Extract<ThreadItem, { type: "agentMessage" }>,
) {
  if (
    existing.phase !== next.phase ||
    stableStringify(existing.memoryCitation) !==
      stableStringify(next.memoryCitation)
  ) {
    return false;
  }
  if (
    existing.completedAtMs !== null &&
    existing.completedAtMs !== undefined &&
    next.completedAtMs !== null &&
    next.completedAtMs !== undefined
  ) {
    return false;
  }
  if (!isStreamingAgentMessage(existing)) {
    return false;
  }

  return (
    existing.text.startsWith(next.text) || next.text.startsWith(existing.text)
  );
}

function haveCompatibleSnapshotAgentMessages(
  existing: Extract<ThreadItem, { type: "agentMessage" }>,
  next: Extract<ThreadItem, { type: "agentMessage" }>,
) {
  return (
    existing.phase === next.phase &&
    stableStringify(existing.memoryCitation) ===
      stableStringify(next.memoryCitation) &&
    existing.text === next.text
  );
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
  semantic: Map<string, Turn[]>;
  agentMessages: Array<{
    turn: Turn;
    item: Extract<ThreadItem, { type: "agentMessage" }>;
  }>;
};

type TurnItemMatcher = {
  index: TurnItemIndex;
  consumedSemantic: Map<string, Set<number>>;
};

function createTurnItemMatcher(index: TurnItemIndex): TurnItemMatcher {
  return {
    index,
    consumedSemantic: new Map(),
  };
}

function buildTurnItemIndex(
  entries: Array<{ turn: Turn; items: ThreadItem[] }>,
): TurnItemIndex {
  const ids = new Set<string>();
  const semantic = new Map<string, Turn[]>();
  const agentMessages: TurnItemIndex["agentMessages"] = [];

  for (const { turn, items } of entries) {
    for (const item of items) {
      ids.add(item.id);
      if (item.type === "agentMessage") {
        agentMessages.push({ turn, item });
      }
      const key = getThreadItemSemanticKey(item);
      const matchingTurns = semantic.get(key) ?? [];
      matchingTurns.push(turn);
      semantic.set(key, matchingTurns);
    }
  }

  return { ids, semantic, agentMessages };
}

function consumeMatchingTurnItem(
  matcher: TurnItemMatcher,
  turn: Turn,
  item: ThreadItem,
) {
  if (matcher.index.ids.has(item.id)) {
    return true;
  }
  if (!canMatchThreadItemSemantically(item)) {
    return false;
  }
  if (item.type === "agentMessage") {
    return consumeMatchingAgentMessage(matcher, turn, item);
  }

  const key = getThreadItemSemanticKey(item);
  const matchingTurns = matcher.index.semantic.get(key) ?? [];
  const consumed = matcher.consumedSemantic.get(key) ?? new Set<number>();
  for (const [index, candidate] of matchingTurns.entries()) {
    if (!consumed.has(index) && haveCompatibleTurnTimes(candidate, turn)) {
      consumed.add(index);
      matcher.consumedSemantic.set(key, consumed);
      return true;
    }
  }
  return false;
}

function consumeMatchingAgentMessage(
  matcher: TurnItemMatcher,
  turn: Turn,
  item: Extract<ThreadItem, { type: "agentMessage" }>,
) {
  const consumed =
    matcher.consumedSemantic.get("agentMessage") ?? new Set<number>();
  for (const [index, candidate] of matcher.index.agentMessages.entries()) {
    if (
      !consumed.has(index) &&
      haveCompatibleTurnTimes(candidate.turn, turn) &&
      haveCompatibleAgentMessageContent(candidate.item, item)
    ) {
      consumed.add(index);
      matcher.consumedSemantic.set("agentMessage", consumed);
      return true;
    }
  }
  return false;
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

function haveCompatibleTurnTimes(left: Turn, right: Turn) {
  if (hasNoTurnTimes(left) || hasNoTurnTimes(right)) {
    return true;
  }
  if (
    left.startedAt !== null &&
    left.startedAt !== undefined &&
    right.startedAt !== null &&
    right.startedAt !== undefined &&
    left.startedAt === right.startedAt
  ) {
    return true;
  }
  return (
    left.completedAt !== null &&
    left.completedAt !== undefined &&
    right.completedAt !== null &&
    right.completedAt !== undefined &&
    left.completedAt === right.completedAt
  );
}

function hasNoTurnTimes(turn: Turn) {
  return (
    turn.startedAt === null &&
    turn.completedAt === null &&
    turn.durationMs === null
  );
}

function canMatchThreadItemSemantically(item: ThreadItem) {
  switch (item.type) {
    case "agentMessage":
    case "collabAgentMessage":
    case "eventCommandCall":
    case "eventCommandEvent":
    case "eventDrivenTool":
    case "eventDrivenToolCall":
    case "collabAgentStatusUpdate":
      return true;
    case "builtinToolCall":
    case "collabAgentToolCall":
    case "commandExecution":
    case "contextCompaction":
    case "dynamicToolCall":
    case "enteredReviewMode":
    case "exitedReviewMode":
    case "fileChange":
    case "imageGeneration":
    case "imageView":
    case "injectedContext":
    case "mcpToolCall":
    case "plan":
    case "reasoning":
    case "userMessage":
    case "webSearch":
      return false;
  }
}

function haveCompatibleAgentMessageContent(
  left: Extract<ThreadItem, { type: "agentMessage" }>,
  right: Extract<ThreadItem, { type: "agentMessage" }>,
) {
  return (
    left.phase === right.phase &&
    stableStringify(left.memoryCitation) ===
      stableStringify(right.memoryCitation) &&
    (left.text.startsWith(right.text) || right.text.startsWith(left.text))
  );
}

function getThreadItemSemanticKey(item: ThreadItem) {
  const {
    id: _id,
    startedAtMs: _startedAtMs,
    completedAtMs: _completedAtMs,
    ...content
  } = item;
  return `${item.type}:${stableStringify(content)}`;
}

function stableStringify(value: unknown): string {
  if (value === undefined) {
    return "undefined";
  }
  if (value === null || typeof value !== "object") {
    return JSON.stringify(value) ?? "undefined";
  }
  if (Array.isArray(value)) {
    return `[${value.map(stableStringify).join(",")}]`;
  }

  const record = value as Record<string, unknown>;
  return `{${Object.keys(record)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${stableStringify(record[key])}`)
    .join(",")}}`;
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

export function threadStatusClass(status: ThreadStatus) {
  switch (status.type) {
    case "active":
      return "doing";
    case "systemError":
      return "blocked";
    default:
      return "todo";
  }
}

export type TreeThreadStatusClass =
  | "todo"
  | "doing"
  | "blocked"
  | "done"
  | "waiting-subagent"
  | "waiting-eventtool";

const MONITOR_TOOL_NAMES = new Set([
  "event_command_subscribe",
  "schedule_subscribe",
]);

export function treeThreadStatusClass(node: TreeNode): TreeThreadStatusClass {
  const selfClass = node.thread
    ? selfTreeThreadStatusClass(node.thread)
    : "todo";

  if (selfClass !== "todo") {
    return selfClass;
  }

  let hasBlocked = false;
  for (const child of node.children) {
    const childClass = treeThreadStatusClass(child);
    if (
      childClass === "doing" ||
      childClass === "waiting-subagent" ||
      childClass === "waiting-eventtool"
    ) {
      return "waiting-subagent";
    }
    if (childClass === "blocked") {
      hasBlocked = true;
    }
  }

  if (hasBlocked) {
    return "blocked";
  }
  return "todo";
}

export function treeThreadStatusLabel(statusClass: TreeThreadStatusClass) {
  switch (statusClass) {
    case "doing":
      return "Active";
    case "waiting-subagent":
      return "Waiting on subagent";
    case "waiting-eventtool":
      return "Waiting on event tool";
    case "blocked":
      return "System error";
    case "done":
      return "Done";
    case "todo":
      return "Inactive";
  }
}

function selfTreeThreadStatusClass(thread: Thread): TreeThreadStatusClass {
  if (thread.status.type === "systemError") {
    return "blocked";
  }
  if (!isEffectivelyActiveThread(thread)) {
    return "todo";
  }
  if (hasActiveTurnWork(thread)) {
    return "doing";
  }
  if (hasActiveMonitorWait(thread)) {
    return "waiting-eventtool";
  }
  if (hasInFlightSubagentWait(thread)) {
    return "waiting-subagent";
  }
  return "doing";
}

function isEffectivelyActiveThread(thread: Thread) {
  if (thread.status.type === "systemError") {
    return true;
  }
  if (thread.status.type !== "active") {
    return false;
  }
  return (
    thread.turns.length === 0 ||
    hasActiveTurnWork(thread) ||
    hasActiveMonitorWait(thread) ||
    hasInFlightSubagentWait(thread)
  );
}

function hasInFlightSubagentWait(thread: Thread) {
  return thread.turns.some(
    (turn) =>
      isTurnInFlight(turn) &&
      turn.items.some((item) => isInFlightSubagentWaitItem(item)),
  );
}

function hasActiveMonitorWait(thread: Thread) {
  return hasActiveMonitors(thread);
}

function hasActiveTurnWork(thread: Thread) {
  return thread.turns.some((turn) => {
    if (!isTurnInFlight(turn)) {
      return false;
    }
    if (turn.items.length === 0) {
      return true;
    }
    return turn.items.some(
      (item) =>
        item.type !== "userMessage" &&
        item.type !== "injectedContext" &&
        !isInFlightSubagentWaitItem(item) &&
        !isMonitorToolItem(item),
    );
  });
}

function isInFlightSubagentWaitItem(item: ThreadItem) {
  return (
    item.type === "collabAgentToolCall" &&
    item.tool.toLowerCase() === "wait" &&
    isItemInProgress(item.status)
  );
}

function isMonitorToolItem(item: ThreadItem) {
  if (item.type === "eventDrivenTool" || item.type === "eventDrivenToolCall") {
    return isMonitorToolName(item.tool);
  }
  if (item.type === "eventCommandCall" || item.type === "eventCommandEvent") {
    return true;
  }
  return false;
}

function isMonitorToolName(tool: string) {
  return MONITOR_TOOL_NAMES.has(tool);
}

function isItemInProgress(status: string) {
  const normalized = status.toLowerCase();
  return normalized === "inprogress" || normalized === "in_progress";
}

export function countDescendants(node: TreeNode): number {
  return node.children.reduce(
    (count, child) => count + 1 + countDescendants(child),
    0,
  );
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

function mapTaskStatus(status: ThreadStatus): Exclude<TaskFilter, "all"> {
  const mapped = threadStatusClass(status);
  if (mapped === "blocked" || mapped === "doing") {
    return mapped;
  }
  return "todo";
}

function getActivePresenceLabel(activeFlags: ThreadActiveFlag[]) {
  if (activeFlags.includes("waitingOnApproval")) {
    return "Waiting on Approval";
  }
  if (activeFlags.includes("waitingOnUserInput")) {
    return "Waiting on Input";
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
