# Agent Rust 测试协调规则

## Brief

用户希望修改 owner、reviewer 和 tester 的 agent 说明，避免多个角色在同一个 worktree 中直接并发运行 Rust/Cargo 构建或测试命令，降低共享 target 目录和 Cargo 文件锁竞争。

## 成功标准

- owner 类 agent 不直接执行 Rust/Cargo 相关测试、构建、格式化、lint 或 benchmark 命令。
- reviewer 只做 code review，不直接执行 Rust/Cargo 相关命令，也不通过 `followup_task` 触发 tester。
- tester 作为项目唯一 Rust/Cargo 执行者，只串行运行请求中收到的 `exec_command`，并回传命令结果。
- owner 在多轮 review 到无阻塞问题后，向固定 tester 发送默认轻量验证任务：修改模块的单元测试/最小 crate 测试，以及 `codex-rs` 下的 `cargo build -p codex-cli`。交接必须包含工作目录、命令顺序和完整 `exec_command` 参数。tester 不做测试设计、补充命令、风险判断、失败归因、修复建议或范围扩展。
- 根 `AGENTS.md` 与 agent 文件保持一致，不再要求 owner 或 reviewer 直接运行 `just fmt`、`cargo test`、`just fix` 等命令。
- 同一 owner 任务只创建一个 `@code-review` reviewer；修复后的复审复用首次创建的 reviewer 线程，不为每轮修复、新 diff 或新增复审创建 reviewer。

## 非目标

- 不修改 Rust 业务代码、测试代码或 Dynamic Workflow 实现。
- 不改变 owner 对测试和构建结果的验收职责；reviewer 只负责 code review。
- 不新增实际测试调度系统，只通过 agent 指令约束协作流程。
- 不把用户明确允许的纯文档简化流程扩展到代码、测试、schema、构建配置或运行时行为改动。

## 设计

本次修改采用文档规则收敛，而不是代码调度改造：

1. owner agent 在委派 review 时必须说明 reviewer 只做 code review，不执行命令也不 followup tester。
2. reviewer 负责审查代码、提出 findings 和测试缺口；owner 修复后必须复用同一 reviewer 线程多轮复审，直到 reviewer 明确无阻塞问题，不得每轮创建新 reviewer。
3. tester 是 Rust/Cargo 命令的统一执行者，只使用请求里的 `exec_command` 参数运行命令；必须使用 `rtk` 前缀、默认共享 target、`exec_command`/`command_wait` 等待机制，并串行运行会竞争锁的命令。
4. review 通过后，owner 通过 `followup_task` 把 JSON 请求发给 `/root/my_codex_pm/rust_cargo_tester`，默认只包含修改模块单元测试/最小 crate 测试和 `cargo build -p codex-cli`；等待 tester 回传命令结果，并由 owner 判断是否满足交付。
5. 根 `AGENTS.md` 同步替换原来要求直接运行 `just fmt`、`cargo test`、`just fix` 的描述，避免项目级规则和 agent 级规则冲突。
6. 用户明确说明“文档类可以直接修改”时，PM 可以跳过复杂 owner/reviewer/tester 流程，但仍需做文本级检查并说明未运行测试的原因。

## 固定 tester 请求格式

```json
{
  "type": "rust_cargo_validation_request",
  "request_id": "<稳定 id>",
  "requested_by": "<请求方 canonical path>",
  "report_to": "<结果回传目标 canonical path>",
  "worktree": "<目标 git worktree 绝对路径>",
  "branch": "<当前分支>",
  "commands": [
    {
      "id": "<命令 id>",
      "exec_command": {
        "cmd": "rtk <command>",
        "workdir": "<命令运行目录>",
        "initial_wait_ms": 30000,
        "notify_on": "exit",
        "max_output_tokens": 20000
      }
    }
  ],
  "notes": "<可选补充>"
}
```

## 风险

- 这是协作指令变更，不会从工具层强制禁止 owner/reviewer 运行命令；执行效果依赖 agent 遵守说明。
- Rust 格式化和 lint 现在经 tester 间接执行；tester 只负责命令执行和结果回传，验证是否充分由 owner 判断。
