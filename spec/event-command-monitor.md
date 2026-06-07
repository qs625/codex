# EventCommand / Monitor 最终目标设计

## 任务 brief

目标是把“监听文件”和“监听命令退出”统一收敛到一个 EventCommand 工具。EventCommand 在后台运行一条命令，命令 stdout 的每一条输出行都会作为一条事件发送回 thread/model；命令退出、启动失败或取消时生成明确 terminal event，并清理 active 状态。

成功标准：

- 文件监听和命令监听都只使用 EventCommand 表达。
- 删除旧文件监听工具 `fs_subscribe` / `fs_unsubscribe`。
- 删除旧命令退出监听工具 `process_exit_subscribe` / `process_exit_unsubscribe`。
- 新增一等 ThreadItem：`EventCommandCall` 和 `EventCommandEvent`，最终协议不复用 `eventDrivenToolCall` / `eventDrivenTool`。
- EventCommand 的 active 列表由持久化 metadata / active summary 提供权威状态，客户端展示当前注册/运行中的 command monitor。
- 每一条 EventCommand stdout 事件都作为独立 thread item 展示，模型也能收到对应事件。
- EventCommand 在 thread resume 时默认重新执行保存的 command，而不是写入 restore failed。
- schedule 仍保留 typed 能力，不并入 EventCommand。

非目标：

- 不删除 `schedule_subscribe` / `schedule_unsubscribe`。
- 不把 app-server 连接级 `fs/watch` 当作 thread/model 事件工具。
- 不引入复杂的 `notify.onExit`、专用 FD 或 event file 协议。
- 不处理 child completion/subagent notification 问题。

## 统一模型

EventCommand 是文件监听和命令监听的唯一 Monitor 原语：

```text
后台运行 command
stdout 每一行 -> EventCommandEvent(kind=output) -> thread item -> model event
进程结束/失败/取消 -> EventCommandEvent(kind=terminal) -> active monitor 清理
thread resume -> 使用持久化 command/cwd/label/subscription_id 重新执行 command
```

文件监听通过命令实现，例如：

```bash
fswatch -0 /path/to/file | while IFS= read -r -d '' path; do echo "changed:$path"; done
```

命令退出监听通过同一个命令实现，例如：

```bash
cargo test -p codex-tui
```

EventCommand 自身会在进程退出时发送 terminal event，所以模型不需要额外执行 `exec_command` 再订阅 process exit。

schedule 是例外：它保留 typed 工具，因为 schedule 需要清晰展示未来触发计划、重复规则和取消入口，这些信息不适合隐藏在长期 shell loop 中。

## 终止态清理和恢复语义

EventCommand 是 active monitor，而不是历史事件回放机制。`exited`、`cancelled`、`failed_to_start`
都表示对应 `subscription_id` 已进入终止态；终止态事件写入 thread history 后，registry 必须从
thread metadata 的 active subscriptions 中移除该订阅。

清空最后一个 active subscription 时，metadata 必须写入显式空列表 `subscriptions: []`。这表示“当前
thread 没有需要恢复的 active subscription”，不同于旧 rollout 中没有 `subscriptions` 字段的未知状态。
resume 读取历史时应以最新显式空列表为准，不能越过它继续读取更早的 EventCommand subscription，否则一次性
命令会在恢复后重新运行并再次注入相同的 `output` / `exited` live event。

删除 active subscription 时，registry 的内存状态和 thread metadata 更新必须保持一致。如果持久化删除失败，
registry 不能先丢弃内存中的 active entry；否则旧 metadata 会继续存在，而当前进程也没有机会再取消或纠正该
subscription。

EventCommand event 的 ThreadItem id 必须由事件身份稳定生成，live notification 和 history replay 共享同一
fallback id 规则。`ResponseItem::Message` 旧历史中可能没有 id；这时不能在 live 路径使用一种 id、replay 路径
使用递增 `item-*`，否则客户端 upsert/dedupe 会把同一条 terminal marker 当成两条不同 live event 展示。

