import type { ConversationCell, ConversationEntry } from "../types";

const RESTART_RECOVERY_NOTICE_ID_PREFIX = "runtime-restart-recovery:";
const RUNTIME_RESTART_OCCURRENCE_ID_PREFIX = "runtime-restart:";
const RUNTIME_RESTART_REQUEST_ID_PATTERN = /\bcall_[A-Za-z0-9]+\b/;
const RUNTIME_RESTART_MARKER_PATTERN =
  /恢复标识\s*[:：]\s*runtime-restart(?:-recovery)?:([A-Za-z0-9_,.-]+)/;

export function filterConversationCellsForDisplay(
  cells: ConversationCell[],
): ConversationCell[] {
  const topLevelRecoveryFacts = collectTopLevelRecoveryFactKeys(cells);
  const topLevelUserRecoveryFacts = collectTopLevelUserRecoveryFactKeys(cells);
  const nestedInitContextFacts = collectNestedInitContextFactKeys(cells);
  return filterConversationCellsForDisplayWithState(
    cells,
    {
      topLevelSeenRecoveryFacts: new Set(),
      nestedSeenRecoveryFacts: new Set(topLevelRecoveryFacts),
      topLevelUserRecoveryFacts,
      nestedInitContextFacts,
    },
    true,
  );
}

type RecoveryFactFilterState = {
  topLevelSeenRecoveryFacts: Set<string>;
  nestedSeenRecoveryFacts: Set<string>;
  topLevelUserRecoveryFacts: Set<string>;
  nestedInitContextFacts: Set<string>;
};

function filterConversationCellsForDisplayWithState(
  cells: ConversationCell[],
  state: RecoveryFactFilterState,
  isTopLevel: boolean,
): ConversationCell[] {
  const displayCells: ConversationCell[] = [];
  for (const cell of cells) {
    const entries: ConversationEntry[] = [];
    for (const entry of cell.entries) {
      if (
        shouldHideDuplicateTopLevelInitContext(entry, isTopLevel, state) ||
        (isTopLevel &&
          shouldPreferTopLevelUserRecoveryFact(entry, state)) ||
        !consumeRecoveryFactIfDuplicate(
          entry,
          isTopLevel
            ? state.topLevelSeenRecoveryFacts
            : state.nestedSeenRecoveryFacts,
        ) ||
        entry.toolCategory === "commandNotification"
      ) {
        continue;
      }
      entries.push(filterConversationEntryForDisplay(entry, state));
    }
    if (entries.length > 0) {
      displayCells.push({ ...cell, entries });
    }
  }
  return displayCells;
}

function shouldHideDuplicateTopLevelInitContext(
  entry: ConversationEntry,
  isTopLevel: boolean,
  state: RecoveryFactFilterState,
) {
  if (!isTopLevel) {
    return false;
  }
  const key = initContextFactKey(entry);
  return key !== null && state.nestedInitContextFacts.has(key);
}

function filterConversationEntryForDisplay(
  entry: ConversationEntry,
  state: RecoveryFactFilterState,
): ConversationEntry {
  return {
    ...entry,
    ...(entry.replacementHistoryCells
      ? {
          replacementHistoryCells: filterConversationCellsForDisplayWithState(
            entry.replacementHistoryCells,
            state,
            false,
          ),
        }
      : {}),
    ...(entry.archivedCells
      ? {
          archivedCells: filterConversationCellsForDisplayWithState(
            entry.archivedCells,
            state,
            false,
          ),
        }
      : {}),
  };
}

function collectTopLevelRecoveryFactKeys(cells: ConversationCell[]) {
  const keys = new Set<string>();
  for (const cell of cells) {
    for (const entry of cell.entries) {
      const key = restartRecoveryFactKey(entry);
      if (key) {
        keys.add(key);
      }
    }
  }
  return keys;
}

