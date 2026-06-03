import test from "node:test";
import assert from "node:assert/strict";

import { decideThreadSelectionAction } from "./threadSelectionPolicy";

test("selection policy does nothing for a loaded and subscribed thread", () => {
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: true,
      isSubscribed: true,
      isLoading: false,
    }),
    "none",
  );
});

test("selection policy subscribes without reading loaded local threads", () => {
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: true,
      isSubscribed: false,
      isLoading: false,
    }),
    "subscribeOnly",
  );
});

test("selection policy reads and subscribes missing or incomplete threads", () => {
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: false,
      isLoaded: false,
      isSubscribed: false,
      isLoading: false,
    }),
    "readAndSubscribe",
  );
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: false,
      isSubscribed: true,
      isLoading: false,
    }),
    "readAndSubscribe",
  );
});

test("selection policy ignores threads already being loaded", () => {
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: false,
      isSubscribed: true,
      isLoading: true,
    }),
    "none",
  );
});
