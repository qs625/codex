# Codex Binary Workflow Notification 编译修复

## 任务 brief

`codex` binary 在编译 `codex-tui` 时失败。用户目标是恢复 `cargo build --bin codex` 的编译能力，不改变 workflow 运行时语义，也不引入新的展示路径。

## 现象

执行：

```bash
rtk cargo build --bin codex
```

编译到 `codex-tui` 时出现两个 exhaustive match 错误：

- `tui/src/app/app_server_event_targets.rs` 未覆盖 `ServerNotification::WorkflowRunUpdated(_)`。
- `tui/src/chatwidget/protocol.rs` 未覆盖 `ServerNotification::WorkflowRunUpdated(_)`。

## 根因

Dynamic Workflow 在 app-server v2 增加了 `workflow/run/updated` notification，但 TUI 侧两个对 `ServerNotification` 的穷尽匹配没有同步新增分支。

该 notification 的 payload 是全局 workflow run 状态：

```text
WorkflowRunUpdatedNotification { run: WorkflowRun }
```

它不携带 `thread_id`。线程内 workflow 进展展示已经通过
`EventMsg::WorkflowRunProgressCompleted -> ThreadItem::WorkflowRunProgress`
投影链路表达，因此 TUI 不应从 `workflow/run/updated` 反推或解析 display item。

## 技术设计

最小修复：

1. 在 `server_notification_thread_target` 中把 `WorkflowRunUpdated` 标记为 global notification。
2. 在 `ChatWidget::handle_server_notification` 中忽略该全局 notification，由 app-server/root-worker 控制面消费 run 状态。
3. 增加路由测试，固定 `WorkflowRunUpdated` 不会被误路由到线程。

## 非目标

- 不新增 TUI workflow 展示。
- 不从 workflow notification 的 run payload 构造 `ThreadItem`。
- 不改变 app-server workflow control plane 或 typed projection。

## 验收

- `cargo build --bin codex` 不再因 `WorkflowRunUpdated` match 遗漏失败。
- 独立 reviewer 覆盖目标编译验证。
