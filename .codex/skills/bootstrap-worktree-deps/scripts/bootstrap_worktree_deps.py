#!/usr/bin/env python3
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


MANAGED_PATHS = (
    ("codex-rs/target", True),
    ("node_modules", False),
    ("apps/root-worker-prototype/node_modules", False),
)


@dataclass
class LinkResult:
    relative_path: str
    status: str
    detail: str


def run_git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=repo,
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


def resolve_repo_root(repo: Path) -> Path:
    return Path(run_git(repo, "rev-parse", "--show-toplevel")).resolve()


def resolve_primary_root(repo_root: Path) -> Path:
    common_dir = Path(run_git(repo_root, "rev-parse", "--git-common-dir"))
    if not common_dir.is_absolute():
        common_dir = (repo_root / common_dir).resolve()
    return common_dir.parent.resolve()


def ensure_primary_path(path: Path, is_creatable: bool, dry_run: bool) -> str | None:
    if path.exists():
        return None
    if not is_creatable:
        return f"缺少共享源目录 {path}；请先在主 checkout 中准备它"
    if dry_run:
        return f"将创建共享源目录 {path}"
    path.mkdir(parents=True, exist_ok=True)
    return f"已创建共享源目录 {path}"


def replace_path(path: Path, dry_run: bool) -> None:
    if dry_run:
        return
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    shutil.rmtree(path)


def link_path(
    repo_root: Path,
    primary_root: Path,
    relative_path: str,
    allow_create_primary: bool,
    force: bool,
    dry_run: bool,
) -> LinkResult:
    current_path = repo_root / relative_path
    shared_path = primary_root / relative_path

    primary_status = ensure_primary_path(shared_path, allow_create_primary, dry_run)
    if primary_status and primary_status.startswith("missing shared source"):
        return LinkResult(relative_path, "blocked", primary_status)

    if repo_root == primary_root:
        detail = primary_status or "当前已经是主 checkout"
        return LinkResult(relative_path, "primary", detail)

    if current_path.is_symlink():
        target = current_path.resolve()
        if target == shared_path.resolve():
            detail = primary_status or f"已经链接到 {shared_path}"
            return LinkResult(relative_path, "ok", detail)
        if not force:
            return LinkResult(
                relative_path,
                "blocked",
                f"{current_path} 指向 {target}；使用 --force 重新运行以替换它",
            )
        replace_path(current_path, dry_run)
    elif current_path.exists():
        if not force:
            return LinkResult(
                relative_path,
                "blocked",
                f"{current_path} 已存在；使用 --force 重新运行以替换它",
            )
        replace_path(current_path, dry_run)

    detail = primary_status or ""
    if dry_run:
        detail = f"{detail}；将链接到 {shared_path}".strip("；")
        return LinkResult(relative_path, "dry-run", detail)

    current_path.parent.mkdir(parents=True, exist_ok=True)
    current_path.symlink_to(shared_path, target_is_directory=True)
    detail = f"{detail}；已链接到 {shared_path}".strip("；")
    return LinkResult(relative_path, "linked", detail)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="将 git worktree 链接到主 checkout 共享的 target 和 node_modules。"
    )
    parser.add_argument(
        "--repo",
        type=Path,
        default=Path.cwd(),
        help="目标 checkout 的 repo 根目录或其中任意路径。默认使用当前工作目录。",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="替换目标 checkout 中冲突的已有路径。",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="只显示计划改动，不修改文件系统。",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo_root = resolve_repo_root(args.repo.resolve())
    primary_root = resolve_primary_root(repo_root)

    print(f"主 checkout: {primary_root}")
    print(f"目标 checkout: {repo_root}")

    has_blocker = False
    for relative_path, allow_create_primary in MANAGED_PATHS:
        result = link_path(
            repo_root=repo_root,
            primary_root=primary_root,
            relative_path=relative_path,
            allow_create_primary=allow_create_primary,
            force=args.force,
            dry_run=args.dry_run,
        )
        print(f"{result.relative_path}: {result.status} - {result.detail}")
        if result.status == "blocked":
            has_blocker = True

    return 1 if has_blocker else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as error:
        sys.stderr.write(error.stderr or str(error))
        raise SystemExit(error.returncode)
