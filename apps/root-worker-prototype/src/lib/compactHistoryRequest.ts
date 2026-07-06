export function advanceCompactHistoryRequestToken(
  requestTokens: ReadonlyMap<string, number>,
  requestKey: string,
) {
  return (requestTokens.get(requestKey) ?? 0) + 1;
}
