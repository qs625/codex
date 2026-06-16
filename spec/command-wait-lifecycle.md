# command_wait lifecycle

## 背景

`command_wait` 用于等待已启动 command session 的下一次 output / exit notification 或当前等待窗口超时。
此前有两个问题：

- command wait backoff 的初始窗口固定为 250ms，没有使用对应 `exec_command` 的 effective
  `initial_wait_ms`。
- `command_wait` 只有等待返回后才记录 completed display item，客户端看不到模型调用开始时的
  `item/started`。

## 目标

- 每个 command session 的 wait backoff 初始窗口使用 `exec_command` 的 effective `initial_wait_ms`
  （即 handler 已解析出的 `initial_wait_ms.unwrap_or(yield_time_ms)`）。
- timeout 后按 backoff 增长，命中 output / exit event 后 reset 到该 command session 的 initial window。
- `command_wait` handler 在真正 await 前发 typed `CommandWaitStarted` display event，等待返回后用同一个
  item id 记录并发 `CommandWaitCompleted` display event；`ResponseItem::CommandWait` 只在模型上下文和旧
  rollout/history 兼容需要时保留。
- started 和 completed payload 的 `wait_timeout_ms` 都表示本次 current window，不展示 hard cap，也不回退到
  250ms。

## 数据流

1. `exec_command` 创建 long-running process 时把 effective `initial_wait_ms` 传给
   `UnifiedExecProcessManager::store_process`。
2. `ProcessEntry.command_wait_backoff` 使用该 initial window 初始化。
3. `command_wait` handler 调用 `begin_command_wait(process_id)` 获取本次 wait token 和 current window；unknown
   process 在这里返回错误，因此不会先发 started item。
4. handler 生成 stable `ResponseItem::CommandWait` id，并发送 `CommandWaitStarted`。
5. handler 调用 `finish_command_wait(wait_token)` 执行真实等待；即使 process 在 begin 后被释放，completed item
   也继续使用 token 中固定的 current window。
6. 等待 timeout 时 manager advance backoff；等待 output / exit 或发现已退出时 manager reset backoff。
7. handler 用 started 的同一个 id 记录 completed item，并把 typed item 投影给 app-server v2
   `ThreadItem::CommandWait`。

## 测试

- unified exec manager：首个 `command_wait` window 等于 exec command initial wait，timeout 后递增，
  notification 后 reset 回 initial wait。
- handler item 构造：started/completed 使用同一个 id，并保留本次 current window。
- app-server-protocol：`CommandWaitStarted` / `CommandWaitCompleted` 映射为 v2 `ItemStarted/Completed(ThreadItem::CommandWait)`；`ResponseItemStarted/Completed(CommandWait)` 只作为旧 rollout/history 兼容路径继续覆盖。
