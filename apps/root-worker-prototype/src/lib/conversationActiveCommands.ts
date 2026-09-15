import type { ConversationEntry, Thread, ThreadItem } from "../types";
import { selectActiveCommandItems } from "./activeCommands";
import { isLegacyOrphanCommandOutputPlaceholder } from "./conversationToolPresentation";

export type ConversationFlatItemState = {
  id: string;
  item: ThreadItem;
  timestamp: string;
  entries: ConversationEntry[];
};

type PreviousActiveCommandFlatItemState = ConversationFlatItemState & {
  itemSignature: string;
};

type BuildConversationItemEntries = (
  item: ThreadItem,
  options: {
    author: string;
    timestamp: string;
    commandLookup: Map<string, string>;
  },
) => ConversationEntry[];

export function buildActiveCommandConversationTail({
  thread,
  author,
  timestamp,
  commandLookup,
  historyItemIds,
  previous,
  buildItemEntries,
}: {
  thread: Pick<Thread, "activeCommandItems">;
  author: string;
  timestamp: string;
  commandLookup: Map<string, string>;
  historyItemIds: ReadonlySet<string>;
  previous: { flatItems: ConversationFlatItemState[] } | null | undefined;
  buildItemEntries: BuildConversationItemEntries;
}) {
  const previousFlatItems = buildPreviousActiveCommandFlatItemLookup(previous);
  const flatItems: ConversationFlatItemState[] = [];
  const entries: ConversationEntry[] = [];

  for (const item of selectActiveCommandItems(thread)) {
    if (
      historyItemIds.has(item.id) ||
      isLegacyOrphanCommandOutputPlaceholder(item)
    ) {
      continue;
    }
    const previousFlatItem = previousFlatItems.get(item.id);
    const activeTurnId = activeCommandTurnId(item.id);
    const rebuiltEntries =
      previousFlatItem &&
      previousFlatItem.id === item.id &&
      previousFlatItem.itemSignature === threadItemSignature(item) &&
      previousFlatItem.timestamp === timestamp
        ? previousFlatItem.entries
        : buildItemEntries(item, {
            author,
            timestamp,
            commandLookup,
          }).map((entry) => ({
            ...entry,
            turnId: activeTurnId,
          }));

    flatItems.push({
      id: item.id,
      item,
      timestamp,
      entries: rebuiltEntries,
    });
    entries.push(...rebuiltEntries);
  }

  return {
    flatItems,
    entries,
  };
}

function buildPreviousActiveCommandFlatItemLookup(
  previous: { flatItems: ConversationFlatItemState[] } | null | undefined,
) {
  const lookup = new Map<string, PreviousActiveCommandFlatItemState>();
  for (const flatItem of previous?.flatItems ?? []) {
    if (
      flatItem.item.type === "commandExecution" &&
      flatItem.entries.every(
        (entry) => entry.turnId === activeCommandTurnId(flatItem.id),
      )
    ) {
      lookup.set(flatItem.id, {
        ...flatItem,
        itemSignature: threadItemSignature(flatItem.item),
      });
    }
  }
  return lookup;
}

function activeCommandTurnId(commandItemId: string) {
  return `active-command:${commandItemId}`;
}

function threadItemSignature(item: ThreadItem) {
  return JSON.stringify(item);
}
