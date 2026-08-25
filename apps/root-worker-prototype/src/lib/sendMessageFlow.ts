import type { ComposerDraft } from "./composerDraft";
import { buildSendMessagePayload } from "./sendMessagePayload";
import type { Thread, Turn } from "../types";

export type SendMessagePayload = ReturnType<typeof buildSendMessagePayload>;

export async function submitThreadMessage({
  draft,
  thread,
  threadId,
  ensureSubscribed,
  sendMessage,
  applyTurn,
  revokeImage,
  clearDraft,
}: {
  draft: ComposerDraft;
  thread: Thread | null;
  threadId: string;
  ensureSubscribed: (threadId: string) => Promise<boolean>;
  sendMessage: (payload: SendMessagePayload) => Promise<{ turn?: Turn | null }>;
  applyTurn: (threadId: string, turn: Turn) => void;
  revokeImage: (image: ComposerDraft["images"][number]) => void;
  clearDraft: (threadId: string) => void;
}) {
  if (!(await ensureSubscribed(threadId))) {
    return false;
  }

  const response = await sendMessage(
    buildSendMessagePayload({
      draft,
      thread,
      threadId,
    }),
  );
  if (response.turn) {
    applyTurn(threadId, response.turn);
  }
  for (const image of draft.images) {
    revokeImage(image);
  }
  clearDraft(threadId);
  return true;
}
