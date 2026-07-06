# PM Progress

## Current Goal
完成 compact runtime 重构：compact 不再以单轮摘要作为主要产物，而是维护项目 memory 文件，并将 replacement history 改为基于 memory 文件构造。

## Active Work
- id: compact-memory-runtime
  owner: /root/project_pm/owner_dev
  checkout: /Users/bytedance/Projects/my-codex-dev
  branch: dev
  task_type: feature
  execution_mode: parallel-development
  depends_on: 无
  files: codex-rs/thread-service compact runtime、context-manager compacted history 构造、compact prompt / memory 文件更新流程、必要的 root-worker / config / tests
  base_commit: 9c3e13d71
  pending_sync_from_main: none
  status: in_progress
  objective: compact 以维护 `.codex/memory/user-preferences.md`、`.codex/memory/project-understanding.md`、各 worktree 的 `current-work.md` 为主；replacement history 不再主要依赖模型摘要，而改为由 memory 文件构造；同时设计并实现 soft compact 判定
  last_update: 2026-07-06
  next_action: 派发 owner_dev 完成设计、实现、代码评审与最小验证
  blockers: 无
  validation: 待 owner 执行
  commit:

## Completed
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
