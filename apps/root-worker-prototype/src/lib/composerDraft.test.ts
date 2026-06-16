import test from "node:test";
import assert from "node:assert/strict";

import {
  clearComposerDraft,
  getComposerDraft,
  isClearComposerCommand,
  isGoalCancelComposerCommand,
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
