const test = require("node:test");
const assert = require("node:assert/strict");

const {
  createRuntimeLauncher,
  invokeLauncher,
} = require("./runtimeLauncher.cjs");

test("runtime launcher adapter uses the frozen command surface", () => {
  const calls = [];
  const launcher = createRuntimeLauncher({
    env: {
      MORPHEUS_LAUNCHER_PATH: "/Applications/Test.app/Contents/MacOS/MorpheusLauncher",
      MORPHEUS_RUNTIME_LAUNCHER_HOME: "/tmp/launcher-state",
    },
    spawnSync(command, args) {
      calls.push({ command, args });
      return {
        status: 0,
        stdout: '{"ok":true,"result":{"done":true}}',
        stderr: "",
      };
    },
  });

  assert.equal(launcher.abortFull("tx-0").result.done, true);
  assert.equal(launcher.commitHot("tx-1").result.done, true);
  assert.equal(launcher.rollbackHot("tx-1").ok, true);
  assert.equal(launcher.ackFailure("recovery-1").ok, true);
  assert.deepEqual(
    calls.map(({ args }) => args),
    [
      ["--state-root", "/tmp/launcher-state", "abort-full", "--transaction", "tx-0"],
      ["--state-root", "/tmp/launcher-state", "commit-hot", "--transaction", "tx-1"],
      ["--state-root", "/tmp/launcher-state", "rollback-hot", "--transaction", "tx-1"],
      [
        "--state-root",
        "/tmp/launcher-state",
        "ack-failure",
        "--recovery-identity",
        "recovery-1",
      ],
    ],
  );
});

test("runtime launcher surfaces structured command errors", () => {
  assert.throws(
    () =>
      invokeLauncher("/launcher", ["status"], {
        spawnSync: () => ({
          status: 1,
          stdout: '{"ok":false,"error":{"message":"state conflict"}}',
          stderr: "",
        }),
      }),
    /state conflict/,
  );
});
