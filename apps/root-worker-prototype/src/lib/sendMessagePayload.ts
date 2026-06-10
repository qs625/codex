import type { ComposerDraft } from "./composerDraft";
import type { RunConfigSelection } from "./runConfig";
import type { Thread } from "../types";

export type RunConfigOverride = RunConfigSelection;

export function applyRunConfigOverride(
  thread: Thread | null,
  override: RunConfigOverride | null,
) {
  if (!thread || !override) {
    return thread;
  }
  return {
    ...thread,
    model: override.model,
    modelProvider: override.modelProvider ?? thread.modelProvider,
    reasoningEffort: override.reasoningEffort,
  };
}

export function buildSendMessagePayload({
  draft,
  thread,
  threadId,
}: {
  draft: ComposerDraft;
  thread: Thread | null;
  threadId: string;
}) {
  return {
    threadId,
    model: thread?.model ?? null,
    modelProvider: thread?.modelProvider ?? null,
    effort: thread?.reasoningEffort ?? null,
    text: draft.text.trim(),
    skills: draft.skills,
    images: draft.images.map(({ name, mimeType, bytes }) => ({
      name,
      mimeType,
      bytes,
    })),
  };
}
