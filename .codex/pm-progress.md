# PM Progress

## Current Goal

推进 `codex-core` 大块拆分，让 core 从“功能聚合根”收缩为过渡 facade。拆分按 domain / owner crate 建立依赖拓扑；低层 crate 只依赖稳定 trait/DTO contract，由 core、app-server、CLI/TUI 等 composition root 注入 concrete implementation。

## Active Work

- id: `core-crate-topology-refactor-plan`
- mode: `performance_refactor_exclusive`
- workdir: `/Users/bytedance/Projects/my-codex`
- owner: root PM direct implementation
- status: active
- current_step: `6C`
- focus: 收缩 session/thread/turn runtime 边界，先把 app-server / core 的 concrete `Codex`、`CodexThread`、`ThreadManager` 调用面切到 `codex-session-api` / `codex-thread-api` trait，再整块迁出 `core/src/session` 和 `core/src/thread`。

## State

- Step 1-5 已完成：policy/proxy API、app-server shared types、config 轻量层、tools/code-mode API、app-server 旁路 core 依赖清理。
- Step 6A 已完成：`core::config` production runtime 迁到 `codex-config`，core 保留兼容 facade。
- Step 6B 已完成：command primitive 迁到 `codex-command-runtime` / `codex-process-exec`。
- Step 6C 进行中：已形成 tool、command、agent、MCP、permission、sandbox、rollout、context、turn item、turn metadata、workflow、session runtime、session API、thread API、model client 等 owner crate；当前重点是 session/thread/tool host 的 API trait 收敛。

当前 core 主要残留：`core/src/session` 约 21.6K 行，`core/src/thread` 约 4.1K 行，`core/src/tools` 约 3.2K 行，`codex-core` 总体仍约 70K 行。

## Guardrails

- Dependency inversion：trait/DTO contract 放在 owner API crate 或 protocol-neutral crate；implementation crate 不得被 facade/re-export 间接拉回。
- owner crate 的 `normal` 和 `normal,dev` 依赖都不得拉回 `codex-core`、`codex-app-server`、`codex-code-mode`、`codex-exec-server`、`codex-state`、`codex-core-skills`、`sqlx`。
- 测试代码也不能通过 dev-dependency indirect 拉回 heavy runtime。
- 不使用 Rust `unsafe`；不为了拆分手写替代成熟第三方库。
- 不做几百行 helper 的机械拆分；优先按 domain 聚合成合理 owner crate。
- `Session + Turn + pending input + turn state + task loop` 属于同一 session/turn runtime 边界，不为短期降行数拆散。

## Latest Completed

- 新增 `codex-session-api` 和 `codex-thread-api`；`Codex`、`CodexThread`、`ThreadManager` 已实现对应 live session/thread trait。
- app-server 中只需要投递 `Op`、查询 thread status、shutdown/unload live thread 的路径已开始改用 `SessionCommandHandle` / `LiveThreadHandle` / `LiveThreadRegistry`。
- `LiveThreadRegistry` 已覆盖 live loaded/status/shutdown/wait 查询；thread unsubscribe、connection closed、startup restore、thread/resume 防重入和 turns-list running 查询已开始使用 registry API。
- `SessionCommandHandle` / `LiveThreadRegistry` 已补充 model-visible conversation item append API；file subscription runtime 的 event-driven tool 和 event-command 注入路径已从 `get_thread -> CodexThread::append_message` 切到 registry trait。
- `MessageProcessor` state-db 注入已补齐 cfg 兼容；`AGENTS.md` 已记录 session/thread live API 归属规则。

## Next Steps

1. 继续把 app-server request processors、MCP refresh、thread lifecycle/status 操作切到 `codex-session-api` / `codex-thread-api`，但不要把 `Config` 或 service registry 塞进轻量 API crate。
2. 收敛 `AgentControl`、workflow bridge、tool host 对 concrete `ThreadManager` / `CodexThread` / `Codex` 的调用面。
3. 设计必要的 `SessionFactory` / `ThreadManagerHandle` / live event sink / scheduler DTO。
4. 调用面稳定后，整块迁出 `core/src/session`、`core/src/thread`、`pending_input`、`state::ActiveTurn` / `RunningTask` 等 session/turn runtime implementation。
5. 每轮迁移后运行窄测试、app-server binary build、owner crate `normal` 和 `normal,dev` dependency gate。

## Verification

最近已通过：`codex-session-api` / `codex-thread-api` fmt、check、test 和 dependency gate；`codex-core --lib` check；`codex-app-server --lib` check；app-server `mcp_refresh`、`thread_lifecycle`、`thread_processor`、`extensions` 窄测试；`codex-app-server` binary build；`git diff --check`；新增 Rust `unsafe` 扫描无新增命中。
