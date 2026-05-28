---
name: bootstrap-worktree-deps
description: Configure a git worktree to reuse the primary checkout's build artifacts and dependencies. Use when Codex is asked to make worktree development share Rust compilation output or JavaScript dependencies instead of rebuilding per worktree, especially in this repo where `codex-rs/target`, the repo root `node_modules`, and `apps/root-worker-prototype/node_modules` should point at the primary checkout.
---

# Bootstrap Worktree Deps

## Overview

Wire the current git worktree to the primary checkout so repeated Rust and prototype builds reuse existing artifacts.

Run the bundled script instead of hand-writing `ln -s` commands. It derives the primary checkout from `git rev-parse --git-common-dir`, so it works from either the main checkout or a linked worktree.

## Workflow

1. Confirm the request is about sharing worktree artifacts, not changing Cargo or pnpm semantics globally.
2. Run the bootstrap script from the repo root or the target worktree:

```bash
python3 .codex/skills/bootstrap-worktree-deps/scripts/bootstrap_worktree_deps.py
```

3. If you want to target a different checkout than the current cwd, pass `--repo <path>`.
4. If an existing worktree has real directories where symlinks should go, rerun with `--force`.
5. Report which paths were linked and which were already correct.

## Managed paths

- `codex-rs/target`
- `node_modules`
- `apps/root-worker-prototype/node_modules`

The script creates `codex-rs/target` in the primary checkout if it does not exist yet. For `node_modules`, the primary checkout must already have the dependency directory from `pnpm install`.

## Safety rules

- Do not rewrite `Cargo.toml` to solve shared-target requests. Cargo target configuration lives in `.cargo/config.toml` or `CARGO_TARGET_DIR`, and that still does not solve shared `node_modules`.
- Do not delete a populated non-symlink worktree directory unless the user asked for replacement or you are explicitly running with `--force`.
- Prefer `--dry-run` first if the worktree state looks unusual.

## Useful commands

Preview changes:

```bash
python3 .codex/skills/bootstrap-worktree-deps/scripts/bootstrap_worktree_deps.py --dry-run
```

Replace conflicting directories in a worktree:

```bash
python3 .codex/skills/bootstrap-worktree-deps/scripts/bootstrap_worktree_deps.py --force
```

Target another checkout explicitly:

```bash
python3 .codex/skills/bootstrap-worktree-deps/scripts/bootstrap_worktree_deps.py --repo /Users/bytedance/Projects/my-codex/.worktrees/prototype-tool-call-opt
```

## Output expectations

State:
- the primary checkout path
- the target repo/worktree path
- each managed path and whether it was linked, already correct, created in the primary checkout, or blocked
