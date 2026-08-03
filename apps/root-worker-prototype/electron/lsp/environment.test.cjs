const test = require("node:test");
const assert = require("node:assert/strict");

const { buildLspSpawnOptions } = require("./client.cjs");
const { buildLspExecOptions } = require("./manager.cjs");

test("LSP command lookup uses enhanced desktop PATH", () => {
  const options = buildLspExecOptions(
    {
      HOME: "/Users/alice",
      PATH: "/usr/bin",
    },
    {
      platform: "darwin",
      shellPath: "/shell/bin",
    },
  );

  assert.ok(options.env.PATH.startsWith("/usr/bin:/shell/bin:"));
  assert.ok(options.env.PATH.includes("/opt/homebrew/bin"));
  assert.ok(options.env.PATH.includes("/usr/local/bin"));
});

test("LSP spawn uses enhanced desktop PATH and preserves cwd", () => {
  const options = buildLspSpawnOptions({
    baseEnv: {
      HOME: "/Users/alice",
      PATH: "/usr/bin",
    },
    commandSpec: {
      command: "gopls",
      args: [],
      cwd: "/work/project",
    },
    environmentOptions: {
      platform: "darwin",
      shellPath: "/shell/bin",
    },
    workspaceRoot: "/work",
  });

  assert.equal(options.cwd, "/work/project");
  assert.deepEqual(options.stdio, ["pipe", "pipe", "pipe"]);
  assert.ok(options.env.PATH.startsWith("/usr/bin:/shell/bin:"));
  assert.ok(options.env.PATH.includes("/Users/alice/go/bin"));
});
