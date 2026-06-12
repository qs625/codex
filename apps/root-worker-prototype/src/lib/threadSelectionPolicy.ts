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
  hasLiveCache: boolean;
};

export function decideThreadSelectionAction({
  selectedThreadId,
  hasLocalThread,
  isLoaded,
  isSubscribed,
  isLoading,
  hasLiveCache,
}: ThreadSelectionPolicyInput): ThreadSelectionAction {
  if (!selectedThreadId) {
    return "none";
  }

  if (isLoading) {
    return "none";
  }

  if (!hasLocalThread) {
    return "readAndSubscribe";
  }

  if (hasLiveCache) {
    return isSubscribed ? "none" : "subscribeOnly";
  }

  if (!isLoaded && !isSubscribed) {
    return "readAndSubscribe";
  }

  if (!isSubscribed) {
    return "subscribeOnly";
  }

  if (!isLoaded) {
    return "none";
  }

  return "none";
}
