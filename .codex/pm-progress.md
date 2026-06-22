# PM Progress

## Current Goal

推进 `codex-core` 大块拆分，让 core 从“聚合根”收缩为过渡 facade。拆分原则是先按功能域确定 owner crate，再迁移代码和 callsite；每一步都要证明没有把 heavy runtime 通过 indirect dependency 拉回。

## Active Work

- id: core-crate-topology-refactor-plan
  execution_mode: performance_refactor_exclusive
  workdir: /Users/bytedance/Projects/my-codex
  owner: root PM direct implementation
  status: active
  started_at: 2026-06-18 16:18:00 CST
  current_step: 6A
  current_focus: 先把 `core::config` 作为大块迁到 `codex-config`，core 只保留兼容 facade；随后再处理 unified exec、session/tool runtime 等更大块。
  baseline:
    - 冷编译测量：`rtk cargo clean && rtk cargo build --timings -p codex-app-server --bin codex-app-server` 通过；Cargo timing total 359.2s，`codex-core` 单 unit 197.7s。
    - Step 6 前 `codex-rs/core/src` 约 293 个 Rust 文件、134123 行，是当前最大的编译和维护瓶颈。
    - `rtk cargo tree -p codex-app-server --invert codex-core --edges normal` 已收敛到 `codex-app-server -> codex-core`；后续重点是缩小 core 自身，而不是继续清 app-server 旁路依赖。
  current_changes:
    - 已将 `codex-rs/core/src/config` 的 production runtime 模块迁到 `codex-rs/config/src/runtime`。
    - `codex-rs/core/src/config/mod.rs` 已改为 `codex-config` re-export facade，并保留 test-only `test_config()`。
    - `Config`、`ConfigBuilder`、`ConfigOverrides`、`Permissions`、`NetworkProxySpec`、feature resolver、permission resolver、loader helper 等 effective config runtime API 由 `codex-config` 拥有。
    - `RolloutConfigView for Config` 已移到 `codex-config`，避免 core 为外部类型实现外部 trait。
    - `config.schema.json` 和 schema 说明已从 `codex-rs/core` 迁到 `codex-rs/config`；`codex-write-config-schema` 默认写入新的 owner 路径，release asset 复制路径同步更新。
    - 已移除 `core-plugins-api` / `hooks-api` 对 `codex-config` 的 optional test-support 回边，避免 `codex-config -> api crate -> codex-config` cycle。
    - 清理了迁移后暴露的 config runtime unused warning，并删除 app-server 中一个不再使用的 core config facade import。
  latest_validation:
    - `rtk cargo test -p codex-config`：通过，393 个测试通过。
    - `rtk cargo check -p codex-core --lib`：通过；仍有既有 unused/dead_code warnings。
    - `rtk cargo build -p codex-app-server --bin codex-app-server`：通过；仍有既有 warnings。
    - `rtk cargo tree -p codex-config --depth 1 --edges normal`：通过，direct deps 为 config/API/DTO/loader 边界。
    - `rtk cargo tree -p codex-config --invert codex-core --edges normal --depth 6`：`codex-core` 不在 graph 中。
    - `rtk cargo tree -p codex-config --invert <heavy> --edges normal --depth 6`：`codex-core`、`codex-app-server-protocol`、`codex-code-mode`、`codex-network-proxy`、`codex-exec-server`、`codex-state`、`sqlx`、`codex-api` 均不在 graph；`codex-execpolicy` 无 normal path。
    - `rtk cargo fmt -p codex-config -p codex-core -p codex-core-plugins-api -p codex-hooks-api -p codex-core-plugins -p codex-hooks` 和 `rtk cargo fmt -p codex-app-server`：通过，仅打印既有 nightly-only rustfmt 配置 warning。
    - `rtk git diff --check`：通过。
    - touched Rust unsafe scan：无 Rust `unsafe` 语法命中；仅测试字符串中出现 “unsafe nickname”。
    - `rtk just bazel-lock-check`：通过，仅打印既有 rules_rs well-known crate annotation warnings。
  remaining_validation:
    - 提交前做最终 `git status` / diff sanity check。
  next_action: 提交 Step 6A 阶段成果 `refactor config runtime out of core`，然后进入 Step 6B unified exec 拆分设计/实施。

