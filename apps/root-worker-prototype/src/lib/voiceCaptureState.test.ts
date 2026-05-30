import test from "node:test";
import assert from "node:assert/strict";

import { beginVoiceCaptureStop } from "./voiceCaptureState";

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
