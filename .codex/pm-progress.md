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
- current_focus: 继续推进 service/domain boundary。最新切片已把 compacted history 的用户消息收集、summary/legacy warning 过滤、token-limited history 构造和 initial-context 插入逻辑迁到 `codex-context-manager`；core 和 `codex-rollout-api` 共同复用 owner helper。下一步继续评估更大的 core 功能域边界，优先选择能显著减少 core 生产代码或测试归属的拆分，不做几百行 helper 式碎片化。

## Current State

- Step 1-5: completed。已完成 policy/proxy API、app-server shared types、config 轻量层、tools/code-mode API、app-server 旁路 core 依赖清理。
- Step 6A: completed。`core::config` production runtime 已迁到 `codex-config`，core 保留兼容 facade。
- Step 6B: completed。command output waiting、process id reservation、command output runtime 等不依赖 core/session 的 command primitive 已迁到 `codex-command-runtime`。
- Step 6C: in_progress。已拆出的 owner crate / domain 包括：
  - `codex-command-runtime`: command session controller、wait/write-stdin DTO、exec timeout/capture policy、command output primitive。
  - `codex-process-exec`: 本地 process output capture/aggregation、process result interpretation、process exec request DTO。
  - `codex-mcp-runtime`: MCP manager/runtime、Apps file/skill dependency runtime、accessible connector discovery/cache。
  - `codex-agent-roles`: built-in agent role catalog/spec 和 spawn tool role description builder。
  - `codex-agent-runtime`: goal hidden-context prompt policy、goal runtime state、direct child completion tracker、post-turn selector。
  - `codex-permissions-runtime`: exec policy manager/loader/update runtime、network approval 纯状态机。
  - `codex-sandboxing-api`: Windows sandbox filesystem override 解析。
  - `codex-rollout-api`: fork snapshot history transform、interrupted-turn marker policy、rollout/history replay reconstruction、`PreviousTurnSettings` / `RolloutReconstruction` API。
  - `codex-context-manager`: context history/fragment normalization、context prompt/instruction renderer、environment context renderer、settings-update 纯 diff/render builder，以及 compacted history 纯转换/过滤。
  - `codex-turn-items`: model-visible `ResponseItem` 到 runtime/UI `TurnItem` 的投影、Init Context display item 构造、typed display lifecycle event helper 和 web search display detail。
- 当前 core 保留范围：Session/TurnContext 编排、tool host adapter、Guardian/hook adapter、goal persistence/event/turn policy、exec spawn/PTY/event emission、sandbox execution glue、thread store/read adapter、core-owned tool-suggest plugin/discoverable 聚合逻辑。
- Step 6 前基线：`codex-rs/core/src` 约 293 个 Rust 文件、134123 行；`codex-app-server` 冷编译 timing 中 `codex-core` 单 unit 约 197.7s。
- 当前 `codex-rs/core/src`：约 101460 行。`core/src/session/rollout_reconstruction.rs` 已收缩到 19 行 wrapper；`core/src/session/rollout_reconstruction_tests.rs` 从约 1500 行纯 replay + hydration 混合测试收缩为 195 行 Session hydration 集成测试；纯 replay 测试迁到 `codex-rollout-api`。`core/src/context` 只保留 `TurnContext`/config requirements 到 `EnvironmentContext` DTO 的 adapter；prompt/instruction renderer、environment renderer、settings update builder 和 compacted history 纯转换均迁到 `codex-context-manager`。`core/src/event_mapping.rs` 只保留对 `codex-turn-items` 的 wrapper，投影逻辑和测试已归属 owner crate。

## Latest Slice

Step 6C 第二十三切片：compacted history 纯转换迁到 `codex-context-manager`。

- 新增 `codex-context-manager::compact_history`，拥有 `content_items_to_text`、`collect_compaction_user_messages`、summary/legacy warning predicate、`insert_initial_context_before_last_real_user_or_summary` 和 token-limited `build_compacted_history`。
- `core/src/compact.rs` 收缩为 Session/model compaction runner + owner helper wrapper；纯 compacted history 测试迁到 `context-manager`，core 只保留依赖 Session / remote compact adapter 的测试。
- `codex-rollout-api::reconstruction` 删除重复的 user message collection、content text join、legacy warning filter 和 compacted history builder，改为复用 `codex-context-manager`。
- `core/src/compact_remote.rs` 显式使用 shared legacy-warning predicate，修正原来注释中“parse_turn_item 会过滤 warning”的隐含假设。

