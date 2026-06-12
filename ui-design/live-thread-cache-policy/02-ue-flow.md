# UE 交互流程

## 主路径：首次打开 thread

1. 用户在 thread 列表选择一个本地 cache 中不存在的 thread。
2. 客户端进入 loading 状态。
3. 客户端调用 `thread/read` 获取初始化 snapshot。
4. reducer 将 snapshot canonicalize 为本地 thread state。
5. 客户端建立或恢复 live subscription。
6. 后续 `item/started` / `item/completed` 继续增量更新可见内容。

用户体验目标：首次进入时可以看到完整历史，不要求等待 live 增量事件重放。

## 主路径：切换回已 loaded thread

1. 用户从 thread A 切换到 thread B。
2. 如果 thread B 已在本地 live cache 中，UI 直接读取 cache 中的 live `ThreadItem` 列表。
3. 客户端不得触发 `thread/read`。
4. 客户端不得用 snapshot/history rebuild 对当前 items 做 destructive 或 non-destructive merge。
5. 如有必要，仅更新 selection、focus、subscription metadata。

用户体验目标：切回 thread 后，用户看到的内容与离开前保持一致，已出现的 `childCompletion` 不丢失、不回退、不重复。

## Subscribe / Resume

1. 用户打开或切换到 thread。
2. 客户端确认 live subscription 状态。
3. 若需要 subscribe/resume，只更新连接状态、thread metadata、last seen cursor 或活动标记。
4. subscribe/resume 响应不得覆盖 `turns` 或 item list。

反馈策略：

- 可以显示连接中、已连接、重连中等状态。
- 不应因为订阅动作让消息区闪烁、清空或回滚。

## Turn Lifecycle

### `turn/started`

- 更新当前 thread 的 active turn metadata。
- 可设置运行中状态、开始时间、模型/agent metadata。
- 不创建 display item，除非后续有对应 `item/started`。

### `turn/completed`

- 更新 turn 完成状态、结束时间、结果 metadata。
- 清理 running indicator。
- 不用 completed payload 中的 snapshot items 覆盖 live items。

## Item Lifecycle

### `item/started`

- 创建或激活一个可见 `ThreadItem`。
- 如果 item id 已存在，按 typed item reducer 更新状态，不从文本 marker 解析。

### `item/completed`

- 完成对应可见 `ThreadItem`。
- 写入最终 typed payload。
- 保留 item 顺序和父子关系，确保 childCompletion 与 subagent 通知可持续显示。

## 空状态

- Cold start 且 `thread/read` 返回空：显示空 thread 状态。
- Loaded cache 为空但 subscription 存在：显示空 thread，并等待后续 live item。
- 不得把 loaded thread 的空 snapshot 当作清空现有 live items 的指令。

## 加载状态

- 只有 cold start 或显式恢复时显示 full loading。
- loaded thread 切换只允许轻量 focus/selection 更新，不应显示全屏 loading。

## 错误状态

- `thread/read` 失败：仅影响 cold start 或显式恢复路径，显示读取失败和重试入口。
- subscribe/resume 失败：保留当前 cached items，显示连接异常或重连状态。
- item lifecycle 缺失前置 item：记录 reducer 诊断，可创建占位 pending item，但不得回退到 raw marker 解析。

