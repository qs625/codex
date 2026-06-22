# PM Progress

## Current Goal

推进 `codex-core` 大块拆分，让 core 从“聚合根”收缩为过渡 facade。优先按功能域确定 owner crate，再迁移代码和 callsite；每个阶段都要证明没有把 heavy runtime 通过 indirect dependency 拉回。

## Active Work

- id: core-crate-topology-refactor-plan
- mode: performance_refactor_exclusive
- workdir: /Users/bytedance/Projects/my-codex
- owner: root PM direct implementation
- status: active
- current_step: 6C
- current_focus: Step 6C 首个 service boundary 已开始：`command_wait` / `command_write_stdin` 通过 `codex-command-runtime::CommandSessionController` trait 调用，tool handler 不再直接依赖 `UnifiedExecProcessManager` concrete。

## Current State

- Step 6A 已完成并提交：`47db37b refactor config runtime out of core`。
  `core::config` production runtime 已迁到 `codex-config`，core 只保留兼容 re-export facade；schema fixture 和生成工具 ownership 已同步迁到 config。
- Step 6B 已完成两个切片：
  `3d7cda6 refactor command output waiting into runtime` 将 command output handles 和 output deadline collector 下沉到 `codex-command-runtime`。
  `81570cb refactor command process ids into runtime` 将 process id reservation、completed process id history 和 pruning policy 下沉到 `codex-command-runtime`。
- Step 6B 第三个切片已把 command output buffer、output runtime hub、local broadcast pump、UTF-8 output delta splitter 和 transcript aggregation helper 集中到 `codex-command-runtime::output`；core `UnifiedExecProcess` 只持有 `CommandOutputRuntime` 并负责 PTY/exec-server wiring 与 EventMsg emission。
- 已补上 Step 6A 迁移后的 config test-support 边界：`codex-config/test-support` 公开测试构造 helper，`codex-core` dev build 启用该 feature，恢复 core 单测对 migrated config API 的访问。
- Step 6C 首个切片已在 `codex-command-runtime` 增加 `CommandSessionController` / `CommandWaitOperation` trait；core 用 `UnifiedExecCommandSessionController` adapter 连接现有 `UnifiedExecProcessManager`，并在 `SessionServices` 中 constructor-inject 该 trait service。`command_wait` 和 `command_write_stdin` handler 现在只消费 command-runtime DTO/trait。
- `UnifiedExecProcess` / `process_manager` 剩余逻辑仍绑定 exec-server protocol、PTY、sandbox denial detection、core error type、Session/TurnContext、ToolEmitter、ToolOrchestrator 和 network approval；继续迁移前需要 Step 6C 的 trait/service 边界，避免只做小 helper 或把 heavy runtime 间接拉回。
- Step 6 前基线：`codex-rs/core/src` 约 293 个 Rust 文件、134123 行；`codex-app-server` 冷编译 timing 中 `codex-core` 单 unit 约 197.7s。
- 当前 `codex-rs/core/src` 约 112207 行；`core/src/unified_exec` 剩余最大文件为 `process_manager.rs` 1226 行、`process.rs` 424 行、`async_watcher.rs` 347 行。

## Last Validation

- `rtk cargo test -p codex-config`：通过。
- `rtk cargo test -p codex-command-runtime`：通过。
- `rtk cargo test -p codex-core command_wait -- --nocapture`：通过。
- `rtk cargo test -p codex-core unified_exec::async_watcher -- --nocapture`：通过。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过，仅既有 warnings。
- `rtk cargo tree -p codex-config --invert <heavy> --edges normal --depth 6`：未把 core、app-server protocol、code-mode、exec-server、state/sqlx 等 heavy runtime 拉回。
- `rtk cargo tree -p codex-command-runtime --depth 2 --edges normal`：direct graph 仍仅为 decoding、rand、tokio/tokio-util。
- workspace 反查 heavy crate 后检查 `codex-command-runtime` 是否出现在反向树：core、app-server protocol、code-mode、exec-server、state/sqlx 等均 PASS。
- `rtk just bazel-lock-check`：通过，仅既有 rules_rs well-known crate annotation warnings。
- `rtk git diff --check`、touched Rust `unsafe` scan：通过。
- 用户要求后已执行 `rtk cargo clean`，当前 `codex-rs/target` 已移除；后续 broad Rust 验证会重新冷编译。

## Next Action

继续 Step 6C：按同样模式继续拆 session/tool runtime concrete service，优先选择已存在 API crate 或轻量 owner crate 能承载的 trait service；避免把 exec-server、sandbox、state/sqlx、app-server protocol 等 heavy runtime 通过 indirect graph 拉回。

## Step Plan

- Step 1: completed。拆 policy/proxy 基础类型到 `codex-execpolicy-api` 和 `codex-network-proxy-api`。
- Step 2: completed。拆 app-server shared types 和 request-plugin-install domain plan。
- Step 3: completed。拆 config 轻量层和 loader/edit/requirements owner crate。
- Step 4: completed。拆 tools/code-mode API，V8-backed runtime 由组合根注入。
- Step 5: completed。清 app-server 旁路 core 依赖；最终剩余主路径为 `codex-app-server -> codex-core`。
- Step 6A: completed。`core::config` 大块迁移到 `codex-config`。
- Step 6B: completed。已把 `core/src/unified_exec` 中不依赖 exec-server/PTY/sandbox/session 的 command runtime primitive 迁到 `codex-command-runtime`；剩余 lifecycle 需要 Step 6C 边界。
- Step 6C: in_progress。以 service registry / constructor injection 拆 session、tool runtime、MCP runtime、workflow manager、thread-store/rollout 边界。
- Step 6D: pending。清理 core facade 和旧 re-export callsite，把新增代码默认落到 owner crate。

## Guardrails

- 新 owner crate 不能只把 direct dependency 变成 indirect dependency；必须用 `cargo tree --invert <heavy-crate> --edges normal` 检查 normal graph。
- runtime trait/API crate 不得依赖 app-server v2 envelope、V8、Starlark、Rama、exec-server、sqlx/state 或 concrete API runtime。
- 不使用 Rust `unsafe`。
- 不为了拆分而手写替代成熟三方库的功能。
- core 拆分优先按功能域做大块迁移；不要做几百行级别的过度碎片化。
