# PM Progress

## Current Goal
修复重启客户端后普通 `exec_command` thread item 在 reload 后仍不可见的问题

## Active Work
- id: init-context-workflow-instructions
  owner: /root/project_pm/owner_dev_2
  checkout: /Users/bytedance/Projects/my-codex-dev-2
  branch: fix/init-context-workflow-instructions
  task_type: bugfix
  depends_on: 无
  files: codex-rs/thread-service/**, codex-rs/app-server/**, 相关 thread start / init context 测试
  base_commit: a0aaf3d06ca0d39bf91729e6a5989a3d0b30d272
  pending_sync_from_main:
  status: merged
  objective: 让 init context 正确包含 config path 下可发现的 workflow init context，以及 config file 中配置的 instruction_files
  last_update: 2026-07-07
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p app-server --test all thread_start_initial_context_includes_project_workflows_and_instruction_files_without_primary_environment -- --exact`；`rtk cargo build -p app-server --bin app-server`
  commit: 3335c130e34140da7a118cc1bf21b91824c28509
- id: poll-event-thread-item-visibility
  owner: /root/project_pm/owner_dev_3
  checkout: /Users/bytedance/Projects/my-codex-dev-3
  branch: fix/poll-event-thread-item-visibility
  task_type: bugfix
  depends_on: 无
  files: codex-rs/app-server-protocol/**, codex-rs/thread-history/**, codex-rs/tool-service/**, apps/root-worker-prototype/**, 相关 thread item / conversation 测试
  base_commit: a0aaf3d06ca0d39bf91729e6a5989a3d0b30d272
  pending_sync_from_main:
  status: merged
  objective: 让 poll_event 在客户端 thread items 中明确可见；同时将 command output / exit event 的客户端展示从 command id 改为具体命令，并提供合适的文案/分类与回归覆盖
  last_update: 2026-07-07
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p app-server-protocol builtin_tool_call_completed_display_event_maps_to_thread_item`；其余 `rollout` / `tool-service` / 前端测试受既有基线问题或本地依赖缺失阻断
  commit: fbb90be4e

- id: unified-poll-event-waiting
  owner: /root/project_pm/owner_dev_2
  checkout: /Users/bytedance/Projects/my-codex-dev-2
  branch: fix/unified-poll-event-waiting
  task_type: bugfix
  depends_on: poll-event-thread-item-visibility merge 到主线
  files: codex-rs/tool-service/**, codex-rs/thread-service/**, codex-rs/command-service/**, app-server protocol / tests, 相关 tool spec / prompt / runtime 测试
  base_commit: eee237bdf2dc6a410d397b6a466caa045192e294
  pending_sync_from_main:
  status: merged
  objective: 删除 `wait_agent` / `command_wait`，统一由 `poll_event` 承担 turn 内等待并在同一 turn 继续执行；event 来源信息通过 pending input 暴露，runtime 不维护硬性等待目标
  last_update: 2026-07-07
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p command-service-api unified_exec_error -- --nocapture`；`rtk cargo test -p thread-service poll_event_wakes_for_command_exit_notification -- --exact --nocapture`；`rtk cargo build -p app-server --bin app-server`
  commit: 646fa4f5a

- id: child-completion-thread-status
  owner: /root/project_pm/owner_dev_3
  checkout: /Users/bytedance/Projects/my-codex-dev-3
  branch: fix/child-completion-thread-status
  task_type: bugfix
  depends_on: unified-poll-event-waiting 已 merge 到主线
  files: codex-rs/agent-runtime/**, codex-rs/thread-service/**, 相关 child completion / thread status / poll_event 测试
  base_commit: 050342997320e0cb7327fa79b39554feecf90dd0
  pending_sync_from_main:
  status: merged
  objective: 调整 child completion 与 thread status 的关系，让 child thread status 只反映 child 自己的本地 turn 生命周期；去掉 parent-side direct child completion bookkeeping 对 `WaitChild` / child status 的直接驱动
  last_update: 2026-07-07
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p thread-service control_tests::post_turn_state_waits_for_active_direct_child_without_active_goal`；`rtk cargo test -p thread-service control_tests::pending_child_completion_bookkeeping_does_not_trigger_wait_child`；`rtk cargo test -p thread-service session::tests::context_and_history::turn_start_consumes_child_completion_before_parent_visible_complete`；`rtk cargo test -p thread-service session::tests::context_and_history::clearing_stale_child_completion_preserves_non_completion_messages`；`rtk cargo test -p thread-service session::tests::context_and_history::aborting_turn_clears_pending_child_completion_tracking_from_turn_state`
  commit: 4baba77cb

- id: exec-command-thread-item-reload
  owner: /root/project_pm/owner_dev_3
  checkout: /Users/bytedance/Projects/my-codex-dev-3
  branch: fix/exec-command-thread-item-reload
  task_type: bugfix
  depends_on: child-completion-thread-status 已 merge 到主线
  files: codex-rs/rollout/**, codex-rs/app-server-protocol/**, codex-rs/app-server/**, codex-rs/thread-service/**, codex-rs/thread-store/**, apps/root-worker-prototype/**, 相关 reload/history/thread item / list_agents 测试
  base_commit: 1b5f3a3961b6113bf6da403724614e0d97545e2d
  pending_sync_from_main:
  status: merged
  objective: 修复重启客户端后普通 `exec_command` thread item 仍不可见的问题，并保留已完成 agent 在 reload 后仍可列出；同时核对 `poll_event` 对应 thread item 的 live / reload 可见性，确认真实 typed item / reload/read 缺口，而不只停留在 agent command replay
  last_update: 2026-07-08
  next_action: 已 merge 到主线；空闲时同步 dev-3 checkout
  blockers: 无
  validation: `rtk cargo test -p thread-history typed_builtin_tool_history_rebuilds_thread_item`；`rtk cargo test -p app-server limited_replay_keeps_poll_event_builtin_tool_items`；`rtk cargo test -p app-server limited_replay_truncates_large_agent_command_execution_output`；`rtk cargo test -p app-server thread_read_after_restart_keeps_unified_exec_command_execution_items`；`rtk cargo build -p app-server --bin app-server`；`rtk cargo test -p rollout limited_mode_sanitizes_unified_exec_command_end_output -- --exact` 仍被仓库既有无关编译问题阻塞
  commit: cdc4896c1（dev-3）；33e930678（main merge）

## Completed
- commit: 33e930678
  summary: 合并 `cdc4896c1` 到主线，修复普通 `exec_command` reload/read 丢 completed 态的问题；对进入 `Limited` 的 `ExecCommandEnd` 统一做有界 sanitize；补齐 `thread-history` 对 builtin tool call 的事件分派，使 `poll_event` thread item 在 `thread/read` / reload 路径恢复可见
  validation: `rtk cargo test -p thread-history typed_builtin_tool_history_rebuilds_thread_item`；`rtk cargo test -p app-server limited_replay_keeps_poll_event_builtin_tool_items`；`rtk cargo test -p app-server limited_replay_truncates_large_agent_command_execution_output`；`rtk cargo test -p app-server thread_read_after_restart_keeps_unified_exec_command_execution_items`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: `rtk cargo test -p rollout limited_mode_sanitizes_unified_exec_command_end_output -- --exact` 仍被仓库现存无关编译问题阻塞；如果后续还要加固，可再补一条“只有 `BuiltinToolCallStarted`、没有 completed 时 reload 仍保留 `InProgress`”测试
- commit: 951f010cd611
  summary: 合并 `cce24f0d7` 到主线，补齐 reload 路径对 agent `exec_command` thread item 的 `Limited` 持久化恢复，并让 `list_agents` 在 live thread 不存在时回退到 persisted completed agent 状态
  validation: `rtk cargo test -p thread-service list_agents_restores_completed_child_from_persisted_history_when_live_thread_is_gone`；`rtk cargo test -p app-server limited_replay_keeps_agent_command_execution_items_visible_after_reload`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: 当前补的是 persisted replay 与 completed-agent fallback；仍建议后续补一条更完整的 app-server reload/integration 测试，串联真实重启后的 thread read/list_agents 行为
- commit: 61d5d4e18
  summary: 合并 `4baba77cb` 到主线，修正 child completion / `WaitChild` 状态语义，使 direct child 本地 active 状态与 pending completion bookkeeping 解耦
  validation: merge 级集成；沿用 owner 已提交验证结果与 reviewer 通过结论
  residual_risk: 仍缺一条更完整的 integration-style 生命周期测试，串联 `spawn_agent -> parent completion -> child completion envelope -> parent wakeup`
- commit: 1dc9c8cba9ae8c446bf8d803dff1486198a75acb
  summary: 合并 `646fa4f5a` 到主线，删除 `wait_agent` / `command_wait`，统一等待入口为 `poll_event`，并让 command output/exit 与 child completion 复用同一 pending-input 唤醒链路
  validation: merge 级集成；沿用 owner 已提交验证结果与 reviewer 通过结论
  residual_risk: `codex-analytics` crate 仍有既有测试基线问题，导致无法补跑一条目标测试；本次改动只显式补齐 `BuiltinToolCall` 穷举覆盖并保持原有 analytics 语义
- commit: eee237bdf2dc6a410d397b6a466caa045192e294
  summary: 合并 `3335c130e34140da7a118cc1bf21b91824c28509`（init context workflow/instruction files 修复）与 `fbb90be4e`（poll_event thread items 可见性与 command 文案）到主线
  validation: merge 级集成；沿用各 owner 已提交验证结果
  residual_risk: `poll-event-thread-item-visibility` 仍受 rollout/tool-service 既有测试问题与前端本地依赖缺失影响，尚无该分支上的完整全链路回归
- commit: ad4dd4c1247552e9f21fda28fda9391f12e5c433
  summary: 合并 compact 展示与 reinject 修复到主线：compact row 可保留 hydrated archived history、compact turn display items 在 reload 路径保持可见、`.codex/memory/current-work.md` 默认忽略；并同步所有 dev checkout 到最新主线
  validation: `rtk pnpm --dir apps/root-worker-prototype test -- src/lib/conversation.test.ts`；`rtk cargo test -p app-server-protocol preserves_compaction_turn_display_items_alongside_compaction_marker`；`rtk cargo test -p thread-service process_compacted_history_reinjects_full_initial_context`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: reviewer 仅保留一条轻量残余风险：若未来 `readCompactHistory()` 真实返回形态再次变化，仍建议在真实 compact/reload 流程下手工观察一次 UI
- commit: 1fb47b58b6467ed815a7450e6feb9b1b2b9419ca
  summary: 合并 unified `poll_event` runtime/tooling 实现与架构收口返修到主线；统一 thread wait primitive、thread-scoped backoff，并将 `command_wait` / `wait_agent` 收敛为兼容壳路径
  validation: `rtk cargo test -p thread-service poll_event_`；`rtk cargo test -p command-service`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: 仍缺一条 `tool-service` 侧回归测试，直接覆盖 `command_wait` started/finished 事件顺序与 `try_finish_now()` 命中后的 shared backoff reset
- commit: 9cfeac59c5e8
  summary: 合并 `58faae6` stale child completion 修复与 `f1d874c` 右侧面板 git/files 视图功能到主线，并将空闲 `dev-2`/`dev-3` checkout fast-forward 到最新集成基线
  validation: merge 级集成；未新增额外验证，沿用各 owner 已提交验证结果
  residual_risk: `right-panel-git-files` 仍保留 owner 提交时已有的前端依赖缺失验证缺口；本次只完成集成与同步
- commit: n/a
  summary: 新增 unified `poll_event` 设计文档，明确 turn 内等待应统一为“新的 thread input 唤醒”，不引入独立 event buffer，backoff 改为 thread-scoped runtime state
  validation: 文本级核对；设计结论已与用户对齐
  residual_risk: 尚未落实现代码；现有部分外部事件路径可能只有 display event、缺少模型可消费 input，后续实现前需补齐契约
- commit: 44c701abcdc3
  summary: 修复 root-worker prototype 中 reload thread 后 tool-like item 显示异常，补齐 builtin tool、event-command、schedule 的前端显示与回归覆盖
  validation: `rtk pnpm --dir apps/root-worker-prototype exec tsx --test --test-name-pattern 'renders event command subscriptions and output events|renders event command exit signals in event summaries|builds visible entries for empty reasoning and builtin schedule tools|mergeThreadSnapshot preserves restored event-driven tool calls with distinct ids|counts event command subscriptions and events in tool usage' src/lib/conversation.test.ts src/lib/thread.test.ts src/lib/contextUsage.test.ts` 通过；`rtk pnpm --dir apps/root-worker-prototype build` 通过
  residual_risk: 相关全文件测试中仍有一条既有 `contextUsage` 断言失败，owner 评估为与本修复无关；本次只锁定新增回归路径
- commit: n/a
  summary: `compact-memory-runtime` 不再继续推进；用户确认该需求已结束，无需 owner 派发或后续集成
  validation: 用户口头确认，无新增实现或验收动作
  residual_risk: 若后续再次开启同主题需求，需要重新建立 active work 与验收范围
- commit: 56adcd785113ff951598794db5e279476d49b7cd
  summary: compact item 默认改为按需加载历史，按 compact 轮次分组展示，折叠后丢弃已加载详情
  validation: `rtk pnpm --dir apps/root-worker-prototype test -- src/lib/compactHistoryRequest.test.ts src/lib/conversation.test.ts src/components/Conversation.test.tsx src/lib/conversationVirtualization.test.ts src/lib/conversationSearch.test.ts`；`rtk pnpm --dir apps/root-worker-prototype build`
  residual_risk: 仍缺一个更贴近 `App.tsx` 异步状态流的竞态测试，以及展开后搜索/焦点联动测试
- commit: 9c3e13d71
  summary: `list_agents` 返回仍注册且可读取状态的 completed agent，并允许其 canonical path 继续复用
  validation: `rtk cargo build -p app-server --bin app-server`
  residual_risk: `thread-service` 现存无关测试编译问题仍未清理，新增回归未能在目标 crate 全量通过
- commit: d9746c6aabc8e6835ba862d9aab6764b1ca011ce
  summary: 删除 UI/UE agent 定义并移除 `ui-design/`、`spec/` 目录
  validation: 文本级核对与目录删除确认
  residual_risk: 可能还有仓库外部引用尚未清理
- commit: 15c66dd
  summary: compact prompt 默认来源切换到 `cwd/.codex/compact/COMPACT.md` 优先、`CODEX_HOME/compact/COMPACT.md` 回退，并解锁 `config-service` 最小测试链
  validation: `rtk cargo test -p config-service compact_prompt`；`rtk cargo test -p config-service config_loader_tests`
  residual_risk: 可选增强是再补一条同时设置 `compact_prompt` 和 `experimental_compact_prompt_file` 的优先级测试
- commit: e90db46
  summary: live command 仅展示 in-progress command，并修复点击后 conversation 被重复拉回导致无法滚动的问题
  validation: `rtk pnpm --dir apps/root-worker-prototype test -- src/lib/threadAnalysis.test.ts`；`rtk pnpm --dir apps/root-worker-prototype test -- src/components/Conversation.test.tsx src/components/RightPanel.test.tsx src/lib/threadAnalysis.test.ts`
  residual_risk: 仍缺少更贴近真实 DOM 副作用的滚动交互测试

## Known Issues
- 原 `.codex/pm-progress.md` 中记录的 thread 重构上下文已过期，且引用的 `spec/` 文档已删除；后续若继续推进该主题，需要重新建立可执行拆分计划。
