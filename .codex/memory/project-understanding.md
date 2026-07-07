# Project Understanding

## Stable Working Rules
- 所有 shell 命令必须以 `rtk` 开头。
- 普通开发应先在对应 `dev` checkout 提交，再 merge 回主分支。
- 不要把 `dev` checkout 的改动文件手工复制、覆盖或 apply 回主仓库代替 merge。
- 当前项目的 PM / owner / reviewer 协作规则以 `.codex/agents/project-pm.agent.md` 及对应 owner agent 定义为准。

## Stable Architecture Rules
- app-server / root-worker conversation display 走 typed `EventMsg -> ThreadItem` 路径。
- `ResponseItem` 主要用于模型交互、模型可见 history/context，不应用作 display-only 展示源。
- 不要从 raw marker、assistant JSON envelope 或 legacy 解析路径反解客户端展示项。
- 等待模型的长期方向已确定为统一收口到 `poll_event`：`wait_agent` / `command_wait` 应删除，不再作为独立 tool surface 保留。
- `poll_event` 需要支持在同一个 turn 内等待并在 event 到达后继续执行；runtime 不应维护会影响状态机的硬性等待目标，event 直接作为 pending input 注入当前 turn，并携带来源信息供模型判断。
- command output / exit、child completion、inter-agent completion 都应复用同一套 pending-input 唤醒链路；不要再为某一类等待事件单独维护平行 wait API。
- parent-side child completion bookkeeping 只用于 completion envelope 的投递、去重和清理，不应定义 child 当前是否 active；`WaitChild` / `IdleWaitChild` 只应由 direct child thread 的本地 active 状态驱动。
- thread init context 中的 workflow discovery 依赖 `TurnContext::discovery_context()`，其 project workflows 来自 `config.config_layer_stack` 中各 project layer 的 `.codex/workflows`。
- session 初始化阶段的 `instruction_files` 应按本地 config 路径读取，不应依赖 primary execution environment 是否存在。

## Key Module Map
- `codex-rs/thread-service/`
  - 负责线程运行时、compact、session、agent control 等核心后端逻辑。
  - `src/session/codex_runtime.rs` 负责 session spawn/init；这里会装载 user instructions。
  - `src/agents_md.rs` 负责 `instruction_files` 与 user instructions 的拼装。
  - `src/session/turn_context.rs` 负责 workflow discovery context 的 project/home roots。
  - `src/tools/` 与 turn wait runtime 相关模块是统一等待语义的主要收口位置；后续等待行为应围绕 `poll_event` 组织，而不是新增平行 wait tool。
- `codex-rs/config/`
  - 负责运行时配置加载，包括 compact prompt 的读取优先级。
- `codex-rs/app-server/`
  - `src/request_processors/thread_processor/ops.rs` 负责 `thread/start` 的 config 派生、thread 创建与初始响应。
  - `tests/suite/thread_start.rs` 是 thread/start init context 与 instruction sources 的端到端回归测试位置。
- `codex-rs/workflow/`
  - workflow runtime bridge 的 agent wait 语义也应统一走 `poll_event`，不要继续依赖独立 `wait_agent` API。
- `apps/root-worker-prototype/`
  - 负责 root-worker prototype 客户端、thread 展示、compact UI 与 renderer 状态。

## Compact Understanding
- compact prompt 支持 workspace 级 `.codex/compact/COMPACT.md` 与 `CODEX_HOME/compact/COMPACT.md`。
- 如果没有自定义 compact prompt，运行时仍会回退到内置 compact prompt。
- root-worker prototype 当前对 compact history 采用按需加载，而不是默认常驻保存。

## Validation Defaults
- 默认只做最小必要验证，不默认运行全量 `cargo test`、广域 `just fix`、snapshot、schema 或 lockfile workflow。
- 涉及 app-server、runtime、protocol 或 root-worker 后端启动路径时，默认在 `codex-rs/` 下运行 `cargo build -p app-server --bin app-server`。
- 只有确实改到 CLI/TUI 或 CLI app-server 包装时，才增加 `cargo build -p codex-cli`。

## Rejected Paths
- 不要把 display 修复建立在 raw marker、assistant JSON envelope 或 legacy 解析路径上。
- 不要把 `dev` checkout 的改动文件手工复制回主仓库代替 merge。
