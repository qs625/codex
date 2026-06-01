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
  const items = [...threads]
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
  const hasExistingTurn = thread.turns.some(
    (existing) => existing.id === turn.id,
  );
  if (hasExistingTurn) {
    const turns = thread.turns.map((existing) =>
      existing.id === turn.id ? mergeTurn(existing, turn) : existing,
    );
    return { ...thread, turns };
  }

  const turnMatcher = createTurnItemMatcher(
    buildTurnItemIndex([{ turn, items: turn.items }]),
  );
  const turns = [
    ...thread.turns.flatMap((existing) =>
      getUnmatchedInFlightTurn(existing, turnMatcher),
    ),
    turn,
  ];
  return { ...thread, turns };
}

export function updateThreadItem(
  thread: Thread,
  turnId: string,
  item: ThreadItem,
) {
  return {
    ...thread,
    turns: thread.turns.map((turn) => {
      if (turn.id !== turnId) {
        return turn;
      }
      const items = turn.items.some((existing) => existing.id === item.id)
        ? turn.items.map((existing) =>
            existing.id === item.id
              ? // The server can refine an in-flight item into eventDrivenToolCall
                // once it recognizes a generic function call as an event-driven tool.
                item
              : existing,
          )
        : [...turn.items, item];
      return { ...turn, items };
    }),
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
                ? { ...item, text: item.text + delta }
                : item,
            )
          : [
              ...turn.items,
              {
                type: "agentMessage",
                id: itemId,
                text: delta,
                phase: null,
                memoryCitation: null,
              } satisfies ThreadItem,
            ];
        return { ...turn, items };
      })
    : [
        ...thread.turns,
        {
          id: turnId,
          items: [
            {
              type: "agentMessage",
              id: itemId,
              text: delta,
              phase: null,
              memoryCitation: null,
            } satisfies ThreadItem,
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
  const existingItemsById = new Map(
    existing.items.map((item) => [item.id, item]),
  );
  const mergedItems = next.items.map((item) => {
    const existingItem = existingItemsById.get(item.id);
    return existingItem ? mergeThreadItem(existingItem, item) : item;
  });

  if (isTurnInFlight(existing) || isTurnInFlight(next)) {
    const mergedItemsMatcher = createTurnItemMatcher(
      buildTurnItemIndex([{ turn: next, items: mergedItems }]),
    );
    for (const item of existing.items) {
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
  if (!existing || existing.id !== next.id) {
    return next;
  }

  const nextTurnIds = new Set(next.turns.map((turn) => turn.id));
  const turns = next.turns.map((turn) => {
    const existingTurn = existing.turns.find(
      (candidate) => candidate.id === turn.id,
    );
    return existingTurn ? mergeTurn(existingTurn, turn) : turn;
  });
  const nextItemsMatcher = createTurnItemMatcher(
    buildTurnItemIndex(turns.map((turn) => ({ turn, items: turn.items }))),
  );

  for (const turn of existing.turns) {
    if (!nextTurnIds.has(turn.id) && isTurnInFlight(turn)) {
      turns.push(...getUnmatchedInFlightTurn(turn, nextItemsMatcher));
    }
  }

  const tokenUsage =
    next.tokenUsage ??
    existing.tokenUsage ??
    existing.threadUsage?.tokenUsage ??
    null;
  const contextUsage =
    next.contextUsage ??
    existing.contextUsage ??
    existing.threadUsage?.contextUsage ??
    null;
  const threadUsage = next.threadUsage
    ? {
        tokenUsage: next.threadUsage.tokenUsage ?? tokenUsage,
        contextUsage: next.threadUsage.contextUsage ?? contextUsage,
      }
    : (existing.threadUsage ?? {
        tokenUsage,
        contextUsage,
      });

  return {
    ...existing,
    ...next,
    threadUsage,
    tokenUsage,
    contextUsage,
    turns,
  };
}

export function upsertThread(threads: Thread[], next: Thread) {
  const existing = threads.find((thread) => thread.id === next.id);
  if (!existing) {
    return [...threads, next];
  }
  return threads.map((thread) =>
    thread.id === next.id ? mergeThreadSnapshot(thread, next) : thread,
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
  if (existing.type === "agentMessage" && next.type === "agentMessage") {
    return {
      ...existing,
      ...next,
      text: preferMoreCompleteText(existing.text, next.text),
    };
  }

  return next;
}

type TurnItemIndex = {
  ids: Set<string>;
  semantic: Map<string, Turn[]>;
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

  for (const { turn, items } of entries) {
    for (const item of items) {
      ids.add(item.id);
      const key = getThreadItemSemanticKey(item);
      const matchingTurns = semantic.get(key) ?? [];
      matchingTurns.push(turn);
      semantic.set(key, matchingTurns);
    }
  }

  return { ids, semantic };
}

function consumeMatchingTurnItem(
  matcher: TurnItemMatcher,
  turn: Turn,
  item: ThreadItem,
) {
  if (matcher.index.ids.has(item.id)) {
    return true;
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

function getUnmatchedInFlightTurn(
  turn: Turn,
  matcher: TurnItemMatcher,
): Turn[] {
  if (!isTurnInFlight(turn)) {
    return [turn];
  }

  const items = turn.items.filter(
    (item) => !consumeMatchingTurnItem(matcher, turn, item),
  );
  if (items.length === 0) {
    return [];
  }
  return [items.length === turn.items.length ? turn : { ...turn, items }];
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

function getThreadItemSemanticKey(item: ThreadItem) {
  const { id: _id, ...content } = item;
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
