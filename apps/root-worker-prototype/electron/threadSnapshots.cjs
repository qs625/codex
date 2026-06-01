function mergeThreadSnapshots(existing, next) {
  if (!existing) {
    return next;
  }

  const turns = mergeTurns(existing.turns ?? [], next.turns ?? []);
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

  return {
    ...existing,
    ...next,
    threadUsage: {
      tokenUsage: next.threadUsage?.tokenUsage ?? tokenUsage,
      contextUsage: next.threadUsage?.contextUsage ?? contextUsage,
    },
    tokenUsage,
    contextUsage,
    turns,
  };
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
    if (!nextTurnIds.has(turn.id) && isTurnInFlight(turn)) {
      const items = (turn.items ?? []).filter(
        (item) => !consumeMatchingTurnItem(nextItemsMatcher, turn, item),
      );
      if (items.length > 0) {
        turns.push(
          items.length === (turn.items ?? []).length
            ? turn
            : { ...turn, items },
        );
      }
    }
  }

  return turns;
}

function mergeTurn(existing, next) {
  const existingItemsById = new Map(
    (existing.items ?? []).map((item) => [item.id, item]),
  );
  const mergedItems = (next.items ?? []).map((item) => {
    const existingItem = existingItemsById.get(item.id);
    return existingItem ? mergeThreadItem(existingItem, item) : item;
  });

  if (isTurnInFlight(existing) || isTurnInFlight(next)) {
    const mergedItemsMatcher = createTurnItemMatcher(
      buildTurnItemIndex([{ turn: next, items: mergedItems }]),
    );
    for (const item of existing.items ?? []) {
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
  if (existing.type === "agentMessage" && next.type === "agentMessage") {
    return {
      ...existing,
      ...next,
      text: preferMoreCompleteText(existing.text, next.text),
    };
  }

  return next;
}

function buildTurnItemIndex(entries) {
  const ids = new Set();
  const semantic = new Map();

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

function getThreadItemSemanticKey(item) {
  const { id: _id, ...content } = item;
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
};
