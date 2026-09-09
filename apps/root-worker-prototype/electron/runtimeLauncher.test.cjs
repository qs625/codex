const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");

const {
  createRuntimeLauncher,
  invokeLauncher,
  invokeLauncherRequest,
  resolveRuntimeLauncherPath,
} = require("./runtimeLauncher.cjs");

test("runtime launcher prefers the stable path injected by its supervisor", () => {
  assert.equal(
    resolveRuntimeLauncherPath({
      env: { MORPHEUS_LAUNCHER_PATH: "/Applications/MorpheusLauncher" },
      platform: "linux",
    }),
    "/Applications/MorpheusLauncher",
  );
});

test("runtime launcher uses request files and parses one JSON result", () => {
  const calls = [];
  const launcher = createRuntimeLauncher({
    env: {
      MORPHEUS_LAUNCHER_PATH: "/launcher",
      MORPHEUS_RUNTIME_LAUNCHER_HOME: "/custom-state",
    },
    platform: "darwin",
    spawnSync: (command, args) => {
      calls.push({ command, args });
      return { status: 0, stdout: '{"ok":true}' };
    },
  });
  assert.deepEqual(
    launcher.activateHot({ schemaVersion: 1, transactionId: "tx" }),
    { ok: true },
  );
  assert.equal(calls[0].command, "/launcher");
  assert.deepEqual(calls[0].args.slice(0, 2), ["activate-hot", "--request"]);
});

test("launcher request JSON preserves missing, null, and string thread provenance", () => {
  const cases = [
    { request: { requestId: "missing" }, expected: undefined },
    {
      request: { requestId: "null", requestedByThreadId: null },
      expected: null,
    },
    {
      request: {
        requestId: "string",
        requestedByThreadId: "thread-1",
      },
      expected: "thread-1",
    },
  ];

  for (const { request, expected } of cases) {
    let serialized;
    const result = invokeLauncherRequest(
      "/launcher",
      "activate-hot",
      { schemaVersion: 1, ...request },
      {
        spawnSync: (_command, args) => {
          assert.deepEqual(args.slice(0, 2), ["activate-hot", "--request"]);
          serialized = JSON.parse(fs.readFileSync(args[2], "utf8"));
          return { status: 0, stdout: '{"ok":true}' };
        },
      },
    );

    assert.deepEqual(result, { ok: true });
    assert.equal(serialized.requestedByThreadId, expected);
    assert.equal(
      Object.hasOwn(serialized, "requestedByThreadId"),
      expected !== undefined,
    );
  }
});

test("runtime launcher does not pretend Windows and Linux activation exists", () => {
  const launcher = createRuntimeLauncher({
    env: {},
    platform: "linux",
    resourcesPath: "/opt/morpheus/resources",
  });
  assert.equal(launcher.supported, false);
  assert.throws(
    () => launcher.rollbackHot("transaction"),
    /Windows and Linux activation adapters are not implemented/,
  );
});

test("runtime launcher preserves nonzero JSON results as typed error evidence", () => {
  assert.throws(
    () =>
      invokeLauncher("/launcher", ["activate-hot"], {
        spawnSync: () => ({
          status: 1,
          stdout: JSON.stringify({
            ok: false,
            error: "signature verification failed",
            rolledBack: true,
            evidencePath: "/state/failure-evidence.json",
          }),
          stderr: "fallback stderr",
        }),
      }),
    (error) => {
      assert.equal(error.name, "RuntimeLauncherError");
      assert.equal(error.rolledBack, true);
      assert.equal(error.evidencePath, "/state/failure-evidence.json");
      assert.equal(error.launcherResult.error, "signature verification failed");
      assert.match(error.message, /signature verification failed/);
      return true;
    },
  );
});

test("identity status returns one strictly validated canonical evidence match", () => {
  const recoveryIdentity = "11111111-1111-4111-8111-111111111111";
  const calls = [];
  const evidence = launcherFailureEvidence({ recoveryIdentity });
  const launcher = createRuntimeLauncher({
    env: {
      MORPHEUS_LAUNCHER_PATH: "/launcher",
      MORPHEUS_RUNTIME_LAUNCHER_HOME: "/custom-state",
    },
    platform: "darwin",
    spawnSync: (command, args) => {
      calls.push({ command, args });
      return {
        status: 0,
        stdout: JSON.stringify({
          ok: true,
          result: {
            state: {},
            failureEvidenceMatch: launcherFailureMatch({ evidence }),
          },
        }),
      };
    },
  });

  const status = launcher.status("/Applications/Morpheus.app", recoveryIdentity);
  assert.equal(
    status.result.failureEvidenceMatch.recoveryIdentity,
    recoveryIdentity,
  );
  assert.deepEqual(status.result.failureEvidenceMatch.evidence, evidence);
  assert.deepEqual(calls, [
    {
      command: "/launcher",
      args: [
        "--state-root",
        "/custom-state",
        "status",
        "--app-bundle",
        "/Applications/Morpheus.app",
        "--recovery-identity",
        recoveryIdentity,
      ],
    },
  ]);
});