## 工具/API 形态

保留并推荐的 Monitor 工具只有：

```text
event_command_subscribe(command, cwd?, label?) -> EventCommandSubscribeResponse
event_command_unsubscribe(subscription_id) -> EventCommandUnsubscribeResponse
schedule_subscribe(...)
schedule_unsubscribe(...)
```

删除的工具：

```text
fs_subscribe
fs_unsubscribe
process_exit_subscribe
process_exit_unsubscribe
```

`event_command_subscribe` 请求字段：

- `command: String`：要后台运行的 shell 命令。stdout 每一行都是一条事件。
- `cwd: Option<String>`：命令工作目录。未提供时继承 app-server 进程当前工作目录。
- `label: Option<String>`：客户端和事件中展示的 monitor 标签。未提供时使用命令摘要。

响应字段：

```text
EventCommandSubscribeResponse {
  subscription_id: String
  command: String
  cwd: Option<String>
  label: Option<String>
}

EventCommandUnsubscribeResponse {
  subscription_id: String
  cancelled: bool
}
```

工具描述必须明确：

- stdout 行就是事件边界。
- noisy command 应由模型自行降噪，只 `echo` 想让模型重新接管的内容。
- stderr 默认不产生事件；需要时由模型在命令中自行 `2>&1` 或重定向。
- 长时间运行命令、命令退出监听、文件监听都应该通过 `event_command_subscribe`。
- 文件监听由模型生成平台可执行的 watch command 或小脚本。

## 事件语义

EventCommand 产生两类事件。

stdout output event：

```text
EventCommandEvent {
  subscription_id
  kind: "output"
  label
  command
  cwd
  line
  sequence
  created_at
  truncated
}
```

terminal event：

```text
EventCommandEvent {
  subscription_id
  kind: "exited" | "cancelled" | "failed_to_start"
  label
  command
  cwd
  exit_code?
  signal?
  message?
  created_at
}
```

行为规则：

- 每读到一条完整 stdout 行，立即生成一条 `kind=output` 的 `EventCommandEvent`。
- 空行默认忽略，避免无意义事件；如后续需要可增加 `include_empty_lines`。
- 单行超过 16 KiB 时截断展示文本，并设置 `truncated=true`。
- 命令退出时无论 exit code 是否为 0，都生成 `kind=exited` terminal event。
- 启动失败生成 `kind=failed_to_start`，并且不进入 active。
- 取消生成 `kind=cancelled`，并从 active 列表移除。
- thread resume 不生成 restore failed；默认重新执行 command。如果重启执行失败，生成 `failed_to_start`。

## 后端设计

EventCommand 后端应复用现有 event-driven subscription 基础设施，但最终对外类型升级为一等 EventCommand：

- 在 `codex-rs/file-subscription` 中用 EventCommand subscription 替代 file/process subscription。
- 删除 file/process subscription 的工具注册、handler、持久化变体和恢复逻辑。
- 保留 schedule subscription 的 typed 注册、持久化和恢复逻辑。
- 复用 registry 的 active count、取消 handle、持久化、恢复和事件注入机制。
- 继续避免把后台命令调度逻辑加入 `codex-core`。

数据流：

1. 模型调用 `event_command_subscribe`。
2. tool handler 校验 `command/cwd/label`，生成 `subscription_id`。
3. registry 持久化 EventCommand monitor：`subscription_id`、`command`、`cwd`、`label`。
4. registry 启动后台 shell 命令。
5. stdout reader 按行生成 `EventCommandEvent(kind=output)`。
6. 进程 wait task 观察退出状态，生成 terminal `EventCommandEvent`。
7. registry 在 terminal event 后移除 active/persisted subscription。
8. app-server 把 EventCommand item 推送给客户端，并写入 thread history。
9. root-worker 使用 EventCommand active summary 和 ThreadItem 展示当前状态。

子进程执行要求：

