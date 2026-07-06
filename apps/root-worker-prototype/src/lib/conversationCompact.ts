import type { ConversationCell, ConversationEntry } from "../types";

export type LoadedCompactConversationDetails = {
  archivedCells: ConversationCell[];
  archivedEntryCount: number;
  replacementHistoryEntries: ConversationEntry[];
  replacementHistoryCells: ConversationCell[];
};

export type ConversationCellBuildOptions = {
  compactDetailsById?: Readonly<
    Record<string, LoadedCompactConversationDetails | undefined>
  >;
};

export function buildConversationCells(
  entries: ConversationEntry[],
  previousCells?: ConversationCell[] | null,
  options?: ConversationCellBuildOptions,
): ConversationCell[] {
  const cells = buildConversationCellsForSegment(entries, options);
  return reuseConversationCells(cells, previousCells);
}

export function extractCompactConversationDetails(
  entries: ConversationEntry[],
  compactEntryId: string,
): LoadedCompactConversationDetails | null {
  let archivedCells: ConversationCell[] = [];
  let segmentEntries: ConversationEntry[] = [];

  const flushSegment = () => {
    if (segmentEntries.length === 0) {
      return;
    }
    archivedCells.push(...buildConversationCellsForSegment(segmentEntries));
    segmentEntries = [];
  };

  for (const entry of entries) {
    if (entry.kind !== "compact") {
      segmentEntries.push(entry);
      continue;
    }

    flushSegment();

    if (entry.id === compactEntryId) {
      const replacementHistoryEntries = entry.replacementHistoryEntries ?? [];
      return {
        archivedCells,
        archivedEntryCount: countConversationEntries(archivedCells),
        replacementHistoryEntries,
        replacementHistoryCells:
          buildConversationCellsForSegment(replacementHistoryEntries),
      };
    }

    const replacementHistoryEntries = entry.replacementHistoryEntries ?? [];
    archivedCells = [
      {
        id: entry.id,
        kind: "compact",
        entries: [
          {
            ...entry,
            archivedCells,
            archivedEntryCount: countConversationEntries(archivedCells),
            replacementHistoryEntries,
            replacementHistoryCells:
              buildConversationCellsForSegment(replacementHistoryEntries),
          },
        ],
      },
    ];
  }

  return null;
}

function buildConversationCellsForSegment(
  entries: ConversationEntry[],
  options?: ConversationCellBuildOptions,
): ConversationCell[] {
  const cells: ConversationCell[] = [];
  let entryIndex = 0;

  while (entryIndex < entries.length) {
    const entry = entries[entryIndex];
    if (!entry) {
      entryIndex += 1;
      continue;
    }

    if (entry.kind === "compact") {
      const loadedDetails = options?.compactDetailsById?.[entry.id];
      cells.push({
        id: entry.id,
        kind: "compact",
        entries: [
          loadedDetails
            ? {
                ...entry,
                archivedCells: loadedDetails.archivedCells,
                archivedEntryCount: loadedDetails.archivedEntryCount,
                replacementHistoryEntries:
                  loadedDetails.replacementHistoryEntries,
                replacementHistoryCells: loadedDetails.replacementHistoryCells,
              }
            : entry,
        ],
      });
      entryIndex += 1;
      continue;
    }

    const nextCellEntries = [entry];
    while (
      entryIndex + nextCellEntries.length < entries.length &&
      shouldMergeConversationEntry(
        {
          id: nextCellEntries[0]?.id ?? entry.id,
          kind: nextCellEntries[0]?.kind ?? entry.kind,
          entries: nextCellEntries,
        },
        entries[entryIndex + nextCellEntries.length]!,
      )
    ) {
      nextCellEntries.push(entries[entryIndex + nextCellEntries.length]!);
    }

    cells.push({
      id: nextCellEntries[0]?.id ?? entry.id,
      kind: nextCellEntries[0]?.kind ?? entry.kind,
      entries: nextCellEntries,
    });

    entryIndex += nextCellEntries.length;
  }

  return cells;
}

function countConversationEntries(cells: ConversationCell[]) {
  return cells.reduce(
    (count, cell) =>
      count +
      cell.entries.reduce(
        (entryCount, entry) => {
          if (entry.kind === "archive") {
            return entryCount + (entry.archivedEntryCount ?? 0);
          }
          if (entry.kind === "compact") {
            return (
              entryCount +
              (entry.archivedEntryCount ?? 0) +
              1 +
              countConversationEntries(entry.replacementHistoryCells ?? [])
            );
          }
          return entryCount + 1;
        },
        0,
      ),
    0,
  );
}

function reuseConversationCells(
  cells: ConversationCell[],
  previousCells?: ConversationCell[] | null,
): ConversationCell[] {
  if (!previousCells) {
    return cells;
  }

  const previousCellsByKey = new Map(
    previousCells.map((cell) => [conversationCellReuseKey(cell), cell]),
  );

  return cells.map((cell) => {
    const existingCell = previousCellsByKey.get(conversationCellReuseKey(cell));
    if (
      existingCell &&
      existingCell.entries.length === cell.entries.length &&
      existingCell.entries.every(
        (entry, entryIndex) => entry === cell.entries[entryIndex],
      )
    ) {
      return existingCell;
    }
    return cell;
  });
}

function conversationCellReuseKey(cell: ConversationCell) {
  return `${cell.kind}:${cell.id}`;
}

function shouldMergeConversationEntry(
  cell: ConversationCell,
  nextEntry: ConversationEntry,
) {
  const previousEntry = cell.entries.at(-1);
  if (!previousEntry) {
    return false;
  }

  if (nextEntry.kind === "compact") {
    return false;
  }

  if (cell.kind === "tool" && nextEntry.kind === "tool") {
    if (isStandaloneNotificationEntry(previousEntry) || isStandaloneNotificationEntry(nextEntry)) {
      return false;
    }
    if (previousEntry.isReplacementHistory !== nextEntry.isReplacementHistory) {
      return false;
    }
    return previousEntry.toolCategory === nextEntry.toolCategory;
  }

  if (
    cell.kind === "message" &&
    nextEntry.kind === "message" &&
    previousEntry.role === "agent" &&
    nextEntry.role === "agent"
  ) {
    if (previousEntry.isReplacementHistory !== nextEntry.isReplacementHistory) {
      return false;
    }
    return true;
  }

  return false;
}

function isStandaloneNotificationEntry(entry: ConversationEntry) {
  return (
    entry.toolCategory === "childCompletion" ||
    entry.toolCategory === "subagentNotification"
  );
}
