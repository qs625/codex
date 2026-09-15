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

function userEntry(id: string, text: string): ConversationEntry {
  return {
    id,
    kind: "message",
    author: "You",
    role: "user",
    text,
    timestamp: "now",
    attachments: [],
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

test("filters duplicate restart recovery user rows across compact details", () => {
  const prompt =
    "Morpheus 已恢复预期的 Runtime Capsule 重启请求 call_LJpiSNwSkFu41mOsORr1Lv3S；该请求已完成。";
  const cells = [
    {
      id: "current-recovery",
      kind: "message",
      entries: [userEntry("item-34", prompt)],
    },
    {
      id: "compact",
      kind: "compact",
      entries: [
        {
          ...entry("compact-entry"),
          archivedCells: [
            {
              id: "archived-recovery",
              kind: "message",
              entries: [
                userEntry(
                  "runtime-restart-recovery:call_LJpiSNwSkFu41mOsORr1Lv3S",
                  prompt,
                ),
              ],
            },
          ],
          replacementHistoryCells: [
            {
              id: "replacement-recovery",
              kind: "message",
              entries: [userEntry("replacement-user", prompt)],
            },
          ],
        },
      ],
    },
  ] satisfies ConversationCell[];

  const result = filterConversationCellsForDisplay(cells);

  assert.deepEqual(
    result.flatMap((cell) =>
      cell.entries.flatMap((displayEntry) => [
        displayEntry.id,
        ...(displayEntry.archivedCells ?? []).flatMap((nestedCell) =>
          nestedCell.entries.map((entry) => entry.id),
        ),
        ...(displayEntry.replacementHistoryCells ?? []).flatMap((nestedCell) =>
          nestedCell.entries.map((entry) => entry.id),
        ),
      ]),
    ),
    ["item-34", "compact-entry"],
  );
});

test("keeps top-level restart recovery row when compact details appear first", () => {
  const prompt =
    "Morpheus 已恢复预期的 Runtime Capsule 重启请求 call_LJpiSNwSkFu41mOsORr1Lv3S；该请求已完成。";
  const cells = [
    {
      id: "compact",
      kind: "compact",
      entries: [
        {
          ...entry("compact-entry"),
          archivedCells: [
            {
              id: "archived-recovery",
              kind: "message",
              entries: [
                userEntry(
                  "runtime-restart-recovery:call_LJpiSNwSkFu41mOsORr1Lv3S",
                  prompt,
                ),
              ],
            },
          ],
        },
      ],
    },
    {
      id: "current-recovery",
      kind: "message",
      entries: [userEntry("item-34", prompt)],
    },
  ] satisfies ConversationCell[];

  const result = filterConversationCellsForDisplay(cells);

  assert.deepEqual(
    result.flatMap((cell) =>
      cell.entries.flatMap((displayEntry) => [
        displayEntry.id,
        ...(displayEntry.archivedCells ?? []).flatMap((nestedCell) =>
          nestedCell.entries.map((entry) => entry.id),
        ),
      ]),
    ),
    ["compact-entry", "item-34"],
  );
});

test("keeps independent same-text non-recovery user rows", () => {
  const cells = [
    {
      id: "first-user",
      kind: "message",
      entries: [userEntry("user-1", "repeat")],
    },
    {
      id: "second-user",
      kind: "message",
      entries: [userEntry("user-2", "repeat")],
    },
  ] satisfies ConversationCell[];

  assert.deepEqual(
    filterConversationCellsForDisplay(cells).flatMap((cell) =>
      cell.entries.map((entry) => entry.id),
    ),
    ["user-1", "user-2"],
  );
});

test("keeps distinct restart recovery request ids", () => {
  const cells = [
    {
      id: "first-recovery",
      kind: "message",
      entries: [
        userEntry(
          "item-1",
          "Morpheus 已恢复预期的 Runtime Capsule 重启请求 call_first；该请求已完成。",
        ),
      ],
    },
    {
      id: "second-recovery",
      kind: "message",
      entries: [
        userEntry(
          "item-2",
          "Morpheus 已恢复预期的 Runtime Capsule 重启请求 call_second；该请求已完成。",
        ),
      ],
    },
  ] satisfies ConversationCell[];

  assert.deepEqual(
    filterConversationCellsForDisplay(cells).flatMap((cell) =>
      cell.entries.map((entry) => entry.id),
    ),
    ["item-1", "item-2"],
  );
});
