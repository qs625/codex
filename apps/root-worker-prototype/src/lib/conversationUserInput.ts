export type ConversationUserInputContent = Array<{
  type: string;
  text?: string;
  image_url?: string;
  imageUrl?: string;
  name?: string;
  path?: string;
}>;

export function formatUserInputContent(
  content: ConversationUserInputContent,
): string {
  const text = content
    .filter((item) => item.type === "text")
    .map((item) => item.text ?? "")
    .join("\n")
    .trim();
  const skillCount = content.filter((item) => item.type === "skill").length;
  const imageCount = content.filter((item) => item.type === "image").length;
  return (
    text ||
    (skillCount > 0
      ? `Activated ${skillCount} skill${skillCount === 1 ? "" : "s"}.`
      : "") ||
    (imageCount > 0
      ? `Attached ${imageCount} image${imageCount === 1 ? "" : "s"}.`
      : "")
  );
}

export function attachmentsFromUserInput(content: ConversationUserInputContent) {
  const skillAttachments = content
    .filter((item) => item.type === "skill")
    .map((item) => ({
      kind: "file" as const,
      label: `/${item.name ?? "skill"}`,
      path: item.path,
    }));
  const imageAttachments = content
    .filter((item) => item.type === "image")
    .map((item, index) => ({
      kind: "image" as const,
      label: item.name ?? `Image ${index + 1}`,
      url: item.image_url ?? item.imageUrl,
    }));
  return [...skillAttachments, ...imageAttachments];
}
