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

test("normalizes network command approval metadata and fallback decisions", () => {
  const request = normalizeApprovalRequest({
    id: "approval-network",
    method: "item/commandExecution/requestApproval",
    params: {
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "cmd-2",
      startedAtMs: 1_725_000_000_001,
      networkApprovalContext: {
        target: "https://example.test",
        host: "example.test",
      },
      additionalPermissions: {
        fileSystem: {
          read: ["/tmp/project/src"],
          write: ["/tmp/project/out"],
        },
        network: { enabled: true },
      },
      availableDecisions: ["maybe", "acceptForSession"],
    },
  });

  assert.ok(request);
  assert.equal(request.kind, "commandExecution");
  assert.equal(request.title, "Network access");
  assert.equal(request.detail, "https://example.test");
  assert.deepEqual(request.availableDecisions, ["acceptForSession"]);
  assert.deepEqual(request.metadata, [
    { label: "turn", value: "turn-1" },
    { label: "item", value: "cmd-2" },
    { label: "host", value: "example.test" },
    { label: "read paths", value: "/tmp/project/src" },
    { label: "write paths", value: "/tmp/project/out" },
    { label: "network", value: "enabled" },
  ]);
});

test("falls back to default command decisions when server decisions are absent", () => {
  const request = normalizeApprovalRequest({
    id: "approval-default-decisions",
    method: "item/commandExecution/requestApproval",
    params: {
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "cmd-3",
      command: "",
      availableDecisions: ["maybe"],
    },
  });

  assert.ok(request);
  assert.equal(request.detail, "Command execution requested approval.");
  assert.deepEqual(request.availableDecisions, ["accept", "decline", "cancel"]);
});

test("normalizes file change approval requests", () => {
  const request = normalizeApprovalRequest({
    id: 7,
    method: "item/fileChange/requestApproval",
    params: {
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "file-1",
      startedAtMs: 1_725_000_000_002,
      grantRoot: "/tmp/project",
    },
  });

  assert.ok(request);
  assert.equal(request.kind, "fileChange");
  assert.equal(request.title, "File change approval");
  assert.equal(request.detail, "Allow file changes under /tmp/project.");
  assert.deepEqual(request.metadata, [
    { label: "turn", value: "turn-1" },
    { label: "item", value: "file-1" },
    { label: "grant root", value: "/tmp/project" },
  ]);
  assert.deepEqual(request.availableDecisions, [
    "accept",
    "acceptForSession",
    "decline",
    "cancel",
  ]);
  assert.deepEqual(buildApprovalResponse(request, "decline"), {
    decision: "decline",
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

test("rejects malformed or unknown approval requests", () => {
  assert.equal(
    normalizeApprovalRequest({
      id: "unknown",
      method: "item/unknown/requestApproval",
      params: {
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "item-1",
      },
    }),
    null,
  );
  assert.equal(
    normalizeApprovalRequest({
      id: "missing-base",
      method: "item/permissions/requestApproval",
      params: {
        threadId: "thread-1",
        itemId: "item-1",
      },
    }),
    null,
  );
  assert.equal(
    normalizeApprovalRequest({
      id: "non-object",
      method: "item/permissions/requestApproval",
      params: null,
    }),
    null,
  );
});
