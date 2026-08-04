const test = require("node:test");
const assert = require("node:assert/strict");

const {
  AppServerClient,
  buildAppServerEnvironment,
  resolveAppServerCommand,
} = require("./appServerClient.cjs");

test("app-server environment includes enhanced PATH and MORPHEUS_HOME", () => {
  const env = buildAppServerEnvironment(
    {
      MORPHEUS_HOME: "/tmp/morpheus-home",
      HOME: "/Users/alice",
      PATH: "/usr/bin",
    },
    {
      platform: "darwin",
      shellPath: "/shell/bin",
    },
  );

  assert.equal(env.MORPHEUS_HOME, "/tmp/morpheus-home");
  assert.ok(env.PATH.startsWith("/usr/bin:/shell/bin:"));
  assert.ok(env.PATH.includes("/opt/homebrew/bin"));
  assert.ok(env.PATH.includes("/usr/local/bin"));
});

test("app-server environment ignores CODEX_HOME for MORPHEUS_HOME", () => {
  const env = buildAppServerEnvironment({
    CODEX_HOME: "/tmp/legacy-codex-home",
    HOME: "/Users/alice",
    PATH: "/usr/bin",
  });

  assert.notEqual(env.MORPHEUS_HOME, "/tmp/legacy-codex-home");
  assert.ok(env.MORPHEUS_HOME.endsWith("/.morpheus"));
});

test("app-server command keeps APP_SERVER_CMD priority over CODEX_APP_SERVER_CMD", () => {
  assert.equal(
    resolveAppServerCommand({
      APP_SERVER_CMD: "/custom/app-server --listen stdio://",
      CODEX_APP_SERVER_CMD: "/other/app-server --listen stdio://",
    }),
    "/custom/app-server --listen stdio://",
  );
});

test("app-server command accepts CODEX_APP_SERVER_CMD when APP_SERVER_CMD is absent", () => {
  assert.equal(
    resolveAppServerCommand({
      CODEX_APP_SERVER_CMD: "/custom/codex-app-server --listen stdio://",
    }),
    "/custom/codex-app-server --listen stdio://",
  );
});

test("app-server client emits server requests for renderer approval handling", () => {
  const client = Object.create(AppServerClient.prototype);
  const requests = [];
  const writes = [];
  client.on("request", (request) => requests.push(request));
  client.write = (message) => writes.push(message);

  client.handleMessage({
    id: "approval-1",
    method: "item/commandExecution/requestApproval",
    params: { threadId: "thread-1" },
  });

  assert.deepEqual(requests, [
    {
      id: "approval-1",
      method: "item/commandExecution/requestApproval",
      params: { threadId: "thread-1" },
    },
  ]);
  assert.deepEqual(writes, []);
});

test("app-server client writes server request responses and rejections", async () => {
  const client = Object.create(AppServerClient.prototype);
  const writes = [];
  client.ready = async () => {};
  client.write = (message) => writes.push(message);

  await client.respondToServerRequest("approval-1", { decision: "accept" });
  await client.rejectServerRequest("approval-2", "unsupported");

  assert.deepEqual(writes, [
    { id: "approval-1", result: { decision: "accept" } },
    {
      id: "approval-2",
      error: {
        code: -32601,
        message: "unsupported",
      },
    },
  ]);
});
