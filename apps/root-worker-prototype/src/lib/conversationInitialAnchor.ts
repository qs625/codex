import type { ConversationCell, ConversationEntry } from "../types";

export type ConversationInitialAnchor = {
  itemId: string;
  reason: "compactSummary";
};

export type ConversationInitialAnchorLoad = {
  threadId: string;
  requestId: number;
};

export function resolveConversationInitialAnchorForLoad(
  cells: readonly ConversationCell[],
  options: {
    selectedThreadId: string | null;
    isLoadingThread: boolean;
    load: ConversationInitialAnchorLoad | null;
  },
): ConversationInitialAnchor | null {
  if (
    !options.selectedThreadId ||
    options.isLoadingThread ||
    options.load?.threadId !== options.selectedThreadId
  ) {
    return null;
  }
  return resolveConversationInitialAnchor(cells);
}

export function resolveConversationInitialAnchor(
  cells: readonly ConversationCell[],
): ConversationInitialAnchor | null {
  const firstVisible = cells[0];
  if (!firstVisible) {
    return null;
  }

  const firstCompact = firstVisible.kind === "compact" ? firstVisible : null;
  if (firstCompact) {
    const compactEntry = firstCompact.entries[0];
    if (!compactEntry) {
      return null;
    }
    const summaryEntry = cells[1]?.entries.find((entry) =>
      isCompactSummaryEntry(entry, compactEntry.id),
    );
    return summaryEntry
      ? { itemId: summaryEntry.id, reason: "compactSummary" }
      : { itemId: compactEntry.id, reason: "compactSummary" };
  }

  const firstSummaryEntry = firstVisible.entries.find((entry) =>
    isPotentialCompactSummaryEntry(entry),
  );
  if (
    firstSummaryEntry &&
    cells[1]?.kind === "compact" &&
    cells[1].entries.some((entry) =>
      isCompactSummaryEntry(firstSummaryEntry, entry.id),
    )
  ) {
    return { itemId: firstSummaryEntry.id, reason: "compactSummary" };
  }

  return null;
}

function isCompactSummaryEntry(
  entry: ConversationEntry,
  compactEntryId: string,
) {
  return (
    entry.kind === "message" &&
    entry.role === "agent" &&
    (entry.id === `${compactEntryId}:summary` || entry.id === "compact-summary")
  );
}

function isPotentialCompactSummaryEntry(entry: ConversationEntry) {
  return (
    entry.kind === "message" &&
    entry.role === "agent" &&
    (entry.id === "compact-summary" || entry.id.endsWith(":summary"))
  );
}
