export type VoiceDraftState = {
  baseDraft: string;
  committedSegments: string[];
  liveSegment: string;
};

export function buildVoiceDraft(state: VoiceDraftState): string {
  const transcript = buildVoiceTranscript(state.committedSegments, state.liveSegment);
  if (!transcript) {
    return state.baseDraft;
  }
  if (!state.baseDraft) {
    return transcript;
  }

  const separator = /\s$/.test(state.baseDraft) ? "" : " ";
  return `${state.baseDraft}${separator}${transcript}`;
}

export function buildVoiceTranscript(committedSegments: string[], liveSegment: string): string {
  const transcriptParts = committedSegments
    .map((segment) => segment.trim())
    .filter(Boolean);
  const normalizedLiveSegment = liveSegment.trim();

  if (normalizedLiveSegment) {
    transcriptParts.push(normalizedLiveSegment);
  }

  return transcriptParts.join(" ");
}

export function appendVoiceTranscriptDelta(state: VoiceDraftState, delta: string): VoiceDraftState {
  return {
    ...state,
    liveSegment: `${state.liveSegment}${delta}`,
  };
}

export function finalizeVoiceTranscriptSegment(
  state: VoiceDraftState,
  text: string,
): VoiceDraftState {
  const normalizedText = text.trim();
  if (!normalizedText) {
    return {
      ...state,
      liveSegment: "",
    };
  }

  return {
    ...state,
    committedSegments: [...state.committedSegments, normalizedText],
    liveSegment: "",
  };
}
