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
    - step: 5
      status: completed
      description: 完整盘点 `codex-file-subscription` 对 `codex-core` 的生产依赖调用面，确认哪些能力属于 thread runtime、event-command 注入、订阅 metadata 持久化，以及哪些 unified exec 参数当前只是透传或未使用。
    - step: 6
      status: completed
      description: 设计并落地 `file-subscription` 的轻量 runtime trait/handle 边界；由 core/app-server 侧 adapter 实现具体 `ThreadManager` 调用，避免把 `ThreadManager`、`CodexThread` 或 unified exec runtime 类型继续暴露给小 crate。
    - step: 7
      status: completed
      description: 将 `codex-file-subscription` 迁移到新 runtime 边界，移除 normal `codex-core` 依赖；保持事件注入、订阅恢复、metadata 更新和 final-status notify 行为不变。
    - step: 8
      status: completed
      description: 对 `file-subscription` 阶段做最小验证：`rtk cargo check -p codex-file-subscription -p codex-app-server --bin codex-app-server`，并用 `rtk cargo tree -p codex-app-server --invert codex-core --edges normal` 确认该路径已脱离 core；通过后提交小阶段。
    - step: 9
      status: in_progress
      description: 单独盘点 `codex-memories-write` 对 core runtime 的依赖，设计 thread/model/runtime facade；该阶段涉及 `CodexThread`、`ModelClient`、`Prompt`、`ResponseEvent`、`RolloutRecorder` 等较大边界，不能和 `file-subscription` patch 混合。
    - step: 10
      status: pending
      description: 在 `memories-write` facade 方案明确后实施拆分、验证 app-server 编译路径和 timing 变化；若风险或改动体量过大，先提交设计记录或中间 adapter，再继续实现。
    - step: 11
      status: pending
      description: 完成所有可行拆分后重新运行 `rtk cargo build --timings -p codex-app-server --bin codex-app-server`，比较 `codex-core` 是否仍在 app-server 必要路径、warm build wall time 和 unit seconds，并更新最终结论。
  findings:
    - 已新增 `codex-utils-backoff`，`codex-core::util::backoff` 保留为 re-export。
    - `codex-cloud-requirements` 已移除 `codex-core` 依赖；`cargo tree -p codex-app-server --invert codex-core --edges normal` 中该 crate 已消失。
    - `codex-app-server-transport` 的 backoff 调用已迁移，并把生产代码的 `find_codex_home` 改为 `codex-utils-home-dir`；`codex-core` 仅保留为 dev-dependency 供测试支持 API 使用。
    - `codex-guardian` 的 thread lifecycle contributor 已改为配置泛型，移除 `codex-core` 依赖。
    - `codex-chatgpt` 默认 feature 已改为不依赖 `codex-core`；CLI-only `apply_command` / `get_task` 放入 `apply-command` feature，`codex-cli` 显式启用该 feature。app-server/TUI 调用 ChatGPT connector/settings API 时先构造轻量 `ChatGptConfig`，accessible/enabled connector 状态直接走 core connector API。
    - 当前 `cargo tree -p codex-app-server --invert codex-core --edges normal` 剩余路径为 `codex-app-server`、`codex-file-subscription`、`codex-memories-write`。其中 `file-subscription`/`memories-write` 牵涉 thread/runtime handle，应作为后续较大拆分处理。
    - `codex-file-subscription` 的 core 依赖不再是单纯 config 类型：生产代码使用 `ThreadManager`、`UnifiedExecManagerHandle` 和 `UnifiedExecProcessManager` 来恢复订阅、注入事件和提供 event-command 工具；下一步需要先在 extension/runtime 边界定义稳定 trait/handle，再让 core adapter 实现。
    - `codex-file-subscription` 已新增 `FileSubscriptionThreadRuntime` 边界，订阅 active count、event-driven trigger、event-command event、订阅 metadata 持久化和历史恢复都通过 host adapter 执行；app-server 侧用 `CoreFileSubscriptionThreadRuntime` 适配 `ThreadManager`。原先透传但未使用的 unified exec manager 参数已从该扩展路径移除。
    - `codex-file-subscription` 已移除 normal `codex-core` 依赖；当前 `cargo tree -p codex-app-server --invert codex-core --edges normal` 剩余路径为 `codex-app-server` 自身和 `codex-memories-write`。
    - `codex-memories-write` 同样直接使用 `CodexThread`、`ThreadManager`、`ModelClient`、`Prompt`、`ResponseEvent`、`RolloutRecorder` 等 core runtime 类型；这应作为 thread runtime facade 的较大拆分，不能和低风险 util/config 拆分混在一个小 patch。
    - `codex-memories-write` 调用面已初步盘点：`start.rs` 负责入口 eligibility 和创建 `MemoryStartupContext`；`runtime.rs` 负责 telemetry、model info、stage-one model streaming、内部 consolidation agent spawn/submit/shutdown；`phase1.rs` 负责 rollout 读取、prompt 构造和 state-db job 结果写入；`phase2.rs` 负责 memory workspace sync、内部 agent loop/heartbeat/token usage；`guard.rs` 只需要 rate-limit 配置字段和 auth manager。
    - `memories-write` 拆 core 的关键不是单个 util，而是需要 host-provided facade：一层负责 thread/runtime 操作（state_db、config snapshot、model info、spawn internal thread、submit prompt、agent status/token usage/shutdown），另一层负责 stage-one model sampling（或把 `ModelClient`/`Prompt` 迁到更小 crate）。直接在小 crate 里复刻 `ThreadManager`/`CodexThread` 会扩大耦合，不应这样做。
    - `memories-write` 已先清理明显 core re-export：`RolloutRecorder` 改为直接使用 `codex-rollout`，`ResponseEvent` 改为直接使用 `codex-api`，简单的 `content_items_to_text` 本地化；剩余 core 引用集中在 `Config`、`Prompt`、`ModelClient`、`ThreadManager`/`CodexThread`、`resolve_installation_id` 和 `build_turn_metadata_header`。
  validation:
    - `rtk cargo check -p codex-utils-backoff -p codex-cloud-requirements -p codex-app-server-transport` 通过。
    - `rtk cargo test -p codex-utils-backoff` 通过，2 个测试通过。
    - `rtk cargo check -p codex-app-server --bin codex-app-server` 通过。
    - `rtk cargo check -p codex-guardian -p codex-app-server --bin codex-app-server` 通过。
    - `rtk cargo check -p codex-chatgpt -p codex-chatgpt --features apply-command -p codex-app-server --bin codex-app-server -p codex-tui -p codex-cli` 通过。
    - `rtk cargo build --timings -p codex-app-server --bin codex-app-server` 通过，warm-target wall time 约 100s；最新 timing 为 `target/cargo-timings/cargo-timing-20260618T062735.642709Z.html`。
    - `codex-chatgpt` 拆分后重新执行 `rtk cargo build --timings -p codex-app-server --bin codex-app-server` 通过，warm-target wall time 约 22s；最新 timing 为 `target/cargo-timings/cargo-timing-20260618T065116.613751Z.html`，本次只重编 `codex-app-server`、bin、`codex-chatgpt`、`codex-guardian`，未重编 `codex-core`。
    - `file-subscription` 拆分后执行 `rtk cargo test -p codex-file-subscription` 通过，退出码 0。
    - `file-subscription` 拆分后执行 `rtk cargo check -p codex-file-subscription -p codex-app-server --bin codex-app-server` 通过，0 errors，42 warnings（均为既有 unused/dead_code 风格警告）。
    - `file-subscription` 拆分后执行 `rtk cargo tree -p codex-app-server --invert codex-core --edges normal`，确认 `codex-file-subscription` 已不在 `codex-core` normal 反向路径中。
    - 对本阶段触碰的 Rust 文件执行 `rtk rustfmt --check ...` 通过；命令仍打印 workspace rustfmt 配置中 nightly-only `imports_granularity = Item` warning。
    - `memories-write` re-export 清理后执行 `rtk cargo check -p codex-memories-write` 通过，退出码 0。
    - `memories-write` re-export 清理后对触碰文件执行 `rtk rustfmt --check memories/write/src/runtime.rs memories/write/src/phase1.rs` 通过；命令仍打印 workspace rustfmt 配置中 nightly-only `imports_granularity = Item` warning。
    - `rtk rustfmt --check ...` 对本次触碰的 Rust 文件通过；全 workspace `cargo fmt --check` 仍被既有未格式化文件阻断。
  next_action: 进入 `memories-write` 阶段，先盘点 `CodexThread`、`ThreadManager`、`ModelClient`、`Prompt`、`ResponseEvent`、`RolloutRecorder` 等调用面，再设计 thread/model/runtime facade；不要把该较大拆分和已完成的 `file-subscription` patch 混合。

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
