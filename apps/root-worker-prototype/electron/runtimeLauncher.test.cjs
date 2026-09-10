const assert = require("node:assert/strict");
const test = require("node:test");

const {
  createRuntimeLauncher,
  invokeLauncher,
} = require("./runtimeLauncher.cjs");

test("runtime launcher adapter selects a complete Capsule without status CAS", () => {
  const calls = [];
  let request = null;
  const launcher = createRuntimeLauncher({
    env: {
      MORPHEUS_HOME: "/tmp/morpheus",
      RUNTIME_CAPSULE_LAUNCHER_PATH: "/tmp/MorpheusLauncher",
    },
    spawnSync(_command, args) {
      calls.push(args);
      if (args.includes("select-candidate")) {
        const requestPath = args.at(-1);
        request = JSON.parse(require("node:fs").readFileSync(requestPath, "utf8"));
        return {
          status: 0,
          stdout: JSON.stringify({
            ok: true,
            result: {
              activationId: "candidate-1",
              releaseId: `sha256:${"a".repeat(64)}`,
              control: { selected: { kind: "external" } },
            },
          }),
          stderr: "",
        };
      }
      throw new Error(`unexpected launcher arguments: ${args.join(" ")}`);
    },
  });

  const result = launcher.selectCandidate({
    activationId: "candidate-1",
    target: { os: "darwin", arch: "arm64" },
    reason: "update",
  });

  assert.equal(calls.length, 1);
  assert.ok(calls[0].includes("select-candidate"));
  assert.deepEqual(request, {
    schemaVersion: 1,
    activationId: "candidate-1",
    target: { os: "darwin", arch: "arm64" },
    reason: "update",
  });
  assert.equal(result.releaseId, `sha256:${"a".repeat(64)}`);
});

test("runtime launcher adapter rejects an invalid selection response", () => {
  const launcher = createRuntimeLauncher({
    env: { RUNTIME_CAPSULE_LAUNCHER_PATH: "/tmp/MorpheusLauncher" },
    spawnSync() {
      return {
        status: 0,
        stdout:
          '{"ok":true,"result":{"activationId":"candidate-1","releaseId":"release-1"}}',
        stderr: "",
      };
    },
  });

  assert.throws(
    () =>
      launcher.selectCandidate({
        activationId: "candidate-1",
        target: { os: "darwin", arch: "arm64" },
      }),
    /selection result is incomplete/,
  );
});

test("launcher invocation surfaces typed launcher failures", () => {
  assert.throws(
    () =>
      invokeLauncher("/tmp/MorpheusLauncher", ["status"], {
        spawnSync() {
          return {
            status: 1,
            stdout: '{"ok":false,"error":{"message":"selection failed"}}',
            stderr: "",
          };
        },
      }),
    /selection failed/,
  );
});
