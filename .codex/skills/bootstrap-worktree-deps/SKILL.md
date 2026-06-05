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
2. 为开发任务创建 worktree 时，目录必须放在当前 cwd 的 `.worktrees/` 下，分支名称必须以 `agent/` 开头：

```bash
rtk git worktree add -b agent/<task-name> .worktrees/<task-name> HEAD
```

3. 从仓库根目录或目标 worktree 运行初始化脚本：

```bash
python3 .codex/skills/bootstrap-worktree-deps/scripts/bootstrap_worktree_deps.py
```

4. 如果要操作的 checkout 不是当前 cwd，传入 `--repo <path>`。
5. 如果现有 worktree 中符号链接目标位置已经是真实目录，使用 `--force` 重新运行。
6. 汇报哪些路径已建立链接，哪些路径本来就正确。

## 管理路径

- `codex-rs/target`
- `node_modules`
- `apps/root-worker-prototype/node_modules`

如果主 checkout 中还没有 `codex-rs/target`，脚本会创建它。对于 `node_modules`，主 checkout 必须已经通过 `pnpm install` 准备好依赖目录。

## 安全规则

- 不要通过改写 `Cargo.toml` 来解决共享 target 的请求。Cargo target 配置属于 `.cargo/config.toml` 或 `CARGO_TARGET_DIR`，而且这仍然不能解决共享 `node_modules`。
- 除非用户要求替换，或你明确使用 `--force` 运行，否则不要删除 worktree 中已有内容的非符号链接目录。
- 如果 worktree 状态看起来异常，优先先运行 `--dry-run`。

## 合并与清理规则

开发任务完成后，必须按 git worktree 生命周期收口，不能用 diff patch 把改动搬回主 checkout。

1. 在开发 worktree 内确认改动范围并提交：

```bash
rtk git status --short
rtk git add <paths>
rtk git commit -m "<message>"
```

2. 回到主 checkout，通过 `git merge` 合并开发分支：

```bash
rtk git merge <branch>
```

3. 合并完成并确认主 checkout 状态正确后，删除对应 worktree 目录和分支：

```bash
rtk git worktree remove <worktree-path>
rtk git branch -d <branch>
```

如果合并出现冲突，在主 checkout 解决冲突并完成 merge commit；不要改用 patch diff 迁移改动。

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
