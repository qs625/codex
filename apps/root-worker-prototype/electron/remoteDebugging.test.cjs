const test = require("node:test");
const assert = require("node:assert/strict");

const {
  DEFAULT_REMOTE_DEBUGGING_PORT,
  REMOTE_DEBUGGING_ADDRESS,
  REMOTE_DEBUGGING_BACKEND_PORT_ENV,
  REMOTE_DEBUGGING_DISABLE_ENV,
  REMOTE_DEBUGGING_PORT_ENV,
  applyRemoteDebuggingConfig,
  parseRemoteDebuggingConfig,
} = require("./remoteDebugging.cjs");

test("parseRemoteDebuggingConfig enables loopback CDP by default", () => {
  assert.deepEqual(parseRemoteDebuggingConfig({}), {
    enabled: true,
    port: DEFAULT_REMOTE_DEBUGGING_PORT,
    backendPort: "9223",
    address: REMOTE_DEBUGGING_ADDRESS,
    cdpUrl: "http://127.0.0.1:9222",
    backendCdpUrl: "http://127.0.0.1:9223",
    proxy: {
      enabled: true,
      port: DEFAULT_REMOTE_DEBUGGING_PORT,
      backendPort: "9223",
      address: REMOTE_DEBUGGING_ADDRESS,
    },
  });
  assert.deepEqual(parseRemoteDebuggingConfig({ [REMOTE_DEBUGGING_PORT_ENV]: "" }), {
    enabled: true,
    port: DEFAULT_REMOTE_DEBUGGING_PORT,
    backendPort: "9223",
    address: REMOTE_DEBUGGING_ADDRESS,
    cdpUrl: "http://127.0.0.1:9222",
    backendCdpUrl: "http://127.0.0.1:9223",
    proxy: {
      enabled: true,
      port: DEFAULT_REMOTE_DEBUGGING_PORT,
      backendPort: "9223",
      address: REMOTE_DEBUGGING_ADDRESS,
    },
  });
});

test("parseRemoteDebuggingConfig disables CDP with explicit disable env", () => {
  for (const value of ["1", "true", "yes", "on", " TRUE "]) {
    assert.deepEqual(
      parseRemoteDebuggingConfig({
        [REMOTE_DEBUGGING_DISABLE_ENV]: value,
        [REMOTE_DEBUGGING_PORT_ENV]: "41235",
      }),
      { enabled: false },
    );
  }
});

test("parseRemoteDebuggingConfig enables loopback CDP for explicit valid port", () => {
  assert.deepEqual(parseRemoteDebuggingConfig({ [REMOTE_DEBUGGING_PORT_ENV]: " 41235 " }), {
    enabled: true,
    port: "41235",
    backendPort: "41236",
    address: REMOTE_DEBUGGING_ADDRESS,
    cdpUrl: "http://127.0.0.1:41235",
    backendCdpUrl: "http://127.0.0.1:41236",
    proxy: {
      enabled: true,
      port: "41235",
      backendPort: "41236",
      address: REMOTE_DEBUGGING_ADDRESS,
    },
  });
});

test("parseRemoteDebuggingConfig accepts explicit backend port for the proxy", () => {
  assert.deepEqual(
    parseRemoteDebuggingConfig({
      [REMOTE_DEBUGGING_PORT_ENV]: "41235",
      [REMOTE_DEBUGGING_BACKEND_PORT_ENV]: "42123",
    }),
    {
      enabled: true,
      port: "41235",
      backendPort: "42123",
      address: REMOTE_DEBUGGING_ADDRESS,
      cdpUrl: "http://127.0.0.1:41235",
      backendCdpUrl: "http://127.0.0.1:42123",
      proxy: {
        enabled: true,
        port: "41235",
        backendPort: "42123",
        address: REMOTE_DEBUGGING_ADDRESS,
      },
    },
  );
});

test("parseRemoteDebuggingConfig rejects invalid backend ports", () => {
  for (const backendPort of ["nope", "9222"]) {
    const config = parseRemoteDebuggingConfig({
      [REMOTE_DEBUGGING_PORT_ENV]: "9222",
      [REMOTE_DEBUGGING_BACKEND_PORT_ENV]: backendPort,
    });
    assert.equal(config.enabled, false);
    assert.match(config.warning, /ROOT_WORKER_REMOTE_DEBUGGING_BACKEND_PORT/);
  }
});

test("parseRemoteDebuggingConfig rejects invalid ports without enabling CDP", () => {
  for (const value of ["0", "-1", "65536", "abc", "123abc"]) {
    const config = parseRemoteDebuggingConfig({ [REMOTE_DEBUGGING_PORT_ENV]: value });
    assert.equal(config.enabled, false);
    assert.match(config.warning, /expected 1\.\.65535/);
  }
});

test("applyRemoteDebuggingConfig appends default port and loopback address", () => {
  const app = fakeApp();
  const logger = fakeLogger();

  const config = applyRemoteDebuggingConfig(
    app,
    { ROOT_WORKER_OPEN_DEVTOOLS: "0" },
    logger,
  );

  assert.equal(config.enabled, true);
  assert.deepEqual(app.switches, [
    ["remote-debugging-port", "9223"],
    ["remote-debugging-address", "127.0.0.1"],
  ]);
  assert.equal(logger.warns.length, 0);
  assert.equal(logger.errors.length, 1);
});

test("applyRemoteDebuggingConfig appends override port and loopback address", () => {
  const app = fakeApp();
  const logger = fakeLogger();

  const config = applyRemoteDebuggingConfig(
    app,
    { [REMOTE_DEBUGGING_PORT_ENV]: "41235", ROOT_WORKER_OPEN_DEVTOOLS: "0" },
    logger,
  );

  assert.equal(config.enabled, true);
  assert.deepEqual(app.switches, [
    ["remote-debugging-port", "41236"],
    ["remote-debugging-address", "127.0.0.1"],
  ]);
  assert.equal(logger.warns.length, 0);
  assert.equal(logger.errors.length, 1);
});

test("applyRemoteDebuggingConfig disables CDP without appending switches", () => {
  const app = fakeApp();
  const logger = fakeLogger();

  const config = applyRemoteDebuggingConfig(
    app,
    { [REMOTE_DEBUGGING_DISABLE_ENV]: "1", ROOT_WORKER_OPEN_DEVTOOLS: "1" },
    logger,
  );

  assert.equal(config.enabled, false);
  assert.deepEqual(app.switches, []);
  assert.equal(logger.warns.length, 0);
  assert.equal(logger.errors.length, 0);
});

test("applyRemoteDebuggingConfig rejects invalid env and does not append switches", () => {
  const app = fakeApp();
  const logger = fakeLogger();

  const config = applyRemoteDebuggingConfig(
    app,
    { [REMOTE_DEBUGGING_PORT_ENV]: "0", ROOT_WORKER_OPEN_DEVTOOLS: "1" },
    logger,
  );

  assert.equal(config.enabled, false);
  assert.deepEqual(app.switches, []);
  assert.equal(logger.warns.length, 1);
  assert.equal(logger.errors.length, 0);
});

function fakeApp() {
  const switches = [];
  return {
    switches,
    commandLine: {
      appendSwitch(name, value) {
        switches.push([name, value]);
      },
    },
  };
}

function fakeLogger() {
  return {
    errors: [],
    warns: [],
    error(...args) {
      this.errors.push(args);
    },
    warn(...args) {
      this.warns.push(args);
    },
  };
}