- 命令通过 shell 执行，以支持管道、重定向和小脚本。
- 取消时优先终止整个 process group，避免 shell 子进程残留。
- stdout reader 和 wait task 必须共享同一个 cancellation path，避免退出后 active 状态不清理。
- stderr 不进入 `EventCommandEvent`，除非命令自身重定向到 stdout。

## 取消语义

取消工具为 `event_command_unsubscribe(subscription_id)`。

取消流程：

- 如果 subscription active，发送取消信号并终止 child process/process group。
- 生成 `EventCommandEvent(kind=cancelled)`。
- 从 active monitor 列表和持久化 metadata 中移除。
- 返回 `cancelled: true`。

幂等规则：

- 如果 subscription 已退出、已取消或不存在，返回 `cancelled: false`。
- 不存在时不注入新的取消事件。
- 如果取消信号发送成功但进程短时间内自然退出，只保留一个 terminal event；优先保留已写入 thread 的第一个 terminal event。

客户端取消入口：

- active EventCommand 列表中每个 monitor 都应有取消动作。
- 取消动作调用 app-server/工具桥接到 `event_command_unsubscribe`。
- 取消完成后 UI 依赖 terminal event 或 active summary 更新，不靠本地乐观删除作为最终状态。

## 恢复语义

EventCommand 的持久化输入是恢复的权威来源：

```text
subscription_id
command
cwd
label
```

thread resume 时默认重新执行保存的 command：

- 读取持久化 EventCommand monitor。
- 使用原 `subscription_id`、`command`、`cwd`、`label` 重新启动后台 shell 命令。
- 保持同一个 active monitor identity，后续 stdout 继续以同一个 `subscription_id` 生成事件。
- 如果重新启动成功，不额外生成 restore event；后续 stdout 和退出事件自然进入 thread。
- 如果重新启动失败，生成 `EventCommandEvent(kind=failed_to_start)` 并清理 active/persisted subscription。

该设计有副作用风险：重启后会重新执行文件监听命令、测试命令或任意 shell 命令。当前设计选择接受这个风险，因为 EventCommand 是 active monitor 的最终模型，重启后 active monitor 应继续运行。工具描述需要提示模型避免把有不可重复副作用的命令注册为 EventCommand，除非用户明确需要。

## 协议 / ThreadItem / Notification 设计

最终协议新增一等 ThreadItem，不复用 `eventDrivenToolCall` / `eventDrivenTool`：

```text
ThreadItem::EventCommandCall {
  subscription_id: String
  command: String
  cwd: Option<String>
  label: Option<String>
  created_at: i64
}

ThreadItem::EventCommandEvent {
  subscription_id: String
  kind: EventCommandEventKind
  label: Option<String>
  command: String
  cwd: Option<String>
  line: Option<String>
  sequence: Option<u32>
  exit_code: Option<i32>
  signal: Option<String>
  message: Option<String>
  truncated: bool
  created_at: i64
}

EventCommandEventKind =
  | output
  | exited
  | cancelled
  | failed_to_start
```

app-server v2 需要同步新增结构化 notification / schema：

- subscribe 成功时发送并持久化 `eventCommandCall`。
- stdout 行、退出、取消、启动失败时发送并持久化 `eventCommandEvent`。
- thread read/history replay 返回同样的一等 item。
- thread status 或 thread summary 中暴露 active EventCommand 列表。

active summary 字段：

```text
EventCommandMonitorSummary {
  subscription_id: String
  command: String
  cwd: Option<String>
  label: Option<String>
  status: "active"
  created_at: i64
  last_event_at: Option<i64>
  event_count: u32
}
```

active summary 和持久化 metadata 是客户端展示当前注册/active EventCommand 列表的权威来源；thread history 用于展示事件流和 legacy 历史。

## root-worker UI 展示

root-worker 需要展示两类信息。

当前 active EventCommand 列表：

