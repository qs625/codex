const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { EventEmitter } = require("node:events");

const {
  AppServerClient,
  buildDefaultAppServerCommand,
  buildAppServerEnvironment,
  buildMobileConnectionLaunch,
  ensureMorpheusHomeDefaults,
  prepareAppServerWorkspace,
  refreshMobileConnectionInfo,
  resolveDefaultAppServerBinary,
  resolveAppServerCommand,
  resolveAppServerLaunch,
  resolveLanEndpoint,
  writeTokenFile,
} = require("./appServerClient.cjs");

function networkInterfaces(address) {
  return {
    en0: [
      {
        family: "IPv4",
        internal: false,
        address,
      },
    ],
  };
}

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

test("app-server environment supports sparse desktop launch env", () => {
  const env = buildAppServerEnvironment(
    {
      HOME: "/Users/alice",
      OPENAI_API_KEY: "keep-key",
      PATH: "/usr/bin",
    },
    {
      platform: "darwin",
      shellPath: "",
    },
  );

  assert.equal(env.HOME, "/Users/alice");
  assert.equal(env.OPENAI_API_KEY, "keep-key");
  assert.equal(env.MORPHEUS_HOME, path.join("/Users/alice", ".morpheus"));
  assert.ok(env.PATH.startsWith("/usr/bin:"));
  assert.ok(env.PATH.includes("/opt/homebrew/bin"));
  assert.ok(env.PATH.includes("/Users/alice/.cargo/bin"));
  assert.equal(env.APP_SERVER_CMD, undefined);
  assert.equal(env.CODEX_APP_SERVER_CMD, undefined);
});

