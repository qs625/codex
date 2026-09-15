import type { Thread, ThreadItem } from "../types";

export type CommandExecutionItem = Extract<
  ThreadItem,
  { type: "commandExecution" }
>;

export function normalizedCommandExecutionStatus(status: string) {
  return status.trim().toLowerCase().replace(/[_-]/g, "");
}

export function isRunningCommandExecutionStatus(status: string) {
  const normalized = normalizedCommandExecutionStatus(status);
  return normalized === "running" || normalized === "inprogress";
}

export function isCommandExecutionItem(
  item: ThreadItem,
): item is CommandExecutionItem {
  return item.type === "commandExecution";
}

export function selectActiveCommandItems(
  thread: Pick<Thread, "activeCommandItems"> | null | undefined,
) {
  return dedupeCommandExecutionItems(thread?.activeCommandItems ?? []);
}

export function selectRunningActiveCommandItems(
  thread: Pick<Thread, "activeCommandItems"> | null | undefined,
) {
  return selectActiveCommandItems(thread).filter((item) =>
    isRunningCommandExecutionStatus(item.status),
  );
}

export function findActiveCommandItem(
  thread: Pick<Thread, "activeCommandItems"> | null | undefined,
  commandItemId: string,
) {
  return (
    selectActiveCommandItems(thread).find((item) => item.id === commandItemId) ??
    null
  );
}

export function countActiveCommandItemsWithProcess(
  thread: Pick<Thread, "activeCommandItems"> | null | undefined,
) {
  return selectActiveCommandItems(thread).filter((item) =>
    Boolean(item.processId),
  ).length;
}

export function dedupeCommandExecutionItems(items: ThreadItem[]) {
  const itemsById = new Map<string, CommandExecutionItem>();
  for (const item of items) {
    if (isCommandExecutionItem(item)) {
      itemsById.set(item.id, item);
    }
  }
  return Array.from(itemsById.values());
}
