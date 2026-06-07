function mergeThreadSnapshots(existing, next) {
  const normalizedNext = normalizeThreadSnapshot(next);
  if (!existing) {
    return normalizedNext;
  }

  const turns = mergeTurns(existing.turns ?? [], normalizedNext.turns ?? []);
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

  return {
    ...existing,
    ...normalizedNext,
    threadUsage: {
      tokenUsage: normalizedNext.threadUsage?.tokenUsage ?? tokenUsage,
      contextUsage: normalizedNext.threadUsage?.contextUsage ?? contextUsage,
    },
    tokenUsage,
    contextUsage,
    turns,
  };
}

function normalizeThreadSnapshot(thread) {
  const turns = (thread.turns ?? []).reduce((normalizedTurns, turn) => {
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
        { turn: normalizedTurn, items: normalizedTurn.items ?? [] },
      ]),
    );
    const retainedExistingTurns = normalizedTurns.flatMap((existing) =>
      getRetainedUnmatchedTurn(existing, incomingMatcher),
    );
    const existingMatcher = createTurnItemMatcher(
      buildTurnItemIndex(
        retainedExistingTurns.map((existing) => ({
          turn: existing,
          items: existing.items ?? [],
        })),
      ),
    );
    return [
      ...retainedExistingTurns,
      ...getRetainedUnmatchedTurn(normalizedTurn, existingMatcher),
    ];
  }, []);

  if (
    turns.length === (thread.turns ?? []).length &&
    turns.every((turn, index) => turn === thread.turns?.[index])
  ) {
    return thread;
  }
  return { ...thread, turns };
}

