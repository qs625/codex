import test from "node:test";
import assert from "node:assert/strict";

import {
  beginVoiceCaptureStop,
  isVoiceCaptureToggleDisabled,
} from "./voiceCaptureState";

test("voice capture toggle stays available while a turn is in progress", () => {
  assert.equal(
    isVoiceCaptureToggleDisabled({
      selectedThreadId: "thread-1",
      isSending: false,
      isStoppingTurn: false,
    }),
    false,
  );
});

test("voice capture toggle is disabled when no thread can receive transcript", () => {
  assert.equal(
    isVoiceCaptureToggleDisabled({
      selectedThreadId: null,
      isSending: false,
      isStoppingTurn: false,
    }),
    true,
  );
});

test("voice capture toggle is disabled while starting or stopping composer actions", () => {
  assert.equal(
    isVoiceCaptureToggleDisabled({
      selectedThreadId: "thread-1",
      isSending: true,
      isStoppingTurn: false,
    }),
    true,
  );
  assert.equal(
    isVoiceCaptureToggleDisabled({
      selectedThreadId: "thread-1",
      isSending: false,
      isStoppingTurn: true,
    }),
    true,
  );
});

test("voice stop keeps the session alive until the realtime close event arrives", () => {
  const pendingStop = beginVoiceCaptureStop("thread-1", false);

  assert.deepEqual(pendingStop, {
    nextSession: {
      threadId: "thread-1",
      status: "stopping",
    },
    nextStatus: "stopping",
    nextMessage: "Stopping voice input…",
  });
});

test("voice stop returns null when there is no active thread to stop", () => {
  assert.equal(beginVoiceCaptureStop(null, false), null);
});
