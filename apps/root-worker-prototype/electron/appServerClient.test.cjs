const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  AppServerClient,
  buildDefaultAppServerCommand,
  buildAppServerEnvironment,
  buildMobileConnectionLaunch,
  resolveAppServerCommand,
  resolveAppServerLaunch,
  resolveLanEndpoint,
  writeTokenFile,
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

test("default app-server launch adds authenticated same-runtime mobile websocket listener", async () => {
  const writes = [];
  const mobileLaunch = await buildMobileConnectionLaunch(
    {
      ROOT_WORKER_MOBILE_LISTEN: "ws://0.0.0.0:8910",
      ROOT_WORKER_MOBILE_ENDPOINT: "wss://tunnel.example/root-worker",
      ROOT_WORKER_MOBILE_TOKEN: "test-token",
    },
    {
      morpheusHome: "/tmp/morpheus-home",
      writeTokenFile: (tokenFile, token) => writes.push({ tokenFile, token }),
      checkListenAvailable: async () => true,
    },
  );

  assert.equal(mobileLaunch.enabled, true);
  assert.deepEqual(writes, [
    {
      tokenFile: "/tmp/morpheus-home/root-worker-mobile-ws-token",
      token: "test-token",
    },
  ]);
  assert.deepEqual(mobileLaunch.info, {
    enabled: true,
    bindEndpoint: "ws://0.0.0.0:8910",
    endpoint: "wss://tunnel.example/root-worker",
    token: "test-token",
    auth: "capability-token",
  });
  assert.equal(
    buildDefaultAppServerCommand({
      appServerBinary: "/app-server",
      mobileLaunch,
    }),
    '/app-server --listen stdio:// --mobile-listen "ws://0.0.0.0:8910" --ws-auth capability-token --ws-token-file "/tmp/morpheus-home/root-worker-mobile-ws-token"',
  );
});

test("default app-server launch disables mobile listener when the bind is unavailable", async () => {
  const writes = [];
  const mobileLaunch = await buildMobileConnectionLaunch(
    {
      ROOT_WORKER_MOBILE_LISTEN: "ws://0.0.0.0:8910",
      ROOT_WORKER_MOBILE_TOKEN: "test-token",
    },
    {
      morpheusHome: "/tmp/morpheus-home",
      writeTokenFile: (tokenFile, token) => writes.push({ tokenFile, token }),
      checkListenAvailable: async () => false,
    },
  );

  assert.equal(mobileLaunch.enabled, false);
  assert.deepEqual(writes, []);
  assert.deepEqual(mobileLaunch.info, {
    enabled: false,
    reason: "Mobile listener bind address is unavailable: ws://0.0.0.0:8910.",
  });
  assert.equal(
    buildDefaultAppServerCommand({
      appServerBinary: "/app-server",
      mobileLaunch,
    }),
    "/app-server --listen stdio://",
  );
});

test("default app-server launch disables mobile listener for non-socket-address listen URLs", async () => {
  for (const listenUrl of [
    "ws://localhost:8910",
    "ws://127.0.0.1:8910/path",
  ]) {
    const writes = [];
    const mobileLaunch = await buildMobileConnectionLaunch(
      {
        ROOT_WORKER_MOBILE_LISTEN: listenUrl,
        ROOT_WORKER_MOBILE_TOKEN: "test-token",
      },
      {
        morpheusHome: "/tmp/morpheus-home",
        writeTokenFile: (tokenFile, token) => writes.push({ tokenFile, token }),
        checkListenAvailable: async () => true,
      },
    );

    assert.equal(mobileLaunch.enabled, false);
    assert.deepEqual(writes, []);
    assert.deepEqual(mobileLaunch.info, {
      enabled: false,
      reason: "ROOT_WORKER_MOBILE_LISTEN must be a ws://IP:PORT bind URL.",
    });
    assert.equal(
      buildDefaultAppServerCommand({
        appServerBinary: "/app-server",
        mobileLaunch,
      }),
      "/app-server --listen stdio://",
    );
  }
});

test("custom app-server command leaves mobile listener ownership to the override", async () => {
  const launch = await resolveAppServerLaunch({
    APP_SERVER_CMD: "/custom/app-server --listen stdio://",
  });

  assert.equal(launch.command, "/custom/app-server --listen stdio://");
  assert.deepEqual(launch.mobileConnection, {
    enabled: false,
    reason:
      "Mobile listener is unavailable when APP_SERVER_CMD overrides app-server launch.",
  });
});

test("mobile websocket token file permissions are tightened on rewrite", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-mobile-token-"));
  const tokenFile = path.join(dir, "token");
  fs.writeFileSync(tokenFile, "old-token\n", { mode: 0o644 });

  writeTokenFile(tokenFile, "new-token");

  assert.equal(fs.readFileSync(tokenFile, "utf8"), "new-token\n");
  assert.equal(fs.statSync(tokenFile).mode & 0o777, 0o600);
});

test("mobile websocket endpoint resolves wildcard binds to a reachable host", () => {
  assert.equal(
    resolveLanEndpoint("ws://0.0.0.0:8910", {
      ROOT_WORKER_MOBILE_ENDPOINT: "wss://tunnel.example/root-worker",
    }),
    "wss://tunnel.example/root-worker",
  );
  assert.match(resolveLanEndpoint("ws://127.0.0.1:8910"), /^ws:\/\/127\.0\.0\.1:8910\/$/);
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
