import assert from "node:assert/strict";
import test from "node:test";

import { filterConversationCellsForDisplay } from "./conversationPresentation";
import type { ConversationCell, ConversationEntry } from "../types";

function entry(
  id: string,
  toolCategory?: "command" | "commandNotification",
): ConversationEntry {
  return {
    id,
    kind: "tool" as const,
    author: "Morpheus",
    role: "agent",
    text: id,
    timestamp: "now",
    attachments: [],
    artifact: null,
    toolCategory,
  };
}

test("keeps command entries visible while filtering command notifications", () => {
  const cells = [
    {
      id: "mixed",
      kind: "tool",
      entries: [entry("message"), entry("command", "command")],
    },
    {
      id: "notification",
      kind: "tool",
      entries: [entry("notification", "commandNotification")],
    },
  ] satisfies ConversationCell[];

  assert.deepEqual(filterConversationCellsForDisplay(cells), [
    {
      id: "mixed",
      kind: "tool",
      entries: [entry("message"), entry("command", "command")],
    },
  ]);
  assert.equal(cells[0].entries.length, 2);
});

test("keeps command entries inside compact and archived display cells", () => {
  const cells = [
    {
      id: "compact",
      kind: "compact",
      entries: [
        {
          ...entry("compact-entry"),
          replacementHistoryCells: [
            {
              id: "replacement",
              kind: "tool",
              entries: [entry("command", "command"), entry("message")],
            },
          ],
          archivedCells: [
            {
              id: "archive",
              kind: "tool",
              entries: [entry("notification", "commandNotification")],
            },
          ],
        },
      ],
    },
  ] satisfies ConversationCell[];

  const result = filterConversationCellsForDisplay(cells);
  assert.deepEqual(result[0]?.entries[0]?.replacementHistoryCells?.[0]?.entries, [
    entry("command", "command"),
    entry("message"),
  ]);
  assert.deepEqual(result[0]?.entries[0]?.archivedCells, []);
});
