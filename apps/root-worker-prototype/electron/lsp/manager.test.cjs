const test = require("node:test");
const assert = require("node:assert/strict");

const { LspManager } = require("./manager.cjs");

const TEST_ADAPTER = {
  id: "test",
  serverLabel: "Test Language Server",
  commands: [{ command: "test-lsp", args: [] }],
  languageIdForFile() {
    return "testlang";
  },
};

function createManager({
  adapter = TEST_ADAPTER,
  commandAvailable = true,
  execFileAsync = null,
  rootResolution = { workspaceRoot: "/workspace", reason: null },
  clientStatus = { phase: "ready", detail: "Ready" },
  definitionLocations = [
    { path: "/workspace/src/target.test", line: 3, column: 5 },
  ],
  readText = "source text",
} = {}) {
  const calls = {
    clientFactory: [],
    clientInitialize: 0,
    clientDefinition: [],
    execFile: [],
    readFile: [],
  };
  let client;

  const manager = new LspManager({
    adapterForFile: () => adapter,
    resolveWorkspaceRoot: async () => rootResolution,
    readFile: async (filePath, encoding) => {
      calls.readFile.push({ filePath, encoding });
      return readText;
    },
    execFileAsync:
      execFileAsync ??
      (async (command, args, options) => {
        calls.execFile.push({ command, args, options });
        if (!commandAvailable) {
          throw new Error("missing command");
        }
        return { stdout: "" };
      }),
    buildExecOptions: () => ({ env: { PATH: "/bin" } }),
    clientFactory: (params) => {
      calls.clientFactory.push(params);
      client = {
        getStatus() {
          return clientStatus;
        },
        initialize() {
          calls.clientInitialize += 1;
          return Promise.resolve();
        },
        async definition(args) {
          calls.clientDefinition.push(args);
          return definitionLocations;
        },
      };
      return client;
    },
  });

  return { calls, client: () => client, manager };
}

test("describeFile projects no-adapter files without command or client work", async () => {
  const { calls, manager } = createManager({ adapter: null });

  assert.deepEqual(await manager.describeFile("/workspace/README.md"), {
    enabled: false,
    languageId: null,
    lspStatus: {
      phase: "plain",
      detail: "No language server is configured for this file type.",
    },
    serverLabel: null,
    workspaceRoot: null,
    reason: "No LSP adapter is configured for this file type.",
  });
  assert.deepEqual(calls.execFile, []);
  assert.deepEqual(calls.clientFactory, []);
});

test("status projects command-unavailable target without starting a client", async () => {
  const { calls, manager } = createManager({ commandAvailable: false });

  assert.deepEqual(await manager.status("/workspace/src/main.test"), {
    enabled: false,
    lspStatus: {
      phase: "unavailable",
      detail: "Test Language Server is not available on PATH.",
    },
    reason: "Test Language Server is not available on PATH.",
    workspaceRoot: "/workspace",
  });
  assert.equal(calls.execFile.length, 1);
  assert.deepEqual(calls.clientFactory, []);
  assert.deepEqual(calls.readFile, []);
});

test("describeFile projects missing workspace root without probing commands", async () => {
  const { calls, manager } = createManager({
    rootResolution: {
      workspaceRoot: null,
      reason: "No project markers found.",
    },
  });

  assert.deepEqual(await manager.describeFile("/loose/main.test"), {
    enabled: false,
    languageId: "testlang",
    lspStatus: {
      phase: "plain",
      detail: "No project markers found.",
    },
    serverLabel: "Test Language Server",
    workspaceRoot: null,
    reason: "No project markers found.",
  });
  assert.deepEqual(calls.execFile, []);
  assert.deepEqual(calls.clientFactory, []);
});

test("definition resolves one file target and reuses the cached client", async () => {
  const { calls, manager } = createManager();

  const response = await manager.definition({
    filePath: "/workspace/src/main.test",
    line: 10,
    column: 4,
  });

  assert.deepEqual(response, {
    enabled: true,
    locations: [{ path: "/workspace/src/target.test", line: 3, column: 5 }],
    reason: null,
  });
  assert.equal(calls.execFile.length, 1);
  assert.equal(calls.clientFactory.length, 1);
  assert.deepEqual(calls.readFile, [
    { filePath: "/workspace/src/main.test", encoding: "utf8" },
  ]);
  assert.deepEqual(calls.clientDefinition, [
    {
      filePath: "/workspace/src/main.test",
      line: 10,
      column: 4,
      text: "source text",
    },
  ]);

  assert.deepEqual(await manager.status("/workspace/src/main.test"), {
    enabled: true,
    lspStatus: { phase: "ready", detail: "Ready" },
    reason: null,
    workspaceRoot: "/workspace",
  });
  assert.equal(calls.clientFactory.length, 1);
  assert.equal(calls.clientInitialize, 1);
});

test("resolveCommand targets use resolved commands for client cache cleanup", async () => {
  const adapter = {
    ...TEST_ADAPTER,
    commands: [
      {
        command: "test-lsp",
        args: ["--stdio"],
        resolveCommand: {
          command: "resolver",
          args: ["test-lsp"],
        },
      },
    ],
  };
  const { calls, manager } = createManager({
    adapter,
    execFileAsync: async (command, args, options) => {
      calls.execFile.push({ command, args, options });
      return { stdout: "/resolved/test-lsp\n" };
    },
  });

  await manager.describeFile("/workspace/src/main.test");
  await manager.status("/workspace/src/main.test");

  assert.equal(calls.clientFactory.length, 1);
  assert.equal(
    calls.clientFactory[0].commandSpec.command,
    "/resolved/test-lsp",
  );
  assert.equal(calls.execFile.length, 2);

  calls.clientFactory[0].onExit();
  await manager.status("/workspace/src/main.test");

  assert.equal(calls.clientFactory.length, 2);
  assert.equal(
    calls.clientFactory[1].commandSpec.command,
    "/resolved/test-lsp",
  );
});
