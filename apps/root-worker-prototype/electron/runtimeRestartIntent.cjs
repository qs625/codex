const path = require("node:path");
const { randomUUID } = require("node:crypto");

const INTENT_VERSION = 1;
const MAX_INTENT_RECORDS = 32;
const MAX_REQUEST_ID_BYTES = 256;
const RECOVERY_LEASE_MS = 60_000;
const RECOVERABLE_PHASES = new Set([
  "received",
  "executing",
  "failed",
  "completed",
]);

function createRuntimeRestartIntentStore(
  filePath,
  { fs, now = Date.now, createId = randomUUID } = {},
) {
  let updateQueue = Promise.resolve();

  const transact = (mutate) => {
    const operation = updateQueue.then(async () => {
      const state = await readState(filePath, fs);
      const result = await mutate(state);
      pruneRecords(state);
      await atomicWriteState(filePath, state, fs, createId);
      return result;
    });
    updateQueue = operation.catch(() => {});
    return operation;
  };

  return {
    accept(notification) {
      const input = normalizeNotification(notification);
      if (!input.ok) {
        return Promise.resolve(input);
      }
      return transact((state) => {
        const duplicate = state.records.find(
          (record) => record.requestId === input.requestId,
        );
        if (duplicate) {
          return { kind: "duplicate", record: { ...duplicate } };
        }

        const active = state.records.find(
          (record) =>
            !record.coalescedInto &&
            (effectivePhase(record) === "received" ||
              effectivePhase(record) === "executing"),
        );
        const timestamp = now();
        if (active && active.mode !== input.mode) {
          const record = {
            ...input,
            phase: "failed",
            createdAtMs: timestamp,
            updatedAtMs: timestamp,
            error: `Runtime refresh mode conflict: requested ${input.mode} while ${active.mode} is in progress`,
            executingRequestId: active.requestId,
          };
          state.records.push(record);
          return { kind: "conflict", record: { ...record } };
        }

        const record = {
          ...input,
          phase: "received",
          createdAtMs: timestamp,
          updatedAtMs: timestamp,
          ...(active ? { coalescedInto: active.requestId } : {}),
        };
        state.records.push(record);
        return {
          kind: active ? "coalesced" : "execute",
          record: { ...record },
          executingRequestId: active?.requestId ?? input.requestId,
        };
      });
    },

    updateGroup(requestId, phase, error = null) {
      return transact((state) => {
        const updated = [];
        const timestamp = now();
        for (const record of state.records) {
          if (
            record.requestId !== requestId &&
            record.coalescedInto !== requestId
          ) {
            continue;
          }
          if (record.phase === "recovering") {
            record.recoveryPhase = phase;
          } else {
            record.phase = phase;
          }
          record.updatedAtMs = timestamp;
          if (error) {
            record.error = error;
          } else {
            delete record.error;
          }
          updated.push({ ...record, phase: effectivePhase(record) });
        }
        return updated;
      });
    },

    claim(requestId, claimId) {
      return transact((state) => {
        const record = state.records.find(
          (candidate) => candidate.requestId === requestId,
        );
        if (!record) {
          return null;
        }
        return claimRecord(record, claimId, now(), RECOVERY_LEASE_MS);
      });
    },

    claimRecoverable(claimId) {
      return transact((state) => {
        const timestamp = now();
        return state.records.flatMap((record) => {
          if (
            typeof record.requestedByThreadId !== "string" ||
            record.requestedByThreadId.length === 0
          ) {
            return [];
          }
          const claimed = claimRecord(
            record,
            claimId,
            timestamp,
            RECOVERY_LEASE_MS,
          );
          return claimed ? [claimed] : [];
        });
      });
    },

    consumeClaim(requestId, claimId) {
      return transact((state) => {
        const record = state.records.find(
          (candidate) => candidate.requestId === requestId,
        );
        if (
          !record ||
          record.phase !== "recovering" ||
          record.recoveryClaimId !== claimId
        ) {
          return null;
        }
        record.outcomePhase = record.recoveryPhase;
        record.phase = "consumed";
        record.updatedAtMs = now();
        clearRecoveryClaim(record);
        return { ...record };
      });
    },

    releaseClaim(requestId, claimId) {
      return transact((state) => {
        const record = state.records.find(
          (candidate) => candidate.requestId === requestId,
        );
        if (
          !record ||
          record.phase !== "recovering" ||
          record.recoveryClaimId !== claimId
        ) {
          return null;
        }
        record.phase = record.recoveryPhase;
        record.updatedAtMs = now();
        clearRecoveryClaim(record);
        return { ...record };
      });
    },

    async recoverable() {
      await updateQueue;
      const state = await readState(filePath, fs);
      return state.records
        .filter((record) => {
          const phase =
            record.phase === "recovering" ? record.recoveryPhase : record.phase;
          return (
            RECOVERABLE_PHASES.has(phase) &&
            typeof record.requestedByThreadId === "string" &&
            record.requestedByThreadId.length > 0
          );
        })
        .map((record) => ({
          ...record,
          phase:
            record.phase === "recovering" ? record.recoveryPhase : record.phase,
        }));
    },
  };
}

