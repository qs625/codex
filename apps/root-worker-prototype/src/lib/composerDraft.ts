import type { ComposerImage, DraftSkill } from "../types";

export type ComposerDraft = {
  text: string;
  skills: DraftSkill[];
  images: ComposerImage[];
};

export type ComposerDraftsByThreadId = Record<string, ComposerDraft>;

export const EMPTY_COMPOSER_DRAFT: ComposerDraft = {
  text: "",
  skills: [],
  images: [],
};

export function getComposerDraft(
  draftsByThreadId: ComposerDraftsByThreadId,
  threadId: string | null,
) {
  if (!threadId) {
    return EMPTY_COMPOSER_DRAFT;
  }
  return draftsByThreadId[threadId] ?? EMPTY_COMPOSER_DRAFT;
}

export function updateComposerDraft(
  draftsByThreadId: ComposerDraftsByThreadId,
  threadId: string | null,
  update: (draft: ComposerDraft) => ComposerDraft,
) {
  if (!threadId) {
    return draftsByThreadId;
  }
  const nextDraft = update(getComposerDraft(draftsByThreadId, threadId));
  return {
    ...draftsByThreadId,
    [threadId]: {
      text: nextDraft.text,
      skills: nextDraft.skills,
      images: nextDraft.images,
    },
  };
}

export function clearComposerDraft(
  draftsByThreadId: ComposerDraftsByThreadId,
  threadId: string | null,
) {
  if (!threadId || !draftsByThreadId[threadId]) {
    return draftsByThreadId;
  }
  const next = { ...draftsByThreadId };
  delete next[threadId];
  return next;
}

export function isClearComposerCommand(draft: ComposerDraft) {
  return (
    draft.text.trim() === "/clear" &&
    draft.skills.length === 0 &&
    draft.images.length === 0
  );
}
