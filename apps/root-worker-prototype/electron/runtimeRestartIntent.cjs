const path = require("node:path");
const { randomUUID } = require("node:crypto");

const INTENT_VERSION = 2;
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
      if (input.kind === "unsupported") {
        return transact((state) => {
          const duplicate = state.records.find(
            (record) => record.requestId === input.requestId,
          );
          if (duplicate) {
            return {
              kind: "unsupported",
              duplicate: true,
              reason: duplicate.error ?? input.reason,
              record: { ...duplicate, phase: effectivePhase(duplicate) },
            };
          }
          const timestamp = now();
          const record = {
            requestId: input.requestId,
            requestedByThreadId: input.requestedByThreadId,
            reason: input.requestedReason,
            phase: "failed",
            createdAtMs: timestamp,
            updatedAtMs: timestamp,
            error: input.reason,
          };
          state.records.push(record);
          return {
            kind: "unsupported",
            reason: input.reason,
            record: { ...record },
          };
        });
      }
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

    updateGroup(
      requestId,
      phase,
      error = null,
      completedByHostInstanceId = null,
    ) {
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
          if (record.phase === "consumed") {
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
          if (phase === "completed" && completedByHostInstanceId) {
            record.completedByHostInstanceId = completedByHostInstanceId;
          } else if (phase !== "completed") {
            delete record.completedByHostInstanceId;
          }
          updated.push({ ...record, phase: effectivePhase(record) });
        }
        return updated;
      });
    },

    claimRecoverable(
      claimId,
      currentHostInstanceId = null,
      requestedByThreadId = null,
      recoverablePhases = RECOVERABLE_PHASES,
    ) {
      return transact((state) => {
        const timestamp = now();
        return state.records.flatMap((record) => {
          if (
            typeof record.requestedByThreadId !== "string" ||
            record.requestedByThreadId.length === 0 ||
            (requestedByThreadId &&
              record.requestedByThreadId !== requestedByThreadId) ||
            !recoverablePhases.has(effectivePhase(record))
          ) {
            return [];
          }
          const claimed = claimRecord(
            record,
            claimId,
            timestamp,
            RECOVERY_LEASE_MS,
            currentHostInstanceId,
          );
          return claimed ? [claimed] : [];
        });
      });
    },

    consumeClaims(requestIds, claimId) {
      return transact((state) => {
        const requested = new Set(requestIds);
        const records = state.records.filter((record) =>
          requested.has(record.requestId),
        );
        if (
          records.length !== requested.size ||
          records.some(
            (record) =>
              record.phase !== "recovering" ||
              record.recoveryClaimId !== claimId,
          )
        ) {
          return null;
        }
        const timestamp = now();
        return records.map((record) => {
          record.outcomePhase = record.recoveryPhase;
          record.phase = "consumed";
          record.updatedAtMs = timestamp;
          clearRecoveryClaim(record);
          return { ...record };
        });
      });
    },

    releaseClaims(requestIds, claimId) {
      return transact((state) => {
        const requested = new Set(requestIds);
        const records = state.records.filter((record) =>
          requested.has(record.requestId),
        );
        if (
          records.length !== requested.size ||
          records.some(
            (record) =>
              record.phase !== "recovering" ||
              record.recoveryClaimId !== claimId,
          )
        ) {
          return null;
        }
        const timestamp = now();
        return records.map((record) => {
          record.phase = record.recoveryPhase;
          record.updatedAtMs = timestamp;
          clearRecoveryClaim(record);
          return { ...record };
        });
      });
    },

    async recoverable(
      requestedByThreadId = null,
      recoverablePhases = RECOVERABLE_PHASES,
    ) {
      await updateQueue;
      const state = await readState(filePath, fs);
      return state.records
        .filter((record) => {
          const phase =
            record.phase === "recovering" ? record.recoveryPhase : record.phase;
          return (
            recoverablePhases.has(phase) &&
            typeof record.requestedByThreadId === "string" &&
            record.requestedByThreadId.length > 0 &&
            (!requestedByThreadId ||
              record.requestedByThreadId === requestedByThreadId)
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
  hostInstanceId = randomUUID(),
  logger = console,
} = {}) {
  const inFlight = new Set();
  const recoveryClaimId = randomUUID();
  const currentRequestIdByThread = new Map();
  const terminalThreadIds = new Set();
  let recoverPendingPromise = null;

  const recoverFailedAfterTerminal = async (requestedByThreadId) => {
    const threadId = normalizeString(requestedByThreadId);
    if (!threadId || !terminalThreadIds.has(threadId)) {
      return emptyRecoveryResult();
    }
    const result = await recoverPendingRuntimeRestarts({
      store,
      recover,
      logger,
      claimId: recoveryClaimId,
      hostInstanceId,
      requestedByThreadId: threadId,
      recoverablePhases: new Set(["failed"]),
    });
    if (
      result.recoveredThreadIds.includes(threadId) &&
      !result.failedThreadIds.includes(threadId)
    ) {
      terminalThreadIds.delete(threadId);
    }
    return result;
  };

  const recoverFailedRecordsAfterTerminal = async (records) => {
    const threadIds = [
      ...new Set(
        records
          .map((record) => normalizeString(record.requestedByThreadId))
          .filter(Boolean),
      ),
    ];
    await Promise.all(
      threadIds.map((threadId) => recoverFailedAfterTerminal(threadId)),
    );
  };

  const clearTerminalRecords = (records) => {
    for (const record of records) {
      const threadId = normalizeString(record.requestedByThreadId);
      if (
        threadId &&
        currentRequestIdByThread.get(threadId) === record.requestId
      ) {
        terminalThreadIds.delete(threadId);
      }
    }
  };

  return {
    async handle(notification) {
      const input = normalizeNotification(notification);
      if (input.ok || input.kind === "unsupported") {
        const currentRequestId = currentRequestIdByThread.get(
          input.requestedByThreadId,
        );
        if (currentRequestId !== input.requestId) {
          currentRequestIdByThread.set(
            input.requestedByThreadId,
            input.requestId,
          );
          terminalThreadIds.delete(input.requestedByThreadId);
        }
      }
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
          phase: "failed",
          error: reason,
        };
        reportFailure(
          broadcastStatus,
          failedRecord,
          reason,
        );
        return { ok: false, persisted: false, reason };
      }

      if (admission.kind === "invalid") {
        reportFailure(broadcastStatus, admission, admission.reason);
        return { ok: false, persisted: false, reason: admission.reason };
      }
      if (admission.kind === "unsupported") {
        reportFailure(broadcastStatus, admission.record, admission.reason);
        return {
          ok: false,
          unsupported: true,
          persisted: true,
          reason: admission.reason,
          record: admission.record,
        };
      }
      if (admission.kind === "duplicate") {
        return {
          ok: admission.record.phase !== "failed",
          duplicate: true,
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
        clearTerminalRecords,
        recoverFailedRecordsAfterTerminal,
        hostInstanceId,
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
          hostInstanceId,
        }).finally(() => {
          recoverPendingPromise = null;
        });
      }
      return recoverPendingPromise;
    },

    recoverPendingForThread(requestedByThreadId) {
      const threadId = normalizeString(requestedByThreadId);
      if (!threadId) {
        return Promise.resolve(emptyRecoveryResult());
      }
      terminalThreadIds.add(threadId);
      return recoverFailedAfterTerminal(threadId);
    },

    async waitForIdle() {
      await Promise.allSettled([...inFlight]);
    },
  };
}