## Step Plan

- step: 1
  status: completed
  summary: 拆 policy/proxy 基础类型到 `codex-execpolicy-api` 和 `codex-network-proxy-api`；保留 Starlark parser/evaluator 与 Rama backend 在 implementation crate。

- step: 2
  status: completed
  summary: 按 owner crate 拆 app-server shared types：`codex-auth-types`、`codex-config-types`、`codex-connectors-types`；`codex-tools` 的 request-plugin-install 改为 domain plan，runtime 边界再投影 app-server protocol payload。

- step: 3
  status: completed
  summary: 拆 config 轻量层：`codex-config-types`、`codex-config-toml`、`codex-config-loader`、`codex-config-state`、`codex-config-edit`、`codex-config-requirements` 承接纯 DTO、TOML shape、loader API、layer stack、edit 和 requirements。

- step: 4
  status: completed
  summary: 拆 tools/code-mode：`codex-code-mode-api` 承接 tool description、public tool names、exec prompt、schema renderer、runtime trait/DTO；V8-backed `codex-code-mode` 由组合根注入。

- step: 5
  status: completed
  summary: 清 app-server 旁路 core 依赖：`codex-cloud-requirements`、`codex-app-server-transport`、`codex-guardian`、`codex-chatgpt`、`codex-file-subscription`、`codex-memories-write` 已脱离 app-server 到 core 的 normal 反向路径；最终剩余只有 `codex-app-server -> codex-core`。

- step: 6A
  status: ready_to_commit
  summary: `core::config` 大块迁移到 `codex-config`，core 只保留兼容 re-export facade；schema fixture 和生成工具 ownership 同步迁到 config；验证已通过，等待提交。

- step: 6B
  status: pending
  summary: 将 `core/src/unified_exec` 的 command runtime primitive 和可独立测试逻辑迁到 `codex-command-runtime`；core 保留 approval/sandbox/spawn/session wiring。

- step: 6C
  status: pending
  summary: 以 service registry / constructor injection 拆 session、tool runtime、MCP runtime、workflow manager、thread-store/rollout 边界；runtime crate 只依赖 trait/API，不依赖 concrete heavy implementation。

- step: 6D
  status: pending
  summary: 清理 core facade 和旧 re-export callsite，把新增代码默认落到 owner crate；每个轻量 crate 都用 `cargo tree --invert <heavy-crate>` 做 indirect dependency 门禁。

## Current Topology Rules

- `ResponseItem`、thread/event display、workflow 等当前无关本 refactor，不在本 progress 中继续展开历史细节。
- 新 owner crate 不能只把 direct dependency 变成 indirect dependency；必须检查 normal graph。
- runtime trait/API crate 不得依赖 app-server v2 envelope、V8、Starlark、Rama、exec-server、sqlx/state 或 concrete API runtime。
- 不用 Rust `unsafe`；不要为了拆分而手写替代成熟三方库的功能。
- 大文件和大 crate 优先按功能域拆分，不做几百行级别的过度碎片化。

## Completed

- commit: 561a53ecb
  summary: 更新 agent/workflow 协作规则，改为主 checkout 与固定开发 checkout 协作；删除旧固定 tester agent 文档；创建并初始化固定开发 checkout。

- commit: 46445c2
  summary: 扩展为四个固定 checkout，并明确任务依赖判断、合并顺序和空闲 checkout 同步策略。

- commit: d198eb35c
  summary: 记录 service registry、测试栈溢出处理方式、重构/性能独占等项目规则。

- commit: 5b9d6d429
  summary: 记录 core 大块拆分计划和 macro/config/schema 拆分方向。

## Known Issues

- GPT/ChatGPT auth settings 客户端功能尚未完成；后续需要按调整后的 workflow/owner 流程重新开发，不应合并旧设计-only 分支。
