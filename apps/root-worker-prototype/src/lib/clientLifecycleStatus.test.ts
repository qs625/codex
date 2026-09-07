import assert from "node:assert/strict";
import test from "node:test";

import { clientLifecycleFailureReason } from "./clientLifecycleStatus";

test("client lifecycle failure exposes the host reason", () => {
  assert.equal(
    clientLifecycleFailureReason({
      connected: true,
      lifecycle: {
        type: "clientRelaunch",
        phase: "failed",
        mode: "hot",
        requestId: "restart-call",
        reason: "Build failed",
      },
    }),
    "Build failed",
  );
});

test("client lifecycle non-failure status does not replace the error surface", () => {
  assert.equal(
    clientLifecycleFailureReason({
      connected: true,
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "building",
        mode: "hot",
        requestId: "restart-call",
      },
    }),
    null,
  );
});

test("client lifecycle failure without a reason has a stable fallback", () => {
  assert.equal(
    clientLifecycleFailureReason({
      connected: true,
      lifecycle: {
        type: "clientRelaunch",
        phase: "failed",
      },
    }),
    "Runtime refresh failed.",
  );
});
