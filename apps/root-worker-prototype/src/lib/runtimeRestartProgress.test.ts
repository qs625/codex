import assert from "node:assert/strict";
import test from "node:test";

import {
  runtimeRestartProgressFromBootstrap,
  runtimeRestartProgressFromStatus,
} from "./runtimeRestartProgress";

test("runtime restart progress maps active status snapshots", () => {
  const progress = runtimeRestartProgressFromStatus(
    {
      connected: true,
      runtimeRestart: {
        requestId: "restart-1",
        requestedByThreadId: "thread-1",
        phase: "executing",
        reason: "install update",
        updatedAtMs: 123,
      },
    },
    null,
    () => 999,
  );

  assert.equal(progress?.status, "active");
  assert.equal(progress?.requestId, "restart-1");
  assert.equal(progress?.originThreadId, "thread-1");
  assert.equal(progress?.stageLabel, "Restart accepted");
  assert.equal(progress?.updatedAtMs, 123);
});

test("runtime restart progress overlays lifecycle capsule stages", () => {
  const progress = runtimeRestartProgressFromStatus(
    {
      connected: true,
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "selected",
        requestId: "restart-1",
        activationId: "activation-1",
        releaseId: "release-1",
      },
    },
    {
      status: "active",
      requestId: "restart-1",
      originThreadId: "thread-1",
      stage: "executing",
      stageLabel: "Restart accepted",
      message: "Runtime restart is being executed.",
      reason: null,
      activationId: null,
      releaseId: null,
      updatedAtMs: 1,
    },
    () => 456,
  );

  assert.equal(progress?.status, "active");
  assert.equal(progress?.stageLabel, "Candidate selected");
  assert.equal(progress?.originThreadId, "thread-1");
  assert.equal(progress?.activationId, "activation-1");
  assert.equal(progress?.releaseId, "release-1");
  assert.equal(progress?.updatedAtMs, 456);
});

test("runtime restart progress restores recent restart from bootstrap", () => {
  const progress = runtimeRestartProgressFromBootstrap(
    {
      recoveredThreadIds: ["thread-1"],
      failedThreadIds: [],
      expectedRequestIds: ["restart-1"],
      expectedThreadIds: ["thread-1"],
      recoveryOccurrenceId: "runtime-restart:restart-1",
      focusThreadId: "thread-1",
    },
    () => 789,
  );

  assert.equal(progress?.status, "recovered");
  assert.equal(progress?.requestId, "restart-1");
  assert.equal(progress?.originThreadId, "thread-1");
  assert.equal(progress?.stageLabel, "Recovered");
  assert.equal(progress?.updatedAtMs, 789);
});

test("runtime restart progress ignores unrelated status without clearing recent state", () => {
  const previous = runtimeRestartProgressFromBootstrap({
    recoveredThreadIds: [],
    failedThreadIds: [],
    expectedRequestIds: ["restart-1"],
    expectedThreadIds: ["thread-1"],
    recoveryOccurrenceId: "runtime-restart:restart-1",
    focusThreadId: null,
  });

  assert.equal(
    runtimeRestartProgressFromStatus({ connected: true }, previous),
    previous,
  );
  assert.equal(runtimeRestartProgressFromBootstrap(null), null);
});

test("runtime restart progress ignores lifecycle without restart context", () => {
  assert.equal(
    runtimeRestartProgressFromStatus({
      connected: true,
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "preparing",
      },
    }),
    null,
  );

  const previous = runtimeRestartProgressFromBootstrap({
    recoveredThreadIds: [],
    failedThreadIds: [],
    expectedRequestIds: ["restart-1"],
    expectedThreadIds: ["thread-1"],
    recoveryOccurrenceId: "runtime-restart:restart-1",
    focusThreadId: null,
  });

  assert.equal(
    runtimeRestartProgressFromStatus(
      {
        connected: true,
        lifecycle: {
          type: "clientRelaunch",
          phase: "completed",
          requestId: "other-restart",
        },
      },
      previous,
    ),
    previous,
  );
});
