import test from "node:test";
import assert from "node:assert/strict";

import {
  buildApprovalResponse,
  normalizeApprovalRequest,
} from "./approvalRequests";

test("normalizes command approval requests and builds decision responses", () => {
  const request = normalizeApprovalRequest({
    id: "approval-1",
    method: "item/commandExecution/requestApproval",
    params: {
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "cmd-1",
      startedAtMs: 1_725_000_000_000,
      reason: "Needs network access",
      command: "rtk pnpm build",
      cwd: "/tmp/project",
      additionalPermissions: {
        network: { enabled: true },
      },
      availableDecisions: ["accept", "decline"],
    },
  });

  assert.ok(request);
  assert.equal(request.title, "Command approval");
  assert.equal(request.detail, "rtk pnpm build");
  assert.deepEqual(request.availableDecisions, ["accept", "decline"]);
  assert.deepEqual(buildApprovalResponse(request, "accept"), {
    decision: "accept",
  });
});

test("builds turn and session scoped permissions approval responses", () => {
  const request = normalizeApprovalRequest({
    id: 9,
    method: "item/permissions/requestApproval",
    params: {
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "permission-1",
      startedAtMs: 1_725_000_000_000,
      permissions: {
        fileSystem: {
          write: ["/tmp/project"],
        },
      },
    },
  });

  assert.ok(request);
  assert.equal(request.kind, "permissions");
  assert.deepEqual(buildApprovalResponse(request, "accept"), {
    permissions: {
      fileSystem: {
        write: ["/tmp/project"],
      },
    },
    scope: "turn",
  });
  assert.deepEqual(buildApprovalResponse(request, "acceptForSession"), {
    permissions: {
      fileSystem: {
        write: ["/tmp/project"],
      },
    },
    scope: "session",
  });
  assert.deepEqual(buildApprovalResponse(request, "decline"), {
    permissions: {},
    scope: "turn",
  });
});

