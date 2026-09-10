export type TerminalCommandFocusRequest = {
  threadId: string;
  commandItemId: string;
  processId?: string | null;
  command?: string | null;
  cwd?: string | null;
  status?: string | null;
  token: number;
};

export function isTerminalCommandFocusRequestForThread(
  request: TerminalCommandFocusRequest | null | undefined,
  threadId: string | null | undefined,
) {
  return Boolean(request && threadId && request.threadId === threadId);
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