async function runAcceptedRestart({
  broadcastStatus,
  clearTerminalRecords,
  execute,
  logger,
  notification,
  recoverFailedRecordsAfterTerminal,
  hostInstanceId,
  requestId,
  store,
}) {
  try {
    await store.updateGroup(requestId, "executing");
    const markExpectedRestartHandoffReady = async () => {
      const records = await store.updateGroup(
        requestId,
        "completed",
        null,
        hostInstanceId,
      );
      clearTerminalRecords(records);
      return records;
    };
    const result = await execute(notification, {
      markExpectedRestartHandoffReady,
    });
    const phase = result?.ok ? "completed" : "failed";
    const reason = result?.reason ?? (result?.ok ? null : "Runtime restart failed");
    const records = await store.updateGroup(
      requestId,
      phase,
      reason,
      phase === "completed" ? hostInstanceId : null,
    );
    if (phase === "failed") {
      reportFailure(broadcastStatus, records[0], reason);
    }
    if (phase === "completed") {
      clearTerminalRecords(records);
      return;
    }
    await recoverFailedRecordsAfterTerminal(records);
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
      return;
    }
    reportFailure(broadcastStatus, records[0], reason);
    await recoverFailedRecordsAfterTerminal(records);
  }
}

async function recoverPendingRuntimeRestarts({
  store,
  recover,
  logger = console,
  claimId = randomUUID(),
  hostInstanceId = randomUUID(),
  requestedByThreadId = null,
  recoverablePhases = RECOVERABLE_PHASES,
}) {
  const recoveredThreadIds = [];
  const failedThreadIds = [];
  const records = await store.claimRecoverable(
    claimId,
    hostInstanceId,
    requestedByThreadId,
    recoverablePhases,
  );
  const expectedRecords = [
    ...records,
    ...(await store.recoverable(requestedByThreadId, recoverablePhases)),
  ];
  for (const group of groupRecoveryRecords(records)) {
    const recovered = await recoverClaimedGroup(
      store,
      group,
      claimId,
      recover,
      logger,
    );
    (recovered ? recoveredThreadIds : failedThreadIds).push(
      group[0].requestedByThreadId,
    );
  }
  return {
    recoveredThreadIds,
    failedThreadIds,
    expectedRequestIds: restartRecoveryRequestIds(expectedRecords),
    expectedThreadIds: [
      ...new Set(expectedRecords.map((record) => record.requestedByThreadId)),
    ],
    recoveryOccurrenceId: restartRecoveryOccurrenceId(expectedRecords),
    focusThreadId: recoveredThreadIds[0] ?? null,
  };
}

function restartRecoveryRequestIds(records) {
  return [
    ...new Set(
      records
        .map((record) => normalizeString(record.coalescedInto) ?? record.requestId)
        .filter(Boolean),
    ),
  ].sort();
}