function createRuntimeRestartController({
  store,
  execute,
  recover,
  broadcastStatus,
  logger = console,
} = {}) {
  const inFlight = new Set();
  const recoveryClaimId = randomUUID();
  let recoverPendingPromise = null;

  const recoverPersistedRecord = async (record) => {
    const claimed = await store.claim(record.requestId, recoveryClaimId);
    if (!claimed) {
      return false;
    }
    return recoverClaimedRecord(
      store,
      claimed,
      recoveryClaimId,
      recover,
      logger,
    );
  };

  return {
    async handle(notification) {
      let admission;
      try {
        admission = await store.accept(notification);
      } catch (error) {
        const reason = `Failed to persist runtime restart intent: ${errorMessage(error)}`;
        const failedRecord = {
          requestId: normalizeString(notification?.params?.requestId) ?? "",
          requestedByThreadId: normalizeString(
            notification?.params?.requestedByThreadId,
          ),
          mode: notification?.params?.mode ?? null,
          phase: "failed",
          error: reason,
        };
        reportFailure(
          broadcastStatus,
          failedRecord,
          reason,
        );
        await recoverWithoutIntent(failedRecord, recover, logger);
        return { ok: false, persisted: false, reason };
      }

      if (admission.kind === "invalid") {
        reportFailure(broadcastStatus, admission, admission.reason);
        return { ok: false, persisted: false, reason: admission.reason };
      }
      if (admission.kind === "duplicate") {
        return {
          ok: admission.record.phase !== "failed",
          duplicate: true,
          persisted: true,
          record: admission.record,
        };
      }
      if (admission.kind === "conflict") {
        reportFailure(broadcastStatus, admission.record, admission.record.error);
        await recoverPersistedRecord(admission.record);
        return {
          ok: false,
          conflict: true,
          persisted: true,
          record: admission.record,
        };
      }
      if (admission.kind === "coalesced") {
        return {
          ok: true,
          coalesced: true,
          persisted: true,
          record: admission.record,
        };
      }

      const execution = runAcceptedRestart({
        broadcastStatus,
        execute,
        logger,
        notification,
        recover,
        recoverPersistedRecord,
        requestId: admission.record.requestId,
        store,
      });
      inFlight.add(execution);
      void execution
        .catch((error) => {
          logger.error?.(
            "[prototype] failed to persist runtime restart outcome",
            JSON.stringify({
              requestId: admission.record.requestId,
              reason: errorMessage(error),
            }),
          );
        })
        .finally(() => inFlight.delete(execution));
      return {
        ok: true,
        persisted: true,
        record: admission.record,
      };
    },

    recoverPending() {
      if (!recoverPendingPromise) {
        recoverPendingPromise = recoverPendingRuntimeRestarts({
          store,
          recover,
          logger,
          claimId: recoveryClaimId,
        }).finally(() => {
          recoverPendingPromise = null;
        });
      }
      return recoverPendingPromise;
    },

    async waitForIdle() {
      await Promise.allSettled([...inFlight]);
    },
  };
}

async function runAcceptedRestart({
  broadcastStatus,
  execute,
  logger,
  notification,
  recover,
  recoverPersistedRecord,
  requestId,
  store,
}) {
  try {
    await store.updateGroup(requestId, "executing");
    const result = await execute(notification);
    const phase = result?.ok ? "completed" : "failed";
    const reason = result?.reason ?? (result?.ok ? null : "Runtime restart failed");
    const records = await store.updateGroup(requestId, phase, reason);
    if (phase === "failed") {
      reportFailure(broadcastStatus, records[0], reason);
    }
    if (phase === "completed" && notification?.params?.mode === "full") {
      return;
    }
    for (const record of records) {
      await recoverPersistedRecord(record);
    }
  } catch (error) {
    const reason = errorMessage(error);
    logger.error?.(
      "[prototype] runtime restart execution failed",
      JSON.stringify({ requestId, reason }),
    );
    let records;
    try {
      records = await store.updateGroup(requestId, "failed", reason);
    } catch (persistError) {
      const failureReason = `${reason}; failed to persist failure outcome: ${errorMessage(persistError)}`;
      const record = failureRecord(notification, requestId, failureReason);
      reportFailure(broadcastStatus, record, failureReason);
      await recoverWithoutIntent(record, recover, logger);
      return;
    }
    reportFailure(broadcastStatus, records[0], reason);
    for (const record of records) {
      await recoverPersistedRecord(record);
    }
  }
}

