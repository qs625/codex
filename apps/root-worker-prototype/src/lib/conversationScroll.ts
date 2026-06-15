export const CONVERSATION_STICK_TO_BOTTOM_THRESHOLD_PX = 24;

export type ConversationScrollMetrics = {
  scrollHeight: number;
  clientHeight: number;
  scrollTop: number;
};

export type ConversationHeightChangeScrollPlan = {
  scrollTopAdjustment: number;
  shouldStickToBottom: boolean;
};

export function isConversationNearBottom({
  scrollHeight,
  clientHeight,
  scrollTop,
}: ConversationScrollMetrics) {
  return (
    scrollHeight - clientHeight - scrollTop <=
    CONVERSATION_STICK_TO_BOTTOM_THRESHOLD_PX
  );
}

export function planConversationHeightChangeScroll({
  cellTop,
  heightDelta,
  metrics,
}: {
  cellTop: number;
  heightDelta: number;
  metrics: ConversationScrollMetrics;
}): ConversationHeightChangeScrollPlan {
  const shouldStickToBottom = isConversationNearBottom(metrics);
  if (shouldStickToBottom) {
    return {
      scrollTopAdjustment: 0,
      shouldStickToBottom,
    };
  }

  return {
    scrollTopAdjustment: cellTop < metrics.scrollTop ? heightDelta : 0,
    shouldStickToBottom,
  };
}
