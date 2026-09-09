const test = require("node:test");
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fsConstants = require("node:fs").constants;
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const {
  buildLauncherRecoveryRecordParams,
  completeLauncherFailureRecovery,
  createLauncherQueueDrainRestartRequester,
  createRuntimeLaunchReadiness,
  normalizeLauncherFailureEvidence,
  prepareCanonicalLauncherFailureEvidence,
  readLauncherFailureEvidence,
  recordClaimedLauncherFailureRecovery,
} = require("./runtimeLaunchState.cjs");

test("ready is durable only after app-server and first renderer are ready", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-ready-test-"));
  const readyPath = path.join(root, "ready.json");
  const readiness = createRuntimeLaunchReadiness({
    env: {
      MORPHEUS_LAUNCH_TRANSACTION_ID: "tx",
      MORPHEUS_LAUNCH_BUILD_ID: "build",
      MORPHEUS_LAUNCH_INSTANCE_ID: "instance",
      MORPHEUS_LAUNCH_READY_PATH: readyPath,
    },
    fs,
    now: () => 42,
  });

  await readiness.markRendererReady();
  await assert.rejects(fs.access(readyPath));
  await readiness.markAppServerReady();
  assert.deepEqual(JSON.parse(await fs.readFile(readyPath, "utf8")), {
    schemaVersion: 1,
    transactionId: "tx",
    buildId: "build",
    instanceId: "instance",
    readyAtMs: 42,
  });
});

test("launcher failure evidence forbids the failed build and hash", async () => {
  const normalized = normalizeLauncherFailureEvidence(
    {
      recoveryIdentity: "11111111-1111-4111-8111-111111111111",
      transactionId: "tx",
      requestedByThreadId: "thread",
      buildId: "bad-build",
      manifestHash: "bad-hash",
      recoveredBuildId: "good-build",
      phase: "ready-timeout",
    },
    "/evidence.json",
  );
  assert.equal(normalized.failedBuildId, "bad-build");
  assert.match(normalized.prompt, /Do not activate the same failed build bad-build/);
  assert.equal(normalized.failedBuildHash, "bad-hash");
  assert.match(normalized.prompt, /artifact hash bad-hash/);

  const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-evidence-test-"));
  const evidencePath = path.join(root, "evidence.json");
  await fs.writeFile(
    evidencePath,
    '{"recoveryIdentity":"11111111-1111-4111-8111-111111111111"}',
  );
  const durableEvidence = await readLauncherFailureEvidence(evidencePath, fs);
  assert.equal(typeof durableEvidence.evidenceSnapshot.contentHash, "string");
});

test("legacy launcher evidence does not invent a requesting thread id", () => {
  const normalized = normalizeLauncherFailureEvidence(
    {
      recoveryIdentity: "11111111-1111-4111-8111-111111111111",
      transactionId: "tx",
      buildId: "bad-build",
      manifestHash: "bad-hash",
    },
    "/evidence.json",
  );

  assert.equal(normalized.requestedByThreadId, null);
});

test("launcher and server provenance retain distinct raw and normalized values", () => {
  const normalized = normalizeLauncherFailureEvidence(
    {
      recoveryIdentity: "11111111-1111-4111-8111-111111111111",
      transactionId: "  tx-with-space  ",
      requestId: "  request-with-space  ",
    },
    "/evidence.json",
  );

  assert.equal(normalized.transactionId, "tx-with-space");
  assert.equal(normalized.requestId, "request-with-space");
  assert.equal(normalized.launcherTransactionId, "  tx-with-space  ");
  assert.equal(normalized.launcherRequestId, "  request-with-space  ");
});

test("launcher failure completion consumes intent before claim finalize and queue drain", async () => {
  const calls = [];
  const evidence = launcherRecoveryEvidence();
  const result = await completeLauncherFailureRecovery({
    evidence,
    consumeRestartIntent: async (requestId) => {
      calls.push(["consume", requestId]);
    },
    finalizeFailureClaim: async (expected) => {
      calls.push(["finalize", expected]);
      return launcherRecoveryAcknowledgement({
        remainingEvidence: true,
      });
    },
    requestQueueDrainRestart: (recoveryIdentity) => {
      calls.push(["exit", recoveryIdentity]);
    },
  });

  assert.deepEqual(calls, [
    ["consume", evidence.requestId],
    [
      "finalize",
      {
        claimId: evidence.launcherClaim.claimId,
        recoveryIdentity: evidence.recoveryIdentity,
      },
    ],
    ["exit", evidence.recoveryIdentity],
  ]);
  assert.deepEqual(result, {
    consumed: true,
    consumedEvidencePath: "/state/consumed.json",
    remainingEvidence: true,
  });
});

test("launcher failure finalization remains resident when the queue is empty", async () => {
  const calls = [];
  const evidence = launcherRecoveryEvidence();
  const result = await completeLauncherFailureRecovery({
    evidence,
    consumeRestartIntent: async (requestId) => {
      calls.push(["consume", requestId]);
    },
    finalizeFailureClaim: async () => {
      calls.push(["finalize"]);
      return launcherRecoveryAcknowledgement({
        remainingEvidence: false,
      });
    },
    requestQueueDrainRestart: () => {
      calls.push(["exit"]);
    },
  });

  assert.deepEqual(calls, [["consume", evidence.requestId], ["finalize"]]);
  assert.equal(result.remainingEvidence, false);
});

