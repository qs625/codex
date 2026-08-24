const test = require("node:test");
const assert = require("node:assert/strict");

const { listThreads } = require("./threadList.cjs");

test("listThreads reads every thread/list page", async () => {
  const requests = [];
  const appServerClient = {
    async request(method, params) {
      requests.push({ method, params });
      if (!params.cursor) {
        return {
          data: [{ id: "thread-1" }],
          nextCursor: "cursor-2",
        };
      }
      assert.equal(params.cursor, "cursor-2");
      return {
        data: [{ id: "thread-2" }],
        nextCursor: null,
      };
    },
  };

  const threads = await listThreads(appServerClient, (thread) => ({
    id: thread.id,
    normalized: true,
  }));

  assert.deepEqual(threads, [
    { id: "thread-1", normalized: true },
    { id: "thread-2", normalized: true },
  ]);
  assert.deepEqual(
    requests.map((request) => ({
      method: request.method,
      cursor: request.params.cursor ?? null,
      useStateDbOnly: request.params.useStateDbOnly,
    })),
    [
      { method: "thread/list", cursor: null, useStateDbOnly: true },
      { method: "thread/list", cursor: "cursor-2", useStateDbOnly: true },
    ],
  );
});

test("listThreads rejects repeated thread/list cursors", async () => {
  const appServerClient = {
    async request() {
      return {
        data: [],
        nextCursor: "cursor-repeat",
      };
    },
  };

  await assert.rejects(
    () => listThreads(appServerClient, (thread) => thread),
    /repeated cursor/,
  );
});
