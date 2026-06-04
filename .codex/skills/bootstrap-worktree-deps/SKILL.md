---
name: bootstrap-worktree-deps
description: 配置 git worktree 复用主 checkout 的构建产物和依赖。适用于 Codex 需要让 worktree 开发共享 Rust 编译输出或 JavaScript 依赖、避免每个 worktree 重新构建的场景；在本仓库中特别用于让 `codex-rs/target`、仓库根目录 `node_modules` 和 `apps/root-worker-prototype/node_modules` 指向主 checkout。
---

# 初始化 Worktree 依赖

## 概览

把当前 git worktree 连接到主 checkout，让重复的 Rust 构建和 prototype 构建复用已有产物。

使用内置脚本，不要手写 `ln -s` 命令。脚本会通过 `git rev-parse --git-common-dir` 推导主 checkout，因此从主 checkout 或任意 linked worktree 运行都可以。

## 流程

1. 确认请求目标是共享 worktree 构建产物，而不是全局修改 Cargo 或 pnpm 语义。
2. 从仓库根目录或目标 worktree 运行初始化脚本：

```bash
python3 .codex/skills/bootstrap-worktree-deps/scripts/bootstrap_worktree_deps.py
```

3. 如果要操作的 checkout 不是当前 cwd，传入 `--repo <path>`。
4. 如果现有 worktree 中符号链接目标位置已经是真实目录，使用 `--force` 重新运行。
5. 汇报哪些路径已建立链接，哪些路径本来就正确。

## 管理路径

- `codex-rs/target`
- `node_modules`
- `apps/root-worker-prototype/node_modules`

如果主 checkout 中还没有 `codex-rs/target`，脚本会创建它。对于 `node_modules`，主 checkout 必须已经通过 `pnpm install` 准备好依赖目录。

## 安全规则

- 不要通过改写 `Cargo.toml` 来解决共享 target 的请求。Cargo target 配置属于 `.cargo/config.toml` 或 `CARGO_TARGET_DIR`，而且这仍然不能解决共享 `node_modules`。
- 除非用户要求替换，或你明确使用 `--force` 运行，否则不要删除 worktree 中已有内容的非符号链接目录。
- 如果 worktree 状态看起来异常，优先先运行 `--dry-run`。

## 常用命令

预览改动：

```bash
python3 .codex/skills/bootstrap-worktree-deps/scripts/bootstrap_worktree_deps.py --dry-run
```

替换 worktree 中冲突的目录：

```bash
python3 .codex/skills/bootstrap-worktree-deps/scripts/bootstrap_worktree_deps.py --force
```

显式指定另一个 checkout：

```bash
python3 .codex/skills/bootstrap-worktree-deps/scripts/bootstrap_worktree_deps.py --repo /Users/bytedance/Projects/my-codex/.worktrees/prototype-tool-call-opt
```

## 输出要求

说明：

- 主 checkout 路径
- 目标 repo/worktree 路径
- 每个管理路径的状态：已建立链接、本来正确、已在主 checkout 创建，或被阻塞