test("failure claim binds version, identity, and independently optional raw provenance", () => {
  const recoveryIdentity = "22222222-2222-4222-8222-222222222222";
  const claimId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  const evidence = launcherFailureEvidence({
    claimId,
    recoveryIdentity,
    transactionId: "  tx  ",
    requestId: null,
  });
  let args;
  const launcher = createRuntimeLauncher({
    env: {
      MORPHEUS_LAUNCHER_PATH: "/launcher",
      MORPHEUS_RUNTIME_LAUNCHER_HOME: "/custom-state",
    },
    platform: "darwin",
    spawnSync: (_command, receivedArgs) => {
      args = receivedArgs;
      return {
        status: 0,
        stdout: JSON.stringify({
          ok: true,
          claimId,
          recoveryIdentity,
          transactionId: "  tx  ",
          requestId: null,
          versionToken: launcherVersion("b"),
          sourceVersionToken: launcherVersion("a"),
          configuredEvidencePath: "/configured/claimed.json",
          activeEvidencePath: "/active/claimed.json",
          claimState: "created",
          requestedVersionMatched: true,
          evidence,
        }),
      };
    },
  });

  const claim = launcher.claimFailure({
    recoveryIdentity,
    transactionId: "  tx  ",
    requestId: null,
    expectedVersion: launcherVersion("a"),
  });
  assert.equal(claim.claimId, claimId);
  assert.deepEqual(args, [
    "--state-root",
    "/custom-state",
    "claim-failure",
    "--recovery-identity",
    recoveryIdentity,
    "--expected-version",
    launcherVersion("a"),
    "--transaction",
    "  tx  ",
  ]);
});

test("failure claim finalization validates claim identity and frozen version", () => {
  const recoveryIdentity = "55555555-5555-4555-8555-555555555555";
  const claimId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  let args;
  const launcher = createRuntimeLauncher({
    env: {
      MORPHEUS_LAUNCHER_PATH: "/launcher",
      MORPHEUS_RUNTIME_LAUNCHER_HOME: "/custom-state",
    },
    platform: "darwin",
    spawnSync: (_command, receivedArgs) => {
      args = receivedArgs;
      return {
        status: 0,
        stdout: JSON.stringify({
          ok: true,
          claimId,
          recoveryIdentity,
          transactionId: "  tx  ",
          requestId: "  request  ",
          acknowledged: true,
          consumed: true,
          remainingEvidence: false,
          versionToken: launcherVersion("b"),
        }),
      };
    },
  });

  const finalized = launcher.finalizeFailureClaim({
    claimId,
    recoveryIdentity,
  });
  assert.equal(finalized.versionToken, launcherVersion("b"));
  assert.deepEqual(args, [
    "--state-root",
    "/custom-state",
    "finalize-failure-claim",
    "--claim-id",
    claimId,
    "--recovery-identity",
    recoveryIdentity,
  ]);
});

test("version mismatch failure preserves typed launcher error fields", () => {
  assert.throws(
    () =>
      invokeLauncher("/launcher", ["claim-failure"], {
        spawnSync: () => ({
          status: 1,
          stdout: JSON.stringify({
            ok: false,
            error: "failure evidence version changed",
            errorCode: "failure-evidence-version-changed",
            expectedVersion: launcherVersion("a"),
            actualVersion: launcherVersion("b"),
          }),
        }),
      }),
    (error) => {
      assert.equal(error.name, "RuntimeLauncherError");
      assert.equal(error.errorCode, "failure-evidence-version-changed");
      assert.equal(error.expectedVersion, launcherVersion("a"));
      assert.equal(error.actualVersion, launcherVersion("b"));
      return true;
    },
  );
});

test("hot rollback returns durable failure evidence from launcher status", () => {
  const calls = [];
  const launcher = createRuntimeLauncher({
    env: {
      MORPHEUS_LAUNCHER_PATH: "/launcher",
      MORPHEUS_HOME: "/morpheus-home",
    },
    platform: "darwin",
    spawnSync: (_command, args) => {
      calls.push(args);
      if (args[0] === "rollback-hot") {
        return { status: 0, stdout: '{"ok":true}' };
      }
      return {
        status: 0,
        stdout: JSON.stringify({
          ok: true,
          result: {
            state: { current: { buildId: "recovered-build" } },
            failureEvidence: {
              transactionId: "tx",
              buildId: "failed-build",
            },
          },
        }),
      };
    },
  });

  assert.deepEqual(
    launcher.rollbackHot("tx", "/Applications/Morpheus.app"),
    {
      ok: true,
      failureEvidence: {
        transactionId: "tx",
        buildId: "failed-build",
        recoveredBuildId: "recovered-build",
      },
      evidencePath:
        "/morpheus-home/runtime-launcher/failure-evidence.json",
    },
  );
  assert.deepEqual(calls.map((args) => args[0]), [
    "rollback-hot",
    "--state-root",
  ]);
  assert.deepEqual(calls[1].slice(0, 3), [
    "--state-root",
    "/morpheus-home/runtime-launcher",
    "status",
  ]);
});

function launcherFailureEvidence(overrides = {}) {
  return {
    recoveryIdentity: "11111111-1111-4111-8111-111111111111",
    transactionId: "tx",
    requestId: "request",
    failurePhase: "launch",
    ...overrides,
  };
}

function launcherFailureMatch(overrides = {}) {
  const evidence = overrides.evidence ?? launcherFailureEvidence();
  return {
    recoveryIdentity: evidence.recoveryIdentity,
    artifactState: "current",
    transactionId: evidence.transactionId ?? null,
    requestId: evidence.requestId ?? null,
    configuredEvidencePath: "/configured/failure-evidence.json",
    activeEvidencePath: "/active/failure-evidence.json",
    versionToken: launcherVersion("a"),
    evidence,
    ...overrides,
  };
}

function launcherVersion(hexDigit) {
  return `sha256:${hexDigit.repeat(64)}`;
}