function collectTopLevelUserRecoveryFactKeys(cells: ConversationCell[]) {
  const keys = new Set<string>();
  for (const cell of cells) {
    for (const entry of cell.entries) {
      if (entry.kind !== "message" || entry.role !== "user") {
        continue;
      }
      const key = restartRecoveryFactKey(entry);
      if (key) {
        keys.add(key);
      }
    }
  }
  return keys;
}

function collectNestedInitContextFactKeys(cells: ConversationCell[]) {
  const keys = new Set<string>();
  const collect = (nestedCells: ConversationCell[] | null | undefined) => {
    for (const cell of nestedCells ?? []) {
      for (const entry of cell.entries) {
        const key = initContextFactKey(entry);
        if (key) {
          keys.add(key);
        }
        collect(entry.replacementHistoryCells);
        collect(entry.archivedCells);
      }
    }
  };
  for (const cell of cells) {
    for (const entry of cell.entries) {
      collect(entry.replacementHistoryCells);
      collect(entry.archivedCells);
    }
  }
  return keys;
}

function shouldPreferTopLevelUserRecoveryFact(
  entry: ConversationEntry,
  state: RecoveryFactFilterState,
) {
  if (entry.kind === "message" && entry.role === "user") {
    return false;
  }
  const factKey = restartRecoveryFactKey(entry);
  return factKey ? state.topLevelUserRecoveryFacts.has(factKey) : false;
}

function consumeRecoveryFactIfDuplicate(
  entry: ConversationEntry,
  seenRecoveryFacts: Set<string>,
) {
  const factKey = restartRecoveryFactKey(entry);
  if (!factKey) {
    return true;
  }
  if (seenRecoveryFacts.has(factKey)) {
    return false;
  }
  seenRecoveryFacts.add(factKey);
  return true;
}

function restartRecoveryFactKey(entry: ConversationEntry) {
  const idFactKey = restartRecoveryIdFactKey(entry.id);
  if (idFactKey && isRestartRecoveryEntry(entry)) {
    return idFactKey;
  }
  if (entry.kind !== "message" || entry.role !== "user") {
    return null;
  }
  const markerFactKey = restartRecoveryMarkerFactKey(entry.text);
  if (markerFactKey) {
    return markerFactKey;
  }
  if (!entry.text.includes("Runtime Capsule")) {
    return null;
  }
  const requestId = entry.text.match(RUNTIME_RESTART_REQUEST_ID_PATTERN)?.[0];
  return requestId ? `${RESTART_RECOVERY_NOTICE_ID_PREFIX}${requestId}` : null;
}

function isRestartRecoveryEntry(entry: ConversationEntry) {
  if (entry.kind === "message" && entry.role === "user") {
    return true;
  }
  return (
    entry.kind === "event" &&
    entry.role === "system" &&
    entry.text.startsWith("Runtime Capsule recovery:")
  );
}

function restartRecoveryIdFactKey(id: string) {
  if (id.startsWith(RESTART_RECOVERY_NOTICE_ID_PREFIX)) {
    return `${RESTART_RECOVERY_NOTICE_ID_PREFIX}${id.slice(
      RESTART_RECOVERY_NOTICE_ID_PREFIX.length,
    )}`;
  }
  if (id.startsWith(RUNTIME_RESTART_OCCURRENCE_ID_PREFIX)) {
    return `${RESTART_RECOVERY_NOTICE_ID_PREFIX}${id.slice(
      RUNTIME_RESTART_OCCURRENCE_ID_PREFIX.length,
    )}`;
  }
  return null;
}

function restartRecoveryMarkerFactKey(text: string) {
  const requestId = text.match(RUNTIME_RESTART_MARKER_PATTERN)?.[1];
  return requestId ? `${RESTART_RECOVERY_NOTICE_ID_PREFIX}${requestId}` : null;
}

function initContextFactKey(entry: ConversationEntry) {
  if (
    entry.kind !== "tool" ||
    entry.toolCategory !== "context" ||
    !entry.toolName?.startsWith("Init Context")
  ) {
    return null;
  }
  return JSON.stringify({
    toolName: entry.toolName,
    text: entry.text,
    details: entry.toolDetails ?? "",
  });
}
