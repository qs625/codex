#!/usr/bin/env bash
set -euo pipefail

repo_root="${ROOT_WORKER_REPO_ROOT:-$(git rev-parse --show-toplevel)}"
app_dir="$repo_root/apps/root-worker-prototype"
debug_root="${ROOT_WORKER_DEBUG_ROOT:-/tmp/my-codex-root-worker-debug}"
debug_morpheus_home="${ROOT_WORKER_DEBUG_MORPHEUS_HOME:-$debug_root/morpheus-home}"
debug_workspace="${ROOT_WORKER_DEBUG_WORKSPACE:-$debug_root/workspace}"
codex_cmd="${CODEX_APP_SERVER_CMD:-$repo_root/codex-rs/target/debug/app-server --listen stdio://}"

mkdir -p "$debug_morpheus_home" "$debug_workspace"

port="$(
  node -e "const net=require('net');const s=net.createServer();s.listen(0,'127.0.0.1',()=>{console.log(s.address().port);s.close();});"
)"

electron_env=(
  "ROOT_WORKER_RENDERER_MODE=dev"
  "ROOT_WORKER_OPEN_DEVTOOLS=${ROOT_WORKER_OPEN_DEVTOOLS:-0}"
)

trim_value() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

disable_cdp_value="$(trim_value "$(printf '%s' "${ROOT_WORKER_DISABLE_CDP:-}" | tr '[:upper:]' '[:lower:]')")"
cdp_port="$(trim_value "${ROOT_WORKER_REMOTE_DEBUGGING_PORT:-}")"

case "$disable_cdp_value" in
  1|true|yes|on)
    cdp_disabled=1
    ;;
  *)
    cdp_disabled=0
    ;;
esac

if [ "$cdp_disabled" = "1" ]; then
  echo "CDP disabled by ROOT_WORKER_DISABLE_CDP=$(trim_value "$ROOT_WORKER_DISABLE_CDP")"
  electron_env+=("ROOT_WORKER_DISABLE_CDP=1")
elif [ -n "$cdp_port" ]; then
  if [[ "$cdp_port" =~ ^[0-9]{1,5}$ ]] && [ "$cdp_port" -ge 1 ] && [ "$cdp_port" -le 65535 ]; then
    echo "CDP_URL=http://127.0.0.1:$cdp_port"
    electron_env+=("ROOT_WORKER_REMOTE_DEBUGGING_PORT=$cdp_port")
  else
    echo "CDP disabled by invalid ROOT_WORKER_REMOTE_DEBUGGING_PORT=$(trim_value "$ROOT_WORKER_REMOTE_DEBUGGING_PORT")"
    electron_env+=("ROOT_WORKER_REMOTE_DEBUGGING_PORT=0")
  fi
else
  cdp_port="9222"
  echo "CDP_URL=http://127.0.0.1:$cdp_port"
fi

ROOT_WORKER_DEV_SERVER_URL="http://127.0.0.1:$port" \
  MORPHEUS_HOME="$debug_morpheus_home" \
  ROOT_WORKER_WORKSPACE="$debug_workspace" \
  CODEX_APP_SERVER_CMD="$codex_cmd" \
  pnpm --dir "$app_dir" exec concurrently -k \
    "vite --host 127.0.0.1 --port $port --strictPort" \
    "wait-on tcp:127.0.0.1:$port && env ${electron_env[*]} electron ."