test("missing or null raw launcher request skips intent cleanup and still drains queued evidence", async () => {
  for (const requestId of [undefined, null]) {
    const rawEvidence = {
      recoveryIdentity: "11111111-1111-4111-8111-111111111111",
      transactionId: "raw-transaction",
    };
    if (requestId === null) {
      rawEvidence.requestId = null;
    }
    const evidence = normalizeLauncherFailureEvidence(
      rawEvidence,
      "/evidence.json",
    );
    evidence.launcherClaim = launcherRecoveryEvidence().launcherClaim;
    const calls = [];
    const result = await completeLauncherFailureRecovery({
      evidence,
      consumeRestartIntent: async (intentRequestId) => {
        calls.push(["consume", intentRequestId]);
      },
      finalizeFailureClaim: async (expected) => {
        calls.push(["finalize", expected]);
        return launcherRecoveryAcknowledgement({
          transactionId: "raw-transaction",
          requestId: null,
          remainingEvidence: true,
        });
      },
      requestQueueDrainRestart: (recoveryIdentity) => {
        calls.push(["exit", recoveryIdentity]);
      },
    });

    assert.match(evidence.requestId, /^legacy-request-/);
    assert.equal(evidence.launcherRequestId, null);
    assert.deepEqual(calls, [
      [
        "finalize",
        {
          claimId: evidence.launcherClaim.claimId,
          recoveryIdentity: evidence.recoveryIdentity,
        },
      ],
      ["exit", evidence.recoveryIdentity],
    ]);
    assert.equal(result.remainingEvidence, true);
  }
});

test("intent cleanup failure leaves the durable claim unfinalized for a later startup", async () => {
  const evidence = launcherRecoveryEvidence();
  let finalizeCalls = 0;
  let exits = 0;

  await assert.rejects(
    completeLauncherFailureRecovery({
      evidence,
      consumeRestartIntent: async () => {
        throw new Error("intent cleanup failed");
      },
      finalizeFailureClaim: async () => {
        finalizeCalls += 1;
      },
      requestQueueDrainRestart: () => {
        exits += 1;
      },
    }),
    /intent cleanup failed/,
  );
  assert.equal(finalizeCalls, 0);
  assert.equal(exits, 0);

  const delivered = [];
  await completeLauncherFailureRecovery({
    evidence,
    consumeRestartIntent: async () => {
      delivered.push("consume-a");
    },
    finalizeFailureClaim: async () => {
      delivered.push("finalize-a");
      return launcherRecoveryAcknowledgement({
        remainingEvidence: true,
      });
    },
    requestQueueDrainRestart: () => {
      delivered.push("exit-a-for-b");
    },
  });
  const nextEvidence = launcherRecoveryEvidence({
    recoveryIdentity: "22222222-2222-4222-8222-222222222222",
    transactionId: "tx-b",
    requestId: "request-b",
    launcherTransactionId: "raw-tx-b",
    launcherRequestId: "raw-request-b",
    launcherClaim: launcherClaim({
      claimId: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      recoveryIdentity: "22222222-2222-4222-8222-222222222222",
      versionToken: versionToken("b"),
    }),
  });
  await completeLauncherFailureRecovery({
    evidence: nextEvidence,
    consumeRestartIntent: async () => {
      delivered.push("consume-b");
    },
    finalizeFailureClaim: async (expected) => {
      delivered.push(["finalize-b", expected]);
      return launcherRecoveryAcknowledgement({
        claimId: nextEvidence.launcherClaim.claimId,
        recoveryIdentity: nextEvidence.recoveryIdentity,
        versionToken: nextEvidence.launcherClaim.versionToken,
        transactionId: nextEvidence.launcherTransactionId,
        requestId: nextEvidence.launcherRequestId,
        remainingEvidence: false,
      });
    },
    requestQueueDrainRestart: () => {
      delivered.push("unexpected-exit-b");
    },
  });
  assert.deepEqual(delivered, [
    "consume-a",
    "finalize-a",
    "exit-a-for-b",
    "consume-b",
    [
      "finalize-b",
      {
        claimId: nextEvidence.launcherClaim.claimId,
        recoveryIdentity: nextEvidence.recoveryIdentity,
      },
    ],
  ]);
});

