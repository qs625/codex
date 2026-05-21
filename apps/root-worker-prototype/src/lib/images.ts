import type { ComposerImage } from "../types";

export async function readImageBlob(blob: Blob, name: string): Promise<ComposerImage> {
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") {
        resolve(reader.result);
        return;
      }
      reject(new Error(`Failed to read ${name}`));
    };
    reader.onerror = () => {
      reject(reader.error ?? new Error(`Failed to read ${name}`));
    };
    reader.readAsDataURL(blob);
  });

  return {
    id: `${name}:${blob.size}:${Date.now()}:${Math.random().toString(36).slice(2, 8)}`,
    name,
    dataUrl,
  };
}

export async function readImageFile(file: File): Promise<ComposerImage> {
  return readImageBlob(file, file.name);
}
