export const WORKSPACE_OBJECT_DRAG_TYPE =
  "application/x-morpheus-workspace-object-tab";

export type WorkspaceObjectDragPayload =
  | { kind: "file" }
  | {
      kind: "browser";
      browserTabId: string;
      title?: string | null;
      url?: string | null;
    }
  | {
      kind: "terminal";
      terminalTabId: string;
      sessionId?: string | null;
      threadId?: string | null;
      title?: string | null;
      cwd?: string | null;
      commandItemId?: string | null;
      command?: string | null;
      status?: string | null;
    };

export type WorkspaceOpenableRightPanelObject =
  WorkspaceObjectDragPayload["kind"];

export function writeWorkspaceObjectDragData(
  dataTransfer: Pick<DataTransfer, "effectAllowed" | "setData">,
  payload: WorkspaceObjectDragPayload | WorkspaceOpenableRightPanelObject,
) {
  const normalizedPayload =
    typeof payload === "string" ? { kind: payload } : payload;
  dataTransfer.effectAllowed = "move";
  dataTransfer.setData(
    WORKSPACE_OBJECT_DRAG_TYPE,
    JSON.stringify(normalizedPayload),
  );
}

export function readWorkspaceObjectDragData(
  dataTransfer: Pick<DataTransfer, "getData">,
): WorkspaceObjectDragPayload | null {
  const rawPayload = dataTransfer.getData(WORKSPACE_OBJECT_DRAG_TYPE);
  if (!rawPayload) {
    return null;
  }
  if (
    rawPayload === "file" ||
    rawPayload === "browser" ||
    rawPayload === "terminal"
  ) {
    return { kind: rawPayload };
  }
  try {
    return normalizeWorkspaceObjectDragPayload(JSON.parse(rawPayload));
  } catch {
    return null;
  }
}

function normalizeWorkspaceObjectDragPayload(
  value: unknown,
): WorkspaceObjectDragPayload | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const payload = value as Partial<WorkspaceObjectDragPayload> & {
    kind?: unknown;
  };
  if (payload.kind === "file") {
    return { kind: "file" };
  }
  if (payload.kind === "browser") {
    return {
      kind: "browser",
      browserTabId:
        typeof payload.browserTabId === "string" && payload.browserTabId
          ? payload.browserTabId
          : "active",
      title: typeof payload.title === "string" ? payload.title : null,
      url: typeof payload.url === "string" ? payload.url : null,
    };
  }
  if (payload.kind === "terminal") {
    return {
      kind: "terminal",
      terminalTabId:
        typeof payload.terminalTabId === "string" && payload.terminalTabId
          ? payload.terminalTabId
          : "active",
      sessionId: typeof payload.sessionId === "string" ? payload.sessionId : null,
      threadId: typeof payload.threadId === "string" ? payload.threadId : null,
      title: typeof payload.title === "string" ? payload.title : null,
      cwd: typeof payload.cwd === "string" ? payload.cwd : null,
      commandItemId:
        typeof payload.commandItemId === "string" ? payload.commandItemId : null,
      command: typeof payload.command === "string" ? payload.command : null,
      status: typeof payload.status === "string" ? payload.status : null,
    };
  }
  return null;
}