test("crash after cleanup but before finalize is equivalent to an idempotent startup retry", async () => {
  const evidence = launcherRecoveryEvidence();
  const calls = [];
  await assert.rejects(
    completeLauncherFailureRecovery({
      evidence,
      consumeRestartIntent: async () => {
        calls.push("consume");
      },
      finalizeFailureClaim: async () => {
        calls.push("finalize-crash");
        throw new Error("process stopped before launcher finalize completed");
      },
      requestQueueDrainRestart: () => {
        calls.push("exit");
      },
    }),
    /process stopped/,
  );
  await completeLauncherFailureRecovery({
    evidence,
    consumeRestartIntent: async () => {
      calls.push("consume-no-op");
    },
    finalizeFailureClaim: async () => {
      calls.push("finalize-retry");
      return launcherRecoveryAcknowledgement({
        remainingEvidence: false,
      });
    },
    requestQueueDrainRestart: () => {
      calls.push("exit");
    },
  });
  assert.deepEqual(calls, [
    "consume",
    "finalize-crash",
    "consume-no-op",
    "finalize-retry",
  ]);
});

test("malformed launcher finalize result never requests queue drain", async () => {
  const evidence = launcherRecoveryEvidence();
  for (const acknowledgement of [
    launcherRecoveryAcknowledgement({ remainingEvidence: undefined }),
    launcherRecoveryAcknowledgement({
      claimId: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      remainingEvidence: true,
    }),
    launcherRecoveryAcknowledgement({
      versionToken: versionToken("b"),
      remainingEvidence: true,
    }),
  ]) {
    let exited = false;
    await assert.rejects(
      completeLauncherFailureRecovery({
        evidence,
        consumeRestartIntent: async () => {},
        finalizeFailureClaim: async () => acknowledgement,
        requestQueueDrainRestart: () => {
          exited = true;
        },
      }),
      /did not acknowledge|remainingEvidence/,
    );
    assert.equal(exited, false);
  }
});

test("queue drain restart requests exit 76 once per recovery identity", () => {
  const exitCodes = [];
  const requestRestart = createLauncherQueueDrainRestartRequester({
    appExit: (code) => exitCodes.push(code),
  });

  assert.equal(
    requestRestart("11111111-1111-4111-8111-111111111111"),
    true,
  );
  assert.equal(
    requestRestart("11111111-1111-4111-8111-111111111111"),
    false,
  );
  assert.equal(
    requestRestart("22222222-2222-4222-8222-222222222222"),
    true,
  );
  assert.deepEqual(exitCodes, [76, 76]);
});

test("failed queue drain exit can be retried for the same identity", () => {
  let calls = 0;
  const requestRestart = createLauncherQueueDrainRestartRequester({
    appExit: () => {
      calls += 1;
      if (calls === 1) {
        throw new Error("exit failed");
      }
    },
  });
  const recoveryIdentity = "11111111-1111-4111-8111-111111111111";

  assert.throws(() => requestRestart(recoveryIdentity), /exit failed/);
  assert.equal(requestRestart(recoveryIdentity), true);
  assert.equal(requestRestart(recoveryIdentity), false);
  assert.equal(calls, 2);
});

test("missing or invalid recovery identity fails closed and retains evidence", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-identity-test-"));
  for (const [name, contents] of [
    ["missing.json", '{"failurePhase":"launch"}'],
    [
      "invalid.json",
      '{"recoveryIdentity":"not-a-uuid","failurePhase":"launch"}',
    ],
  ]) {
    const evidencePath = path.join(root, name);
    await fs.writeFile(evidencePath, contents);
    await assert.rejects(
      readLauncherFailureEvidence(evidencePath, fs),
      /valid persistent recoveryIdentity UUID/,
    );
    assert.equal(await fs.readFile(evidencePath, "utf8"), contents);
  }
});

test("requestedByThreadId preserves missing/null legacy and rejects present empty modern values", () => {
  for (const evidence of [
    { recoveryIdentity: "11111111-1111-4111-8111-111111111111" },
    {
      recoveryIdentity: "22222222-2222-4222-8222-222222222222",
      requestedByThreadId: null,
    },
  ]) {
    assert.equal(
      normalizeLauncherFailureEvidence(evidence, "/evidence.json")
        .requestedByThreadId,
      null,
    );
  }

  for (const requestedByThreadId of ["", "   "]) {
    assert.throws(
      () =>
        normalizeLauncherFailureEvidence(
          {
            recoveryIdentity: "33333333-3333-4333-8333-333333333333",
            requestedByThreadId,
          },
          "/evidence.json",
        ),
      /requestedByThreadId must be null\/omitted/,
    );
  }

  const requesterId = "11111111-1111-4111-8111-111111111111";
  assert.equal(
    normalizeLauncherFailureEvidence(
      {
        recoveryIdentity: "44444444-4444-4444-8444-444444444444",
        requestedByThreadId: `  ${requesterId}  `,
      },
      "/evidence.json",
    ).requestedByThreadId,
    requesterId,
  );
});