async function recoverPendingRuntimeRestarts({
  store,
  recover,
  logger = console,
  claimId = randomUUID(),
}) {
  const recoveredThreadIds = [];
  const failedThreadIds = [];
  const records = await store.claimRecoverable(claimId);
  const expectedRecords = [...records, ...(await store.recoverable())];
  for (const record of records) {
    const recovered = await recoverClaimedRecord(
      store,
      record,
      claimId,
      recover,
      logger,
    );
    (recovered ? recoveredThreadIds : failedThreadIds).push(
      record.requestedByThreadId,
    );
  }
  return {
    recoveredThreadIds,
    failedThreadIds,
    expectedThreadIds: [
      ...new Set(expectedRecords.map((record) => record.requestedByThreadId)),
    ],
    focusThreadId: recoveredThreadIds[0] ?? null,
  };
}

async function recoverClaimedRecord(store, record, claimId, recover, logger) {
  if (typeof recover !== "function" || !record.requestedByThreadId) {
    await store.releaseClaim(record.requestId, claimId);
    return false;
  }
  try {
    await recover(record);
    const consumed = await store.consumeClaim(record.requestId, claimId);
    if (!consumed) {
      logger.warn?.(
        "[prototype] runtime restart recovery claim was lost before consumption",
        JSON.stringify({
          requestId: record.requestId,
          threadId: record.requestedByThreadId,
        }),
      );
      return false;
    }
    return true;
  } catch (error) {
    await store.releaseClaim(record.requestId, claimId).catch(() => {});
    logger.warn?.(
      "[prototype] failed to recover runtime restart intent",
      JSON.stringify({
        requestId: record.requestId,
        threadId: record.requestedByThreadId,
        reason: errorMessage(error),
      }),
    );
    return false;
  }
}

async function recoverWithoutIntent(record, recover, logger) {
  if (typeof recover !== "function" || !record.requestedByThreadId) {
    return false;
  }
  try {
    await recover(record);
    return true;
  } catch (error) {
    logger.warn?.(
      "[prototype] failed to report unpersisted runtime restart failure",
      JSON.stringify({
        requestId: record.requestId,
        threadId: record.requestedByThreadId,
        reason: errorMessage(error),
      }),
    );
    return false;
  }
}

function expectedRuntimeRestartPrompt(record) {
  const outcome =
    record.phase === "failed"
      ? `failed before restart completed: ${record.error ?? "unknown failure"}`
      : record.phase === "completed"
        ? "completed"
        : "was interrupted after the Host durably accepted it";
  return [
    `Morpheus recovered expected runtime restart request ${record.requestId} (${record.mode}); it ${outcome}.`,
    "This is an expected restart recovery, not a generic crash.",
    "Do not call request_runtime_restart again automatically.",
    "Review the recovered context and continue from the durable outcome.",
  ].join(" ");
}

function normalizeNotification(notification) {
  const requestId = normalizeString(notification?.params?.requestId);
  const requestedByThreadId = normalizeString(
    notification?.params?.requestedByThreadId,
  );
  const mode = notification?.params?.mode;
  if (
    !requestId ||
    Buffer.byteLength(requestId, "utf8") > MAX_REQUEST_ID_BYTES ||
    !requestedByThreadId ||
    (mode !== "hot" && mode !== "full")
  ) {
    return {
      kind: "invalid",
      requestId: requestId ?? "",
      requestedByThreadId,
      mode: mode ?? null,
      reason: !requestId
        ? "Invalid runtime restart requestId"
        : Buffer.byteLength(requestId, "utf8") > MAX_REQUEST_ID_BYTES
          ? `Runtime restart requestId exceeds ${MAX_REQUEST_ID_BYTES} UTF-8 bytes`
        : !requestedByThreadId
          ? "Invalid runtime restart requestedByThreadId"
        : `Invalid runtime restart mode: ${String(mode ?? "missing")}`,
    };
  }
  return {
    ok: true,
    requestId,
    requestedByThreadId,
    mode,
    reason: normalizeString(notification?.params?.reason),
  };
}