## Last Validation

- `rtk cargo test -p codex-context-manager -- --nocapture`：通过，105 条。
- `rtk cargo test -p codex-rollout-api -- --nocapture`：通过，28 条。
- `rtk cargo test -p codex-core --lib compact::tests -- --nocapture`：通过，7 条。
- `rtk cargo test -p codex-core --lib thread_context_usage_counts_compaction_summary_as_compact -- --nocapture`：通过。
- `rtk cargo test -p codex-core --lib record_initial_history_resumed_turn_context_after_compaction_reestablishes_reference_context_item -- --nocapture`：通过。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过。
- `rtk cargo tree -p codex-context-manager --depth 2 --edges normal`：direct graph 保持 agent/workflow/plugin/skills/execpolicy/MCP/protocol/utils 等轻量 API/DTO crate；不依赖 core。
- `codex-context-manager` normal graph 精确 grep 门禁：`codex-core`、`codex-app-server-protocol`、`codex-code-mode`、`codex-network-proxy`、`codex-exec-server`、`codex-state`、`sqlx`、`codex-api`、`codex-openai-files`、`codex-core-skills` 均 PASS。
- `rtk git diff --check`：通过。
- `rtk cargo fmt -p codex-context-manager -p codex-rollout-api -p codex-core -- --check`：通过，仅既有 rustfmt unstable option warning。
- Rust diff `rtk git diff -U0 -- "*.rs" | rg "^\+.*unsafe"`：无命中，本切片没有新增 unsafe。

## Next Action

继续 Step 6C，优先从以下大块里选下一个拆分边界：

- thread-store / rollout persistence runtime 边界：把不依赖 Session/TurnContext 的 thread history read/write、resume metadata DTO 或 rollout file helpers 继续向 rollout/store owner crate 收缩。
- workflow/thread runtime 边界：评估 workflow manager 与 agent runtime/tool host 的 API 分层，避免 workflow 逻辑继续扩大 core。
- unified-exec process manager service 边界：只在能避免 exec-server/PTY/sandbox/session heavy graph 回流时迁移；否则先补 trait/service seam，不做小 helper 拆分。
- core facade 清理：进入 Step 6D 前，把新增 owner crate 的直接 consumer 改为依赖 owner crate，减少继续经 core re-export 的路径。

## Step Plan

- Step 1: completed。拆 policy/proxy 基础类型到 `codex-execpolicy-api` 和 `codex-network-proxy-api`。
- Step 2: completed。拆 app-server shared types 和 request-plugin-install domain plan。
- Step 3: completed。拆 config 轻量层和 loader/edit/requirements owner crate。
- Step 4: completed。拆 tools/code-mode API，V8-backed runtime 由组合根注入。
- Step 5: completed。清 app-server 旁路 core 依赖；最终剩余主路径为 `codex-app-server -> codex-core`。
- Step 6A: completed。`core::config` 大块迁移到 `codex-config`。
- Step 6B: completed。拆出 command runtime primitive。
- Step 6C: in_progress。以 service registry / constructor injection 拆 session、tool runtime、MCP runtime、workflow manager、thread-store/rollout 边界。
- Step 6D: pending。清理 core facade 和旧 re-export callsite，把新增代码默认落到 owner crate。

## Guardrails

- 新 owner crate 不能只把 direct dependency 变成 indirect dependency；必须用 `cargo tree --invert <heavy-crate> --edges normal` 检查 normal graph。
- runtime trait/API crate 不得依赖 app-server v2 envelope、V8、Starlark、Rama、exec-server、sqlx/state 或 concrete API runtime。
- 不使用 Rust `unsafe`。
- 不为了拆分而手写替代成熟三方库的功能。
- core 拆分优先按功能域做大块迁移；不要做几百行级别的过度碎片化。