test("durable evidence reads retain identity across retries and distinguish files", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-identity-test-"));
  const firstPath = path.join(root, "first.json");
  const secondPath = path.join(root, "second.json");
  await fs.writeFile(
    firstPath,
    '{"recoveryIdentity":"11111111-1111-4111-8111-111111111111","failurePhase":"launch"}',
  );
  await fs.writeFile(
    secondPath,
    '{"recoveryIdentity":"22222222-2222-4222-8222-222222222222","failurePhase":"launch"}',
  );

  const first = await readLauncherFailureEvidence(firstPath, fs);
  const retry = await readLauncherFailureEvidence(firstPath, fs);
  const second = await readLauncherFailureEvidence(secondPath, fs);
  assert.equal(first.recoveryIdentity, retry.recoveryIdentity);
  assert.notEqual(first.recoveryIdentity, second.recoveryIdentity);

  const firstNormalized = normalizeLauncherFailureEvidence(first, firstPath);
  const retryNormalized = normalizeLauncherFailureEvidence(retry, firstPath);
  const secondNormalized = normalizeLauncherFailureEvidence(second, secondPath);
  assert.equal(firstNormalized.evidenceSnapshot, first.evidenceSnapshot);
  assert.equal(firstNormalized.transactionId, retryNormalized.transactionId);
  assert.equal(firstNormalized.requestId, retryNormalized.requestId);
  assert.notEqual(firstNormalized.transactionId, secondNormalized.transactionId);
  assert.notEqual(firstNormalized.requestId, secondNormalized.requestId);
});

test(
  "persistent identity is stable across capability path aliases",
  { skip: process.platform === "win32" },
  async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-capability-test-"));
    const stateRoot = path.join(root, "state");
    const firstAlias = path.join(root, "capability-one");
    const secondAlias = path.join(root, "capability-two");
    await fs.mkdir(stateRoot);
    await fs.writeFile(
      path.join(stateRoot, "failure-evidence.json"),
      '{"recoveryIdentity":"11111111-1111-4111-8111-111111111111","failurePhase":"launch"}',
    );
    await fs.symlink(stateRoot, firstAlias);
    await fs.symlink(stateRoot, secondAlias);

    const first = await readLauncherFailureEvidence(
      path.join(firstAlias, "failure-evidence.json"),
      fs,
    );
    const second = await readLauncherFailureEvidence(
      path.join(secondAlias, "failure-evidence.json"),
      fs,
    );
    assert.equal(first.recoveryIdentity, second.recoveryIdentity);
  },
);

test("evidence content and identity come from one file handle snapshot", async () => {
  const recoveryIdentity = "11111111-1111-4111-8111-111111111111";
  const stat = regularFileStat({ dev: 7, ino: 11, size: 64 });
  let closeCalls = 0;
  const evidence = await readLauncherFailureEvidence("/capability/evidence", {
    open: async (_evidencePath, flags) => {
      assert.equal(
        flags & (fsConstants.O_WRONLY | fsConstants.O_RDWR),
        fsConstants.O_RDONLY,
      );
      if (fsConstants.O_NOFOLLOW) {
        assert(flags & fsConstants.O_NOFOLLOW);
      }
      return {
        stat: async () => stat,
        readFile: async () =>
          JSON.stringify({ recoveryIdentity, marker: "content-a" }),
        close: async () => {
          closeCalls += 1;
        },
      };
    },
  });

  assert.equal(evidence.marker, "content-a");
  assert.equal(evidence.recoveryIdentity, recoveryIdentity);
  assert.equal(evidence.evidenceSnapshot.dev, 7);
  assert.equal(evidence.evidenceSnapshot.ino, 11);
  assert.equal(closeCalls, 1);
});

test(
  "evidence reader rejects symlink and non-regular leaf entries",
  { skip: process.platform === "win32" },
  async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-leaf-test-"));
    const regularPath = path.join(root, "regular.json");
    const symlinkPath = path.join(root, "symlink.json");
    const directoryPath = path.join(root, "directory");
    await fs.writeFile(
      regularPath,
      '{"recoveryIdentity":"11111111-1111-4111-8111-111111111111"}',
    );
    await fs.symlink(regularPath, symlinkPath);
    await fs.mkdir(directoryPath);

    await assert.rejects(
      readLauncherFailureEvidence(symlinkPath, fs),
      (error) => ["ELOOP", "EMLINK"].includes(error?.code),
    );
    await assert.rejects(
      readLauncherFailureEvidence(directoryPath, fs),
      /must be a regular file/,
    );
  },
);

test("evidence read close failure does not override the primary read error", async () => {
  const primaryError = new Error("read failed");
  await assert.rejects(
    readLauncherFailureEvidence("/capability/evidence", {
      open: async () => ({
        stat: async () => regularFileStat({ dev: 7, ino: 11, size: 64 }),
        readFile: async () => {
          throw primaryError;
        },
        close: async () => {
          throw new Error("close failed");
        },
      }),
    }),
    (error) => error === primaryError,
  );
});