function failureRecord(notification, requestId, reason) {
  return {
    requestId,
    requestedByThreadId: normalizeString(
      notification?.params?.requestedByThreadId,
    ),
    mode: notification?.params?.mode ?? null,
    phase: "failed",
    error: reason,
  };
}

function normalizeString(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function claimRecord(record, claimId, timestamp, leaseMs) {
  const recovering =
    record.phase === "recovering" &&
    typeof record.recoveryPhase === "string" &&
    RECOVERABLE_PHASES.has(record.recoveryPhase);
  if (
    recovering &&
    Number.isFinite(record.recoveryLeaseExpiresAtMs) &&
    record.recoveryLeaseExpiresAtMs > timestamp
  ) {
    return null;
  }
  const recoveryPhase = recovering ? record.recoveryPhase : record.phase;
  if (!RECOVERABLE_PHASES.has(recoveryPhase)) {
    return null;
  }
  record.phase = "recovering";
  record.recoveryPhase = recoveryPhase;
  record.recoveryClaimId = claimId;
  record.recoveryLeaseExpiresAtMs = timestamp + leaseMs;
  record.updatedAtMs = timestamp;
  return { ...record, phase: recoveryPhase };
}

function clearRecoveryClaim(record) {
  delete record.recoveryPhase;
  delete record.recoveryClaimId;
  delete record.recoveryLeaseExpiresAtMs;
}

function effectivePhase(record) {
  return record.phase === "recovering" && typeof record.recoveryPhase === "string"
    ? record.recoveryPhase
    : record.phase;
}

async function readState(filePath, fs) {
  try {
    const parsed = JSON.parse(await fs.readFile(filePath, "utf8"));
    return {
      version: INTENT_VERSION,
      records: Array.isArray(parsed?.records)
        ? parsed.records.filter(isValidRecord)
        : [],
    };
  } catch (error) {
    if (error?.code === "ENOENT") {
      return { version: INTENT_VERSION, records: [] };
    }
    throw error;
  }
}

async function atomicWriteState(filePath, state, fs, createId) {
  const directory = path.dirname(filePath);
  const temporaryPath = path.join(
    directory,
    `.${path.basename(filePath)}.${createId()}.tmp`,
  );
  await fs.mkdir(directory, { recursive: true });
  let handle;
  try {
    handle = await fs.open(temporaryPath, "wx", 0o600);
    await handle.writeFile(`${JSON.stringify(state, null, 2)}\n`, "utf8");
    await handle.sync();
    await handle.close();
    handle = null;
    await fs.rename(temporaryPath, filePath);
    await syncDirectoryBestEffort(directory, fs);
  } catch (error) {
    await handle?.close().catch(() => {});
    await fs.unlink(temporaryPath).catch(() => {});
    throw error;
  }
}

async function syncDirectoryBestEffort(directory, fs) {
  let handle;
  try {
    handle = await fs.open(directory, "r");
    await handle.sync();
  } catch {
    // Some platforms do not support fsync on directory handles.
  } finally {
    await handle?.close().catch(() => {});
  }
}

function pruneRecords(state) {
  if (state.records.length <= MAX_INTENT_RECORDS) {
    return;
  }
  const activePrimaryIds = new Set(
    state.records
      .filter(
        (record) =>
          !record.coalescedInto &&
          (effectivePhase(record) === "received" ||
            effectivePhase(record) === "executing"),
      )
      .map((record) => record.requestId),
  );
  const primaryRecords = state.records.filter((record) =>
    activePrimaryIds.has(record.requestId),
  );
  const remaining = state.records.filter(
    (record) => !activePrimaryIds.has(record.requestId),
  );
  state.records = [
    ...primaryRecords,
    ...remaining.slice(
      Math.max(0, remaining.length - (MAX_INTENT_RECORDS - primaryRecords.length)),
    ),
  ].slice(0, MAX_INTENT_RECORDS);
}

function isValidRecord(record) {
  return (
    record &&
    typeof record.requestId === "string" &&
    (record.mode === "hot" || record.mode === "full") &&
    typeof record.phase === "string"
  );
}

function reportFailure(broadcastStatus, record, reason) {
  broadcastStatus?.({
    lifecycle: {
      type: "clientRelaunch",
      phase: "failed",
      mode: record?.mode ?? null,
      requestId: record?.requestId ?? "",
      reason,
    },
    relaunch: {
      ok: false,
      relaunching: false,
      mode: record?.mode ?? null,
      reason,
    },
  });
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

module.exports = {
  createRuntimeRestartController,
  createRuntimeRestartIntentStore,
  expectedRuntimeRestartPrompt,
  MAX_REQUEST_ID_BYTES,
  recoverPendingRuntimeRestarts,
};