- 展示 label、command 摘要、cwd、运行时长、event count、last event。
- 每个 active monitor 提供取消入口。
- 多个 monitor 同时 active 时按创建时间或最近事件时间排序。
- active 列表来自 app-server active summary，不再依赖历史文本推导。

EventCommand 事件流：

- 每条 stdout 行显示为独立 EventCommand event。
- terminal event 显示为独立事件，状态包括 exited、cancelled、failed_to_start。
- 事件应能折叠/展开查看 command、cwd、subscription id 等详情。
- 事件不能被普通 tool grouping 吞掉；同一 monitor 的多条事件可以视觉关联，但仍应作为独立 item 可见。

thread 状态：

- 只要 active summary 中存在 EventCommand monitor，thread 应显示等待 event tool / monitor 状态。
- stdout output event 不结束 active 状态。
- terminal event 必须结束对应 monitor。
- schedule active 状态仍按 typed schedule 独立计算。

## active 状态规则

权威状态来自 subscription registry 和 app-server active summary。

状态转换：

```text
subscribe accepted -> active
output event -> active
thread resume restart success -> active
exited -> inactive
cancelled -> inactive
failed_to_start -> inactive
unsubscribe missing -> unchanged
```

历史只用于展示旧 item，不作为新 active 状态的权威来源。若客户端离线恢复时暂时没有 active summary，可从 `EventCommandCall` 和 `EventCommandEvent` 临时重建，但一旦 app-server active summary 到达，应以 summary 覆盖本地推导。

## 破坏性迁移 / 删除旧工具

### 删除 `process_exit_subscribe`

删除点：

- 模型工具注册中移除 `process_exit_subscribe` / `process_exit_unsubscribe`。
- file-subscription 后端移除 process-exit subscription 类型、handler、恢复逻辑和持久化写入。
- app-server live/history 映射不再生成新的 process monitor item。
- root-worker 不再把新 tool call 识别为 process monitor。

替代方式：

- 需要等待命令退出时，直接调用 `event_command_subscribe(command, cwd, label)`。
- 命令 stdout 输出关键进度；命令退出由 EventCommand 自动发送 `kind=exited`。

旧调用行为：

- 新模型上下文中不再暴露 `process_exit_subscribe` 工具，因此模型无法调用。
- 如果外部客户端仍发送旧工具调用，应返回 unknown tool / unsupported tool，而不是隐式转换。

### 删除 `fs_subscribe`

删除点：

- 模型工具注册中移除 `fs_subscribe` / `fs_unsubscribe`。
- file-subscription 后端移除 fs subscription 类型、handler、恢复逻辑和持久化写入。
- root-worker 不再把新 tool call 识别为 filesystem monitor。

替代方式：

- 需要监听文件时，模型生成平台可执行的 watch command，并通过 `event_command_subscribe` 注册。
- recursive、debounce、过滤规则由命令或脚本表达。

旧调用行为：

- 新模型上下文中不再暴露 `fs_subscribe` 工具，因此模型无法调用。
- 如果外部客户端仍发送旧工具调用，应返回 unknown tool / unsupported tool，而不是隐式转换。

### 旧历史与 migration

旧 thread history 可能已经包含 `eventDrivenToolCall` / `eventDrivenTool`、filesystem monitor 或 process monitor item。迁移策略：

- 不把旧历史重写成新的 EventCommand item，避免伪造 command/cwd/sequence 等缺失字段。
- app-server history reader 保留 legacy reader，只负责把旧 item 读成 legacy display item 或普通 historical event，不恢复 active。
- 旧 fs/process subscription metadata 在 thread resume 时被清理，不再恢复、不再重跑、不再生成新的 active monitor。
- root-worker 可展示旧历史事件，但不把旧 fs/process monitor 计入 active summary。
- 若需要一次性数据 migration，只迁移 metadata 清理，不迁移 thread item 内容。

### 保留 `schedule_subscribe`

schedule 不迁移到 EventCommand：

- schedule 需要 typed rule、未来触发时间、重复周期和清晰取消管理。
- 客户端继续展示 schedule 列表。
- schedule event 仍独立于 EventCommand event。