test("identity-scoped pending B is claimed, recorded, and finalized without consuming current A", async () => {
  const recoveryIdentity = "22222222-2222-4222-8222-222222222222";
  const locator = failureEvidence({
    recoveryIdentity,
    transactionId: "  transaction-b  ",
    requestId: "  request-b  ",
    failurePhase: "locator-phase",
  });
  const frozen = failureEvidence({
    ...locator,
    claimId: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
    failurePhase: "pending-b-phase",
    failedBuildHash: "pending-b-hash",
    summary: "pending B canonical summary",
  });
  const calls = [];
  const evidence = await prepareCanonicalLauncherFailureEvidence({
    initialEvidence: locator,
    runtimeLauncher: {
      supported: true,
      status: async (appBundlePath, requestedIdentity) => {
        calls.push(["status", appBundlePath, requestedIdentity]);
        return launcherStatus({
          match: failureMatch({
            recoveryIdentity,
            artifactState: "pending",
            transactionId: locator.transactionId,
            requestId: locator.requestId,
            evidence: locator,
          }),
          recoveredBuildId: "current-a-build",
        });
      },
      claimFailure: async (expected) => {
        calls.push(["claim", expected]);
        return failureClaim({
          recoveryIdentity,
          transactionId: locator.transactionId,
          requestId: locator.requestId,
          evidence: frozen,
        });
      },
    },
  });
  const recordedParams = buildLauncherRecoveryRecordParams(
    evidence,
    "/self-thread",
  );
  await recordClaimedLauncherFailureRecovery({
    evidence,
    recordRecovery: async () => {
      calls.push(["record", recordedParams]);
      return { accepted: true };
    },
    consumeRestartIntent: async (requestId) => {
      calls.push(["consume", requestId]);
    },
    finalizeFailureClaim: async (expected) => {
      calls.push(["finalize", expected]);
      return launcherRecoveryAcknowledgement({
        claimId: evidence.launcherClaim.claimId,
        recoveryIdentity,
        versionToken: evidence.launcherClaim.versionToken,
        transactionId: locator.transactionId,
        requestId: locator.requestId,
        remainingEvidence: true,
      });
    },
    requestQueueDrainRestart: (identity) => {
      calls.push(["exit", identity]);
    },
  });

  assert.equal(recordedParams.failurePhase, "pending-b-phase");
  assert.equal(recordedParams.failedBuildHash, "pending-b-hash");
  assert.equal(
    recordedParams.recoveredBuildId,
    "recovered-build",
    "Host must not enrich frozen evidence from status.current",
  );
  assert.equal(
    recordedParams.launcherClaimId,
    evidence.launcherClaim.claimId,
  );
  assert.equal(
    recordedParams.launcherEvidenceVersion,
    evidence.launcherClaim.versionToken,
  );
  assert.match(recordedParams.prompt, /pending B canonical summary/);
  assert.deepEqual(
    calls.map(([name]) => name),
    ["status", "claim", "record", "consume", "finalize", "exit"],
  );
  assert.deepEqual(calls[1][1], {
    recoveryIdentity,
    transactionId: locator.transactionId,
    requestId: locator.requestId,
    expectedVersion: versionToken("a"),
  });
});

test("missing identity-scoped launcher evidence fails before record or finalize", async () => {
  const locator = failureEvidence();
  let recordCalls = 0;
  let finalizeCalls = 0;
  await assert.rejects(
    (async () => {
      const evidence = await prepareCanonicalLauncherFailureEvidence({
        initialEvidence: locator,
        runtimeLauncher: {
          supported: true,
          status: async () => {
            throw new Error(
              "no canonical failure evidence artifact matches recovery identity",
            );
          },
          claimFailure: async () => {
            throw new Error("claim must not be attempted");
          },
        },
      });
      await recordClaimedLauncherFailureRecovery({
        evidence,
        recordRecovery: async () => {
          recordCalls += 1;
          return { accepted: true };
        },
        consumeRestartIntent: async () => {},
        finalizeFailureClaim: async () => {
          finalizeCalls += 1;
        },
        requestQueueDrainRestart: () => {},
      });
    })(),
    /no canonical failure evidence/,
  );
  assert.equal(recordCalls, 0);
  assert.equal(finalizeCalls, 0);
});

test("claimed wrapper startup resumes the frozen claim without creating another claim", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-claim-test-"));
  const evidencePath = path.join(
    root,
    ".failure-evidence.claimed-11111111-1111-4111-8111-111111111111.json",
  );
  const frozen = failureEvidence({
    claimId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    launcherOwnerNonce: "owner-nonce",
    failurePhase: "frozen-claimed-phase",
    summary: "frozen claimed summary",
  });
  const frozenVersion = failureEvidenceVersion(frozen);
  await fs.writeFile(
    evidencePath,
    JSON.stringify({
      schemaVersion: 1,
      claimId: frozen.claimId,
      recoveryIdentity: frozen.recoveryIdentity,
      launcherOwnerNonce: "owner-nonce",
      sourceVersionToken: versionToken("a"),
      versionToken: frozenVersion,
      evidence: frozen,
    }),
  );
  const locator = await readLauncherFailureEvidence(evidencePath, fs);
  let claimCalls = 0;
  const calls = [];
  const evidence = await prepareCanonicalLauncherFailureEvidence({
    initialEvidence: locator,
    runtimeLauncher: {
      supported: true,
      status: async () => {
        calls.push("status");
        return launcherStatus({
          match: failureMatch({
            artifactState: "claimed",
            versionToken: frozenVersion,
            evidence: frozen,
          }),
        });
      },
      claimFailure: async () => {
        claimCalls += 1;
        throw new Error("claimed evidence must not be claimed again");
      },
    },
  });

  assert.equal(claimCalls, 0);
  assert.equal(evidence.failurePhase, "frozen-claimed-phase");
  assert.match(evidence.prompt, /frozen claimed summary/);
  assert.equal(evidence.launcherClaim.claimId, frozen.claimId);
  assert.equal(evidence.launcherClaim.versionToken, frozenVersion);
  await recordClaimedLauncherFailureRecovery({
    evidence,
    recordRecovery: async () => {
      calls.push("record");
      return { accepted: true };
    },
    consumeRestartIntent: async () => {
      calls.push("consume");
    },
    finalizeFailureClaim: async () => {
      calls.push("finalize");
      return launcherRecoveryAcknowledgement({
        claimId: frozen.claimId,
        versionToken: frozenVersion,
      });
    },
    requestQueueDrainRestart: () => {
      calls.push("exit");
    },
  });
  assert.deepEqual(calls, ["status", "record", "consume", "finalize"]);
});

