import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  ArchivedHistoryRow,
  CompactRow,
  MessageRow,
  ToolRow,
} from "./Conversation";
import {
  buildConversationRowClassName,
  shouldHandleFocusedItemRequest,
} from "./ConversationVirtualList";
import type { ConversationEntry } from "../types";

const entries: ConversationEntry[] = [
  {
    id: "tool-1",
    kind: "tool",
    author: "root",
    role: "system",
    text: "first summary",
    timestamp: "09:41",
    attachments: [],
    toolName: "first tool",
    toolStatus: "completed",
    toolDetails: "first details",
    toolCategory: "eventDrivenSubscription",
  },
  {
    id: "tool-2",
    kind: "tool",
    author: "root",
    role: "system",
    text: "second summary",
    timestamp: "09:42",
    attachments: [],
    toolName: "second tool",
    toolStatus: "completed",
    toolDetails: "second details",
    toolCategory: "eventDrivenEvent",
  },
];

test("tool rows show compact lists first and only reveal the selected detail body", () => {
  const listMarkup = renderToStaticMarkup(
    <ToolRow entries={entries} isOpen selectedEntryId={null} />,
  );

  assert.match(listMarkup, /tool-card-list/);
  assert.match(listMarkup, /2\/2 done/);
  assert.match(listMarkup, /tool-status-badge done/);
  assert.doesNotMatch(listMarkup, /first details/);
  assert.doesNotMatch(listMarkup, /second details/);

  const detailMarkup = renderToStaticMarkup(
    <ToolRow entries={entries} isOpen selectedEntryId="tool-2" />,
  );

  assert.match(detailMarkup, /second details/);
  assert.doesNotMatch(detailMarkup, /first details/);
  assert.match(detailMarkup, /second summary[\s\S]*second details/);
  assert.match(detailMarkup, /tool-card-item-head selected/);
});

test("message rows expose role classes for chat alignment", () => {
  const userEntry: ConversationEntry = {
    id: "user-1",
    kind: "message",
    author: "You",
    role: "user",
    text: "please update the layout",
    timestamp: "09:40",
    attachments: [],
  };
  const agentEntry: ConversationEntry = {
    id: "agent-1",
    kind: "message",
    author: "Codex",
    role: "agent",
    text: "layout updated",
    timestamp: "09:41",
    attachments: [],
  };

  const userMarkup = renderToStaticMarkup(<MessageRow entries={[userEntry]} />);
  const agentMarkup = renderToStaticMarkup(
    <MessageRow entries={[agentEntry]} />,
  );

  assert.match(userMarkup, /class="message-row message-row-user"/);
  assert.match(userMarkup, /class="message-avatar user"/);
  assert.match(agentMarkup, /class="message-row message-row-agent"/);
  assert.match(agentMarkup, /class="message-avatar agent"/);
});

