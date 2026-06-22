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
- current_focus: Step 6C 继续推进 service/domain boundary：command session controller 已 trait 化；MCP runtime、Apps file/skill dependency runtime 已迁到 `codex-mcp-runtime`；agent role catalog/spec 已迁到 `codex-agent-roles`；exec policy manager/loader trait/update runtime 和 network approval 纯状态机已迁到 `codex-permissions-runtime`；Windows sandbox filesystem override 解析已迁到 `codex-sandboxing-api`。core 继续保留 session/tool host adapter、Guardian/hook adapter、exec spawn/PTY/event emission 和 sandbox execution glue。

## Current State

- Step 6A 已完成并提交：`47db37b refactor config runtime out of core`。
  `core::config` production runtime 已迁到 `codex-config`，core 只保留兼容 re-export facade；schema fixture 和生成工具 ownership 已同步迁到 config。
- Step 6B 已完成两个切片：
  `3d7cda6 refactor command output waiting into runtime` 将 command output handles 和 output deadline collector 下沉到 `codex-command-runtime`。
  `81570cb refactor command process ids into runtime` 将 process id reservation、completed process id history 和 pruning policy 下沉到 `codex-command-runtime`。
- Step 6B 第三个切片已把 command output buffer、output runtime hub、local broadcast pump、UTF-8 output delta splitter 和 transcript aggregation helper 集中到 `codex-command-runtime::output`；core `UnifiedExecProcess` 只持有 `CommandOutputRuntime` 并负责 PTY/exec-server wiring 与 EventMsg emission。
- 已补上 Step 6A 迁移后的 config test-support 边界：`codex-config/test-support` 公开测试构造 helper，`codex-core` dev build 启用该 feature，恢复 core 单测对 migrated config API 的访问。
- Step 6C 首个切片已在 `codex-command-runtime` 增加 `CommandSessionController` / `CommandWaitOperation` trait；core 用 `UnifiedExecCommandSessionController` adapter 连接现有 `UnifiedExecProcessManager`，并在 `SessionServices` 中 constructor-inject 该 trait service。`command_wait` 和 `command_write_stdin` handler 现在只消费 command-runtime DTO/trait。
- Step 6C 第二个切片新增 `codex-mcp-runtime` owner crate，迁出 `McpManager`、Codex Apps auth provider/context、MCP runtime environment 构造、Apps tool enable/approval policy、`with_app_enabled_state` 和 `build_mcp_tool_exposure`；`codex-core` 的 `mcp.rs`、`connectors.rs`、`mcp_tool_exposure.rs` 只保留兼容 re-export，后续 Step 6D 再清 callsite facade。
- Step 6C 第三个切片把 Apps SDK `openai/fileParams` 执行期参数重写、OpenAI file upload auth adapter 和上传 payload 生成迁到 `codex-mcp-runtime::openai_file`；core `mcp_openai_file` 现在只把 `Session`/`TurnContext` 转成 auth/uploader/base URL/path resolver 调用 runtime。
- Step 6C 第四个切片把 Skill MCP dependency install 的 first-party gating、missing dependency collection、prompt decision、global config persistence、OAuth login retry policy 和 refresh server merge 迁到 `codex-mcp-runtime::skill_dependencies`；core `mcp_skill_dependencies` 现在只实现 `McpSkillDependencyHost`，提供 Session prompt/state、MCP manager/auth runtime 和 live refresh adapter。
- Step 6C 第五个切片把 built-in agent role declarations、built-in role config content、role resolution helper 和 spawn-agent tool role description builder 迁到 `codex-agent-roles`；core `agent::role` 现在只保留把 resolved role config layer 应用到 `Config` 的 adapter，spawn tool spec、agent nickname resolution 和默认 role 名直接依赖 `codex-agent-roles`。
- Step 6C 第六个切片把 `ExecPolicyManager`、`ExecPolicyLoader`、`ExecPolicyLoadResult`、`ExecPolicyUpdateError` 和 exec policy rules append/update runtime 迁到 `codex-permissions-runtime`；core `exec_policy.rs` 只保留 `child_uses_parent_exec_policy` 和 test-only Starlark loader helpers，`codex-execpolicy-loader` 直接依赖 `codex-permissions-runtime` trait/result，不再依赖 `codex-core`。
- Step 6C 第七个切片把 network approval 的 host/protocol/port key、pending approval 去重、session allow/deny cache、active call outcome/cancellation、blocked request denial message 和 approval-flow gating 迁到 `codex-permissions-runtime::network_approval`；core `tools::network_approval` 保留 `NetworkApprovalService` wrapper、`Session`/Guardian/hook prompt、network policy amendment persistence/display 和 deferred `ToolError` 映射。
- Step 6C 第八个切片把 Windows sandbox filesystem override 解析迁到 `codex-sandboxing-api::windows_filesystem_overrides`：包括 restricted-token/elevated backend selection、unsupported reason、read/write root override、deny-read/deny-write overlay 和原 core 单测；core `exec.rs` 只在 build/execute path 调用该 API，`core::sandboxing::ExecRequest` 保存 sandboxing-api owner type。
- `UnifiedExecProcess` / `process_manager` 剩余逻辑仍绑定 exec-server protocol、PTY、sandbox denial detection、core error type、Session/TurnContext、ToolEmitter、ToolOrchestrator 和 network approval；继续迁移前需要 Step 6C 的 trait/service 边界，避免只做小 helper 或把 heavy runtime 间接拉回。
- Step 6 前基线：`codex-rs/core/src` 约 293 个 Rust 文件、134123 行；`codex-app-server` 冷编译 timing 中 `codex-core` 单 unit 约 197.7s。
- 当前 `codex-rs/core/src` 约 270 个 Rust 文件、109162 行；`core/src/exec.rs` 已从 1517 行降到 1141 行，Windows sandbox override 单测迁到 `codex-sandboxing-api`；`core/src/unified_exec` 剩余最大文件为 `process_manager.rs` 1226 行、`process.rs` 424 行、`async_watcher.rs` 347 行。

