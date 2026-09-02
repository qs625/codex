const test = require("node:test");
const assert = require("node:assert/strict");

const {
  ensureSelfProjectThread,
  isSelfProjectThread,
} = require("./selfProjectThread.cjs");

const selfProject = {
  id: "/self",
  path: "/self",
  workspace: "/Users/example/.morpheus/source_workspace",
  hidden: true,
  system: true,
};

test("isSelfProjectThread matches self roots without agentPath", () => {
  assert.equal(
    isSelfProjectThread(
      {
        id: "self-root",
        name: "/self",
        path: "/self",
        cwd: "/Users/example/.morpheus/source_workspace/",
        agentPath: null,
      },
      selfProject,
    ),
    true,
  );
});

test("isSelfProjectThread does not match ordinary same-workspace roots", () => {
  assert.equal(
    isSelfProjectThread(
      {
        id: "workspace-root",
        name: "Project chat",
        path: null,
        cwd: "/Users/example/.morpheus/source_workspace",
        agentPath: null,
      },
      selfProject,
    ),
    false,
  );
});

test("ensureSelfProjectThread reuses an existing self root", async () => {
  const requests = [];
  const selfRoot = {
    id: "self-root",
    name: "/self",
    path: null,
    cwd: "/Users/example/.morpheus/source_workspace",
    agentPath: null,
  };

  const result = await ensureSelfProjectThread(
    {
      async request(method, params) {
        requests.push({ method, params });
        throw new Error("unexpected app-server request");
      },
    },
    (thread) => thread,
    selfProject,
    [selfRoot],
  );

  assert.equal(result.created, false);
  assert.equal(result.thread, selfRoot);
  assert.deepEqual(result.threads, [selfRoot]);
  assert.deepEqual(requests, []);
});

test("ensureSelfProjectThread creates and names a real self root when missing", async () => {
  const requests = [];
  const workspaceRoot = {
    id: "workspace-root",
    name: "Project chat",
    path: null,
    cwd: "/Users/example/.morpheus/source_workspace",
    agentPath: null,
  };
  const appServerClient = {
    async request(method, params) {
      requests.push({ method, params });
      if (method === "thread/start") {
        return {
          thread: {
            id: "created-self-root",
            cwd: "/Users/example/.morpheus/source_workspace",
            path: null,
            agentPath: null,
          },
          model: "gpt-5",
          modelProvider: "openai",
          reasoningEffort: null,
        };
      }
      assert.equal(method, "thread/name/set");
      assert.equal(params.threadId, "created-self-root");
      assert.equal(params.name, "/self");
      return {};
    },
  };

  const result = await ensureSelfProjectThread(
    appServerClient,
    (thread, runtime) => ({ ...thread, runtime }),
    selfProject,
    [workspaceRoot],
  );

  assert.equal(result.created, true);
  assert.equal(result.thread.id, "created-self-root");
  assert.equal(result.thread.name, "/self");
  assert.deepEqual(result.runtime, {
    model: "gpt-5",
    modelProvider: "openai",
    reasoningEffort: null,
  });
  assert.deepEqual(
    result.threads.map((thread) => thread.id),
    ["created-self-root", "workspace-root"],
  );
  assert.equal(requests[0].method, "thread/start");
  assert.equal(requests[0].params.cwd, selfProject.workspace);
  assert.equal(requests[0].params.taskName, "self");
  assert.equal(requests[1].method, "thread/name/set");
});
