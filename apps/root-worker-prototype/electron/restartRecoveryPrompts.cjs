"use strict";

const RESTART_RECOVERY_PROMPTS = Object.freeze({
  projectRootFanout:
    "Morpheus 客户端已完成重启。请检查当前持久化上下文，判断是否需要继续执行、重新运行必要检查、说明中断影响，或停止当前工作。",
  expectedRestart: Object.freeze({
    completed: (requestId, sourceThreadId) =>
      `Morpheus 已恢复预期的 Runtime Capsule 重启请求 ${requestId}；该请求已完成。原始 thread id：${sourceThreadId}。`,
    failed: (requestId, error, sourceThreadId) =>
      `Morpheus 已恢复预期的 Runtime Capsule 重启请求 ${requestId}；该请求在重启完成前失败：${error ?? "未知错误"}。原始 thread id：${sourceThreadId}。`,
    interrupted: (requestId, sourceThreadId) =>
      `Morpheus 已恢复预期的 Runtime Capsule 重启请求 ${requestId}；未找到 Host 退出前的正常交接完成记录。原始 thread id：${sourceThreadId}。`,
    followUp:
      "这是预期的重启恢复，不是通用崩溃。不要自动再次调用 request_runtime_restart；请基于持久化结果继续处理。",
  }),
});

function expectedRuntimeRestartRecoveryPrompt(record) {
  const prompt = RESTART_RECOVERY_PROMPTS.expectedRestart;
  const sourceThreadId = record?.requestedByThreadId ?? "未知";
  const outcome =
    record?.phase === "failed"
      ? prompt.failed(record.requestId, record.error, sourceThreadId)
      : record?.phase === "completed"
        ? prompt.completed(record.requestId, sourceThreadId)
        : prompt.interrupted(record?.requestId, sourceThreadId);
  return `${outcome} ${prompt.followUp}`;
}

function shouldNotifyRuntimeRestartErrorOnSelf(record) {
  return record?.phase !== "completed";
}

function formatPayloadRuntimeRecoveryPrompt({
  failedReleaseId,
  reason,
  exitCode,
  signal,
} = {}) {
  const releaseId = nonEmptyString(failedReleaseId) ?? "未知 release";
  const writtenReason = nonEmptyString(reason);
  const exitDetail = [
    Number.isInteger(exitCode) ? `exit code ${exitCode}` : null,
    nonEmptyString(signal) ? `signal ${signal}` : null,
  ]
    .filter(Boolean)
    .join("，");
  const detail = writtenReason
    ? `payload 记录的故障原因：${writtenReason}。`
    : exitDetail
      ? `未找到 payload 写入的故障原因；Launcher 观察到 ${exitDetail}。`
      : "未找到 payload 写入的故障原因，也没有可用的退出信息。";
  return `Morpheus 已从失败的 Runtime Capsule ${releaseId} 回退。${detail} 请检查持久化上下文与运行日志后继续处理。`;
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

module.exports = {
  formatPayloadRuntimeRecoveryPrompt,
  RESTART_RECOVERY_PROMPTS,
  expectedRuntimeRestartRecoveryPrompt,
  shouldNotifyRuntimeRestartErrorOnSelf,
};
