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
      hasLiveCache: false,
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
      hasLiveCache: false,
    }),
    "subscribeOnly",
  );
});

test("selection policy reads and subscribes missing threads", () => {
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: false,
      isLoaded: false,
      isSubscribed: false,
      isLoading: false,
      hasLiveCache: true,
    }),
    "readAndSubscribe",
  );
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: false,
      isSubscribed: false,
      isLoading: false,
      hasLiveCache: false,
    }),
    "readAndSubscribe",
  );
});

test("selection policy keeps subscribed local live threads without reading", () => {
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: false,
      isSubscribed: true,
      isLoading: false,
      hasLiveCache: true,
    }),
    "none",
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
      hasLiveCache: true,
    }),
    "none",
  );
});

test("selection policy subscribes live cached threads without reading", () => {
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: false,
      isSubscribed: false,
      isLoading: false,
      hasLiveCache: true,
    }),
    "subscribeOnly",
  );
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: true,
      isSubscribed: false,
      isLoading: false,
      hasLiveCache: true,
    }),
    "subscribeOnly",
  );
});
