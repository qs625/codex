#!/usr/bin/env bash
set -euo pipefail

repo_root="${ROOT_WORKER_REPO_ROOT:-$(git rev-parse --show-toplevel)}"
skill_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app_dir="$repo_root/apps/root-worker-prototype"
debug_root="${ROOT_WORKER_DEBUG_ROOT:-/tmp/my-codex-root-worker-debug}"
debug_codex_home="${ROOT_WORKER_DEBUG_CODEX_HOME:-$debug_root/codex-home}"
debug_workspace="${ROOT_WORKER_DEBUG_WORKSPACE:-$debug_root/workspace}"
screenshot_path="${ROOT_WORKER_SCREENSHOT_PATH:-/tmp/root-worker-electron-playwright-app.png}"
codex_cmd="${CODEX_APP_SERVER_CMD:-$repo_root/codex-rs/target/debug/codex app-server --listen stdio://}"

mkdir -p "$debug_codex_home" "$debug_workspace"

port="$(
  node -e "const net=require('net');const s=net.createServer();s.listen(0,'127.0.0.1',()=>{console.log(s.address().port);s.close();});"
)"

cleanup() {
  if [ -n "${vite_pid:-}" ]; then
    kill "$vite_pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

pnpm --dir "$app_dir" exec vite \
  --host 127.0.0.1 \
  --port "$port" \
  --strictPort \
  >/tmp/root-worker-vite-playwright.log 2>&1 &
vite_pid=$!

pnpm --dir "$app_dir" exec wait-on "tcp:127.0.0.1:$port" --timeout 20000

(
  cd "$repo_root"
  APP_DIR="$app_dir" \
    ROOT_WORKER_RENDERER_MODE=dev \
    ROOT_WORKER_DEV_SERVER_URL="http://127.0.0.1:$port" \
    ROOT_WORKER_OPEN_DEVTOOLS=0 \
    CODEX_HOME="$debug_codex_home" \
    ROOT_WORKER_WORKSPACE="$debug_workspace" \
    CODEX_APP_SERVER_CMD="$codex_cmd" \
    ROOT_WORKER_SCREENSHOT_PATH="$screenshot_path" \
    node "$skill_dir/scripts/electron-smoke.cjs"
)
