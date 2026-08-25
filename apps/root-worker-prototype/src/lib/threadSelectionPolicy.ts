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

  if (!isLoaded && !isSubscribed) {
    return "readAndSubscribe";
  }

  if (!isLoaded) {
    return "readAndSubscribe";
  }

  if (hasLiveCache) {
    return isSubscribed ? "none" : "subscribeOnly";
  }

  if (!isSubscribed) {
    return "subscribeOnly";
  }

  return "none";
}

type ThreadReadSnapshotPolicyInput = {
  threadId: string;
  selectedThreadId: string | null;
  requestId: number;
  latestRequestId: number | null;
  isLoaded: boolean;
};

export function shouldApplyThreadReadSnapshot({
  threadId,
  selectedThreadId,
  requestId,
  latestRequestId,
  isLoaded,
}: ThreadReadSnapshotPolicyInput) {
  return (
    selectedThreadId === threadId &&
    requestId === latestRequestId &&
    !isLoaded
  );
}

export function nextThreadReadRequestId(
  requestIdsByThreadId: ReadonlyMap<string, number>,
  threadId: string,
) {
  return (requestIdsByThreadId.get(threadId) ?? 0) + 1;
}

export function isSelectedThreadLoading(
  selectedThreadId: string | null,
  loadingThreadIds: ReadonlySet<string>,
) {
  return selectedThreadId !== null && loadingThreadIds.has(selectedThreadId);
}
