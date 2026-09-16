export type TerminalCommandFocusRequest = {
  threadId: string;
  commandItemId: string;
  processId?: string | null;
  command?: string | null;
  cwd?: string | null;
  status?: string | null;
  token: number;
};

export type PendingTerminalViewportFocusRequest = {
  token: number;
  tabId: string | null;
};

export function isTerminalCommandFocusRequestForThread(
  request: TerminalCommandFocusRequest | null | undefined,
  threadId: string | null | undefined,
) {
  return Boolean(request && threadId && request.threadId === threadId);
}

export function shouldApplyTerminalViewportFocusRequest({
  request,
  lastAppliedToken,
  activeTabId,
  terminalAvailable,
}: {
  request: PendingTerminalViewportFocusRequest | null | undefined;
  lastAppliedToken: number;
  activeTabId: string | null | undefined;
  terminalAvailable: boolean;
}) {
  return Boolean(
    request &&
      request.token > lastAppliedToken &&
      terminalAvailable &&
      (!request.tabId || request.tabId === activeTabId),
  );
}

export function createTerminalStateRequestSequencer() {
  let current = 0;
  return {
    begin() {
      current += 1;
      return current;
    },
    isCurrent(requestSeq: number) {
      return requestSeq === current;
    },
  };
}
