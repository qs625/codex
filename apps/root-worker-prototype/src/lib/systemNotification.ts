import {
  getRootThreadConversationTitle,
  normalizeProjectCwd,
  shouldNotifyProjectThreadCompleted,
} from "./thread";
import type { Thread } from "../types";

const PROJECT_THREAD_COMPLETED_TITLE = "Project thread completed";
const MAX_NOTIFICATION_BODY_LENGTH = 96;

export function maybeNotifyProjectThreadCompleted(
  thread: Thread | null,
  nextLifecycleStatus: Thread["lifecycleStatus"],
) {
  if (!thread || !shouldNotifyProjectThreadCompleted(thread, nextLifecycleStatus)) {
    return false;
  }

  notifyProjectThreadCompleted(thread);
  return true;
}

export function notifyProjectThreadCompleted(thread: Thread) {
  try {
    const showSystemNotification =
      window.codexDesktop?.showSystemNotification;
    if (typeof showSystemNotification !== "function") {
      return;
    }
    void showSystemNotification(
      buildProjectThreadCompletedNotificationPayload(thread),
    ).catch(() => {
      // Desktop notifications are best-effort and must not break UI state.
    });
  } catch {
    // Some runtimes may not expose the Electron preload bridge.
  }
}

export function buildProjectThreadCompletedNotificationPayload(thread: Thread) {
  return {
    title: PROJECT_THREAD_COMPLETED_TITLE,
    body: getProjectThreadCompletionNotificationBody(thread),
  };
}

function getProjectThreadCompletionNotificationBody(thread: Thread) {
  const label =
    thread.name?.trim() ||
    normalizeProjectCwd(thread.cwd) ||
    getRootThreadConversationTitle(thread);
  return trimNotificationBody(label);
}

function trimNotificationBody(label: string) {
  if (label.length <= MAX_NOTIFICATION_BODY_LENGTH) {
    return label;
  }
  return `...${label.slice(-(MAX_NOTIFICATION_BODY_LENGTH - 3))}`;
}