test("grouped agent message rows render one bubble with multiple segments", () => {
  const firstEntry: ConversationEntry = {
    id: "agent-1",
    kind: "message",
    author: "Codex",
    role: "agent",
    text: "first",
    timestamp: "09:41",
    attachments: [],
  };
  const secondEntry: ConversationEntry = {
    ...firstEntry,
    id: "agent-2",
    text: "second",
    timestamp: "09:42",
  };

  const markup = renderToStaticMarkup(
    <MessageRow entries={[firstEntry, secondEntry]} />,
  );

  assert.equal(markup.match(/class="message-bubble/g)?.length ?? 0, 1);
  assert.match(markup, /class="message-bubble message-bubble-combined"/);
  assert.equal(markup.match(/class="message-segment"/g)?.length ?? 0, 2);
  assert.match(markup, /first[\s\S]*second/);
});

test("message rows only combine bubbles when every entry is an agent message", () => {
  const agentEntry: ConversationEntry = {
    id: "agent-1",
    kind: "message",
    author: "Codex",
    role: "agent",
    text: "agent",
    timestamp: "09:41",
    attachments: [],
  };
  const userEntry: ConversationEntry = {
    id: "user-1",
    kind: "message",
    author: "You",
    role: "user",
    text: "user",
    timestamp: "09:42",
    attachments: [],
  };

  const markup = renderToStaticMarkup(
    <MessageRow entries={[agentEntry, userEntry]} />,
  );

  assert.equal(markup.match(/class="message-bubble"/g)?.length ?? 0, 2);
  assert.doesNotMatch(markup, /message-bubble-combined/);
});

test("single tool rows render a single inline item and auto-expand details with the card", () => {
  const [singleEntry] = entries;
  assert.ok(singleEntry);

  const collapsedMarkup = renderToStaticMarkup(
    <ToolRow entries={[singleEntry]} selectedEntryId={null} />,
  );

  assert.doesNotMatch(collapsedMarkup, /tool-card-list/);
  assert.doesNotMatch(collapsedMarkup, /tool-card-item-single/);
  assert.doesNotMatch(collapsedMarkup, /first details/);

  const expandedMarkup = renderToStaticMarkup(
    <ToolRow entries={[singleEntry]} isOpen />,
  );

  assert.match(expandedMarkup, /first summary[\s\S]*first details/);
  assert.doesNotMatch(expandedMarkup, /tool-card-item-single/);
  assert.equal(expandedMarkup.match(/first summary/g)?.length ?? 0, 1);
});

test("tool rows treat partially completed lists as in progress", () => {
  const mixedEntries: ConversationEntry[] = [
    entries[0]!,
    {
      ...entries[1]!,
      id: "tool-3",
      toolStatus: "running",
    },
  ];

  const markup = renderToStaticMarkup(
    <ToolRow entries={mixedEntries} isOpen selectedEntryId={null} />,
  );

  assert.match(markup, /1\/2 done/);
  assert.match(markup, /tool-status-badge doing/);
  assert.match(markup, /tool-status-badge done[^>]*>completed/);
  assert.match(markup, /tool-status-badge doing[^>]*>running/);
});

test("conversation virtual rows expose search match and current highlight classes", () => {
  assert.equal(
    buildConversationRowClassName({
      highlighted: false,
      searchCurrent: false,
      searchMatch: true,
    }),
    "conversation-virtual-row search-match",
  );
  assert.equal(
    buildConversationRowClassName({
      highlighted: true,
      searchCurrent: true,
      searchMatch: true,
    }),
    "conversation-virtual-row highlighted search-match search-current",
  );
});

test("focused conversation jumps only consume each focus token once", () => {
  assert.equal(
    shouldHandleFocusedItemRequest({
      focusedItem: { itemId: "cmd-1", token: 1 },
      lastHandledRequest: null,
    }),
    true,
  );
  assert.equal(
    shouldHandleFocusedItemRequest({
      focusedItem: { itemId: "cmd-1", token: 1 },
      lastHandledRequest: { itemId: "cmd-1", token: 1 },
    }),
    false,
  );
  assert.equal(
    shouldHandleFocusedItemRequest({
      focusedItem: { itemId: "cmd-1", token: 2 },
      lastHandledRequest: { itemId: "cmd-1", token: 1 },
    }),
    true,
  );
  assert.equal(
    shouldHandleFocusedItemRequest({
      focusedItem: { itemId: "search-hit-1", token: 1 },
      lastHandledRequest: { itemId: "cmd-1", token: 1 },
    }),
    true,
  );
});

test("compact rows point to replacement history in the active chat list", () => {
  const markup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-1",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Previous conversation was archived; compacted model context continues below.",
        timestamp: "09:43",
        attachments: [],
        replacementHistoryStatus: "available",
        replacementHistoryCount: 2,
        replacementHistoryEntries: [],
      }}
    />,
  );

  assert.match(markup, /Context compacted/);
  assert.match(markup, /2 replacement items/);
  assert.match(markup, /compacted context is shown below/);
  assert.doesNotMatch(markup, /recent request/);
  assert.doesNotMatch(markup, /functions\/exec_command/);
});

test("compact rows explain unavailable replacement history", () => {
  const markup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-1",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Previous conversation was archived; compacted model context continues below.",
        timestamp: "09:43",
        attachments: [],
        replacementHistoryStatus: "missing",
        replacementHistoryCount: null,
        replacementHistoryEntries: null,
      }}
    />,
  );

  assert.match(markup, /replacement history unavailable/);
  assert.match(markup, /Replacement history is unavailable/);
});

test("archived history rows collapse previous conversation by default", () => {
  const markup = renderToStaticMarkup(
    <ArchivedHistoryRow
      entry={{
        id: "archive-1",
        kind: "archive",
        author: "Root",
        role: "system",
        text: "Previous conversation is no longer the active model context.",
        timestamp: "09:43",
        attachments: [],
        archivedEntryCount: 1,
        archivedCells: [
          {
            id: "old-message",
            kind: "message",
            entries: [
              {
                id: "old-message",
                kind: "message",
                author: "You",
                role: "user",
                text: "old request",
                timestamp: "09:41",
                attachments: [],
              },
            ],
          },
        ],
      }}
    />,
  );

  assert.match(markup, /Previous conversation/);
  assert.match(markup, /1 archived item/);
  assert.match(markup, /<details class="archive-card">/);
  assert.doesNotMatch(markup, /<details class="archive-card" open/);
  assert.match(markup, /old request/);
});

test("archived history rows show archived tool details when expanded", () => {
  const markup = renderToStaticMarkup(
    <ArchivedHistoryRow
      entry={{
        id: "archive-1",
        kind: "archive",
        author: "Root",
        role: "system",
        text: "Previous conversation is no longer the active model context.",
        timestamp: "09:43",
        attachments: [],
        archivedEntryCount: 1,
        archivedCells: [
          {
            id: "tool-1",
            kind: "tool",
            entries: [
              {
                id: "tool-1",
                kind: "tool",
                author: "Root",
                role: "system",
                text: "pwd",
                timestamp: "09:41",
                attachments: [],
                toolName: "shell",
                toolStatus: "completed",
                toolDetails: "Command\npwd",
                toolCategory: "command",
              },
            ],
          },
        ],
      }}
    />,
  );

  assert.match(markup, /archive-tool-stack/);
  assert.match(markup, /Command[\s\S]*pwd/);
});
