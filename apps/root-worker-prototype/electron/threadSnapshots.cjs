function mergeThreadSnapshots(existing, next) {
  if (!existing) {
    return next;
  }

  const tokenUsage =
    next.tokenUsage ?? existing.tokenUsage ?? existing.threadUsage?.tokenUsage ?? null;
  const contextUsage =
    next.contextUsage ?? existing.contextUsage ?? existing.threadUsage?.contextUsage ?? null;

  return {
    ...existing,
    ...next,
    threadUsage: {
      tokenUsage: next.threadUsage?.tokenUsage ?? tokenUsage,
      contextUsage: next.threadUsage?.contextUsage ?? contextUsage,
    },
    tokenUsage,
    contextUsage,
  };
}

module.exports = {
  mergeThreadSnapshots,
};
