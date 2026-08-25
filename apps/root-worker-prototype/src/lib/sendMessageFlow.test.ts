import test from "node:test";
import assert from "node:assert/strict";

import { submitThreadMessage, type SendMessagePayload } from "./sendMessageFlow";
import type { ComposerDraft } from "./composerDraft";
import type { Thread, Turn } from "../types";

function makeDraft(): ComposerDraft {
  return {
    text: "  continue  ",
    skills: [],
    images: [
      {
        id: "image-1",
        name: "example.png",
        mimeType: "image/png",
        byteSize: 3,
        bytes: Uint8Array.from([1, 2, 3]).buffer,
        previewUrl: "blob:example",
      },
    ],
  };
}

function makeThread(): Thread {
  return {
    id: "cp-http-api-root",
    sessionId: "session-1",
    forkedFromId: null,
    preview: "",
    ephemeral: false,
    modelProvider: "openai",
    model: "gpt-5",
    reasoningEffort: "high",
    createdAt: 1,
    updatedAt: 1,
    lifecycleStatus: { type: "final", result: { type: "completed" } },
    path: null,
    cwd: "/work/project",
    cliVersion: "test",
    source: "appServer",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    agentPath: "/cp_http_api",
    gitInfo: null,
    name: null,
    skills: [],
    turns: [],
  };
}

function makeTurn(): Turn {
  return {
    id: "turn-1",
    items: [],
    itemsView: "notLoaded",
    status: "inProgress",
    error: null,
    startedAt: null,
    completedAt: null,
    durationMs: null,
  };
}

test("submitThreadMessage subscribes the selected thread before sending", async () => {
  const calls: string[] = [];
  const sentPayloads: SendMessagePayload[] = [];
  const appliedTurns: Array<{ threadId: string; turn: Turn }> = [];
  const revokedImages: string[] = [];
  const clearedDrafts: string[] = [];
  const turn = makeTurn();

  const result = await submitThreadMessage({
    draft: makeDraft(),
    thread: makeThread(),
    threadId: "cp-http-api-root",
    ensureSubscribed: async (threadId) => {
      calls.push(`subscribe:${threadId}`);
      return true;
    },
    sendMessage: async (payload) => {
      calls.push(`send:${payload.threadId}`);
      sentPayloads.push(payload);
      return { turn };
    },
    applyTurn: (threadId, responseTurn) => {
      calls.push(`apply:${threadId}:${responseTurn.id}`);
      appliedTurns.push({ threadId, turn: responseTurn });
    },
    revokeImage: (image) => {
      calls.push(`revoke:${image.id}`);
      revokedImages.push(image.id);
    },
    clearDraft: (threadId) => {
      calls.push(`clear:${threadId}`);
      clearedDrafts.push(threadId);
    },
  });

  assert.equal(result, true);
  assert.deepEqual(calls, [
    "subscribe:cp-http-api-root",
    "send:cp-http-api-root",
    "apply:cp-http-api-root:turn-1",
    "revoke:image-1",
    "clear:cp-http-api-root",
  ]);
  assert.equal(sentPayloads[0]?.threadId, "cp-http-api-root");
  assert.equal(sentPayloads[0]?.text, "continue");
  assert.equal(appliedTurns[0]?.turn, turn);
  assert.deepEqual(revokedImages, ["image-1"]);
  assert.deepEqual(clearedDrafts, ["cp-http-api-root"]);
});

test("submitThreadMessage leaves draft and images intact when subscribe fails", async () => {
  const calls: string[] = [];

  const result = await submitThreadMessage({
    draft: makeDraft(),
    thread: makeThread(),
    threadId: "cp-http-api-root",
    ensureSubscribed: async (threadId) => {
      calls.push(`subscribe:${threadId}`);
      return false;
    },
    sendMessage: async () => {
      calls.push("send");
      return { turn: makeTurn() };
    },
    applyTurn: () => calls.push("apply"),
    revokeImage: () => calls.push("revoke"),
    clearDraft: () => calls.push("clear"),
  });

  assert.equal(result, false);
  assert.deepEqual(calls, ["subscribe:cp-http-api-root"]);
});
