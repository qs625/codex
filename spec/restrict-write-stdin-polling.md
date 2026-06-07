# 限制 write_stdin 空输入轮询

## 任务 brief

- 用户：Codex CLI/API 的模型工具调用者，以及依赖长命令监控能力的开发者。
- 缺陷：`write_stdin` 允许缺省或空 `chars`，底层会跳过真实 stdin 写入并收集 session 输出，导致它被用作读取/刷新命令输出和轮询命令状态。
- 成功标准：
  - `write_stdin` 仍能向已有 PTY session 发送非空输入。
  - `write_stdin` 的缺省或空 `chars` 被明确拒绝，并提示使用 `event_command_subscribe` 处理长命令完成、日志或文件监听。
  - event command 可以通过稳定 `subscription_id` 写入 stdin，不暴露底层 process/session id 作为 API 语义。
- 非目标：
  - 不删除 `write_stdin`。
  - 不改变 `exec_command` 的普通执行和输出返回语义。
  - 不把 event command 的 stdin 运行时句柄持久化到 thread metadata。

## 技术设计

### write_stdin

- tool schema 将 `chars` 加入 required，并更新描述，明确该工具只发送真实输入，不用于读取输出、等待完成或刷新状态。
- handler 使用 `Option<String>` 区分缺省字段，缺省或空字符串统一返回面向模型的错误。
- `UnifiedExecProcessManager::write_stdin` 增加同样的空输入保护，避免后续绕过 handler 重新引入轮询路径。

### event command stdin

- 新增 `event_command_write_stdin` tool：
  - 参数：`subscription_id: String`、`chars: String`。
  - `subscription_id` 是 `event_command_subscribe` 返回的稳定目标。
  - `chars` 必须非空。
- `FsSubscriptionRegistry` 在启动 event command 时将子进程 stdin 设为 piped，保存运行时 `ChildStdin` 句柄到 active subscription entry。
- 写入时通过 `(thread_id, subscription_id)` 找到运行中 event command 的 stdin writer 并写入 bytes。
- subscription 完成、取消或失败后仍由现有移除流程清理 active entry；stdin 句柄随 entry 清理。

## 风险

- event command stdin 只覆盖当前运行的 subscription 进程；持久化恢复后的新进程会使用同一个 `subscription_id` 重新建立新的 stdin writer。
- 如果命令快速退出，`event_command_write_stdin` 可能返回订阅不存在或 stdin 尚不可用，这是预期运行时状态。
