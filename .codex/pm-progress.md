# PM Progress

## Current Goal

优化 `codex-app-server` 编译时间：基于主 checkout 的 Cargo timing 和依赖拓扑，逐步拆除 app-server 路径上不必要的 `codex-core` 依赖，并完成验证与交付。

## Active Work

- id: app-server-compile-time-topology
  execution_mode: performance_refactor_exclusive
  workdir: /Users/bytedance/Projects/my-codex
  owner: root PM direct implementation
  status: active
  started_at: 2026-06-18 14:14:52 CST
  context:
    - 测量命令：`rtk cargo build --timings -p codex-app-server --bin codex-app-server`，在主 checkout `codex-rs` 执行。
    - 本次使用主 checkout 独立 `codex-rs/target`，不使用共享 target。
    - wall time 约 106.67s；Cargo timing 中 `codex-core` 单 unit 202.58s，是最大瓶颈。
    - app-server 路径上依赖 `codex-core` 的本地 crate 包括 `codex-app-server`、`codex-app-server-transport`、`codex-chatgpt`、`codex-cloud-requirements`、`codex-file-subscription`、`codex-guardian`、`codex-memories-write`。
  plan:
    - step: 1
      status: completed
      description: 拆出低风险 `backoff` helper 到轻量 util crate，让 `codex-cloud-requirements` 和 `codex-app-server-transport` 不再为了 `codex_core::util::backoff` 依赖 `codex-core`。
    - step: 2
      status: completed
      description: 运行最小验证，确认相关 crate 编译通过，并检查 `cargo tree -p codex-app-server --invert codex-core --edges normal` 中 `codex-cloud-requirements` 是否脱离 core；`app-server-transport` 若仍因测试或其他 API 依赖 core，则记录剩余原因。
    - step: 3
      status: completed
      description: 评估下一层拆分：`Config` 边界、connector helper、unified exec/command runtime handle、thread runtime facade，按收益和风险排序。
    - step: 4
      status: completed
      description: 若第 1 阶段收益明确且验证通过，提交本阶段改动；更大拆分另开后续步骤，避免一次改动过大。
  findings:
    - 已新增 `codex-utils-backoff`，`codex-core::util::backoff` 保留为 re-export。
    - `codex-cloud-requirements` 已移除 `codex-core` 依赖；`cargo tree -p codex-app-server --invert codex-core --edges normal` 中该 crate 已消失。
    - `codex-app-server-transport` 的 backoff 调用已迁移，并把生产代码的 `find_codex_home` 改为 `codex-utils-home-dir`；`codex-core` 仅保留为 dev-dependency 供测试支持 API 使用。
    - 当前 `cargo tree -p codex-app-server --invert codex-core --edges normal` 剩余路径为 `codex-app-server`、`codex-chatgpt`、`codex-file-subscription`、`codex-guardian`、`codex-memories-write`。其中 `chatgpt`/`guardian` 主要卡 `Config` 仍在 core，`file-subscription`/`memories-write` 牵涉 thread/runtime handle，应作为后续较大拆分处理。
  validation:
    - `rtk cargo check -p codex-utils-backoff -p codex-cloud-requirements -p codex-app-server-transport` 通过。
    - `rtk cargo test -p codex-utils-backoff` 通过，2 个测试通过。
    - `rtk cargo check -p codex-app-server --bin codex-app-server` 通过。
    - `rtk cargo build --timings -p codex-app-server --bin codex-app-server` 通过，warm-target wall time 约 100s；最新 timing 为 `target/cargo-timings/cargo-timing-20260618T062735.642709Z.html`。
    - `rtk rustfmt --check ...` 对本次触碰的 Rust 文件通过；全 workspace `cargo fmt --check` 仍被既有未格式化文件阻断。
  next_action: 更大收益的下一阶段应优先设计 `Config` 从 core 拆出的边界，再处理 runtime handle；本阶段改动可先独立提交。

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
- commit: current
  summary: 允许 refactor/performance 全局独占任务直接在主 checkout 工作，由固定 `owner_main` 承接；普通开发仍只派发到三个 dev checkout，PM 负责最终验收和同步空闲 dev。
  validation: `rtk node --check .codex/workflows/feature-dev/workflow.ts` 通过；文档搜索确认 `owner_main` 只承接 refactor/performance 独占任务，普通开发仍只派发到 dev checkout。
  residual_risk: refactor/performance 在主 checkout 工作时会占用集成分支；PM 必须确保 Active Work 为空且 dev 已同步后再启动。

## Known Issues

- GPT/ChatGPT auth settings 客户端功能尚未完成；后续需要重新按调整后的 workflow/owner 流程开发，不应合并旧设计-only 分支。
