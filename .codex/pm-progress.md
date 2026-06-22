# PM Progress

## Current Goal

推进 `codex-core` 大块拆分，让 core 从“聚合根”收缩为过渡 facade。优先按功能域确定 owner crate，再迁移代码和 callsite；每个阶段都要证明没有把 heavy runtime 通过 indirect dependency 拉回。

## Active Work

- id: core-crate-topology-refactor-plan
- mode: performance_refactor_exclusive
- workdir: /Users/bytedance/Projects/my-codex
- owner: root PM direct implementation
- status: active
- current_step: 6B
- current_focus: Step 6B 第三个切片已实现；下一步继续判断 `UnifiedExecProcess` 剩余 lifecycle 是否需要先拆 exec-server/sandbox error trait 边界，或转入 Step 6C 的 session/tool runtime trait 边界。

## Current State

- Step 6A 已完成并提交：`47db37b refactor config runtime out of core`。
  `core::config` production runtime 已迁到 `codex-config`，core 只保留兼容 re-export facade；schema fixture 和生成工具 ownership 已同步迁到 config。
- Step 6B 已完成两个切片：
  `3d7cda6 refactor command output waiting into runtime` 将 command output handles 和 output deadline collector 下沉到 `codex-command-runtime`。
  `81570cb refactor command process ids into runtime` 将 process id reservation、completed process id history 和 pruning policy 下沉到 `codex-command-runtime`。
- Step 6B 第三个切片已把 command output buffer、output runtime hub、local broadcast pump、UTF-8 output delta splitter 和 transcript aggregation helper 集中到 `codex-command-runtime::output`；core `UnifiedExecProcess` 只持有 `CommandOutputRuntime` 并负责 PTY/exec-server wiring 与 EventMsg emission。
- `UnifiedExecProcess` 仍依赖 exec-server protocol、PTY、sandbox denial detection 和 core error type，不能在没有边界拆分前整体搬迁。
- Step 6 前基线：`codex-rs/core/src` 约 293 个 Rust 文件、134123 行；`codex-app-server` 冷编译 timing 中 `codex-core` 单 unit 约 197.7s。

## Last Validation

- `rtk cargo test -p codex-config`：通过。
- `rtk cargo test -p codex-command-runtime`：通过。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过，仅既有 warnings。
- `rtk cargo tree -p codex-config --invert <heavy> --edges normal --depth 6`：未把 core、app-server protocol、code-mode、exec-server、state/sqlx 等 heavy runtime 拉回。
- `rtk cargo tree -p codex-command-runtime --depth 2 --edges normal`：direct graph 仍仅为 decoding、rand、tokio/tokio-util。
- workspace 反查 heavy crate 后检查 `codex-command-runtime` 是否出现在反向树：core、app-server protocol、code-mode、exec-server、state/sqlx 等均 PASS。
- `rtk git diff --check`、touched Rust `unsafe` scan：通过；Cargo 依赖未变更，未重跑 Bazel lockfile。
- 本轮尝试 `rtk cargo test -p codex-core unified_exec::async_watcher -- --nocapture`：被既有 Step 6A config test-support API 编译问题阻塞，先不作为 Step 6B output runtime 切片门禁；本轮相关测试已迁入 `codex-command-runtime` 并通过。

## Next Action

完成本轮切片提交后，继续盘点 `UnifiedExecProcess` 剩余 lifecycle：如果不先引入 exec-server/sandbox/core-error trait 边界就只能做小块 helper，则记录阻塞原因并转向 Step 6C 的 session/tool runtime trait 边界。

## Step Plan

- Step 1: completed。拆 policy/proxy 基础类型到 `codex-execpolicy-api` 和 `codex-network-proxy-api`。
- Step 2: completed。拆 app-server shared types 和 request-plugin-install domain plan。
- Step 3: completed。拆 config 轻量层和 loader/edit/requirements owner crate。
- Step 4: completed。拆 tools/code-mode API，V8-backed runtime 由组合根注入。
- Step 5: completed。清 app-server 旁路 core 依赖；最终剩余主路径为 `codex-app-server -> codex-core`。
- Step 6A: completed。`core::config` 大块迁移到 `codex-config`。
- Step 6B: in_progress。拆 `core/src/unified_exec` 到 `codex-command-runtime`。
- Step 6C: pending。以 service registry / constructor injection 拆 session、tool runtime、MCP runtime、workflow manager、thread-store/rollout 边界。
- Step 6D: pending。清理 core facade 和旧 re-export callsite，把新增代码默认落到 owner crate。

## Guardrails

- 新 owner crate 不能只把 direct dependency 变成 indirect dependency；必须用 `cargo tree --invert <heavy-crate> --edges normal` 检查 normal graph。
- runtime trait/API crate 不得依赖 app-server v2 envelope、V8、Starlark、Rama、exec-server、sqlx/state 或 concrete API runtime。
- 不使用 Rust `unsafe`。
- 不为了拆分而手写替代成熟三方库的功能。
- core 拆分优先按功能域做大块迁移；不要做几百行级别的过度碎片化。
