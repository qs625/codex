import test from "node:test";
import assert from "node:assert/strict";

import {
  decideThreadSelectionAction,
  isSelectedThreadLoading,
  nextThreadReadRequestId,
  shouldApplyThreadReadSnapshot,
} from "./threadSelectionPolicy";

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

test("selection policy reads subscribed local live threads before initialization", () => {
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: false,
      isSubscribed: true,
      isLoading: false,
      hasLiveCache: true,
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
      hasLiveCache: true,
    }),
    "none",
  );
});

test("selection policy reads local live cached threads before initialization", () => {
  assert.equal(
    decideThreadSelectionAction({
      selectedThreadId: "thread-1",
      hasLocalThread: true,
      isLoaded: false,
      isSubscribed: false,
      isLoading: false,
      hasLiveCache: true,
    }),
    "readAndSubscribe",
  );
});

test("selection policy subscribes initialized live cached threads without reading", () => {
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

test("thread read snapshots apply only to the current uninitialized request", () => {
  assert.equal(
    shouldApplyThreadReadSnapshot({
      threadId: "thread-a",
      selectedThreadId: "thread-a",
      requestId: 2,
      latestRequestId: 2,
      isLoaded: false,
    }),
    true,
  );

  assert.equal(
    shouldApplyThreadReadSnapshot({
      threadId: "thread-a",
      selectedThreadId: "thread-b",
      requestId: 2,
      latestRequestId: 2,
      isLoaded: false,
    }),
    false,
  );

  assert.equal(
    shouldApplyThreadReadSnapshot({
      threadId: "thread-a",
      selectedThreadId: "thread-a",
      requestId: 1,
      latestRequestId: 2,
      isLoaded: false,
    }),
    false,
  );

  assert.equal(
    shouldApplyThreadReadSnapshot({
      threadId: "thread-a",
      selectedThreadId: "thread-a",
      requestId: 2,
      latestRequestId: 2,
      isLoaded: true,
    }),
    false,
  );
});

test("thread read request ids are scoped per thread for switch away and back", () => {
  const requestIdsByThreadId = new Map<string, number>();
  const threadARequestId = nextThreadReadRequestId(
    requestIdsByThreadId,
    "thread-a",
  );
  requestIdsByThreadId.set("thread-a", threadARequestId);

  const threadBRequestId = nextThreadReadRequestId(
    requestIdsByThreadId,
    "thread-b",
  );
  requestIdsByThreadId.set("thread-b", threadBRequestId);

  assert.equal(threadARequestId, 1);
  assert.equal(threadBRequestId, 1);
  assert.equal(
    shouldApplyThreadReadSnapshot({
      threadId: "thread-a",
      selectedThreadId: "thread-a",
      requestId: threadARequestId,
      latestRequestId: requestIdsByThreadId.get("thread-a") ?? null,
      isLoaded: false,
    }),
    true,
  );
});

test("selected thread loading is derived only from the current selected thread", () => {
  const loadingThreadIds = new Set(["thread-a"]);

  assert.equal(isSelectedThreadLoading("thread-a", loadingThreadIds), true);
  assert.equal(isSelectedThreadLoading("thread-b", loadingThreadIds), false);
  assert.equal(isSelectedThreadLoading(null, loadingThreadIds), false);

  loadingThreadIds.delete("thread-a");
  loadingThreadIds.add("thread-c");

  assert.equal(isSelectedThreadLoading("thread-c", loadingThreadIds), true);
  assert.equal(isSelectedThreadLoading("thread-a", loadingThreadIds), false);
});
