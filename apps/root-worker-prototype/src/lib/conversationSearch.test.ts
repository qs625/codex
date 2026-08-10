import test from "node:test";
import assert from "node:assert/strict";

import {
  buildConversationSearchResults,
  getNextConversationSearchIndex,
} from "./conversationSearch";
import type { ConversationCell, ConversationEntry } from "../types";

function makeEntry(
  id: string,
  text: string,
  fields: Partial<ConversationEntry> = {},
): ConversationEntry {
  return {
    id,
    kind: "message",
    author: "Codex",
    role: "agent",
    text,
    timestamp: "09:41",
    attachments: [],
    ...fields,
  };
}

function makeCell(
  id: string,
  entries: ConversationEntry[],
  kind: ConversationCell["kind"] = "message",
): ConversationCell {
  return { id, kind, entries };
}

test("returns no conversation search results for blank queries", () => {
  const cells = [makeCell("cell-1", [makeEntry("entry-1", "hello world")])];

  assert.deepEqual(buildConversationSearchResults(cells, "  "), []);
});

test("matches conversation text case-insensitively in display order", () => {
  const cells = [
    makeCell("cell-1", [makeEntry("entry-1", "Alpha beta alpha")]),
    makeCell("cell-2", [makeEntry("entry-2", "Beta")]),
  ];

  const results = buildConversationSearchResults(cells, "ALPHA");

  assert.deepEqual(
    results.map((result) => ({
      cellId: result.cellId,
      entryId: result.entryId,
      source: result.source,
      start: result.start,
    })),
    [
      { cellId: "cell-1", entryId: "entry-1", source: "text", start: 0 },
      { cellId: "cell-1", entryId: "entry-1", source: "text", start: 11 },
    ],
  );
});

test("searches tool details and attachment text", () => {
  const cells = [
    makeCell(
      "tool-cell",
      [
        makeEntry("tool-1", "summary", {
          kind: "tool",
          toolName: "exec_command",
          toolStatus: "completed",
          toolDetails: "Notify On\noutput",
          attachments: [
            {
              kind: "file",
              label: "build-log.txt",
              path: "/tmp/build-log.txt",
            },
          ],
        }),
      ],
      "tool",
    ),
  ];

  assert.deepEqual(
    buildConversationSearchResults(cells, "notify").map(
      (result) => result.source,
    ),
    ["toolDetails"],
  );
  assert.deepEqual(
    buildConversationSearchResults(cells, "build-log").map(
      (result) => result.source,
    ),
    ["attachment-0-label", "attachment-0-path"],
  );
});

test("searches collapsed tool output text", () => {
  const cells = [
    makeCell(
      "tool-cell",
      [
        makeEntry("tool-1", "summary", {
          kind: "tool",
          toolName: "Command notification",
          toolStatus: "failed",
          toolOutput: {
            label: "Output",
            text: "unexpected failure line",
          },
        }),
      ],
      "tool",
    ),
  ];

  assert.deepEqual(
    buildConversationSearchResults(cells, "failure").map((result) => ({
      source: result.source,
      sourceLabel: result.sourceLabel,
    })),
    [{ source: "toolOutput", sourceLabel: "Output" }],
  );
});

test("keeps same text in different thread item entries as separate results", () => {
  const cells = [
    makeCell("cell-1", [makeEntry("entry-1", "repeat")]),
    makeCell("cell-2", [makeEntry("entry-2", "repeat")]),
  ];

  const results = buildConversationSearchResults(cells, "repeat");

  assert.deepEqual(
    results.map((result) => [result.cellId, result.entryId]),
    [
      ["cell-1", "entry-1"],
      ["cell-2", "entry-2"],
    ],
  );
});

test("searches replacement history and archived cell text", () => {
  const replacementEntry = makeEntry("replacement-1", "replacement needle");
  const archivedEntry = makeEntry("archived-1", "archived needle");
  const cells = [
    makeCell(
      "compact-1",
      [
        makeEntry("compact-entry", "current", {
          kind: "compact",
          replacementHistoryEntries: [replacementEntry],
          archivedCells: [makeCell("archived-cell", [archivedEntry])],
        }),
      ],
      "compact",
    ),
  ];

  assert.deepEqual(
    buildConversationSearchResults(cells, "needle").map((result) => ({
      entryId: result.entryId,
      source: result.source,
      sourceLabel: result.sourceLabel,
    })),
    [
      {
        entryId: "compact-entry",
        source: "replacement-0-text",
        sourceLabel: "Replacement Text",
      },
      {
        entryId: "compact-entry",
        source: "archive-0-0-text",
        sourceLabel: "Archive Text",
      },
    ],
  );
});

test("wraps conversation search navigation indexes", () => {
  assert.equal(getNextConversationSearchIndex(0, 3, 1), 1);
  assert.equal(getNextConversationSearchIndex(2, 3, 1), 0);
  assert.equal(getNextConversationSearchIndex(0, 3, -1), 2);
  assert.equal(getNextConversationSearchIndex(1, 0, 1), 0);
});
