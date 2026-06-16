import test from "node:test";
import assert from "node:assert/strict";

import {
  clearComposerDraft,
  getComposerDraft,
  isClearComposerCommand,
  isGoalCancelComposerCommand,
  parseGoalComposerCommand,
  updateComposerDraft,
  type ComposerDraftsByThreadId,
} from "./composerDraft";

test("keeps composer drafts isolated by thread", () => {
  let drafts: ComposerDraftsByThreadId = {};

  drafts = updateComposerDraft(drafts, "thread-a", (draft) => ({
    ...draft,
    text: "work on A",
  }));
  drafts = updateComposerDraft(drafts, "thread-b", (draft) => ({
    ...draft,
    text: "work on B",
  }));

  assert.equal(getComposerDraft(drafts, "thread-a").text, "work on A");
  assert.equal(getComposerDraft(drafts, "thread-b").text, "work on B");
  assert.equal(getComposerDraft(drafts, "thread-c").text, "");
});

test("clears only the selected thread draft", () => {
  let drafts: ComposerDraftsByThreadId = {};
  drafts = updateComposerDraft(drafts, "thread-a", (draft) => ({
    ...draft,
    text: "send this",
  }));
  drafts = updateComposerDraft(drafts, "thread-b", (draft) => ({
    ...draft,
    text: "keep this",
  }));

  drafts = clearComposerDraft(drafts, "thread-a");

  assert.equal(getComposerDraft(drafts, "thread-a").text, "");
  assert.equal(getComposerDraft(drafts, "thread-b").text, "keep this");
});

test("stores attachments and skills with the thread draft", () => {
  const drafts = updateComposerDraft({}, "thread-a", (draft) => ({
    ...draft,
    skills: [{ name: "review", path: "skills/review.md" }],
    images: [
      {
        id: "image-1",
        name: "screen.png",
        mimeType: "image/png",
        byteSize: 3,
        bytes: new ArrayBuffer(3),
        previewUrl: "blob:screen",
      },
    ],
  }));

  assert.deepEqual(getComposerDraft(drafts, "thread-a").skills, [
    { name: "review", path: "skills/review.md" },
  ]);
  assert.deepEqual(getComposerDraft(drafts, "thread-a").images, [
    {
      id: "image-1",
      name: "screen.png",
      mimeType: "image/png",
      byteSize: 3,
      bytes: new ArrayBuffer(3),
      previewUrl: "blob:screen",
    },
  ]);
});

test("detects clear command only when the draft has no attachments", () => {
  assert.equal(
    isClearComposerCommand({
      text: " /clear ",
      skills: [],
      images: [],
    }),
    true,
  );
  assert.equal(
    isClearComposerCommand({
      text: "/clear",
      skills: [{ name: "review", path: "skills/review.md" }],
      images: [],
    }),
    false,
  );
  assert.equal(
    isClearComposerCommand({
      text: "/clear",
      skills: [],
      images: [
        {
          id: "image-1",
          name: "screen.png",
          mimeType: "image/png",
          byteSize: 3,
          bytes: new ArrayBuffer(3),
          previewUrl: "blob:screen",
        },
      ],
    }),
    false,
  );
});

test("detects goal cancel command only when the draft has no attachments", () => {
  assert.equal(
    isGoalCancelComposerCommand({
      text: " /goal cancel ",
      skills: [],
      images: [],
    }),
    true,
  );
  assert.equal(
    isGoalCancelComposerCommand({
      text: "/cancel-goal",
      skills: [],
      images: [],
    }),
    true,
  );
  assert.equal(
    isGoalCancelComposerCommand({
      text: "/goal cancel",
      skills: [{ name: "review", path: "skills/review.md" }],
      images: [],
    }),
    false,
  );
});

test("parses goal commands before sending", () => {
  assert.deepEqual(
    parseGoalComposerCommand({
      text: "/goal Ship goal command actions",
      skills: [],
      images: [],
    }),
    {
      type: "set",
      objective: "Ship goal command actions",
      status: "active",
    },
  );
  assert.deepEqual(
    parseGoalComposerCommand({
      text: "/goal pause",
      skills: [],
      images: [],
    }),
    {
      type: "status",
      status: "paused",
    },
  );
  assert.deepEqual(
    parseGoalComposerCommand({
      text: "/goal resume",
      skills: [],
      images: [],
    }),
    {
      type: "status",
      status: "active",
    },
  );
  assert.deepEqual(
    parseGoalComposerCommand({
      text: "/goal clear",
      skills: [],
      images: [],
    }),
    {
      type: "clear",
    },
  );
});

test("parses only exact goal action commands as actions", () => {
  assert.deepEqual(
    parseGoalComposerCommand({
      text: "/goal pause this migration",
      skills: [],
      images: [],
    }),
    {
      type: "set",
      objective: "pause this migration",
      status: "active",
    },
  );
  assert.equal(
    parseGoalComposerCommand({
      text: "/goal pause",
      skills: [{ name: "review", path: "skills/review.md" }],
      images: [],
    }),
    null,
  );
});

test("parses empty goal objective as invalid command", () => {
  assert.deepEqual(
    parseGoalComposerCommand({
      text: "/goal",
      skills: [],
      images: [],
    }),
    {
      type: "invalid",
      message: "Enter a goal objective.",
    },
  );
  assert.deepEqual(
    parseGoalComposerCommand({
      text: "/goal ",
      skills: [],
      images: [],
    }),
    {
      type: "invalid",
      message: "Enter a goal objective.",
    },
  );
});
