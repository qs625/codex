#!/usr/bin/env bash
set -euo pipefail

repo_root="${ROOT_WORKER_REPO_ROOT:-$(git rev-parse --show-toplevel)}"
app_dir="$repo_root/apps/root-worker-prototype"
debug_root="${ROOT_WORKER_DEBUG_ROOT:-/tmp/my-codex-root-worker-debug}"
debug_codex_home="${ROOT_WORKER_DEBUG_CODEX_HOME:-$debug_root/codex-home}"
debug_workspace="${ROOT_WORKER_DEBUG_WORKSPACE:-$debug_root/workspace}"
codex_cmd="${CODEX_APP_SERVER_CMD:-$repo_root/codex-rs/target/debug/codex-app-server --listen stdio://}"

mkdir -p "$debug_codex_home" "$debug_workspace"

port="$(
  node -e "const net=require('net');const s=net.createServer();s.listen(0,'127.0.0.1',()=>{console.log(s.address().port);s.close();});"
)"

ROOT_WORKER_DEV_SERVER_URL="http://127.0.0.1:$port" \
  CODEX_HOME="$debug_codex_home" \
  ROOT_WORKER_WORKSPACE="$debug_workspace" \
  CODEX_APP_SERVER_CMD="$codex_cmd" \
  pnpm --dir "$app_dir" exec concurrently -k \
    "vite --host 127.0.0.1 --port $port --strictPort" \
    "wait-on tcp:127.0.0.1:$port && ROOT_WORKER_RENDERER_MODE=dev ROOT_WORKER_OPEN_DEVTOOLS=${ROOT_WORKER_OPEN_DEVTOOLS:-0} electron ."
