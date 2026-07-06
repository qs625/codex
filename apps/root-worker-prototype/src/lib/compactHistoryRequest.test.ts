import test from "node:test";
import assert from "node:assert/strict";

import { advanceCompactHistoryRequestToken } from "./compactHistoryRequest";

test("compact history request tokens keep advancing across collapse and re-expand cycles", () => {
  const requestTokens = new Map<string, number>();
  const requestKey = "thread-1:compact-1";

  const firstExpand = advanceCompactHistoryRequestToken(requestTokens, requestKey);
  requestTokens.set(requestKey, firstExpand);
  assert.equal(firstExpand, 1);

  const collapseInvalidation = advanceCompactHistoryRequestToken(
    requestTokens,
    requestKey,
  );
  requestTokens.set(requestKey, collapseInvalidation);
  assert.equal(collapseInvalidation, 2);

  const secondExpand = advanceCompactHistoryRequestToken(requestTokens, requestKey);
  requestTokens.set(requestKey, secondExpand);
  assert.equal(secondExpand, 3);
});
