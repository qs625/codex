export function toErrorMessage(error: unknown) {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

export function isThreadNotFoundError(message: string) {
  return /thread\s+[0-9a-f-]+\s+not found/i.test(message);
}
