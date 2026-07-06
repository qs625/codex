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

## Key Module Map
- `codex-rs/thread-service/`
  - 负责线程运行时、compact、session、agent control 等核心后端逻辑。
- `codex-rs/config/`
  - 负责运行时配置加载，包括 compact prompt 的读取优先级。
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
