import test from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";

import { ToolRow } from "./Conversation";
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

test("single tool rows render a single inline item and auto-expand details with the card", () => {
  const [singleEntry] = entries;
  assert.ok(singleEntry);

  const collapsedMarkup = renderToStaticMarkup(
    <ToolRow entries={[singleEntry]} isOpen selectedEntryId={null} />,
  );

  assert.doesNotMatch(collapsedMarkup, /tool-card-list/);
  assert.match(collapsedMarkup, /tool-card-item-single/);
  assert.doesNotMatch(collapsedMarkup, /first details/);

  const expandedMarkup = renderToStaticMarkup(
    <ToolRow entries={[singleEntry]} isOpen />,
  );

  assert.match(expandedMarkup, /first summary[\s\S]*first details/);
});
