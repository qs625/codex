const test = require("node:test");
const assert = require("node:assert/strict");

const {
  applyProgressNotification,
  createClientStatus,
  markClientError,
  markClientReady,
  snapshotClientStatus,
} = require("./status.cjs");

test("lsp status reports indexing until active progress completes", () => {
  const status = createClientStatus();

  applyProgressNotification(status, {
    method: "$/progress",
    params: {
      token: "index-1",
      value: {
        kind: "begin",
        title: "Indexing",
      },
    },
  });
  markClientReady(status);
  assert.deepEqual(snapshotClientStatus(status), {
    phase: "indexing",
    detail: "Indexing",
  });

  applyProgressNotification(status, {
    method: "$/progress",
    params: {
      token: "index-1",
      value: {
        kind: "end",
      },
    },
  });
  assert.deepEqual(snapshotClientStatus(status), {
    phase: "ready",
    detail: "Ready",
  });
});

test("lsp status captures terminal errors", () => {
  const status = createClientStatus();
  markClientError(status, "rust-analyzer exited");

  assert.deepEqual(snapshotClientStatus(status), {
    phase: "error",
    detail: "rust-analyzer exited",
  });
});
