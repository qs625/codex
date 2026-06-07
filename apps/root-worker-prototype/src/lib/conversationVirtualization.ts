import type { ConversationCell, ConversationEntry } from "../types";

export const CONVERSATION_ROW_GAP = 18;
export const CONVERSATION_OVERSCAN_PX = 640;

const DEFAULT_MESSAGE_ROW_HEIGHT = 104;
const DEFAULT_EVENT_ROW_HEIGHT = 72;
const DEFAULT_TOOL_ROW_HEIGHT = 112;
const DEFAULT_COMPACT_ROW_HEIGHT = 156;
const DEFAULT_ARCHIVE_ROW_HEIGHT = 112;
const TEXT_LINE_HEIGHT = 22;
const TEXT_CHARS_PER_LINE = 72;
const TOOL_CHARS_PER_LINE = 84;
const ATTACHMENT_BLOCK_HEIGHT = 176;

export type ConversationVirtualLayout = {
  heights: number[];
  offsets: number[];
  totalHeight: number;
};

export function estimateConversationCellHeight(cell: ConversationCell): number {
  if (cell.kind === "event") {
    return DEFAULT_EVENT_ROW_HEIGHT;
  }

  if (cell.kind === "tool") {
    return estimateToolCellHeight(cell.entries);
  }

  if (cell.kind === "archive") {
    return DEFAULT_ARCHIVE_ROW_HEIGHT;
  }

  if (cell.kind === "compact") {
    return DEFAULT_COMPACT_ROW_HEIGHT;
  }

  return estimateMessageCellHeight(cell.entries);
}

export function buildConversationVirtualLayout(
  cells: ConversationCell[],
  measuredHeights: ReadonlyMap<string, number>,
): ConversationVirtualLayout {
  const heights = cells.map(
    (cell) => measuredHeights.get(cell.id) ?? estimateConversationCellHeight(cell),
  );
  const offsets = new Array<number>(cells.length);
  let cursor = 0;

  for (let index = 0; index < heights.length; index += 1) {
    offsets[index] = cursor;
    cursor += heights[index];
    if (index < heights.length - 1) {
      cursor += CONVERSATION_ROW_GAP;
    }
  }

  return {
    heights,
    offsets,
    totalHeight: cursor,
  };
}

export function findConversationWindow(
  layout: ConversationVirtualLayout,
  scrollTop: number,
  viewportHeight: number,
  overscanPx = CONVERSATION_OVERSCAN_PX,
) {
  const { offsets, heights } = layout;
  if (offsets.length === 0) {
    return {
      startIndex: 0,
      endIndex: 0,
    };
  }

  const safeViewportHeight = Math.max(0, viewportHeight);
  const startOffset = Math.max(0, scrollTop - overscanPx);
  const endOffset = scrollTop + safeViewportHeight + overscanPx;

  const startIndex = findFirstIntersectingIndex(offsets, heights, startOffset);
  const endIndex = Math.min(
    offsets.length,
    findLastIntersectingIndex(offsets, heights, endOffset) + 1,
  );

  return { startIndex, endIndex };
}

function estimateMessageCellHeight(entries: ConversationEntry[]) {
  let height = DEFAULT_MESSAGE_ROW_HEIGHT;
  for (const entry of entries) {
    height += estimateWrappedTextHeight(entry.text, TEXT_CHARS_PER_LINE, 1);
    const attachmentCount = entry.attachments.length;
    if (attachmentCount > 0) {
      height += ATTACHMENT_BLOCK_HEIGHT;
      if (attachmentCount > 1) {
        height += (attachmentCount - 1) * 18;
      }
    }
  }
  return height;
}

function estimateToolCellHeight(entries: ConversationEntry[]) {
  let height = DEFAULT_TOOL_ROW_HEIGHT;
  for (const entry of entries) {
    height += estimateWrappedTextHeight(entry.text, TOOL_CHARS_PER_LINE, 1);
    if (entry.toolDetails) {
      height += estimateWrappedTextHeight(entry.toolDetails, TOOL_CHARS_PER_LINE, 3);
      height += 24;
    }
  }
  if (entries.length > 1) {
    height += (entries.length - 1) * 24;
  }
  return height;
}

function estimateWrappedTextHeight(
  text: string,
  charsPerLine: number,
  minLines: number,
) {
  const normalized = text.trim();
  if (normalized.length === 0) {
    return minLines * TEXT_LINE_HEIGHT;
  }

  const wrappedLines = normalized.split("\n").reduce((count, line) => {
    const lineLength = Math.max(1, line.length);
    return count + Math.ceil(lineLength / charsPerLine);
  }, 0);

  return Math.max(minLines, wrappedLines) * TEXT_LINE_HEIGHT;
}

function findFirstIntersectingIndex(
  offsets: number[],
  heights: number[],
  targetOffset: number,
) {
  let low = 0;
  let high = offsets.length - 1;
  let result = offsets.length - 1;

  while (low <= high) {
    const mid = Math.floor((low + high) / 2);
    const rowEnd = offsets[mid] + heights[mid];
    if (rowEnd >= targetOffset) {
      result = mid;
      high = mid - 1;
    } else {
      low = mid + 1;
    }
  }

  return result;
}

function findLastIntersectingIndex(
  offsets: number[],
  heights: number[],
  targetOffset: number,
) {
  let low = 0;
  let high = offsets.length - 1;
  let result = 0;

  while (low <= high) {
    const mid = Math.floor((low + high) / 2);
    if (offsets[mid] <= targetOffset) {
      result = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }

  while (
    result < offsets.length - 1 &&
    offsets[result] + heights[result] < targetOffset
  ) {
    result += 1;
  }

  return result;
}
