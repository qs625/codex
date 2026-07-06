import test from "node:test";
import assert from "node:assert/strict";

import {
  buildConversationVirtualLayout,
  CONVERSATION_ROW_GAP,
  estimateConversationCellHeight,
  findConversationWindow,
} from "./conversationVirtualization";
import type { ConversationCell } from "../types";

function makeMessageCell(
  id: string,
  text: string,
  attachmentCount = 0,
): ConversationCell {
  return {
    id,
    kind: "message",
    entries: [
      {
        id,
        kind: "message",
        author: "You",
        role: "user",
        text,
        timestamp: "09:41",
        attachments: Array.from({ length: attachmentCount }, (_, index) => ({
          kind: "file" as const,
          label: `file-${index}`,
        })),
      },
    ],
  };
}

test("uses measured heights when available and keeps row gaps in layout", () => {
  const cells = [
    makeMessageCell("first", "short"),
    makeMessageCell("second", "short"),
    makeMessageCell("third", "short"),
  ];
  const measuredHeights = new Map<string, number>([
    ["second", 240],
  ]);

  const layout = buildConversationVirtualLayout(cells, measuredHeights);

  assert.equal(layout.offsets[0], 0);
  assert.equal(
    layout.offsets[1],
    estimateConversationCellHeight(cells[0]) + CONVERSATION_ROW_GAP,
  );
  assert.equal(
    layout.offsets[2],
    layout.offsets[1] + 240 + CONVERSATION_ROW_GAP,
  );
});

test("expands message estimates when attachments are present", () => {
  const plainCell = makeMessageCell("plain", "hello");
  const attachmentCell = makeMessageCell("attachment", "hello", 2);

  assert.ok(
    estimateConversationCellHeight(attachmentCell) >
      estimateConversationCellHeight(plainCell),
  );
});

test("estimates archived history rows separately from compact rows", () => {
  const archiveCell: ConversationCell = {
    id: "archive",
    kind: "archive",
    entries: [
      {
        id: "archive",
        kind: "archive",
        author: "Root",
        role: "system",
        text: "previous conversation",
        timestamp: "09:41",
        attachments: [],
        archivedEntryCount: 2,
        archivedCells: [
          makeMessageCell("replacement-1", "one"),
          makeMessageCell("replacement-2", "two"),
        ],
      },
    ],
  };
  const compactCell: ConversationCell = {
    id: "compact",
    kind: "compact",
    entries: [
      {
        id: "compact",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "compacted",
        timestamp: "09:41",
        attachments: [],
        replacementHistoryStatus: "available",
        replacementHistoryCount: 2,
        replacementHistoryEntries: [],
      },
    ],
  };

  assert.ok(estimateConversationCellHeight(archiveCell) > 0);
  assert.ok(estimateConversationCellHeight(compactCell) > 0);
});

test("expanded compact rows account for loaded round details in height estimates", () => {
  const collapsedCompactCell: ConversationCell = {
    id: "compact-collapsed",
    kind: "compact",
    entries: [
      {
        id: "compact-collapsed",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "compacted",
        timestamp: "09:41",
        attachments: [],
        replacementHistoryStatus: "available",
        replacementHistoryCount: 1,
        replacementHistoryEntries: [],
        replacementHistoryCells: [],
      },
    ],
  };
  const expandedCompactCell: ConversationCell = {
    id: "compact-expanded",
    kind: "compact",
    entries: [
      {
        ...collapsedCompactCell.entries[0]!,
        id: "compact-expanded",
        archivedEntryCount: 1,
        archivedCells: [makeMessageCell("archived", "old request")],
        replacementHistoryCells: [makeMessageCell("replacement", "recent request")],
      },
    ],
  };

  assert.ok(
    estimateConversationCellHeight(expandedCompactCell) >
      estimateConversationCellHeight(collapsedCompactCell),
  );
});

test("finds a stable virtualized window with overscan", () => {
  const cells = Array.from({ length: 6 }, (_, index) =>
    makeMessageCell(`cell-${index}`, `row ${index}`),
  );
  const layout = buildConversationVirtualLayout(
    cells,
    new Map(
      cells.map((cell, index) => [
        cell.id,
        120 + index * 10,
      ]),
    ),
  );

  const window = findConversationWindow(layout, 170, 150, 50);

  assert.deepEqual(window, {
    startIndex: 0,
    endIndex: 3,
  });
});
