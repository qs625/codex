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

module.exports = {
  listThreads,
};
