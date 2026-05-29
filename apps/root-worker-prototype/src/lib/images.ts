import type { ComposerImage } from "../types";

export async function readImageBlob(blob: Blob, name: string): Promise<ComposerImage> {
  const bytes = await blob.arrayBuffer();
  return {
    id: `${name}:${blob.size}:${Date.now()}:${Math.random().toString(36).slice(2, 8)}`,
    name,
    mimeType: blob.type || "application/octet-stream",
    byteSize: blob.size,
    bytes,
    previewUrl: URL.createObjectURL(blob),
  };
}

export async function readImageFile(file: File): Promise<ComposerImage> {
  return readImageBlob(file, file.name);
}

export function revokeComposerImage(image: ComposerImage) {
  URL.revokeObjectURL(image.previewUrl);
}