test("app-server environment supplies HOME when desktop env omits it", () => {
  const env = buildAppServerEnvironment(
    {
      OPENAI_API_KEY: "keep-key",
      PATH: "/usr/bin",
    },
    {
      home: "/Users/alice",
      platform: "darwin",
      shellPath: "",
    },
  );

  assert.equal(env.HOME, "/Users/alice");
  assert.equal(env.OPENAI_API_KEY, "keep-key");
  assert.equal(env.MORPHEUS_HOME, path.join("/Users/alice", ".morpheus"));
  assert.ok(env.PATH.includes("/Users/alice/.cargo/bin"));
  assert.ok(env.PATH.includes("/opt/homebrew/bin"));
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

test("default app-server binary prefers packaged resources", () => {
  const resourcesPath = "/Applications/Root Worker.app/Contents/Resources";
  const packagedBinary = path.join(resourcesPath, "bin/app-server");
  const workspaceBinary = "/repo/codex-rs/target/debug/app-server";
  const seen = [];

  assert.equal(
    resolveDefaultAppServerBinary({
      resourcesPath,
      startDir: "/repo/apps/root-worker-prototype/electron",
      existsSync: (candidate) => {
        seen.push(candidate);
        return candidate === packagedBinary || candidate === workspaceBinary;
      },
    }),
    `"${packagedBinary}"`,
  );
  assert.deepEqual(seen, [packagedBinary]);
});

test("default app-server binary follows the selected external runtime resources", () => {
  const runtimeRoot = "/tmp/runtime-launcher/artifacts/build-2";
  const resourcesPath = path.join(
    runtimeRoot,
    "payload/Root Worker Runtime.app/Contents/Resources",
  );
  const packagedBinary = path.join(resourcesPath, "bin/app-server");

  assert.equal(
    resolveDefaultAppServerBinary({
      resourcesPath,
      existsSync: (candidate) => candidate === packagedBinary,
    }),
    `"${packagedBinary}"`,
  );
});

test("default app-server launch uses bundled binary without PATH dependency", async () => {
  const resourcesPath = "/Applications/Root Worker.app/Contents/Resources";
  const packagedBinary = path.join(resourcesPath, "bin/app-server");

  const launch = await resolveAppServerLaunch(
    {
      HOME: "/Users/alice",
      PATH: "/usr/bin",
      ROOT_WORKER_MOBILE_LISTEN: "off",
    },
    {
      resourcesPath,
      existsSync: (candidate) => candidate === packagedBinary,
    },
  );

  assert.equal(launch.command, `"${packagedBinary}" --listen stdio://`);
  assert.equal(launch.mobileConnection.enabled, false);
});

test("packaged app-server launch without source uses operational cwd and no self project", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-packaged-workspace-"));
  const resourcesPath = path.join(dir, "resources");
  const packagedBinary = path.join(resourcesPath, "bin/app-server");
  const morpheusHome = path.join(dir, "home");
  const env = {
    MORPHEUS_HOME: morpheusHome,
  };
  const calls = [];

  const cwd = prepareAppServerWorkspace(env, {
    cwd: "/source-tree",
    resourcesPath,
    existsSync: (candidate) =>
      candidate === packagedBinary || fs.existsSync(candidate),
    spawnSync: (...args) => calls.push(args),
  });

  const workspace = path.join(morpheusHome, "root_workspace");
  assert.equal(cwd, workspace);
  assert.equal(env.ROOT_WORKER_WORKSPACE, undefined);
  assert.equal(
    fs.existsSync(
      path.join(morpheusHome, "instructions/morpheus-source-workspace.md"),
    ),
    false,
  );
  assert.equal(fs.existsSync(path.join(morpheusHome, "self-project.json")), false);
  assert.deepEqual(calls, []);

  fs.rmSync(dir, { recursive: true, force: true });
});

test("packaged launch without source removes stale managed source metadata", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-stale-source-"));
  try {
    const resourcesPath = path.join(dir, "resources");
    const packagedBinary = path.join(resourcesPath, "bin/app-server");
    const morpheusHome = path.join(dir, "home");
    const instructionPath = path.join(
      morpheusHome,
      "instructions/morpheus-source-workspace.md",
    );
    const projectPath = path.join(morpheusHome, "self-project.json");
    fs.mkdirSync(path.dirname(instructionPath), { recursive: true });
    fs.writeFileSync(
      instructionPath,
      "<!-- managed-by-morpheus-source-workspace -->\nstale\n",
    );
    fs.writeFileSync(
      projectPath,
      `${JSON.stringify({
        id: "/self",
        path: "/self",
        workspace: "/missing/source",
        managedBy: "morpheus",
      })}\n`,
    );

    prepareAppServerWorkspace(
      { MORPHEUS_HOME: morpheusHome },
      {
        resourcesPath,
        existsSync: (candidate) =>
          candidate === packagedBinary || fs.existsSync(candidate),
      },
    );

    assert.equal(fs.existsSync(instructionPath), false);
    assert.equal(fs.existsSync(projectPath), false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("packaged app-server launch keeps explicit workspace without clone", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-explicit-workspace-"));
  const resourcesPath = path.join(dir, "resources");
  const packagedBinary = path.join(resourcesPath, "bin/app-server");
  const morpheusHome = path.join(dir, "home");
  const explicitWorkspace = path.join(dir, "custom-workspace");
  fs.mkdirSync(
    path.join(explicitWorkspace, "apps/root-worker-prototype"),
    { recursive: true },
  );
  fs.writeFileSync(
    path.join(explicitWorkspace, "apps/root-worker-prototype/package.json"),
    "{}\n",
  );
  fs.mkdirSync(path.join(explicitWorkspace, "codex-rs"), { recursive: true });
  fs.writeFileSync(path.join(explicitWorkspace, "codex-rs/Cargo.toml"), "\n");
  const env = {
    MORPHEUS_HOME: morpheusHome,
    ROOT_WORKER_WORKSPACE: explicitWorkspace,
  };
  const calls = [];

  const cwd = prepareAppServerWorkspace(env, {
    cwd: "/source-tree",
    resourcesPath,
    existsSync: (candidate) =>
      candidate === packagedBinary || fs.existsSync(candidate),
    spawnSync: (...args) => {
      calls.push(args);
      return { status: 0, stderr: "" };
    },
  });

  assert.equal(cwd, explicitWorkspace);
  assert.equal(env.ROOT_WORKER_WORKSPACE, explicitWorkspace);
  assert.deepEqual(calls, []);
  assert.match(
    fs.readFileSync(
      path.join(morpheusHome, "instructions/morpheus-source-workspace.md"),
      "utf8",
    ),
    new RegExp(escapeRegExp(explicitWorkspace)),
  );
  assert.equal(
    JSON.parse(fs.readFileSync(path.join(morpheusHome, "self-project.json"), "utf8"))
      .workspace,
    explicitWorkspace,
  );

  fs.rmSync(dir, { recursive: true, force: true });
});

test("default app-server binary falls back to dev workspace binary", () => {
  const workspaceBinary = "/repo/codex-rs/target/debug/app-server";

  assert.equal(
    resolveDefaultAppServerBinary({
      resourcesPath: "/missing/resources",
      startDir: "/repo/apps/root-worker-prototype/electron",
      existsSync: (candidate) => candidate === workspaceBinary,
    }),
    `"${workspaceBinary}"`,
  );
});

test("default app-server binary falls back to PATH when no local binary exists", () => {
  assert.equal(
    resolveDefaultAppServerBinary({
      resourcesPath: "/missing/resources",
      startDir: "/repo/apps/root-worker-prototype/electron",
      existsSync: () => false,
    }),
    "app-server",
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

test("default app-server launch falls back when the mobile bind is unavailable", async () => {
  const writes = [];
  const checks = [];
  const mobileLaunch = await buildMobileConnectionLaunch(
    {
      ROOT_WORKER_MOBILE_LISTEN: "ws://0.0.0.0:8910",
      ROOT_WORKER_MOBILE_TOKEN: "test-token",
    },
    {
      morpheusHome: "/tmp/morpheus-home",
      writeTokenFile: (tokenFile, token) => writes.push({ tokenFile, token }),
      checkListenAvailable: async (listenUrl) => {
        checks.push(listenUrl);
        return listenUrl === "ws://0.0.0.0:8911";
      },
    },
  );

  assert.equal(mobileLaunch.enabled, true);
  assert.deepEqual(checks, ["ws://0.0.0.0:8910", "ws://0.0.0.0:8911"]);
  assert.deepEqual(writes, [
    {
      tokenFile: "/tmp/morpheus-home/root-worker-mobile-ws-token",
      token: "test-token",
    },
  ]);
  assert.deepEqual(mobileLaunch.info, {
    enabled: true,
    bindEndpoint: "ws://0.0.0.0:8911",
    endpoint: resolveLanEndpoint("ws://0.0.0.0:8911"),
    token: "test-token",
    auth: "capability-token",
  });
  assert.equal(
    buildDefaultAppServerCommand({
      appServerBinary: "/app-server",
      mobileLaunch,
    }),
    '/app-server --listen stdio:// --mobile-listen "ws://0.0.0.0:8911" --ws-auth capability-token --ws-token-file "/tmp/morpheus-home/root-worker-mobile-ws-token"',
  );
});

test("default app-server launch fallback resolves IPv6 wildcard endpoints to a reachable host", async () => {
  const mobileLaunch = await buildMobileConnectionLaunch(
    {
      ROOT_WORKER_MOBILE_LISTEN: "ws://[::]:8910",
      ROOT_WORKER_MOBILE_TOKEN: "test-token",
    },
    {
      morpheusHome: "/tmp/morpheus-home",
      writeTokenFile: () => {},
      checkListenAvailable: async (listenUrl) => listenUrl === "ws://[::]:8911",
    },
  );

  assert.equal(mobileLaunch.enabled, true);
  assert.equal(mobileLaunch.listenUrl, "ws://[::]:8911");
  assert.equal(mobileLaunch.info.bindEndpoint, "ws://[::]:8911");
  assert.equal(mobileLaunch.info.endpoint, resolveLanEndpoint("ws://[::]:8911"));
  assert.notEqual(mobileLaunch.info.endpoint, "ws://[::]:8911/");
});

test("default app-server launch disables mobile listener when no fallback port is available", async () => {
  const writes = [];
  const checks = [];
  const mobileLaunch = await buildMobileConnectionLaunch(
    {
      ROOT_WORKER_MOBILE_LISTEN: "ws://0.0.0.0:8910",
      ROOT_WORKER_MOBILE_TOKEN: "test-token",
    },
    {
      morpheusHome: "/tmp/morpheus-home",
      writeTokenFile: (tokenFile, token) => writes.push({ tokenFile, token }),
      checkListenAvailable: async (listenUrl) => {
        checks.push(listenUrl);
        return false;
      },
    },
  );

  assert.equal(mobileLaunch.enabled, false);
  assert.equal(checks.length, 21);
  assert.equal(checks[0], "ws://0.0.0.0:8910");
  assert.equal(checks.at(-1), "ws://0.0.0.0:8930");
  assert.deepEqual(writes, []);
  assert.deepEqual(mobileLaunch.info, {
    enabled: false,
    reason:
      "Mobile listener bind address is unavailable: ws://0.0.0.0:8910. No fallback port was available.",
  });
  assert.equal(
    buildDefaultAppServerCommand({
      appServerBinary: "/app-server",
      mobileLaunch,
    }),
    "/app-server --listen stdio://",
  );
});

test("default app-server launch does not fake endpoint overrides when the bind is unavailable", async () => {
  const writes = [];
  const checks = [];
  const mobileLaunch = await buildMobileConnectionLaunch(
    {
      ROOT_WORKER_MOBILE_LISTEN: "ws://0.0.0.0:8910",
      ROOT_WORKER_MOBILE_ENDPOINT: "wss://tunnel.example/root-worker",
      ROOT_WORKER_MOBILE_TOKEN: "test-token",
    },
    {
      morpheusHome: "/tmp/morpheus-home",
      writeTokenFile: (tokenFile, token) => writes.push({ tokenFile, token }),
      checkListenAvailable: async (listenUrl) => {
        checks.push(listenUrl);
        return false;
      },
    },
  );

  assert.equal(mobileLaunch.enabled, false);
  assert.deepEqual(checks, ["ws://0.0.0.0:8910"]);
  assert.deepEqual(writes, []);
  assert.deepEqual(mobileLaunch.info, {
    enabled: false,
    reason:
      "Mobile listener bind address is unavailable: ws://0.0.0.0:8910. Automatic port fallback is disabled because ROOT_WORKER_MOBILE_ENDPOINT is set.",
  });
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

test("custom CODEX_APP_SERVER_CMD leaves mobile listener ownership to the override", async () => {
  const launch = await resolveAppServerLaunch({
    CODEX_APP_SERVER_CMD: "/custom/codex-app-server --listen stdio://",
  });

  assert.equal(launch.command, "/custom/codex-app-server --listen stdio://");
  assert.deepEqual(launch.mobileConnection, {
    enabled: false,
    reason:
      "Mobile listener is unavailable when APP_SERVER_CMD overrides app-server launch.",
  });
});

test("app-server client restart rejects pending requests and starts a fresh child", async () => {
  const oldChild = new EventEmitter();
  oldChild.pid = 11;
  oldChild.exitCode = null;
  oldChild.kill = () => {
    oldChild.exitCode = 0;
    queueMicrotask(() => oldChild.emit("exit", 0, null));
    return true;
  };
  const client = new AppServerClient({ autoStart: false });
  const statuses = [];
  let rejectedPending = null;
  let startCount = 0;
  client.child = oldChild;
  client.pending.set(1, {
    reject: (error) => {
      rejectedPending = error;
    },
  });
  client.on("status", (status) => statuses.push(status));
  client.start = async () => {
    startCount += 1;
    client.child = { pid: 22, exitCode: null };
    client.readyResolve();
  };

  const result = await client.restart("runtime refresh");

  assert.deepEqual(result, {
    ok: true,
    restarted: true,
    pid: 22,
    reason: "runtime refresh",
  });
  assert.equal(startCount, 1);
  assert.equal(client.pending.size, 0);
  assert.match(rejectedPending.message, /app-server restarting: runtime refresh/);
  assert.equal(
    statuses.some(
      (status) => status.restarting && status.reason === "runtime refresh",
    ),
    true,
  );
});

test("app-server client stop rejects pending requests and terminates owned child", async () => {
  const child = new EventEmitter();
  child.pid = 11;
  child.exitCode = null;
  child.kill = () => {
    child.exitCode = 0;
    queueMicrotask(() => child.emit("exit", 0, null));
    return true;
  };
  const client = new AppServerClient({ autoStart: false });
  let rejectedPending = null;
  client.child = child;
  client.pending.set(1, {
    reject: (error) => {
      rejectedPending = error;
    },
  });

  const result = await client.stop("application quit");

  assert.deepEqual(result, { ok: true, forced: false });
  assert.equal(client.pending.size, 0);
  assert.equal(client.child, null);
  assert.match(rejectedPending.message, /app-server stopping: application quit/);
});

test("app-server client graceful shutdown uses lifecycle request without killing child", async () => {
  const child = new EventEmitter();
  child.pid = 11;
  child.exitCode = null;
  child.kill = () => {
    throw new Error("graceful shutdown should not signal the child");
  };
  const requests = [];
  const client = new AppServerClient({ autoStart: false });
  client.child = child;
  client.request = async (method, params) => {
    requests.push([method, params]);
    return { shutdown: true };
  };

  const result = await client.gracefulShutdown("capsule switch");

  assert.equal(result.ok, true);
  assert.deepEqual(requests, [
    ["client/lifecycle/shutdown", { reason: "capsule switch" }],
  ]);
  assert.equal(client.child, child);
});

test("app-server client treats signal-exited children as stopped", async () => {
  const child = new EventEmitter();
  child.pid = 11;
  child.exitCode = null;
  child.signalCode = "SIGTERM";
  child.kill = () => {
    throw new Error("already stopped child should not be killed again");
  };
  const client = new AppServerClient({ autoStart: false });
  client.child = child;

  assert.equal(client.status.connected, false);
  assert.deepEqual(await client.stop("application relaunch"), { ok: true });
  assert.equal(client.child, null);
});

test("app-server client force kills owned child when graceful stop times out", async () => {
  const child = new EventEmitter();
  child.pid = 11;
  child.exitCode = null;
  const signals = [];
  child.kill = (signal = "SIGTERM") => {
    signals.push(signal);
    if (signal === "SIGKILL") {
      child.exitCode = 0;
      queueMicrotask(() => child.emit("exit", 0, signal));
    }
    return true;
  };
  const client = new AppServerClient({ autoStart: false });
  client.child = child;

  const result = await client.stopChildProcess({
    reason: "unit test",
    pendingMessage: "stopping",
    timeoutMs: 1,
    forceKill: true,
  });

  assert.deepEqual(result, { ok: true, forced: true });
  assert.equal(client.child, null);
  assert.deepEqual(signals, ["SIGTERM", "SIGKILL"]);
});

test("app-server client restart rejects ready when start fails", async () => {
  const oldChild = new EventEmitter();
  oldChild.pid = 11;
  oldChild.exitCode = 0;
  const client = new AppServerClient({ autoStart: false });
  client.child = oldChild;
  client.start = async () => {
    throw new Error("launch failed");
  };

  const result = await client.restart("runtime refresh");

  assert.deepEqual(result, {
    ok: false,
    restarted: false,
    reason: "launch failed",
  });
  await assert.rejects(client.ready(), /launch failed/);
});

test("morpheus home defaults seed missing compact prompt", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-home-seed-"));
  const seedPath = path.join(dir, "seed-COMPACT.md");
  const morpheusHome = path.join(dir, "home");
  fs.writeFileSync(seedPath, "default compact prompt\n");

  const result = ensureMorpheusHomeDefaults(morpheusHome, {
    defaultCompactPromptSeedPath: seedPath,
  });

  const compactPromptPath = path.join(morpheusHome, "compact/COMPACT.md");
  assert.deepEqual(result, {
    compactPromptPath,
    seededCompactPrompt: true,
    compactPromptSeedPath: seedPath,
  });
  assert.equal(fs.readFileSync(compactPromptPath, "utf8"), "default compact prompt\n");
});

test("morpheus home defaults do not overwrite custom compact prompt", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-home-custom-"));
  const seedPath = path.join(dir, "seed-COMPACT.md");
  const compactPromptPath = path.join(dir, "home/compact/COMPACT.md");
  fs.mkdirSync(path.dirname(compactPromptPath), { recursive: true });
  fs.writeFileSync(seedPath, "default compact prompt\n");
  fs.writeFileSync(compactPromptPath, "custom compact prompt\n");

  const result = ensureMorpheusHomeDefaults(path.join(dir, "home"), {
    defaultCompactPromptSeedPath: seedPath,
  });

  assert.equal(result.seededCompactPrompt, false);
  assert.equal(result.compactPromptSeedPath, null);
  assert.equal(fs.readFileSync(compactPromptPath, "utf8"), "custom compact prompt\n");
});

test("morpheus home defaults use packaged compact seed before source fallback", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-packaged-seed-"));
  const resourcesPath = path.join(dir, "resources");
  const packagedSeed = path.join(
    resourcesPath,
    "default-config/compact/COMPACT.md",
  );
  const sourceSeed = path.join(
    dir,
    "codex-rs/thread-service/templates/compact/prompt.md",
  );
  fs.mkdirSync(path.dirname(packagedSeed), { recursive: true });
  fs.mkdirSync(path.dirname(sourceSeed), { recursive: true });
  fs.writeFileSync(packagedSeed, "packaged compact prompt\n");
  fs.writeFileSync(sourceSeed, "source compact prompt\n");

  ensureMorpheusHomeDefaults(path.join(dir, "home"), {
    resourcesPath,
    startDir: path.join(dir, "apps/root-worker-prototype/electron"),
  });

  assert.equal(
    fs.readFileSync(path.join(dir, "home/compact/COMPACT.md"), "utf8"),
    "packaged compact prompt\n",
  );
});

test("morpheus home defaults follow the selected external runtime resources", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-external-seed-"));
  const runtimeRoot = path.join(dir, "artifacts/build-2");
  const resourcesPath = path.join(
    runtimeRoot,
    "payload/Root Worker Runtime.app/Contents/Resources",
  );
  const selectedSeed = path.join(
    resourcesPath,
    "default-config/compact/COMPACT.md",
  );
  fs.mkdirSync(path.dirname(selectedSeed), { recursive: true });
  fs.writeFileSync(selectedSeed, "external compact prompt\n");

  ensureMorpheusHomeDefaults(path.join(dir, "home"), {
    resourcesPath,
  });

  assert.equal(
    fs.readFileSync(path.join(dir, "home/compact/COMPACT.md"), "utf8"),
    "external compact prompt\n",
  );
});

test("morpheus home defaults use source compact seed in dev mode", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-source-seed-"));
  const sourceSeed = path.join(
    dir,
    "codex-rs/thread-service/templates/compact/prompt.md",
  );
  fs.mkdirSync(path.dirname(sourceSeed), { recursive: true });
  fs.writeFileSync(sourceSeed, "source compact prompt\n");

  const result = ensureMorpheusHomeDefaults(path.join(dir, "home"), {
    resourcesPath: path.join(dir, "missing-resources"),
    startDir: path.join(dir, "apps/root-worker-prototype/electron"),
  });

  assert.equal(result.seededCompactPrompt, true);
  assert.equal(result.compactPromptSeedPath, sourceSeed);
  assert.equal(
    fs.readFileSync(path.join(dir, "home/compact/COMPACT.md"), "utf8"),
    "source compact prompt\n",
  );
});

test("morpheus home defaults skip compact seed non-fatally when no seed exists", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-missing-seed-"));
  const warnings = [];

  const result = ensureMorpheusHomeDefaults(path.join(dir, "home"), {
    resourcesPath: path.join(dir, "missing-resources"),
    startDir: path.join(dir, "apps/root-worker-prototype/electron"),
    existsSync: () => false,
    warn: (message) => warnings.push(message),
  });

  assert.equal(result.seededCompactPrompt, false);
  assert.equal(result.compactPromptSeedPath, null);
  assert.deepEqual(warnings, [
    `Default compact prompt seed was not found; ${path.join(dir, "home/compact/COMPACT.md")} was not created.`,
  ]);
  assert.equal(fs.existsSync(path.join(dir, "home/compact/COMPACT.md")), false);
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

test("mobile websocket endpoint refresh follows wildcard LAN address changes", () => {
  const first = refreshMobileConnectionInfo(
    {
      enabled: true,
      bindEndpoint: "ws://0.0.0.0:8910",
      endpoint: "ws://192.168.1.10:8910/",
      token: "test-token",
      auth: "capability-token",
    },
    {},
    { networkInterfaces: () => networkInterfaces("192.168.1.44") },
  );

  assert.deepEqual(first, {
    enabled: true,
    bindEndpoint: "ws://0.0.0.0:8910",
    endpoint: "ws://192.168.1.44:8910/",
    token: "test-token",
    auth: "capability-token",
  });
});

test("mobile websocket endpoint refresh keeps explicit endpoint overrides stable", () => {
  const info = {
    enabled: true,
    bindEndpoint: "ws://0.0.0.0:8910",
    endpoint: "wss://tunnel.example/root-worker",
    token: "test-token",
    auth: "capability-token",
  };

  assert.equal(
    refreshMobileConnectionInfo(
      info,
      { ROOT_WORKER_MOBILE_ENDPOINT: "wss://tunnel.example/root-worker" },
      { networkInterfaces: () => networkInterfaces("192.168.1.44") },
    ),
    info,
  );
});

test("mobile websocket endpoint refresh keeps concrete listen hosts stable", () => {
  const info = {
    enabled: true,
    bindEndpoint: "ws://192.168.1.10:8910",
    endpoint: "ws://192.168.1.10:8910/",
    token: "test-token",
    auth: "capability-token",
  };

  assert.equal(
    refreshMobileConnectionInfo(
      info,
      {},
      { networkInterfaces: () => networkInterfaces("192.168.1.44") },
    ),
    info,
  );
});

test("mobile websocket endpoint refresh leaves unavailable listener state unchanged", () => {
  const info = {
    enabled: false,
    reason: "Mobile listener is disabled by ROOT_WORKER_MOBILE_LISTEN=off.",
  };

  assert.equal(
    refreshMobileConnectionInfo(
      info,
      {},
      { networkInterfaces: () => networkInterfaces("192.168.1.44") },
    ),
    info,
  );
});

test("app-server client refresh updates mobile connection and broadcasts status", () => {
  const client = new EventEmitter();
  Object.setPrototypeOf(client, AppServerClient.prototype);
  client.child = { exitCode: null, pid: 1234 };
  client.mobileConnectionRefreshEnv = {};
  client.mobileConnection = {
    enabled: true,
    bindEndpoint: "ws://0.0.0.0:8910",
    endpoint: "ws://192.168.1.10:8910/",
    token: "test-token",
    auth: "capability-token",
  };
  const statuses = [];
  client.on("status", (status) => statuses.push(status));

  assert.equal(
    client.refreshMobileConnectionInfo({
      networkInterfaces: () => networkInterfaces("192.168.1.44"),
    }),
    true,
  );

  assert.equal(client.mobileConnection.endpoint, "ws://192.168.1.44:8910/");
  assert.equal(statuses.length, 1);
  assert.equal(
    statuses[0].mobileConnection.endpoint,
    "ws://192.168.1.44:8910/",
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

test("app-server client registers the Host lifecycle consumer before becoming ready", async () => {
  const client = Object.create(AppServerClient.prototype);
  const calls = [];
  client.hostInstanceId = "host-1";
  client.request = async (method) => {
    calls.push(["request", method]);
    return { serverInfo: { name: "app-server" } };
  };
  client.notify = async (method) => {
    calls.push(["notify", method]);
  };
  client.sendRequest = async (method, params) => {
    calls.push(["sendRequest", method, params]);
    return { registered: true, hostId: params.hostId };
  };
  client.readyResolve = () => calls.push(["ready"]);
  client.readyReject = (error) => {
    throw error;
  };
  client.emit = (event) => calls.push(["emit", event]);
  client.child = { exitCode: null, pid: 1234 };
  client.mobileConnection = { enabled: false };

  await client.initialize();

  assert.deepEqual(calls.slice(0, 4), [
    ["request", "initialize"],
    ["notify", "initialized"],
    [
      "sendRequest",
      "client/lifecycle/register",
      { hostId: "host-1" },
    ],
    ["ready"],
  ]);
});

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
