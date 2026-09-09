const test = require("node:test");
const assert = require("node:assert/strict");

const {
  ensureSelfProjectThread,
  isSelfProjectThread,
  sendSelfCommandToThread,
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

test("ensureSelfProjectThread ignores an earlier /self root from another workspace", async () => {
  const requests = [];
  const otherWorkspaceSelfRoot = {
    id: "other-self-root",
    name: "/self",
    path: "/self",
    cwd: "/Users/example/other-workspace",
    agentPath: null,
  };
  const currentWorkspaceSelfRoot = {
    id: "current-self-root",
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
    [otherWorkspaceSelfRoot, currentWorkspaceSelfRoot],
  );

  assert.equal(result.created, false);
  assert.equal(result.thread, currentWorkspaceSelfRoot);
  assert.deepEqual(result.threads, [
    otherWorkspaceSelfRoot,
    currentWorkspaceSelfRoot,
  ]);
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

test("sendSelfCommandToThread reuses an existing self root and starts a turn", async () => {
  const requests = [];
  const loadedThreadIds = [];
  const turns = [];
  const rememberedRuntime = [];
  const selfRoot = {
    id: "self-root",
    name: "/self",
    path: "/self",
    cwd: "/Users/example/.morpheus/source_workspace",
    agentPath: null,
    model: "gpt-5",
    modelProvider: "openai",
    reasoningEffort: "medium",
  };

  const result = await sendSelfCommandToThread({
    appServerClient: {
      async request(method, params) {
        requests.push({ method, params });
        throw new Error("unexpected app-server request");
      },
    },
    buildTurnInput(payload) {
      return [{ type: "text", text: payload.text, text_elements: [] }];
    },
    async loadThreadForTurn(threadId) {
      loadedThreadIds.push(threadId);
      return {
        ...selfRoot,
        model: "gpt-5.5",
        modelProvider: "modelhub-gpt",
        reasoningEffort: "high",
      };
    },
    normalizeThread: (thread) => thread,
    project: selfProject,
    rememberThreadRuntime(threadId, runtime) {
      rememberedRuntime.push({ threadId, runtime });
    },
    async startThreadTurn(payload, input) {
      turns.push({ payload, input });
      return { id: "turn-1" };
    },
    text: "  fix packaging  ",
    threads: [selfRoot],
  });

  assert.equal(result.materializedSelfThreadId, null);
  assert.deepEqual(loadedThreadIds, ["self-root"]);
  assert.equal(result.thread.model, "gpt-5.5");
  assert.equal(result.thread.modelProvider, "modelhub-gpt");
  assert.equal(result.thread.reasoningEffort, "high");
  assert.equal(result.turn.id, "turn-1");
  assert.deepEqual(requests, []);
  assert.deepEqual(rememberedRuntime, []);
  assert.deepEqual(turns, [
    {
      payload: {
        threadId: "self-root",
        model: "gpt-5.5",
        modelProvider: "modelhub-gpt",
        effort: "high",
        text: "fix packaging",
        skills: [],
        images: [],
      },
      input: [{ type: "text", text: "fix packaging", text_elements: [] }],
    },
  ]);
});

test("sendSelfCommandToThread materializes without putting user text in thread start params", async () => {
  const requests = [];
  const turns = [];
  const rememberedRuntime = [];
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
      return {};
    },
  };

  const result = await sendSelfCommandToThread({
    appServerClient,
    buildTurnInput(payload) {
      return [{ type: "text", text: payload.text, text_elements: [] }];
    },
    async loadThreadForTurn() {
      throw new Error("newly materialized self root should already be live");
    },
    normalizeThread: (thread, runtime) => ({ ...thread, runtime }),
    project: selfProject,
    rememberThreadRuntime(threadId, runtime) {
      rememberedRuntime.push({ threadId, runtime });
    },
    async startThreadTurn(payload, input) {
      turns.push({ payload, input });
      return { id: "turn-1" };
    },
    text: "fix packaging",
    threads: [],
  });

  assert.equal(result.materializedSelfThreadId, "created-self-root");
  assert.equal(result.thread.id, "created-self-root");
  assert.deepEqual(rememberedRuntime, [
    {
      threadId: "created-self-root",
      runtime: {
        model: "gpt-5",
        modelProvider: "openai",
        reasoningEffort: null,
      },
    },
  ]);
  assert.equal(requests[0].method, "thread/start");
  assert.equal(requests[0].params.cwd, selfProject.workspace);
  assert.equal(requests[0].params.taskName, "self");
  assert.equal(
    JSON.stringify(requests[0].params).includes("fix packaging"),
    false,
  );
  assert.equal(requests[1].method, "thread/name/set");
  assert.equal(turns[0].payload.threadId, "created-self-root");
  assert.equal(turns[0].payload.text, "fix packaging");
});

test("sendSelfCommandToThread rejects blank input before materializing", async () => {
  const requests = [];
  await assert.rejects(
    () =>
      sendSelfCommandToThread({
        appServerClient: {
          async request(method, params) {
            requests.push({ method, params });
            throw new Error("unexpected app-server request");
          },
        },
        buildTurnInput: () => [],
        loadThreadForTurn: async () => {
          throw new Error("blank input should not load a thread");
        },
        normalizeThread: (thread) => thread,
        project: selfProject,
        rememberThreadRuntime: () => {},
        startThreadTurn: async () => ({}),
        text: "   ",
        threads: [],
      }),
    /Self command requires task text/,
  );
  assert.deepEqual(requests, []);
});
