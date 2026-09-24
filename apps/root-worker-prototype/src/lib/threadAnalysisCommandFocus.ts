import { findActiveCommandItem } from "./activeCommands";
import type { MonitorSummary } from "./threadAnalysis";
import type { Thread } from "../types";

export type ThreadAnalysisCommandFocusTarget = {
  threadId: string;
  commandItemId: string;
  processId?: string | null;
  command?: string | null;
  cwd?: string | null;
  status?: string | null;
};

export function resolveThreadAnalysisCommandFocus(
  thread: Thread | null,
  monitor: MonitorSummary,
): ThreadAnalysisCommandFocusTarget | null {
  if (monitor.kind !== "command") {
    return null;
  }
  if (!thread) {
    return null;
  }
  const command = findActiveCommandItem(thread, monitor.id);
  return {
    threadId: thread.id,
    commandItemId: monitor.id,
    processId: command ? command.processId : null,
    command: command ? command.command : null,
    cwd: command ? command.cwd : null,
    status: command ? command.status : null,
  };
}
