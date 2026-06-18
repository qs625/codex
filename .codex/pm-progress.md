# PM Progress

## Current Goal

None

## Active Work

None

## Completed

- commit: 561a53ecb
  summary: 更新 agent/workflow 协作规则，改为当前主 checkout 与固定开发 checkout `~/Projects/my-codex-dev` 双目录协作；删除旧固定 tester agent 文档；创建并初始化固定开发 checkout。
  validation: `rtk node --check .codex/workflows/feature-dev/workflow.ts` 通过；确认开发目录只保留主 checkout 和 `~/Projects/my-codex-dev`。
  residual_risk: GPT auth settings 仍是未完成设计方向，不在本次合并范围内。
- commit: 46445c2
  summary: 将 PM 协作规则扩展为四个固定 checkout：`~/Projects/my-codex`、`~/Projects/my-codex-dev`、`~/Projects/my-codex-dev-2`、`~/Projects/my-codex-dev-3`，并明确任务依赖判断、合并顺序和空闲 checkout 同步策略。
  validation: `rtk node --check .codex/workflows/feature-dev/workflow.ts` 通过；`rtk git worktree list` 确认四个 checkout 已同步到同一基线；原生 `find -type l` 确认没有 target/node_modules 共享链接。
  residual_risk: 新增 checkout 尚未安装 JS 依赖或生成 Rust target；首次验证会各自独立构建。
- commit: current
  summary: 将 PM 派发规则改为四个 checkout 各绑定一个长期 owner thread，PM 复用固定 owner 串行处理该 checkout 的任务，不再为每个任务新建 owner；同时标明 dynamic workflow 的 owner 绑定范围是单个 run，不作为 PM 固定 owner 池入口。
  validation: `rtk node --check .codex/workflows/feature-dev/workflow.ts` 通过；文档搜索确认固定 owner 映射已写入 PM 规则，workflow 文档已标明 run-scoped owner 绑定限制。
  residual_risk: 当前 workflow SDK 仍按 workflow run 绑定 `Agent(id)`；PM 常规派发应直接复用固定 owner thread，不通过 `feature-dev` workflow 入口。
- commit: current
  summary: 将主 checkout `~/Projects/my-codex` 改为仅用于 PM 集成合并；开发任务只派发到三个固定开发 checkout，对应长期 owner 为 `owner_dev`、`owner_dev_2`、`owner_dev_3`。owner 只在开发 checkout 提交和验证，最终合并、冲突处理和同步由 PM 在主 checkout 完成。
  validation: `rtk node --check .codex/workflows/feature-dev/workflow.ts` 通过；文档搜索确认主 checkout 不再作为开发目录，dev 同步主 checkout 的时机已明确为派发前、合并后和 active dev 延后同步。
  residual_risk: active dev 延后同步期间，依赖该 commit 的新任务不能派发到该 dev，必须等待同步或选择已同步的空闲 dev。
- commit: current
  summary: 补充 PM 调度规则：refactor、代码健康和 performance 任务为全局独占，只有没有 active 开发任务且 dev checkout 同步完成后才能启动；独占任务运行时不得并行启动开发任务或第二个独占任务。
  validation: `rtk node --check .codex/workflows/feature-dev/workflow.ts` 通过；文档搜索确认 PM 工作规则、标准流程和 progress 模板都包含 execution_mode 与独占约束。
  residual_risk: 独占规则依赖 PM 按 progress file 判断 active work；后续派发前必须先更新 progress。

## Known Issues

- GPT/ChatGPT auth settings 客户端功能尚未完成；后续需要重新按调整后的 workflow/owner 流程开发，不应合并旧设计-only 分支。