function restartRecoveryOccurrenceId(records) {
  const requestIds = restartRecoveryRequestIds(records);
  return requestIds.length > 0
    ? `runtime-restart:${requestIds.join(",")}`
    : null;
}

function recoverRuntimeRestartAfterThreadTerminal(controller, notification) {
  const threadId = normalizeString(notification?.params?.threadId);
  if (
    notification?.method !== "thread/status/changed" ||
    notification?.params?.lifecycleStatus?.type !== "final" ||
    !threadId
  ) {
    return Promise.resolve(emptyRecoveryResult());
  }
  return controller.recoverPendingForThread(threadId);
}

function emptyRecoveryResult() {
  return {
    recoveredThreadIds: [],
    failedThreadIds: [],
    expectedRequestIds: [],
    expectedThreadIds: [],
    recoveryOccurrenceId: null,
    focusThreadId: null,
  };
}

function groupRecoveryRecords(records) {
  const groups = new Map();
  for (const record of records) {
    const primaryRequestId = record.coalescedInto ?? record.requestId;
    const key = `${primaryRequestId}\0${record.requestedByThreadId}`;
    const group = groups.get(key) ?? [];
    group.push(record);
    groups.set(key, group);
  }
  return [...groups.values()].map((group) =>
    group.sort((left, right) => {
      const leftIsCoalesced = left.coalescedInto ? 1 : 0;
      const rightIsCoalesced = right.coalescedInto ? 1 : 0;
      return leftIsCoalesced - rightIsCoalesced;
    }),
  );
}

async function recoverClaimedGroup(store, records, claimId, recover, logger) {
  const record = records[0];
  const requestIds = records.map((candidate) => candidate.requestId);
  if (typeof recover !== "function" || !record?.requestedByThreadId) {
    await store.releaseClaims(requestIds, claimId);
    return false;
  }
  try {
    await recover(record);
    const consumed = await store.consumeClaims(requestIds, claimId);
    if (!consumed) {
      logger.warn?.(
        "[prototype] runtime restart recovery claim was lost before consumption",
        JSON.stringify({
          requestIds,
          threadId: record.requestedByThreadId,
        }),
      );
      return false;
    }
    return true;
  } catch (error) {
    await store.releaseClaims(requestIds, claimId).catch(() => {});
    logger.warn?.(
      "[prototype] failed to recover runtime restart intent",
      JSON.stringify({
        requestIds,
        threadId: record.requestedByThreadId,
        reason: errorMessage(error),
      }),
    );
    return false;
  }
}

function normalizeNotification(notification) {
  const requestId = normalizeString(notification?.params?.requestId);
  const requestedByThreadId = normalizeString(
    notification?.params?.requestedByThreadId,
  );
  if (
    !requestId ||
    Buffer.byteLength(requestId, "utf8") > MAX_REQUEST_ID_BYTES ||
    !requestedByThreadId
  ) {
    return {
      kind: "invalid",
      requestId: requestId ?? "",
      requestedByThreadId,
      reason: !requestId
        ? "Invalid runtime restart requestId"
        : Buffer.byteLength(requestId, "utf8") > MAX_REQUEST_ID_BYTES
          ? `Runtime restart requestId exceeds ${MAX_REQUEST_ID_BYTES} UTF-8 bytes`
          : "Invalid runtime restart requestedByThreadId",
    };
  }
  if (Object.hasOwn(notification?.params ?? {}, "mode")) {
    return {
      kind: "invalid",
      requestId,
      requestedByThreadId,
      reason: "Runtime restart requests do not support mode.",
    };
  }
  return {
    ok: true,
    requestId,
    requestedByThreadId,
    reason: normalizeString(notification?.params?.reason),
  };
}

function failureRecord(notification, requestId, reason) {
  return {
    requestId,
    requestedByThreadId: normalizeString(
      notification?.params?.requestedByThreadId,
    ),
    phase: "failed",
    error: reason,
  };
}

function normalizeString(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function claimRecord(
  record,
  claimId,
  timestamp,
  leaseMs,
  currentHostInstanceId = null,
) {
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
  if (
    recoveryPhase === "completed" &&
    currentHostInstanceId &&
    record.completedByHostInstanceId === currentHostInstanceId
  ) {
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
        ? parsed.records.flatMap(normalizeStoredRecord)
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

function normalizeStoredRecord(record) {
  if (
    !record ||
    typeof record.requestId !== "string" ||
    typeof record.phase !== "string"
  ) {
    return [];
  }
  if (Object.hasOwn(record, "mode")) {
    return [];
  }
  return [{ ...record }];
}

function reportFailure(broadcastStatus, record, reason) {
  broadcastStatus?.({
    lifecycle: {
      type: "clientRelaunch",
      phase: "failed",
      requestId: record?.requestId ?? "",
      reason,
    },
    relaunch: {
      ok: false,
      relaunching: false,
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
  MAX_REQUEST_ID_BYTES,
  recoverPendingRuntimeRestarts,
  recoverRuntimeRestartAfterThreadTerminal,
};
