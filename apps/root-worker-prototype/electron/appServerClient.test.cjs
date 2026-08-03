const test = require("node:test");
const assert = require("node:assert/strict");

const {
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
