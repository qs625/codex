import test from "node:test";
import assert from "node:assert/strict";

import { isConversationNearBottom } from "./conversationScroll";

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