function normalizeTurnSnapshot(turn) {
  const items = (turn.items ?? []).reduce((normalizedItems, item) => {
    const normalizedItem = normalizeThreadItemSnapshot(item);
    const existingIndex = findMatchingThreadItemIndex(
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

  if (
    items.length === (turn.items ?? []).length &&
    items.every((item, index) => item === turn.items?.[index])
  ) {
    return turn;
  }
  return { ...turn, items };
}

function mergeTurns(existingTurns, nextTurns) {
  const nextTurnIds = new Set(nextTurns.map((turn) => turn.id));
  const turns = nextTurns.map((turn) => {
    const existingTurn = existingTurns.find(
      (candidate) => candidate.id === turn.id,
    );
    return existingTurn ? mergeTurn(existingTurn, turn) : turn;
  });
  const nextItemsIndex = buildTurnItemIndex(
    turns.map((turn) => ({ turn, items: turn.items ?? [] })),
  );
  const nextItemsMatcher = createTurnItemMatcher(nextItemsIndex);

  for (const turn of existingTurns) {
    if (nextTurnIds.has(turn.id)) {
      continue;
    }
    turns.push(...getRetainedUnmatchedTurn(turn, nextItemsMatcher));
  }

  return turns;
}

function getRetainedUnmatchedTurn(turn, matcher) {
  const normalizedItems = (turn.items ?? []).map(normalizeThreadItemSnapshot);
  const normalizedTurn = normalizedItems.every(
    (item, index) => item === turn.items?.[index],
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

function isLiveDerivedCompletedAgentTurn(turn) {
  return (
    turn.status === "completed" &&
    turn.itemsView !== "full" &&
    (turn.items ?? []).length > 0 &&
    (turn.items ?? []).every((item) => item.type === "agentMessage")
  );
}

function haveCompatibleAgentMessageContent(left, right) {
  return (
    left.phase === right.phase &&
    stableStringify(left.memoryCitation) ===
      stableStringify(right.memoryCitation) &&
    (left.text.startsWith(right.text) || right.text.startsWith(left.text))
  );
}

function consumeMatchingAgentMessage(matcher, turn, item) {
  const consumed = matcher.consumedSemantic.get("agentMessage") ?? new Set();
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

function mergeTurn(existing, next) {
  const existingItems = (existing.items ?? []).map(normalizeThreadItemSnapshot);
  const nextItems = (next.items ?? []).map(normalizeThreadItemSnapshot);
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

function mergeThreadItem(existing, next) {
  const normalizedExisting = normalizeThreadItemSnapshot(existing);
  const normalizedNext = normalizeThreadItemSnapshot(next);
  if (normalizedExisting !== existing || normalizedNext !== next) {
    return mergeThreadItem(normalizedExisting, normalizedNext);
  }

  if (existing.type === "agentMessage" && next.type === "agentMessage") {
    return {
      ...existing,
      ...next,
      text: preferMoreCompleteText(existing.text, next.text),
    };
  }

  return next;
}

function normalizeThreadItemSnapshot(item) {
  if (item.type !== "agentMessage") {
    return item;
  }

  const trigger = parseEventDrivenToolTrigger(item.text);
  if (!trigger) {
    return item;
  }

  const normalized = {
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

function parseEventDrivenToolTrigger(text) {
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
      return parsed;
    }
  } catch {
    return null;
  }
  return null;
}

function findMatchingThreadItemIndex(items, nextItem) {
  const idIndex = items.findIndex((item) => item.id === nextItem.id);
  if (idIndex !== -1) {
    return idIndex;
  }

  return items.findIndex((item) => canMergeSameTurnThreadItems(item, nextItem));
}

function canMergeSameTurnThreadItems(existing, next) {
  if (
    existing.type !== next.type ||
    !canMatchThreadItemSemantically(existing)
  ) {
    return false;
  }

  if (existing.type === "agentMessage" && next.type === "agentMessage") {
    return haveCompatibleAgentMessageContent(existing, next);
  }

  return getThreadItemSemanticKey(existing) === getThreadItemSemanticKey(next);
}

function buildTurnItemIndex(entries) {
  const ids = new Set();
  const semantic = new Map();
  const agentMessages = [];

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

function createTurnItemMatcher(index) {
  return {
    index,
    consumedSemantic: new Map(),
  };
}

function consumeMatchingTurnItem(matcher, turn, item) {
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
  const consumed = matcher.consumedSemantic.get(key) ?? new Set();
  for (const [index, candidate] of matchingTurns.entries()) {
    if (!consumed.has(index) && haveCompatibleTurnTimes(candidate, turn)) {
      consumed.add(index);
      matcher.consumedSemantic.set(key, consumed);
      return true;
    }
  }
  return false;
}

function haveCompatibleTurnTimes(left, right) {
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

function hasNoTurnTimes(turn) {
  return (
    turn.startedAt === null &&
    turn.completedAt === null &&
    turn.durationMs === null
  );
}

function canMatchThreadItemSemantically(item) {
  switch (item.type) {
    case "agentMessage":
    case "collabAgentMessage":
    case "eventDrivenTool":
    case "eventDrivenToolCall":
      return true;
    case "collabAgentStatusUpdate":
      return !isTerminalCollabAgentStatus(item.status?.status);
    default:
      return false;
  }
}

function isTerminalCollabAgentStatus(status) {
  return (
    status === "completed" ||
    status === "errored" ||
    status === "shutdown" ||
    status === "notFound"
  );
}

function getThreadItemSemanticKey(item) {
  const {
    id: _id,
    startedAtMs: _startedAtMs,
    completedAtMs: _completedAtMs,
    ...content
  } = item;
  return `${item.type}:${stableStringify(content)}`;
}

function stableStringify(value) {
  if (value === undefined) {
    return "undefined";
  }
  if (value === null || typeof value !== "object") {
    return JSON.stringify(value) ?? "undefined";
  }
  if (Array.isArray(value)) {
    return `[${value.map(stableStringify).join(",")}]`;
  }

  return `{${Object.keys(value)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
    .join(",")}}`;
}

function isTurnInFlight(turn) {
  return turn.status === "running" || turn.status === "inProgress";
}

function preferMoreCompleteText(existing, next) {
  if (existing === next) {
    return next;
  }
  if (existing.startsWith(next)) {
    return existing;
  }
  return next;
}

module.exports = {
  mergeThreadSnapshots,
  normalizeThreadSnapshot,
};
