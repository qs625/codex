export type ThreadSelectionAction =
  | "none"
  | "subscribeOnly"
  | "readAndSubscribe";

type ThreadSelectionPolicyInput = {
  selectedThreadId: string | null;
  hasLocalThread: boolean;
  isLoaded: boolean;
  isSubscribed: boolean;
  isLoading: boolean;
};

export function decideThreadSelectionAction({
  selectedThreadId,
  hasLocalThread,
  isLoaded,
  isSubscribed,
  isLoading,
}: ThreadSelectionPolicyInput): ThreadSelectionAction {
  if (!selectedThreadId) {
    return "none";
  }

  if (isLoading) {
    return "none";
  }

  if (!hasLocalThread || !isLoaded) {
    return "readAndSubscribe";
  }

  if (!isSubscribed) {
    return "subscribeOnly";
  }

  return "none";
}
