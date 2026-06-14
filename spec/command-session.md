# 统一 Command Session

## Brief

当前 `exec_command` 与 `event_command_subscribe` 分别维护前台命令和后台命令监听，模型需要在不同工具之间切换，客户端也需要同时展示 unified exec 与 event command 两套命令形态。目标是以 `exec_command` 作为唯一命令启动入口，所有持续运行、等待输出、等待退出和 stdin 写入都围绕同一个 Command Session。

## 成功标准

- `exec_command` 启动命令后始终生成可复用的 command session；短命令在初始等待窗口内退出时仍直接返回结果。
- `exec_command.initial_wait_ms` 控制初始等待窗口；旧 `yield_time_ms` 作为兼容别名继续解析。
- `exec_command.notify_on` 控制模型后续被唤醒的 notification 粒度：`output` 或 `exit`。
- `command_wait({ command_id })` 只等待调用开始之后的下一条 notification；命令已退出时立即返回 completed，不回放旧输出。
- `command_write_stdin({ command_id, chars })` 只负责写 stdin，不读取输出、不刷新状态。
- 旧 `event_command_subscribe` 与 `event_command_write_stdin` 不再作为 command 工具注册；schedule subscribe/unsubscribe 保留。
- 客户端展示继续以 typed `ResponseItem -> ThreadItem` 和 command execution lifecycle 为主，不从 raw marker 或 assistant JSON 反解命令展示。

## 非目标

- 不保留旧 event command subscribe/write 的向后兼容工具入口。
- 不在 `command_wait` 暴露 timeout、poll interval 或 patience 参数。
- 不把 `notify_on=exit` 用作客户端 live 输出开关；客户端仍消费 command execution output delta。
- 第一版不实现按 cwd/project/cmd glob 的 hard cap 覆盖解析，只复用现有全局 `background_terminal_max_timeout` 作为 wait hard cap。

## 技术设计

### 工具面

- `exec_command` 新增：
  - `initial_wait_ms?: number`：首轮等待输出/退出的窗口。
  - `notify_on?: "output" | "exit"`：后续 `command_wait` 被唤醒的事件类型。
- `yield_time_ms` 保留为兼容输入，未设置 `initial_wait_ms` 时才使用。
- `command_wait` 输入只包含 `command_id`，返回 command 状态、exit code 和 notification kind，不返回输出正文。
- `command_write_stdin` 输入包含 `command_id` 与非空 `chars`，只写 stdin。旧 `write_stdin` handler 逻辑迁移到新工具名。

### 运行时

- `UnifiedExecProcessManager` 保存全局 wait hard cap。
- `ProcessEntry` 保存 `notify_on`、原始 call id、输出/退出 notification 的 `Notify`。
- 输出 streaming task 在收到输出 delta 后根据 `notify_on=output` 唤醒 waiter；exit watcher 在结束事件发出后唤醒 exit waiter。
- `command_wait` 获取调用开始后的 receiver/notify，等待 future notification；如果 entry 不存在但 process 已被移除，则按 completed 返回。

### 展示链路

- live 输出继续走 `ExecCommandBegin/OutputDelta/End -> ThreadItem::CommandExecution`。
- 删除 `response_item_projection.rs` 中针对旧 `event_command_subscribe` 的 command call 特判；schedule 仍作为 EventDrivenToolCall。
- root-worker Conversation 侧消费 `ThreadItem::CommandExecution` 与 output delta 更新同一个 command cell；Live Commands index 从 command execution 状态派生。

## 风险

- 旧 event command 的“线程恢复自动重启 monitor”语义不会迁移到第一版 Command Session。
- 当前 hard cap 只有全局配置，未实现 project/cwd/cmd glob 覆盖。
- 如果某些客户端仍依赖旧 `EventCommandCall/EventCommandEvent`，本次删除会要求客户端迁移到 command execution 展示。
