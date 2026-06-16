# 统一 Command Session

## Brief

当前 `exec_command` 与 `event_command_subscribe` 分别维护前台命令和后台命令监听，模型需要在不同工具之间切换，客户端也需要同时展示 unified exec 与 event command 两套命令形态。目标是以 `exec_command` 作为唯一命令启动入口，所有持续运行、等待输出、等待退出和 stdin 写入都围绕同一个 Command Session。

## 成功标准

- `exec_command` 启动命令后始终生成可复用的 command session；短命令在初始等待窗口内退出时仍直接返回结果。
- `exec_command.initial_wait_ms` 控制初始等待窗口；旧 `yield_time_ms` 作为兼容别名继续解析。
- `exec_command.notify_on` 控制模型后续被唤醒的 notification 粒度：`output` 或 `exit`。
- `command_wait({ command_id })` 只等待调用开始之后的下一条 notification；命令已退出时立即返回 completed，不回放旧输出。每次调用只等待当前 runtime window，超时后返回 running 并推进该 command session 的 backoff window；下一次同 command 调用使用推进后的窗口。收到 output/exit 或发现命令已结束时重置 backoff。
- `command_write_stdin({ command_id, chars })` 只负责写 stdin，不读取输出、不刷新状态。
- 旧 `event_command_subscribe` 与 `event_command_write_stdin` 不再作为 command 工具注册；schedule subscribe/unsubscribe 保留。
- 客户端展示继续以 typed `EventMsg -> ThreadItem` display lifecycle 和 command execution lifecycle 为主；确实需要模型可见的 command wait/stdin 事实再双写 `ResponseItem`，不从 raw marker 或 assistant JSON 反解命令展示。

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
- `command_wait` 输入只包含 `command_id`，返回 command 状态、exit code、notification kind 和本次等待窗口 `wait_timeout_ms`，不返回输出正文。
- `command_write_stdin` 输入包含 `command_id` 与非空 `chars`，只写 stdin。旧 `write_stdin` handler 逻辑迁移到新工具名。

### 运行时

- `UnifiedExecProcessManager` 保存全局 wait hard cap，作为单次 backoff window 上限，不在一次 `command_wait` 内部持续等待到 hard cap。
- `ProcessEntry` 保存 `notify_on`、原始 call id、输出/退出 notification 的 `Notify`，以及该 command session 的 wait backoff state。
- 输出 streaming task 在收到输出 delta 后根据 `notify_on=output` 唤醒 waiter；exit watcher 在结束事件发出后唤醒 exit waiter。
- `command_wait` 获取调用开始后的 receiver/notify，只等待当前 backoff window 内的 future notification；如果 entry 不存在但 process 已被移除，则按 completed 返回。当前 window 超时后返回 running/no notification 并推进 backoff；output/exit 或 completed 命中后 reset。
- command session notification 只在 `exec_command` 超过 `initial_wait_ms`、确认返回 `command_id` 后激活；在初始等待窗口内完成的命令只通过 `exec_command` 工具结果和 `CommandExecution` 完成状态表达，不额外生成 output/exit notification，也不额外唤醒模型。

### 展示链路

- live 输出继续走 `ExecCommandBegin/OutputDelta/End -> ThreadItem::CommandExecution`。
- 删除 `response_item_projection.rs` 中针对旧 `event_command_subscribe` 的 command call 特判；schedule 仍作为 EventDrivenToolCall。
- root-worker Conversation 侧消费 `ThreadItem::CommandExecution` 与 output delta 更新同一个 command cell；Live Commands index 从 command execution 状态派生，且只展示 running / in-progress command。已经 completed、failed、declined 或其他终态的 command 不继续作为 active activity 显示。
- `ThreadItem::CommandExecution` 携带 `initial_wait_ms` 与 `notify_on`，客户端 command details 必须展示这两个 session 参数；旧 shell/user shell 或 approval 派生项没有该参数时展示可省略。
- command notification 本身使用独立 typed item 展示：`CommandExecutionNotification { command_item_id, kind, message, output, exit_code, created_at_ms }`。
  - `notify_on=exit`：中间 `ExecCommandOutputDelta` 只更新 command cell 的 live tail，不生成 output notification item；`ExecCommandEnd` 生成 exit notification item。
  - `notify_on=output`：每个唤醒模型的 output delta 生成 output notification item；`ExecCommandEnd` 仍生成 exit notification item。
  - 如果命令在 `initial_wait_ms` 内完成并且没有返回可复用 `command_id`，`ExecCommandEnd` 只更新原 command execution item，不生成 exit notification item；同一初始窗口内的 output delta 也不得生成 output notification item。
  - notification item 的 `id` 必须不同于 command execution item，关联只通过 typed `command_item_id`，不得按 command/output 文本匹配。
- `command_wait` 与 `command_write_stdin` 的工具行为也使用独立 typed item 展示：
  - `CommandWait { command_id, status, notification, exit_code, wall_time_seconds, wait_timeout_ms, created_at_ms }` 表示模型等待 command session 的结果，其中 `wait_timeout_ms` 是本次实际等待窗口，不是 hard cap。
  - `CommandWriteStdin { command_id, bytes_written, contains_newline, created_at_ms }` 表示模型向 command session 写入 stdin；不持久化原始 stdin 文本。
  - 两者必须通过 shared projector 生成 `ThreadItem`，不得只依赖函数返回 JSON、terminal interaction event 或 raw marker 展示。
- root-worker Conversation 以独立 event row 展示 command output/exit notification、`CommandWait` 和 `CommandWriteStdin`；RightPanel Live Commands 仍以 command execution 状态决定显示/清理，latest event 优先显示 typed notification 摘要。
- RightPanel Live Commands 点击 command row 时按 `ThreadItem.id` 定位到 conversation 中对应 command cell，短暂高亮，不展开详情、不打断 composer 输入。

## 风险

- 旧 event command 的“线程恢复自动重启 monitor”语义不会迁移到第一版 Command Session。
- 当前 hard cap 只有全局配置，未实现 project/cwd/cmd glob 覆盖。
- 如果某些客户端仍依赖旧 `EventCommandCall/EventCommandEvent`，本次删除会要求客户端迁移到 command execution 展示。
