const { parentPort, workerData } = require("node:worker_threads");

const {
  resolveInstalledArtifactUpdatePlan,
  updateInstalledArtifacts,
} = require("./installedArtifactUpdate.cjs");

function serializeError(error) {
  return {
    name: error instanceof Error ? error.name : "Error",
    message: error instanceof Error ? error.message : String(error),
    stack: error instanceof Error ? error.stack : null,
  };
}

async function run() {
  switch (workerData?.operation) {
    case "resolvePlan":
      return resolveInstalledArtifactUpdatePlan(workerData.payload);
    case "update":
      return updateInstalledArtifacts(workerData.payload?.plan);
    default:
      throw new Error(
        `Unsupported installed artifact worker operation: ${String(workerData?.operation)}`,
      );
  }
}

void run().then(
  (result) => parentPort.postMessage({ ok: true, result }),
  (error) => parentPort.postMessage({ ok: false, error: serializeError(error) }),
);
