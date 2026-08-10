import type { ConversationCell, ConversationEntry } from "../types";

export type ConversationSearchResult = {
  id: string;
  cellId: string;
  entryId: string;
  source: string;
  sourceLabel: string;
  matchIndex: number;
  start: number;
  end: number;
  preview: string;
};

type SearchTextSource = {
  key: string;
  label: string;
  text: string;
};

export function buildConversationSearchResults(
  cells: ConversationCell[],
  query: string,
): ConversationSearchResult[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  if (!normalizedQuery) {
    return [];
  }

  const results: ConversationSearchResult[] = [];

  for (const cell of cells) {
    for (const entry of cell.entries) {
      const sources = collectEntrySearchSources(entry);
      for (const source of sources) {
        const normalizedText = source.text.toLocaleLowerCase();
        let fromIndex = 0;
        let matchIndex = normalizedText.indexOf(normalizedQuery, fromIndex);
        while (matchIndex !== -1) {
          const end = matchIndex + normalizedQuery.length;
          results.push({
            id: `${cell.id}:${entry.id}:${source.key}:${matchIndex}`,
            cellId: cell.id,
            entryId: entry.id,
            source: source.key,
            sourceLabel: source.label,
            matchIndex,
            start: matchIndex,
            end,
            preview: buildSearchPreview(source.text, matchIndex, end),
          });
          fromIndex = Math.max(end, matchIndex + 1);
          matchIndex = normalizedText.indexOf(normalizedQuery, fromIndex);
        }
      }
    }
  }

  return results;
}

export function getNextConversationSearchIndex(
  currentIndex: number,
  resultCount: number,
  direction: -1 | 1,
) {
  if (resultCount <= 0) {
    return 0;
  }
  return (
    (((currentIndex + direction) % resultCount) + resultCount) % resultCount
  );
}

function collectEntrySearchSources(
  entry: ConversationEntry,
): SearchTextSource[] {
  const sources: SearchTextSource[] = [];
  pushTextSource(sources, "text", "Text", entry.text);
  pushTextSource(sources, "toolName", "Tool", entry.toolName);
  pushTextSource(sources, "toolStatus", "Status", entry.toolStatus);
  pushTextSource(sources, "toolDetails", "Details", entry.toolDetails);
  pushTextSource(sources, "toolOutput", "Output", entry.toolOutput?.text);

  entry.attachments.forEach((attachment, index) => {
    const prefix = `attachment-${index}`;
    pushTextSource(sources, `${prefix}-label`, "Attachment", attachment.label);
    pushTextSource(sources, `${prefix}-url`, "Attachment URL", attachment.url);
    pushTextSource(
      sources,
      `${prefix}-path`,
      "Attachment Path",
      attachment.path,
    );
  });

  entry.replacementHistoryEntries?.forEach((replacementEntry, index) => {
    collectEntrySearchSources(replacementEntry).forEach((source) => {
      sources.push({
        ...source,
        key: `replacement-${index}-${source.key}`,
        label: `Replacement ${source.label}`,
      });
    });
  });

  entry.archivedCells?.forEach((cell, cellIndex) => {
    cell.entries.forEach((archivedEntry, entryIndex) => {
      collectEntrySearchSources(archivedEntry).forEach((source) => {
        sources.push({
          ...source,
          key: `archive-${cellIndex}-${entryIndex}-${source.key}`,
          label: `Archive ${source.label}`,
        });
      });
    });
  });

  return sources;
}

function pushTextSource(
  sources: SearchTextSource[],
  key: string,
  label: string,
  value: string | null | undefined,
) {
  const text = value?.trim();
  if (!text) {
    return;
  }
  sources.push({ key, label, text });
}

function buildSearchPreview(text: string, start: number, end: number) {
  const previewStart = Math.max(0, start - 36);
  const previewEnd = Math.min(text.length, end + 36);
  const prefix = previewStart > 0 ? "..." : "";
  const suffix = previewEnd < text.length ? "..." : "";
  return `${prefix}${text.slice(previewStart, previewEnd)}${suffix}`;
}
