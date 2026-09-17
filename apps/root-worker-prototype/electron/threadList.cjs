const { buildThreadListParams } = require("./threadConfig.cjs");

async function listThreads(appServerClient, normalizeThread) {
  const threads = [];
  const seenCursors = new Set();
  let cursor = null;
  do {
    const response = await appServerClient.request(
      "thread/list",
      buildThreadListParams(cursor),
    );
    threads.push(...response.data.map(normalizeThread));
    cursor = response.nextCursor ?? null;
    if (cursor && seenCursors.has(cursor)) {
      throw new Error(`thread/list returned a repeated cursor: ${cursor}`);
    }
    if (cursor) {
      seenCursors.add(cursor);
    }
  } while (cursor);
  return threads;
}

async function listLoadedThreadIds(appServerClient) {
  const threadIds = [];
  const seenCursors = new Set();
  let cursor = null;
  do {
    const response = await appServerClient.request("thread/loaded/list", {
      ...(cursor ? { cursor } : {}),
      limit: 200,
    });
    threadIds.push(...(Array.isArray(response.data) ? response.data : []));
    cursor = response.nextCursor ?? null;
    if (cursor && seenCursors.has(cursor)) {
      throw new Error(`thread/loaded/list returned a repeated cursor: ${cursor}`);
    }
    if (cursor) {
      seenCursors.add(cursor);
    }
  } while (cursor);
  return threadIds;
}

module.exports = {
  listLoadedThreadIds,
  listThreads,
};
