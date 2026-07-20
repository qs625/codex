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
    const notificationApi = globalThis.Notification;
    if (typeof notificationApi !== "function") {
      return;
    }

    const send = () => {
      try {
        new notificationApi(PROJECT_THREAD_COMPLETED_TITLE, {
          body: getProjectThreadCompletionNotificationBody(thread),
        });
      } catch {
        // Desktop notifications are best-effort and must not break UI state.
      }
    };

    if (notificationApi.permission === "granted") {
      send();
      return;
    }

    if (
      notificationApi.permission === "default" &&
      typeof notificationApi.requestPermission === "function"
    ) {
      void notificationApi
        .requestPermission()
        .then((permission) => {
          if (permission === "granted") {
            send();
          }
        })
        .catch(() => {
          // Permission prompts may be denied or unavailable in some shells.
        });
    }
  } catch {
    // Some runtimes expose Notification partially or behind permissions.
  }
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
