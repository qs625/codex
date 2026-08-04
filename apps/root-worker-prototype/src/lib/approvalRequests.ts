import type {
  ApprovalDecision,
  ApprovalRequest,
  JsonRpcRequestId,
} from "../types";

type ServerRequest = {
  id: JsonRpcRequestId;
  method: string;
  params?: unknown;
};

const COMMAND_APPROVAL_METHOD = "item/commandExecution/requestApproval";
const FILE_CHANGE_APPROVAL_METHOD = "item/fileChange/requestApproval";
const PERMISSIONS_APPROVAL_METHOD = "item/permissions/requestApproval";

export function approvalRequestKey(requestId: JsonRpcRequestId) {
  return String(requestId);
}

export function normalizeApprovalRequest(
  request: ServerRequest,
): ApprovalRequest | null {
  const params = objectValue(request.params);
  if (!params) {
    return null;
  }

  const base = approvalBase(request.id, params);
  if (!base) {
    return null;
  }

  switch (request.method) {
    case COMMAND_APPROVAL_METHOD: {
      const command = stringValue(params.command);
      const cwd = stringValue(params.cwd);
      const networkContext = objectValue(params.networkApprovalContext);
      const networkTarget = stringValue(networkContext?.target);
      const networkHost = stringValue(networkContext?.host);
      return {
        ...base,
        kind: "commandExecution",
        title: networkTarget ? "Network access" : "Command approval",
        detail:
          command || networkTarget || "Command execution requested approval.",
        metadata: [
          ...base.metadata,
          ...metadataValue("cwd", cwd),
          ...metadataValue("host", networkHost),
          ...formatPermissionMetadata(params.additionalPermissions),
        ],
        availableDecisions: commandDecisions(params.availableDecisions),
      };
    }
    case FILE_CHANGE_APPROVAL_METHOD: {
      return {
        ...base,
        kind: "fileChange",
        title: "File change approval",
        detail: stringValue(params.grantRoot)
          ? `Allow file changes under ${stringValue(params.grantRoot)}.`
          : "Approve the proposed file changes.",
        metadata: [
          ...base.metadata,
          ...metadataValue("grant root", stringValue(params.grantRoot)),
        ],
        availableDecisions: ["accept", "acceptForSession", "decline", "cancel"],
      };
    }
    case PERMISSIONS_APPROVAL_METHOD: {
      return {
        ...base,
        kind: "permissions",
        title: "Permissions request",
        detail: "Grant the requested runtime permissions.",
        metadata: [
          ...base.metadata,
          ...metadataValue("cwd", stringValue(params.cwd)),
          ...formatPermissionMetadata(params.permissions),
        ],
        permissions: params.permissions ?? null,
        availableDecisions: ["accept", "acceptForSession", "decline", "cancel"],
      };
    }
    default:
      return null;
  }
}

export function buildApprovalResponse(
  request: ApprovalRequest,
  decision: ApprovalDecision,
) {
  if (request.kind === "permissions") {
    if (decision === "accept" || decision === "acceptForSession") {
      return {
        permissions: request.permissions ?? {},
        scope: decision === "acceptForSession" ? "session" : "turn",
      };
    }
    return { permissions: {}, scope: "turn" };
  }

  return { decision };
}

function approvalBase(
  requestId: JsonRpcRequestId,
  params: Record<string, unknown>,
): Omit<
  ApprovalRequest,
  "kind" | "title" | "detail" | "permissions" | "availableDecisions"
> | null {
  const threadId = stringValue(params.threadId);
  const turnId = stringValue(params.turnId);
  const itemId = stringValue(params.itemId);
  if (!threadId || !turnId || !itemId) {
    return null;
  }
  return {
    requestId,
    threadId,
    turnId,
    itemId,
    startedAtMs: numberValue(params.startedAtMs) ?? Date.now(),
    reason: stringValue(params.reason) || null,
    metadata: [
      { label: "turn", value: turnId },
      { label: "item", value: itemId },
    ],
    status: "pending",
    error: null,
  };
}

function commandDecisions(value: unknown): ApprovalDecision[] {
  if (!Array.isArray(value)) {
    return ["accept", "acceptForSession", "decline", "cancel"];
  }
  const decisions = value.filter(isApprovalDecision);
  return decisions.length > 0 ? decisions : ["accept", "decline", "cancel"];
}

function formatPermissionMetadata(value: unknown) {
  const permissions = objectValue(value);
  if (!permissions) {
    return [];
  }
  const metadata: Array<{ label: string; value: string }> = [];
  const fileSystem = objectValue(permissions.fileSystem);
  for (const key of ["read", "write"] as const) {
    const paths = stringArray(fileSystem?.[key]);
    if (paths.length > 0) {
      metadata.push({ label: `${key} paths`, value: paths.join(", ") });
    }
  }
  const network = objectValue(permissions.network);
  if (network?.enabled === true) {
    metadata.push({ label: "network", value: "enabled" });
  }
  return metadata;
}

function isApprovalDecision(value: unknown): value is ApprovalDecision {
  return (
    value === "accept" ||
    value === "acceptForSession" ||
    value === "decline" ||
    value === "cancel"
  );
}

function objectValue(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function stringValue(value: unknown) {
  return typeof value === "string" ? value : "";
}

function numberValue(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function metadataValue(label: string, value: string) {
  return value ? [{ label, value }] : [];
}

function stringArray(value: unknown) {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}