test("claimed wrapper rejects unknown outer fields", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-claim-outer-"));
  const evidencePath = path.join(root, "claimed.json");
  const evidence = failureEvidence({
    claimId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  });
  await fs.writeFile(
    evidencePath,
    JSON.stringify({
      schemaVersion: 1,
      claimId: evidence.claimId,
      recoveryIdentity: evidence.recoveryIdentity,
      launcherOwnerNonce: evidence.launcherOwnerNonce,
      sourceVersionToken: versionToken("a"),
      versionToken: failureEvidenceVersion(evidence),
      failurePhase: "outer-field-is-not-part-of-the-claim-schema",
      evidence,
    }),
  );
  await assert.rejects(
    readLauncherFailureEvidence(evidencePath, fs),
    /claim wrapper contains unknown field failurePhase/,
  );
});

test("claimed wrapper rejects unsupported outer schema versions", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-claim-v2-"));
  const evidencePath = path.join(root, "claimed.json");
  const evidence = failureEvidence({
    claimId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  });
  await fs.writeFile(
    evidencePath,
    JSON.stringify({
      schemaVersion: 2,
      claimId: evidence.claimId,
      recoveryIdentity: evidence.recoveryIdentity,
      launcherOwnerNonce: evidence.launcherOwnerNonce,
      sourceVersionToken: versionToken("a"),
      versionToken: failureEvidenceVersion(evidence),
      evidence,
    }),
  );
  await assert.rejects(
    readLauncherFailureEvidence(evidencePath, fs),
    /claim wrapper requires schemaVersion 1/,
  );
});

test("claimed wrapper rejects Host-local fields inside frozen launcher evidence", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-claim-pollution-"));
  const evidencePath = path.join(root, "claimed.json");
  const evidence = failureEvidence({
    claimId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    evidenceSnapshot: { contentHash: "host-local" },
  });
  await fs.writeFile(
    evidencePath,
    JSON.stringify({
      schemaVersion: 1,
      claimId: evidence.claimId,
      recoveryIdentity: evidence.recoveryIdentity,
      launcherOwnerNonce: evidence.launcherOwnerNonce,
      sourceVersionToken: versionToken("a"),
      versionToken: versionToken("b"),
      evidence,
    }),
  );
  await assert.rejects(
    readLauncherFailureEvidence(evidencePath, fs),
    /unknown field evidenceSnapshot/,
  );
});

test("claimed wrapper rejects unsupported nested evidence schema versions", async () => {
  const root = await fs.mkdtemp(
    path.join(os.tmpdir(), "launch-claim-nested-v2-"),
  );
  const evidencePath = path.join(root, "claimed.json");
  const evidence = failureEvidence({
    schemaVersion: 2,
    claimId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  });
  await fs.writeFile(
    evidencePath,
    JSON.stringify({
      schemaVersion: 1,
      claimId: evidence.claimId,
      recoveryIdentity: evidence.recoveryIdentity,
      launcherOwnerNonce: evidence.launcherOwnerNonce,
      sourceVersionToken: versionToken("a"),
      versionToken: failureEvidenceVersion(evidence),
      evidence,
    }),
  );
  await assert.rejects(
    readLauncherFailureEvidence(evidencePath, fs),
    /failure evidence requires schemaVersion 1/,
  );
});

test("created claims require requestedVersionMatched while existing claims may resume a frozen version", async () => {
  const locator = failureEvidence();
  for (const [claimState, shouldReject] of [
    ["created", true],
    ["existing", false],
  ]) {
    const frozen = failureEvidence({
      ...locator,
      claimId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      failurePhase: `${claimState}-frozen`,
    });
    const operation = prepareCanonicalLauncherFailureEvidence({
      initialEvidence: locator,
      runtimeLauncher: {
        supported: true,
        status: async () =>
          launcherStatus({
            match: failureMatch({ evidence: locator }),
          }),
        claimFailure: async () =>
          failureClaim({
            claimState,
            requestedVersionMatched: false,
            evidence: frozen,
          }),
      },
    });
    if (shouldReject) {
      await assert.rejects(operation, /unexpected evidence version/);
    } else {
      assert.equal((await operation).failurePhase, "existing-frozen");
    }
  }
});

