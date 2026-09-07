import type { NotificationEnvelope } from "../types";

export function clientLifecycleFailureReason(
  status: NotificationEnvelope["status"],
): string | null {
  if (status?.lifecycle?.phase !== "failed") {
    return null;
  }
  const reason = status.lifecycle.reason?.trim();
  return reason || "Runtime refresh failed.";
}
