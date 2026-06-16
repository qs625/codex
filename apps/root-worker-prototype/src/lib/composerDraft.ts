import type { ComposerImage, DraftSkill } from "../types";
import type { ThreadGoalStatus } from "../types";

export type ComposerDraft = {
  text: string;
  skills: DraftSkill[];
  images: ComposerImage[];
};

export type ComposerDraftsByThreadId = Record<string, ComposerDraft>;

export type GoalComposerCommand =
  | {
      type: "set";
      objective: string;
      status: Extract<ThreadGoalStatus, "active">;
    }
  | {
      type: "status";
      status: Extract<ThreadGoalStatus, "active" | "paused">;
    }
  | {
      type: "clear";
    }
  | {
      type: "invalid";
      message: string;
    };

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

export function isGoalCancelComposerCommand(draft: ComposerDraft) {
  return parseGoalComposerCommand(draft)?.type === "clear";
}

export function parseGoalComposerCommand(
  draft: ComposerDraft,
): GoalComposerCommand | null {
  if (draft.skills.length > 0 || draft.images.length > 0) {
    return null;
  }

  const text = draft.text.trim();
  const command = text.toLowerCase();
  if (command === "/goal") {
    return { type: "invalid", message: "Enter a goal objective." };
  }
  if (command === "/cancel-goal") {
    return { type: "clear" };
  }

  return (
    parseExactGoalAction(command) ??
    parseGoalObjective(text)
  );
}

function parseExactGoalAction(command: string): GoalComposerCommand | null {
  switch (command) {
    case "/goal pause":
      return { type: "status", status: "paused" };
    case "/goal resume":
      return { type: "status", status: "active" };
    case "/goal cancel":
    case "/goal clear":
      return { type: "clear" };
    default:
      return null;
  }
}

function parseGoalObjective(text: string): GoalComposerCommand | null {
  if (!text.toLowerCase().startsWith("/goal ")) {
    return null;
  }

  const objective = text.slice("/goal ".length).trim();
  if (!objective) {
    return null;
  }

  return { type: "set", objective, status: "active" };
}
