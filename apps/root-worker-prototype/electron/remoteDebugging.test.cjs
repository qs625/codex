const test = require("node:test");
const assert = require("node:assert/strict");

const {
  REMOTE_DEBUGGING_ADDRESS,
  REMOTE_DEBUGGING_PORT_ENV,
  applyRemoteDebuggingConfig,
  parseRemoteDebuggingConfig,
} = require("./remoteDebugging.cjs");

test("parseRemoteDebuggingConfig leaves CDP disabled by default", () => {
  assert.deepEqual(parseRemoteDebuggingConfig({}), { enabled: false });
  assert.deepEqual(parseRemoteDebuggingConfig({ [REMOTE_DEBUGGING_PORT_ENV]: "" }), {
    enabled: false,
  });
  assert.deepEqual(parseRemoteDebuggingConfig({ [REMOTE_DEBUGGING_PORT_ENV]: "   " }), {
    enabled: false,
  });
});

test("parseRemoteDebuggingConfig enables loopback CDP for explicit valid port", () => {
  assert.deepEqual(parseRemoteDebuggingConfig({ [REMOTE_DEBUGGING_PORT_ENV]: " 41235 " }), {
    enabled: true,
    port: "41235",
    address: REMOTE_DEBUGGING_ADDRESS,
    cdpUrl: "http://127.0.0.1:41235",
  });
});

test("parseRemoteDebuggingConfig rejects invalid ports without enabling CDP", () => {
  for (const value of ["0", "-1", "65536", "abc", "123abc"]) {
    const config = parseRemoteDebuggingConfig({ [REMOTE_DEBUGGING_PORT_ENV]: value });
    assert.equal(config.enabled, false);
    assert.match(config.warning, /expected 1\.\.65535/);
  }
});

test("applyRemoteDebuggingConfig appends port and loopback address only when enabled", () => {
  const app = fakeApp();
  const logger = fakeLogger();

  const config = applyRemoteDebuggingConfig(
    app,
    { [REMOTE_DEBUGGING_PORT_ENV]: "41235", ROOT_WORKER_OPEN_DEVTOOLS: "0" },
    logger,
  );

  assert.equal(config.enabled, true);
  assert.deepEqual(app.switches, [
    ["remote-debugging-port", "41235"],
    ["remote-debugging-address", "127.0.0.1"],
  ]);
  assert.equal(logger.warns.length, 0);
  assert.equal(logger.errors.length, 1);
});

test("applyRemoteDebuggingConfig ignores invalid env and does not append switches", () => {
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
