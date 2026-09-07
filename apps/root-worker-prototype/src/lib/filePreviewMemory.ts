import type { FilePanelView, FilePreview, RightPanelView } from "../types";

export type FilePreviewMemoryByRootId = Record<string, FilePreview>;

export function rememberProjectFilePreview(
  memory: FilePreviewMemoryByRootId,
  rootId: string | null,
  preview: FilePreview,
): FilePreviewMemoryByRootId {
  if (!rootId) {
    return memory;
  }
  return {
    ...memory,
    [rootId]: preview,
  };
}

export function rememberSavedProjectFilePreview(
  memory: FilePreviewMemoryByRootId,
  currentRootId: string | null,
  saveRootId: string | null,
  currentPreview: FilePreview | null,
  savedPreview: FilePreview,
): FilePreviewMemoryByRootId {
  if (
    !currentRootId ||
    currentRootId !== saveRootId ||
    currentPreview?.path !== savedPreview.path
  ) {
    return memory;
  }
  return rememberProjectFilePreview(memory, currentRootId, savedPreview);
}

export function getProjectFilePreview(
  memory: FilePreviewMemoryByRootId,
  rootId: string | null,
) {
  return rootId ? (memory[rootId] ?? null) : null;
}

export function shouldRestoreProjectFilePreview(
  rightPanelView: RightPanelView,
  filePanelView: FilePanelView,
) {
  return rightPanelView === "preview" && filePanelView === "preview";
}
