export const CHAT_COMPAT_CWD_BASENAME = ".my-codex-root-worker-chat-cwd";

export function isChatCompatCwd(cwd: string | null | undefined) {
  const normalized = cwd?.trim().replaceAll("\\", "/").replace(/\/+$/, "");
  if (!normalized) {
    return false;
  }
  return normalized.split("/").at(-1) === CHAT_COMPAT_CWD_BASENAME;
}