test("active claim version mismatch retries are bounded before any record or finalize", async () => {
  const locator = failureEvidence();
  let statusCalls = 0;
  let claimCalls = 0;
  let recordCalls = 0;
  let consumeCalls = 0;
  let finalizeCalls = 0;
  let exitCalls = 0;

  await assert.rejects(
    (async () => {
      const evidence = await prepareCanonicalLauncherFailureEvidence({
        initialEvidence: locator,
        maxClaimAttempts: 3,
        runtimeLauncher: {
          supported: true,
          status: async () => {
            statusCalls += 1;
            return launcherStatus({
              match: failureMatch({
                versionToken: versionToken(
                  String.fromCharCode(96 + statusCalls),
                ),
                evidence: locator,
              }),
            });
          },
          claimFailure: async ({ expectedVersion }) => {
            claimCalls += 1;
            const error = new Error("failure evidence version changed");
            error.errorCode = "failure-evidence-version-changed";
            error.expectedVersion = expectedVersion;
            error.actualVersion = versionToken("f");
            throw error;
          },
        },
      });
      await recordClaimedLauncherFailureRecovery({
        evidence,
        recordRecovery: async () => {
          recordCalls += 1;
          return { accepted: true };
        },
        consumeRestartIntent: async () => {
          consumeCalls += 1;
        },
        finalizeFailureClaim: async () => {
          finalizeCalls += 1;
        },
        requestQueueDrainRestart: () => {
          exitCalls += 1;
        },
      });
    })(),
    /version changed/,
  );
  assert.equal(statusCalls, 3);
  assert.equal(claimCalls, 3);
  assert.equal(recordCalls, 0);
  assert.equal(consumeCalls, 0);
  assert.equal(finalizeCalls, 0);
  assert.equal(exitCalls, 0);
});

test("record failure leaves the frozen claim unfinalized for restart recovery", async () => {
  const evidence = launcherRecoveryEvidence();
  let finalizeCalls = 0;
  await assert.rejects(
    recordClaimedLauncherFailureRecovery({
      evidence,
      recordRecovery: async () => {
        throw new Error("record transport failed");
      },
      consumeRestartIntent: async () => {},
      finalizeFailureClaim: async () => {
        finalizeCalls += 1;
      },
      requestQueueDrainRestart: () => {},
    }),
    /record transport failed/,
  );
  assert.equal(finalizeCalls, 0);
});

test("failure evidence recovery runs once after both ready signals without ready env", async () => {
  let recoveries = 0;
  const readiness = createRuntimeLaunchReadiness({
    env: {},
    fs,
    onReady: async (payload) => {
      recoveries += 1;
      assert.equal(payload, null);
    },
  });

  assert.deepEqual(await readiness.markAppServerReady(), { written: false });
  assert.deepEqual(await readiness.markRendererReady(), {
    written: false,
    payload: null,
  });
  await readiness.markRendererReady();
  assert.equal(recoveries, 1);
});

test(
  "ready temp creation rejects a preplanted symlink collision without removing it",
  { skip: process.platform === "win32" },
  async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "launch-ready-link-"));
    const readyPath = path.join(root, "ready.json");
    const victimPath = path.join(root, "victim.json");
    const temporaryPath = `${readyPath}.${process.pid}.collision.tmp`;
    await fs.writeFile(victimPath, "preserve");
    await fs.symlink(victimPath, temporaryPath);
    const readiness = createRuntimeLaunchReadiness({
      env: readyEnvironment(readyPath),
      fs,
      createId: () => "collision",
    });

    await readiness.markAppServerReady();
    await assert.rejects(readiness.markRendererReady(), (error) =>
      ["EEXIST", "ELOOP"].includes(error?.code),
    );
    assert.equal(await fs.readFile(victimPath, "utf8"), "preserve");
    assert((await fs.lstat(temporaryPath)).isSymbolicLink());
  },
);

for (const failureStage of ["write", "close", "rename"]) {
  test(`ready ${failureStage} failure preserves the primary error and cleans its temp`, async () => {
    const primaryError = new Error(`${failureStage} failed`);
    const cleanupError = new Error("cleanup failed");
    let closeCalls = 0;
    let removeCalls = 0;
    const fakeFs = {
      mkdir: async () => {},
      open: async (_temporaryPath, flags, mode) => {
        assert(flags & fsConstants.O_EXCL);
        if (fsConstants.O_NOFOLLOW) {
          assert(flags & fsConstants.O_NOFOLLOW);
        }
        assert.equal(mode, 0o600);
        return {
          writeFile: async () => {
            if (failureStage === "write") {
              throw primaryError;
            }
          },
          sync: async () => {},
          close: async () => {
            closeCalls += 1;
            if (failureStage === "close" || closeCalls > 1) {
              throw failureStage === "close" && closeCalls === 1
                ? primaryError
                : cleanupError;
            }
          },
        };
      },
      rename: async () => {
        if (failureStage === "rename") {
          throw primaryError;
        }
      },
      rm: async () => {
        removeCalls += 1;
        throw cleanupError;
      },
    };
    const readiness = createRuntimeLaunchReadiness({
      env: readyEnvironment("/tmp/ready.json"),
      fs: fakeFs,
      createId: () => "failure",
    });

    await readiness.markAppServerReady();
    let observedError;
    try {
      await readiness.markRendererReady();
    } catch (error) {
      observedError = error;
    }
    assert.equal(observedError, primaryError);
    assert.equal(removeCalls, 1);
  });
}

