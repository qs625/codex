import test from "node:test";
import assert from "node:assert/strict";

import {
  isConversationNearBottom,
  planConversationHeightChangeScroll,
} from "./conversationScroll";

test("treats the conversation as sticky only within the bottom threshold", () => {
  assert.equal(
    isConversationNearBottom({
      scrollHeight: 1000,
      clientHeight: 400,
      scrollTop: 576,
    }),
    true,
  );
  assert.equal(
    isConversationNearBottom({
      scrollHeight: 1000,
      clientHeight: 400,
      scrollTop: 575,
    }),
    false,
  );
});

test("keeps sticky conversations pinned when a measured cell height changes", () => {
  assert.deepEqual(
    planConversationHeightChangeScroll({
      cellTop: 100,
      heightDelta: -80,
      metrics: {
        scrollHeight: 1000,
        clientHeight: 400,
        scrollTop: 590,
      },
    }),
    {
      scrollTopAdjustment: 0,
      shouldStickToBottom: true,
    },
  );
});

test("preserves the viewport when a non-sticky cell above it changes height", () => {
  assert.deepEqual(
    planConversationHeightChangeScroll({
      cellTop: 100,
      heightDelta: 64,
      metrics: {
        scrollHeight: 1000,
        clientHeight: 400,
        scrollTop: 500,
      },
    }),
    {
      scrollTopAdjustment: 64,
      shouldStickToBottom: false,
    },
  );
});

test("does not move a non-sticky viewport for height changes below the viewport top", () => {
  assert.deepEqual(
    planConversationHeightChangeScroll({
      cellTop: 520,
      heightDelta: 64,
      metrics: {
        scrollHeight: 1000,
        clientHeight: 400,
        scrollTop: 500,
      },
    }),
    {
      scrollTopAdjustment: 0,
      shouldStickToBottom: false,
    },
  );
});
