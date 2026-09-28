import test from "node:test";
import assert from "node:assert/strict";

import {
  resolveConversationInitialAnchor,
  resolveConversationInitialAnchorForLoad,
} from "./conversationInitialAnchor";
import type { ConversationCell } from "../types";

function compactCell(id = "compact-1"): ConversationCell {
  return {
    id,
    kind: "compact",
    entries: [
      {
        id,
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Context compacted",
        timestamp: "09:43",
        attachments: [],
      },
    ],
  };
}

function agentMessageCell(id: string, text = "summary"): ConversationCell {
  return {
    id,
    kind: "message",
    entries: [
      {
        id,
        kind: "message",
        author: "Codex",
        role: "agent",
        text,
        timestamp: "09:43",
        attachments: [],
      },
    ],
  };
}

function toolCell(id = "ctx-1:section:0"): ConversationCell {
  return {
    id,
    kind: "tool",
    entries: [
      {
        id,
        kind: "tool",
        author: "Root",
        role: "system",
        text: "Init Context",
        timestamp: "09:44",
        attachments: [],
        toolName: "Init Context",
        toolStatus: "completed",
        toolCategory: "context",
      },
    ],
  };
}

test("initial anchor targets compact summary after a compact boundary", () => {
  assert.deepEqual(
    resolveConversationInitialAnchor([
      compactCell("item-1"),
      agentMessageCell("item-1:summary", "compact summary"),
      toolCell(),
    ]),
    { itemId: "item-1:summary", reason: "compactSummary" },
  );
});

test("initial anchor falls back to the compact boundary when summary is absent", () => {
  assert.deepEqual(resolveConversationInitialAnchor([compactCell("item-1")]), {
    itemId: "item-1",
    reason: "compactSummary",
  });
});

test("initial anchor supports legacy summary before the compact boundary", () => {
  assert.deepEqual(
    resolveConversationInitialAnchor([
      agentMessageCell("compact-summary", "legacy compact summary"),
      compactCell("item-1"),
      toolCell(),
    ]),
    { itemId: "compact-summary", reason: "compactSummary" },
  );
});

test("initial anchor ignores ordinary conversations", () => {
  assert.equal(
    resolveConversationInitialAnchor([
      agentMessageCell("agent-1", "ordinary response"),
      toolCell(),
    ]),
    null,
  );
});

test("initial anchor only resolves for the selected read load window", () => {
  const cells = [
    compactCell("item-1"),
    agentMessageCell("item-1:summary", "compact summary"),
  ];

  assert.deepEqual(
    resolveConversationInitialAnchorForLoad(cells, {
      selectedThreadId: "thread-1",
      isLoadingThread: false,
      load: { threadId: "thread-1", requestId: 1 },
    }),
    { itemId: "item-1:summary", reason: "compactSummary" },
  );
  assert.equal(
    resolveConversationInitialAnchorForLoad(cells, {
      selectedThreadId: "thread-1",
      isLoadingThread: false,
      load: null,
    }),
    null,
  );
  assert.equal(
    resolveConversationInitialAnchorForLoad(cells, {
      selectedThreadId: "thread-1",
      isLoadingThread: false,
      load: { threadId: "thread-2", requestId: 1 },
    }),
    null,
  );
  assert.equal(
    resolveConversationInitialAnchorForLoad(cells, {
      selectedThreadId: "thread-1",
      isLoadingThread: true,
      load: { threadId: "thread-1", requestId: 1 },
    }),
    null,
  );
});
