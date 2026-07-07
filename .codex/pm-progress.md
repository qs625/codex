# PM Progress

## Current Goal
None

## Active Work

## Completed
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
