export const CONVERSATION_STICK_TO_BOTTOM_THRESHOLD_PX = 24;

export type ConversationScrollMetrics = {
  scrollHeight: number;
  clientHeight: number;
  scrollTop: number;
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