## Last Validation

- `rtk cargo test -p codex-config`：通过。
- `rtk cargo test -p codex-command-runtime`：通过。
- `rtk cargo test -p codex-core command_wait -- --nocapture`：通过。
- `rtk cargo test -p codex-core unified_exec::async_watcher -- --nocapture`：通过。
- `rtk cargo check -p codex-mcp-runtime`：通过。
- `rtk cargo test -p codex-core mcp_tool_exposure -- --nocapture`：通过。
- `rtk cargo test -p codex-core connectors::tests::app_tool_policy -- --nocapture`：通过。
- `rtk cargo test -p codex-mcp-runtime openai_file -- --nocapture`：通过，覆盖迁入后的 Apps SDK file 参数重写。
- `rtk cargo test -p codex-core mcp_openai_file -- --nocapture`：通过，覆盖 core wrapper 到 runtime trait 的桥接。
- `rtk cargo test -p codex-mcp-runtime -- --nocapture`：通过，覆盖 Apps SDK file 参数重写和 Skill MCP dependency install domain 逻辑。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过，仅既有 warnings。
- `rtk cargo tree -p codex-config --invert <heavy> --edges normal --depth 6`：未把 core、app-server protocol、code-mode、exec-server、state/sqlx 等 heavy runtime 拉回。
- `rtk cargo tree -p codex-command-runtime --depth 2 --edges normal`：direct graph 仍仅为 decoding、rand、tokio/tokio-util。
- workspace 反查 heavy crate 后检查 `codex-command-runtime` 是否出现在反向树：core、app-server protocol、code-mode、exec-server、state/sqlx 等均 PASS。
- `rtk cargo tree -p codex-mcp-runtime --depth 2 --edges normal`：direct graph 通过 `codex-config`、MCP/API/config/types 和 exec-server-api 轻量边界承载 MCP registry/policy，不依赖 core。
- workspace 反查 heavy crate 后检查 `codex-mcp-runtime` 是否出现在反向树：core、app-server protocol、code-mode、network-proxy、exec-server、state/sqlx、codex-api 均 PASS。
- `codex-mcp-runtime` OpenAI file 迁移后 normal graph grep 门禁：core、app-server protocol、code-mode、network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files` 均 PASS；runtime 只依赖 `codex-openai-files-api` trait 边界。
- `codex-mcp-runtime` Skill dependency 迁移后 normal graph grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、network-proxy、concrete exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS；runtime 只依赖 `codex-core-skills-api` 和 MCP/config/API 边界。
- `rtk cargo test -p codex-agent-roles -- --nocapture`：通过，覆盖迁入后的 spawn tool spec builder 和 built-in role catalog。
- `rtk cargo test -p codex-core agent::role -- --nocapture`：通过，覆盖 core apply-role Config reload adapter。
- `codex-agent-roles` normal graph grep 门禁：core、app-server protocol、code-mode、network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 absent。
- workspace 反查 heavy crate 后检查 `codex-agent-roles` 是否出现在反向树：core、app-server protocol、code-mode、network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS。
- `rtk cargo test -p codex-permissions-runtime -- --nocapture`：通过，覆盖迁入 owner crate 后的 permissions runtime 测试。
- `rtk cargo check -p codex-execpolicy-loader`：通过，确认 Starlark exec policy loader 不再需要 `codex-core`。
- `rtk cargo test -p codex-core exec_policy -- --nocapture`：通过，覆盖 core exec policy config reuse、test-only loader 和 integration harness 的真实 Starlark loader。
- `codex-permissions-runtime` 和 `codex-execpolicy-loader` normal graph grep 门禁：core、app-server protocol、code-mode、network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 absent。
- workspace 反查 heavy crate 后检查 `codex-permissions-runtime` / `codex-execpolicy-loader` 是否出现在反向树：core、app-server protocol、code-mode、network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS。
- `rtk cargo test -p codex-permissions-runtime network_approval -- --nocapture`：通过 16 条，覆盖迁入的 network approval pending/cache/outcome/blocked-request 状态机。
- `rtk cargo test -p codex-core --lib network_approval -- --nocapture`：通过 5 条，覆盖 core wrapper、真实 Guardian trigger 保存和 deferred `ToolError` 映射。
- `rtk cargo test -p codex-permissions-runtime -- --nocapture`：通过 27 条。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过，仅既有 warnings。
- `rtk cargo tree -p codex-permissions-runtime --depth 2 --edges normal`：direct graph 仍只通过 config-state、execpolicy-api、network-proxy-api、protocol、shell helper、globset、tokio/tokio-util 和 tracing 承载 permissions runtime，不依赖 core。
- `codex-permissions-runtime` network approval 迁移后 normal graph grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS。
- `rtk just bazel-lock-check`：通过，仅既有 rules_rs well-known crate annotation warnings。
- `rtk cargo test -p codex-sandboxing-api windows_ -- --nocapture`：通过 24 条，覆盖迁入的 Windows sandbox override 行为。
- `rtk cargo test -p codex-sandboxing-api -- --nocapture`：通过 57 条。
- `rtk cargo test -p codex-core --lib process_exec_tool_call_uses_platform_sandbox_for_network_only_restrictions -- --nocapture`：通过，覆盖 core 调用 sandboxing-api selection path。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过。
- `rtk cargo tree -p codex-sandboxing-api --depth 2 --edges normal`：direct graph 只依赖 network-proxy-api、permissions-runtime、protocol、absolute-path、utils-path 和 dunce，不依赖 core。
- `codex-sandboxing-api` Windows override 迁移后 normal graph 精确 grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、concrete network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS。
- `rtk just bazel-lock-check`：通过，仅既有 rules_rs well-known crate annotation warnings。
- `rtk git diff --check`：通过；touched Rust `unsafe` scan 只命中 `core/src/exec_tests.rs` 中既有 `libc::kill(pid, 0)` 测试代码，本切片没有新增 unsafe。
- 未作为本切片通过门禁：`rtk cargo test -p codex-core --lib exec -- --nocapture` 编译通过，但测试过滤过宽，额外命中既有 `tools::spec::*unified_exec_web_search` workflow 顺序断言失败；本切片改用更窄 core 调用路径测试覆盖。
- 未作为本切片通过门禁：`rtk cargo test -p codex-core --test all permission_request_hook_allows_network_approval_without_prompt -- --nocapture` 当前等待 hook log 超时；普通 `permission_request_hook_allows_shell_command_without_user_approval` 也等待事件超时，说明失败不局限于本次 network approval 状态迁移，后续需要单独排查 integration hook harness。
- 用户要求后曾执行 `rtk cargo clean`；本轮 Windows sandbox override 验证和 app-server build 已重新生成 `codex-rs/target` 增量缓存，后续 broad Rust 验证不再是完全冷编译。

## Next Action

继续 Step 6C：优先评估 `ExecExpiration` / `ExecCapturePolicy` / `ExecRequest` 的 owner 边界，目标是把 exec DTO 和纯 sandbox request glue 继续从 core 收缩到 `codex-command-runtime` / `codex-sandboxing-api`，但不把 exec-server、sandbox、state/sqlx、app-server protocol 等 heavy runtime 通过 indirect graph 拉回。

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