function readyEnvironment(readyPath) {
  return {
    MORPHEUS_LAUNCH_TRANSACTION_ID: "tx",
    MORPHEUS_LAUNCH_BUILD_ID: "build",
    MORPHEUS_LAUNCH_INSTANCE_ID: "instance",
    MORPHEUS_LAUNCH_READY_PATH: readyPath,
  };
}

function launcherRecoveryEvidence(overrides = {}) {
  return {
    recoveryIdentity: "11111111-1111-4111-8111-111111111111",
    transactionId: "tx",
    requestId: "request",
    launcherTransactionId: "  tx  ",
    launcherRequestId: "  request  ",
    launcherClaim: launcherClaim(),
    ...overrides,
  };
}

function launcherRecoveryAcknowledgement(overrides = {}) {
  return {
    acknowledged: true,
    consumed: true,
    claimId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    recoveryIdentity: "11111111-1111-4111-8111-111111111111",
    versionToken: versionToken("a"),
    transactionId: "  tx  ",
    requestId: "  request  ",
    consumedEvidencePath: "/state/consumed.json",
    remainingEvidence: false,
    ...overrides,
  };
}

function failureEvidence(overrides = {}) {
  return {
    schemaVersion: 1,
    recoveryIdentity: "11111111-1111-4111-8111-111111111111",
    launcherOwnerNonce: "owner-nonce",
    occurredAt: "2026-09-08T00:00:00Z",
    transactionId: "  tx  ",
    requestId: "  request  ",
    requestedByThreadId: null,
    mode: "full",
    buildId: "failed-build",
    sourceCommit: "source-commit",
    manifestHash: "manifest-hash",
    failedBuildHash: "failed-hash",
    failurePhase: "launch",
    summary: "failure summary",
    reason: null,
    appBundlePath: "/Applications/Morpheus.app",
    exitCode: null,
    signal: null,
    readyTimeoutMs: null,
    logPath: null,
    transactionPath: null,
    recoveredBuildId: "recovered-build",
    claimId: null,
    acknowledged: false,
    ...overrides,
  };
}

function failureMatch(overrides = {}) {
  const evidence = overrides.evidence ?? failureEvidence();
  return {
    recoveryIdentity: evidence.recoveryIdentity,
    artifactState: "current",
    transactionId: evidence.transactionId ?? null,
    requestId: evidence.requestId ?? null,
    configuredEvidencePath: "/configured/failure-evidence.json",
    activeEvidencePath: "/active/failure-evidence.json",
    versionToken: versionToken("a"),
    evidence,
    ...overrides,
  };
}

function launcherStatus({ match, recoveredBuildId = "recovered-build" }) {
  return {
    ok: true,
    result: {
      state: { current: { buildId: recoveredBuildId } },
      failureEvidenceMatch: match,
    },
  };
}

function failureClaim(overrides = {}) {
  const evidence =
    overrides.evidence ??
    failureEvidence({
      claimId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    });
  return {
    ok: true,
    claimId: evidence.claimId,
    recoveryIdentity: evidence.recoveryIdentity,
    transactionId: evidence.transactionId ?? null,
    requestId: evidence.requestId ?? null,
    versionToken: versionToken("a"),
    sourceVersionToken: versionToken("a"),
    configuredEvidencePath: "/configured/claimed.json",
    activeEvidencePath: "/active/claimed.json",
    claimState: "created",
    requestedVersionMatched: true,
    evidence,
    ...overrides,
  };
}

function launcherClaim(overrides = {}) {
  return {
    claimId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    recoveryIdentity: "11111111-1111-4111-8111-111111111111",
    versionToken: versionToken("a"),
    sourceVersionToken: versionToken("a"),
    activeEvidencePath: "/active/claimed.json",
    configuredEvidencePath: "/configured/claimed.json",
    claimState: "created",
    ...overrides,
  };
}

function versionToken(hexDigit) {
  return `sha256:${hexDigit.repeat(64)}`;
}

function failureEvidenceVersion(evidence) {
  return `sha256:${crypto
    .createHash("sha256")
    .update(JSON.stringify(evidence))
    .digest("hex")}`;
}

function regularFileStat({
  dev,
  ino,
  size,
  mtimeMs = 10,
  ctimeMs = 20,
}) {
  return {
    dev,
    ino,
    size,
    mtimeMs,
    ctimeMs,
    isFile: () => true,
  };
}
