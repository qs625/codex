import test from "node:test";
import assert from "node:assert/strict";

import {
  applyRunConfigOverride,
  buildSendMessagePayload,
} from "./sendMessagePayload";
import {
  buildProjectAgentSidebar,
  findProjectByRootIdentity,
} from "./thread";
import type { ComposerDraft } from "./composerDraft";
import type { Thread } from "../types";

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: "thread-1",
    sessionId: "session-1",
    forkedFromId: null,
    preview: "",
    ephemeral: false,
    modelProvider: "openai",
    model: "gpt-5.5",
    reasoningEffort: "high",
    createdAt: 1,
    updatedAt: 1,
    lifecycleStatus: { type: "final", result: { type: "completed" } },
    path: null,
    cwd: "/tmp",
    cliVersion: "test",
    source: "appServer",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name: null,
    skills: [],
    turns: [],
    ...overrides,
  };
}

test("buildSendMessagePayload includes the selected thread run config", () => {
  const draft: ComposerDraft = {
    text: "  hello  ",
    skills: [{ name: "review", path: "/skills/review" }],
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

  assert.deepEqual(
    buildSendMessagePayload({
      draft,
      thread: makeThread(),
      threadId: "thread-1",
    }),
    {
      threadId: "thread-1",
      model: "gpt-5.5",
      modelProvider: "openai",
      effort: "high",
      text: "hello",
      skills: [{ name: "review", path: "/skills/review" }],
      images: [
        {
          name: "example.png",
          mimeType: "image/png",
          bytes: draft.images[0]!.bytes,
        },
      ],
    },
  );
});

test("applyRunConfigOverride uses pending config for immediate sends", () => {
  const thread = makeThread({
    model: "gpt-5",
    reasoningEffort: "medium",
  });

  assert.deepEqual(
    buildSendMessagePayload({
      draft: {
        text: "next",
        skills: [],
        images: [],
      },
      thread: applyRunConfigOverride(thread, {
        model: "gpt-5.5",
        modelProvider: "modelhub",
        reasoningEffort: "high",
        contextWindow: 128000,
        maxContextWindow: 256000,
        autoCompactTokenLimit: 90000,
      }),
      threadId: "thread-1",
    }),
    {
      threadId: "thread-1",
      model: "gpt-5.5",
      modelProvider: "modelhub",
      effort: "high",
      text: "next",
      skills: [],
      images: [],
    },
  );
});

test("buildSendMessagePayload targets the selected cp_http_api project root", () => {
  const cpHttpApiRoot = makeThread({
    id: "cp-http-api-root",
    cwd: "/work/project",
    agentPath: "/cp_http_api",
  });
  const workspaceRoot = makeThread({
    id: "workspace-root",
    cwd: "/work/project",
    agentPath: null,
  });
  const otherRoot = makeThread({
    id: "other-root",
    cwd: "/work/project",
    agentPath: "/other_api",
  });
  const sidebar = buildProjectAgentSidebar([
    cpHttpApiRoot,
    workspaceRoot,
    otherRoot,
  ]);
  const project = findProjectByRootIdentity(
    sidebar.projects,
    "/work/project",
    "/cp_http_api",
  );

  assert.equal(project?.tree.threadId, "cp-http-api-root");
  assert.equal(
    buildSendMessagePayload({
      draft: {
        text: "continue",
        skills: [],
        images: [],
      },
      thread: cpHttpApiRoot,
      threadId: project!.tree.threadId,
    }).threadId,
    "cp-http-api-root",
  );
});
