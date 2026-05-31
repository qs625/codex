import test from "node:test";
import assert from "node:assert/strict";

import {
  appendVoiceTranscriptDelta,
  buildVoiceDraft,
  finalizeVoiceTranscriptSegment,
} from "./voiceInput";

test("voice draft appends committed and live transcript onto the base draft", () => {
  const draft = buildVoiceDraft({
    baseDraft: "Summarize this",
    committedSegments: ["please review the latest diff"],
    liveSegment: " before lunch",
  });

  assert.equal(
    draft,
    "Summarize this please review the latest diff before lunch",
  );
});

test("voice transcript done commits the finalized segment and clears the live buffer", () => {
  const initialState = {
    baseDraft: "",
    committedSegments: [],
    liveSegment: "partial hypothesis",
  };

  const deltaState = appendVoiceTranscriptDelta(initialState, " with more");
  const finalState = finalizeVoiceTranscriptSegment(
    deltaState,
    "partial hypothesis with more",
  );

  assert.deepEqual(finalState, {
    baseDraft: "",
    committedSegments: ["partial hypothesis with more"],
    liveSegment: "",
  });
});
