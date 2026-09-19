import type { FilePanelView, FilePreview } from "../types";

function hasHtmlFileExtension(value: string | null | undefined) {
  return /\.(?:html|htm)$/i.test(value ?? "");
}

export function isHtmlFilePreview(preview: FilePreview | null) {
  if (!preview) {
    return false;
  }

  if (preview.language.trim().toLowerCase() === "html") {
    return true;
  }

  return (
    hasHtmlFileExtension(preview.path) ||
    hasHtmlFileExtension(preview.displayPath)
  );
}

export function filePreviewOpenInBrowserActionVisible({
  filePanelView,
  preview,
  previewError,
  previewLoading,
}: {
  filePanelView: FilePanelView;
  preview: FilePreview | null;
  previewError: string | null;
  previewLoading: boolean;
}) {
  return (
    filePanelView === "preview" &&
    !previewLoading &&
    !previewError &&
    isHtmlFilePreview(preview)
  );
}

function encodePathSegments(path: string) {
  return path
    .split("/")
    .map((segment) => encodeURIComponent(segment))
    .join("/");
}

export function localPathToFileUrl(path: string) {
  if (path.startsWith("file://")) {
    return path;
  }

  const normalized = path.replace(/\\/g, "/");
  const uncMatch = normalized.match(/^\/\/([^/]+)(\/.*)?$/);
  if (uncMatch) {
    const host = encodeURIComponent(uncMatch[1]);
    const rest = encodePathSegments(uncMatch[2] ?? "/");
    return `file://${host}${rest}`;
  }

  const windowsDriveMatch = normalized.match(/^([A-Za-z]):(\/.*)?$/);
  if (windowsDriveMatch) {
    const drive = windowsDriveMatch[1].toUpperCase();
    const rest = encodePathSegments(windowsDriveMatch[2] ?? "/");
    return `file:///${drive}:${rest}`;
  }

  if (normalized.startsWith("/")) {
    return `file://${encodePathSegments(normalized)}`;
  }

  return `file://${encodePathSegments(`/${normalized}`)}`;
}
