import type { VoiceCaptureStatus } from "../types";

export type ActiveVoiceSession = {
  threadId: string;
  status: VoiceCaptureStatus;
};

export type PendingVoiceStop = {
  nextSession: ActiveVoiceSession;
  nextStatus: VoiceCaptureStatus;
  nextMessage: string | null;
};

export function beginVoiceCaptureStop(
  threadId: string | null | undefined,
  silent: boolean,
): PendingVoiceStop | null {
  if (!threadId) {
    return null;
  }

  return {
    nextSession: {
      threadId,
      status: "stopping",
    },
    nextStatus: "stopping",
    nextMessage: silent ? null : "Stopping voice input…",
  };
}
