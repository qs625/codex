# Agent Rust 测试协调规则

## Brief

用户希望修改 owner、reviewer 和 tester 的 agent 说明，避免多个角色在同一个 worktree 中直接并发运行 Rust/Cargo 构建或测试命令，降低共享 target 目录和 Cargo 文件锁竞争。

## 成功标准

- owner 类 agent 不直接执行 Rust/Cargo 相关测试、构建、格式化、lint 或 benchmark 命令。
- reviewer 不直接执行 Rust/Cargo 相关命令；需要验证时把最小命令清单、运行目录和风险点交给 `@test_agent`。
- tester 作为统一执行者，串行运行会竞争共享 target 或 Cargo 文件锁的 Rust/Cargo 命令，并回传命令结果、失败摘要和未覆盖范围。
- 根 `AGENTS.md` 与 agent 文件保持一致，不再要求 owner 或 reviewer 直接运行 `just fmt`、`cargo test`、`just fix` 等命令。
- 同一 owner 任务内，修复后的复审复用首次创建的 reviewer 线程，不为每轮修复新建 reviewer。

## 非目标

- 不修改 Rust 业务代码、测试代码或 Dynamic Workflow 实现。
- 不改变非 Rust/Cargo 验证职责；reviewer 仍可执行委派范围内必要且非破坏性的非 Rust/Cargo 验证。
- 不新增实际测试调度系统，只通过 agent 指令约束协作流程。

## 设计

本次修改采用文档规则收敛，而不是代码调度改造：

1. owner agent 在委派 review 时必须列出 Rust/Cargo 验证需求，但不能自己执行这些命令。
2. reviewer 负责审查、确定最小验证集，并在涉及 Rust/Cargo 时委派 `@test_agent`，然后在 review 结论中引用 tester 的结果。
3. tester 是 Rust/Cargo 命令的统一执行者，必须使用 `rtk` 前缀、默认共享 target、`exec_command`/`command_wait` 等待机制，并串行运行会竞争锁的命令。
4. owner 修复 reviewer findings 后通过 followup 请求同一 reviewer 复审；只有 reviewer 线程不可用或用户明确要求更换时才创建新 reviewer。
5. 根 `AGENTS.md` 同步替换原来要求直接运行 `just fmt`、`cargo test`、`just fix` 的描述，避免项目级规则和 agent 级规则冲突。

## 风险

- 这是协作指令变更，不会从工具层强制禁止 owner/reviewer 运行命令；执行效果依赖 agent 遵守说明。
- Rust 格式化和 lint 现在经 tester 间接执行，交付链路会多一次委派，但能减少共享 target/Cargo lock 竞争。