## 测试计划

后端测试：

- `event_command_subscribe` 创建 active monitor，返回 subscription id。
- stdout 多行生成多条独立 `EventCommandEvent(kind=output)`，并保持 active。
- 命令 exit code 0 生成 `kind=exited`，并清理 active。
- 命令 exit code 非 0 同样生成 `kind=exited`，并记录 exit code。
- 启动失败生成 `kind=failed_to_start`，不残留 active。
- unsubscribe 生成 `kind=cancelled`，终止进程并清理 active。
- 取消与自然退出竞争时只产生一个 terminal event。
- thread resume 读取持久化 command/cwd/label/subscription id，并重新执行 command。
- resume 后 stdout 继续使用同一个 subscription id 生成事件。
- resume 后重启失败生成 `kind=failed_to_start` 并清理 active。
- `fs_subscribe` / `fs_unsubscribe`、`process_exit_subscribe` / `process_exit_unsubscribe` 不再注册。

协议/app-server 测试：

- subscribe call 被序列化为一等 `eventCommandCall` ThreadItem。
- output/terminal event 被序列化为一等 `eventCommandEvent` ThreadItem。
- live notification、thread read、history replay 三条路径 schema 一致。
- active summary 包含当前 EventCommand monitor，并在 terminal event 后移除。
- app-server v2 schema fixture 包含新的 EventCommand item 和 active summary。
- 旧 `eventDrivenToolCall` / `eventDrivenTool` 历史可被 legacy reader 读取，但不计入 active summary。
- 旧 fs/process metadata 在 resume 时清理，不恢复 active。
- 外部调用已删除旧工具时返回 unknown/unsupported tool。

root-worker 测试：

- active EventCommand 列表展示 command、label、cwd、event count 和 last event。
- 每一条 stdout event 都生成独立可见 item。
- output event 不结束 waiting-eventtool 状态。
- exited/cancelled/failed_to_start 结束 waiting 状态。
- 取消入口调用 `event_command_unsubscribe`，并在 active summary 更新后移除 monitor。
- schedule typed monitor 不受 EventCommand 状态影响。
- 新 ThreadItem schema 可正常渲染，旧 legacy fs/process 历史只作为历史展示，不显示 active。

回归测试：

- 工具列表不再包含 `fs_subscribe` / `fs_unsubscribe`。
- 工具列表不再包含 `process_exit_subscribe` / `process_exit_unsubscribe`。
- app-server 连接级 `fs/watch` notification 不被误识别为 EventCommand。
- 旧 event-driven raw marker 不回退为普通 agent message；只能通过 legacy reader 进入历史展示。

## 风险与开放问题

- 重启后默认重跑 command 可能重复执行有副作用的命令；这是当前设计选择，需要工具描述和模型策略约束。
- 文件监听命令的跨平台差异：`fswatch`、`find`、`tail -f` 等可用性不同，模型需要根据环境生成命令。
- stdout 高频输出会放大 thread event 数量；第一版依赖模型降噪，后续可加节流或最大事件频率。
- 取消子进程树需要可靠 process group 管理，否则 shell 管道可能残留。
- stderr 默认不产生事件，可能让部分工具的关键信息不可见；模型可用 `2>&1` 显式重定向。
- 删除旧 fs/process 工具是破坏性变更；外部客户端如果硬编码旧工具名会收到 unsupported tool。
- legacy reader 保留会增加历史读取复杂度，但它只服务旧 thread 展示，不承担 active 状态。

## 推荐决策

EventCommand 作为文件监听和命令监听的统一最终模型推进：新增一等 `EventCommandCall` / `EventCommandEvent` ThreadItem；删除旧 `fs_subscribe` 和 `process_exit_subscribe` 工具族；thread resume 默认根据持久化输入重新执行 command；active summary 和 EventCommand metadata 成为客户端展示 active monitor 的权威来源；schedule 保持 typed 能力。
