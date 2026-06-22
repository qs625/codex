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
- current_focus: Step 6C 继续推进 service/domain boundary：command session controller、exec timeout/capture policy DTO 已迁到 `codex-command-runtime`；本地进程 output capture/aggregation、process result interpretation 和 process exec request DTO 已迁到 `codex-process-exec`；MCP runtime、Apps file/skill dependency runtime 已迁到 `codex-mcp-runtime`；agent role catalog/spec、goal hidden-context prompt policy、goal runtime state、direct child completion tracker 和 post-turn selector 已迁到 `codex-agent-runtime` / `codex-agent-roles`；exec policy manager/loader trait/update runtime 和 network approval 纯状态机已迁到 `codex-permissions-runtime`；Windows sandbox filesystem override 解析已迁到 `codex-sandboxing-api`；unified-exec 专属 exec-server env policy 已从通用 `sandboxing::ExecRequest` 剥离。core 继续保留 session/tool host adapter、Guardian/hook adapter、goal persistence/event/turn policy、exec spawn/PTY/event emission 和 sandbox execution glue。

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
- Step 6C 第九个切片把 `ExecExpiration`、`ExecExpirationOutcome`、`ExecCapturePolicy`、默认 exec timeout、IO drain timeout 和 output delta 上限迁到 `codex-command-runtime::exec_control`；core `exec.rs` 短期 re-export 兼容旧测试/外部 crate，app-server streaming/process exec 路径已直接依赖 `codex-command-runtime`，不再通过 `codex-core::exec` 获取这些基础 DTO。
- Step 6C 第十个切片把 unified-exec 专属 `ExecServerEnvConfig` 从通用 `core::sandboxing::ExecRequest` 中剥离，移动到 `core::unified_exec::process_manager` 边界；`open_session_with_exec_env` 显式接收可选 exec-server env policy，普通 shell/user-shell 和 sandbox execution path 不再被迫携带 `None` 或 destructure 占位字段。这样 `ExecRequest` 更接近纯 sandbox execution DTO，后续可继续评估迁到 `codex-sandboxing-api` 或拆出更稳定 runtime API。
- Step 6C 第十一个切片新增 `codex-process-exec` owner crate，迁出本地 `tokio::process::Child` output capture、timeout/cancellation kill、stdout/stderr retained-byte cap、aggregation、byte decoding helper 和 output chunk DTO；core `exec.rs` 只保留 spawn/sandbox/Windows sandbox/CodexErr 映射以及 `StdoutStream -> EventMsg::ExecCommandOutputDelta` forwarder，并在返回 exec result 前等待 forwarder drain，避免 live output delta 晚于最终结果。原 `read_output_*` / `aggregate_output_*` 单测迁到 `codex-process-exec`，core 仍保留 process_exec_tool_call 和 sandbox 入口测试。
- Step 6C 第十二个切片把 process result interpretation 迁到 `codex-process-exec`：`finalize_captured_process_output` 现在负责 byte output decoding、timeout/signal mapping、sandbox denied heuristic 和 `ExecToolCallOutput` / `CodexErr::Sandbox` 构造；core、unified exec、apply_patch 和 shell escalation callsite 直接依赖 `codex_process_exec::is_likely_sandbox_denied`，不再通过 `core::exec` facade。原 sandbox detection 单测迁到 `codex-process-exec`，并保留 raw exec error tracing。
- Step 6C 第十三个切片把 goal continuation / budget limit / objective-updated 的 hidden `<goal_context>` prompt policy 和模板迁到 `codex-agent-runtime::goal_context`；core `goals.rs` 只负责读取 goal state、调度 continuation turn、注入 pending input 和发 typed events。core 旧 `context::GoalContext` wrapper 已删除，相关 prompt 单测迁到 `codex-agent-runtime`，core 仅保留 display/parser 边界和 session 行为测试。
- Step 6C 第十四个切片把 process exec public request DTO `ExecParams` 迁到 `codex-process-exec`，`codex-core::exec::ExecParams` 仅作为兼容 re-export；app-server command-exec request processor 已直接依赖 `codex_process_exec::ExecParams` 和 protocol-owned `SandboxPermissions`，不再为了 DTO 构造走 core exec/sandboxing facade。真实 sandbox transform、env 注入、spawn 和 EventMsg emission 仍留 core，避免触碰 sandbox env var 约束或把执行编排迁入 lightweight owner crate。
- Step 6C 第十五个切片把 goal continuation/accounting 的 mutable runtime state 迁到 `codex-agent-runtime::GoalRuntimeState`：state DB cache、budget-limit reported goal id、accounting semaphore/snapshot 和 continuation turn reservation 由 agent runtime owner 持有；core `goals.rs` 继续保留 goal persistence 查询、metrics accounting policy、typed event emission、pending input 注入和 turn lifecycle 编排，避免把 Session/TurnContext 或 concrete state runtime 迁入 lightweight owner crate。
- Step 6C 第十六个切片把 direct child completion tracker 和 post-turn selector 迁到 `codex-agent-runtime`：parent-visible completion 的 active gate、pending direct child completion counts、typed completion consumed 后才减少 pending 的时序，以及 pending input -> active goal -> wait child -> wait command -> complete 的 selector 顺序由 agent runtime owner 表达；core `Session` 只收集 active turn/mailbox/goal/direct child/command 事实并执行 parent delivery。
- `UnifiedExecProcess` / `process_manager` 剩余逻辑仍绑定 exec-server protocol、PTY、sandbox denial detection、core error type、Session/TurnContext、ToolEmitter、ToolOrchestrator 和 network approval；继续迁移前需要 Step 6C 的 trait/service 边界，避免只做小 helper 或把 heavy runtime 间接拉回。
- Step 6 前基线：`codex-rs/core/src` 约 293 个 Rust 文件、134123 行；`codex-app-server` 冷编译 timing 中 `codex-core` 单 unit 约 197.7s。
- 当前 `codex-rs/core/src` 约 270 个 Rust 文件、108036 行；`core/src/exec.rs` 已从 1517 行降到约 619 行，`core/src/goals.rs` 约 1329 行，`core/src/session/mod.rs` 已降到约 4014 行，Windows sandbox override 单测迁到 `codex-sandboxing-api`，exec timeout/capture DTO 单测迁到 `codex-command-runtime`，本地 process output capture、sandbox-denial/result interpretation 和 process exec request DTO 迁到 `codex-process-exec`，goal hidden-context prompt 单测、goal runtime state、direct child completion tracker 和 post-turn selector 迁到 `codex-agent-runtime`，unified-exec exec-server env policy 不再污染通用 `ExecRequest`；`core/src/unified_exec` 剩余最大文件为 `process_manager.rs`、`process.rs`、`async_watcher.rs`。

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
- `rtk cargo test -p codex-command-runtime -- --nocapture`：通过 34 条，覆盖迁入后的 exec control DTO、command output runtime 和 wait backoff。
- `rtk cargo test -p codex-core --lib exec_full_buffer_capture_ignores_expiration -- --nocapture`：通过，覆盖 core exec full-buffer 行为继续消费迁出后的 capture/expiration DTO。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过，仅既有 warnings。
- `rtk cargo tree -p codex-command-runtime --depth 2 --edges normal`：direct graph 仍仅为 decoding、rand、tokio/tokio-util。
- `codex-command-runtime` exec control 迁移后 normal graph 精确 grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、concrete network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS。
- `rtk git diff --check`：通过；新增 Rust diff 中没有新增 unsafe。
- `rtk cargo check -p codex-core --lib`：通过，覆盖第十切片 `ExecRequest` 字段剥离后的 core lib 编译。
- `rtk cargo test -p codex-core --lib unified_exec::process_manager::tests::exec_server_params_use_env_policy_overlay_contract -- --nocapture`：通过，覆盖 exec-server env policy overlay 仍由 unified-exec 显式参数传入。
- `codex-command-runtime` / `codex-sandboxing-api` 第十切片后 normal graph 精确 grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、concrete network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS。
- `rtk rg -n "unsafe" <touched Rust files>`：无命中，本切片没有新增 unsafe。
- `rtk cargo check -p codex-process-exec`：通过。
- `rtk cargo test -p codex-process-exec -- --nocapture`：通过 7 条，覆盖迁出的 read/aggregate output 行为。
- `rtk cargo check -p codex-core --lib`：通过，覆盖 core 接入 `codex-process-exec` 后的 lib 编译。
- `rtk proxy cargo test -p codex-core --lib exec_full_buffer_capture -- --nocapture`：通过 2 条，覆盖 core spawn 到 process-exec capture 的 full-buffer 桥接。
- `rtk cargo test -p codex-core --lib process_exec_tool_call_preserves_full_buffer_capture_policy -- --nocapture`：通过，覆盖 sandbox transform 入口到 process-exec capture 的路径。
- `codex-process-exec` normal graph 精确 grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、concrete network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS；normal graph 只通过 `codex-command-runtime`、`codex-utils-pty` 和 tokio 承载 process execution primitive。
- `rtk just bazel-lock-check`：通过，仅既有 rules_rs well-known crate annotation warnings。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过。
- `rtk git diff --check`：通过；touched Rust unsafe scan 只命中 `core/src/exec_tests.rs` 既有 `libc::kill(pid, 0)`。
- `rtk cargo test -p codex-process-exec -- --nocapture`：通过 15 条，覆盖迁出的 read/aggregate output 和 sandbox-denial heuristic。
- `rtk cargo check -p codex-core --lib`：通过，覆盖 core/unified-exec/apply_patch/shell escalation 对 `codex-process-exec` result interpretation 的接线。
- `rtk cargo test -p codex-core --lib exec_full_buffer_capture -- --nocapture`：通过 2 条，覆盖 core spawn 到 process-exec finalization 的 full-buffer 桥接。
- `rtk cargo test -p codex-core --lib process_exec_tool_call_preserves_full_buffer_capture_policy -- --nocapture`：通过，覆盖 sandbox transform 入口到 process-exec finalization 的路径。
- `codex-process-exec` result interpretation 迁移后 normal graph 精确 grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、concrete network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS；normal graph 增加 `codex-protocol` / `codex-sandboxing-api` / `tracing`，仍不依赖 heavy runtime。
- `rtk just bazel-lock-check`：通过，仅既有 rules_rs well-known crate annotation warnings。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过。
- 未作为本切片通过门禁：`rtk cargo test -p codex-core --test all permission_request_hook_allows_network_approval_without_prompt -- --nocapture` 当前等待 hook log 超时；普通 `permission_request_hook_allows_shell_command_without_user_approval` 也等待事件超时，说明失败不局限于本次 network approval 状态迁移，后续需要单独排查 integration hook harness。
- 用户要求后曾执行 `rtk cargo clean`；本轮 Windows sandbox override 验证和 app-server build 已重新生成 `codex-rs/target` 增量缓存，后续 broad Rust 验证不再是完全冷编译。
- `rtk cargo test -p codex-agent-runtime -- --nocapture`：通过 23 条，覆盖迁入后的 goal continuation / budget-limit / objective-updated prompt rendering、XML delimiter escaping 和 hidden `<goal_context>` ResponseInputItem 构造。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings，覆盖 core 调用 `codex-agent-runtime` goal context helper 后的 lib 编译。
- `rtk cargo test -p codex-core --lib goal_context_does_not_parse_as_visible_turn_item -- --nocapture`：通过，覆盖 `<goal_context>` 仍不被解析成可见 turn item。
- `rtk cargo test -p codex-core --lib active_goal_continuation_runs_again_after_no_tool_turn -- --nocapture`：通过，覆盖 goal continuation 仍能注入 hidden context。
- `rtk cargo test -p codex-core --lib external_objective_change_steers_active_turn -- --nocapture`：通过，覆盖 objective-updated steering item 仍进入 pending input。
- `codex-agent-runtime` goal context 迁移后 normal graph 精确 grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、concrete network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS；normal graph 只新增 `codex-utils-template` 这类轻量模板 helper。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过，覆盖 core/agent-runtime 接线后的 app-server 入口编译。
- `rtk just bazel-lock-check`：通过，仅既有 rules_rs well-known crate annotation warnings。
- `rtk git diff --check`：通过；现存 touched Rust 文件 `rtk rg -n "unsafe"` 无命中，本切片没有新增 unsafe。
- `rtk cargo test -p codex-process-exec -- --nocapture`：通过 15 条，覆盖 `ExecParams` owner crate 仍能编译并保留既有 process capture/result 测试。
- `rtk cargo check -p codex-process-exec`：通过，覆盖新增 DTO 文档/import 整理后的 owner crate 编译。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings，覆盖 core 对 `codex_process_exec::ExecParams` 的 re-export 和内部 exec 接线。
- `rtk cargo test -p codex-core --lib shell_command_handler_to_exec_params_uses_session_shell_and_turn_context -- --nocapture`：通过，覆盖 shell handler 构造迁出后的 `ExecParams`。
- `rtk cargo test -p codex-core --lib exec_full_buffer_capture_ignores_expiration -- --nocapture`：通过，覆盖 public DTO 到 core exec 执行路径。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过，覆盖 app-server 直接使用 `codex_process_exec::ExecParams` 后的入口编译。
- `codex-process-exec` `ExecParams` 迁移后 normal graph 精确 grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、concrete network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS；normal graph 只新增 `codex-network-proxy-api` 和 `codex-utils-absolute-path` 轻量依赖。
- `rtk just bazel-lock-check`：通过，仅既有 rules_rs well-known crate annotation warnings。
- `rtk git diff --check`：通过；现存 touched Rust 文件 `rtk rg -n "unsafe"` 无命中，本切片没有新增 unsafe。
- `rtk cargo test -p codex-agent-runtime -- --nocapture`：通过 23 条，覆盖 agent runtime 现有 goal prompt/status/post-turn 逻辑在新增 `GoalRuntimeState` 后仍可编译运行。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings，覆盖 core 改用 `codex_agent_runtime::GoalRuntimeState` 后的 lib 编译。
- `rtk cargo test -p codex-core --lib active_goal_continuation_runs_again_after_no_tool_turn -- --nocapture`：通过，覆盖 continuation turn reservation 迁出后仍能再次注入 goal context。
- `rtk cargo test -p codex-core --lib external_objective_change_steers_active_turn -- --nocapture`：通过，覆盖 objective update steering 路径。
- `rtk cargo test -p codex-core --lib budget_limited_accounting_steers_active_turn_without_aborting -- --nocapture`：通过，覆盖 accounting snapshot/lock 迁出后的 budget-limit steering。
- `codex-agent-runtime` `GoalRuntimeState` 迁移后 normal graph 精确 grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、concrete network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS；新增依赖只经过 `codex-state-api` API 边界，不依赖 concrete state runtime。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过，覆盖 app-server 组合根接入新的 agent runtime dependency graph。
- `rtk just bazel-lock-check`：通过，仅既有 rules_rs well-known crate annotation warnings。
- `rtk git diff --check`：通过；touched Rust 文件 `rtk rg -n "unsafe"` 无命中，本切片没有新增 unsafe。
- `rtk cargo test -p codex-agent-runtime -- --nocapture`：通过 28 条，新增覆盖 post-turn selector 优先级，确认 `ChildCompletionState` 编译通过。
- `rtk cargo check -p codex-core --lib`：通过，仅既有 warnings，覆盖 core `Session` 改用 `ChildCompletionState` 和 post-turn selector 接线。
- `rtk cargo test -p codex-core --lib turn_start_consumes_child_completion_before_parent_visible_complete -- --nocapture`：通过，覆盖 parent 消费 typed child completion 后才 parent-visible complete 的时序。
- `rtk cargo test -p codex-core --lib goal_post_turn_state_continues_despite_live_direct_child -- --nocapture`：通过，覆盖 active goal continuation 优先于 live direct child。
- `rtk cargo test -p codex-core --lib post_turn_state_waits_for_live_direct_child_without_active_goal -- --nocapture`：通过，覆盖无 active goal 时进入 WaitChild。
- `rtk cargo test -p codex-core --lib post_turn_state_waits_for_active_event_subscription_without_active_goal -- --nocapture`：通过，覆盖无 active goal/child 时进入 WaitCommand。
- `rtk cargo test -p codex-core --lib multi_agent_v2_completion_waits_for_pending_mailbox_input -- --nocapture`：通过，覆盖 child completion delivery 等待 pending mailbox input。
- `codex-agent-runtime` child completion/post-turn selector 迁移后 normal graph 精确 grep 门禁和 `cargo tree --workspace --edges normal --invert <heavy>` 反向门禁：core、app-server protocol、code-mode、concrete network-proxy、exec-server、state/sqlx、codex-api、concrete `codex-openai-files`、concrete `codex-core-skills` 均 PASS；本切片未新增 Cargo dependency。
- `rtk cargo build -p codex-app-server --bin codex-app-server`：通过，仅既有 warnings。
- `rtk git diff --check`：通过；touched Rust 文件 `rtk rg -n "unsafe"` 无命中，本切片没有新增 unsafe。

## Next Action

继续 Step 6C：在 process capture、process result interpretation、process exec request DTO、goal prompt policy、goal runtime state、direct child completion tracker/post-turn selector 和 unified-exec env policy 已拆出/收口后，继续评估 `ExecRequest` / Windows sandbox execution glue、unified-exec process manager service 边界、tool orchestrator host trait 或 workflow/thread-store runtime 边界；真实 sandbox selection、EventMsg emission 和 Session/TurnContext 编排暂留 core，避免把 heavy runtime 通过 indirect graph 拉回。

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
