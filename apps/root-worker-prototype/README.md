# Root Worker Prototype

Electron product shell for a root-agent plus worker-agent workspace.

## Run

```bash
pnpm install
pnpm --filter @my-codex/root-worker-prototype dev
```

This starts:

- Vite on `http://localhost:5173` for the Electron renderer
- Electron, which starts `../../codex-rs/target/debug/codex app-server --listen stdio://` by default when that local build exists, otherwise it falls back to `codex app-server --listen stdio://`
- the prototype defaults `CODEX_HOME` to `~/.codex-home`

You can override the app-server command or Codex home with:

```bash
ROOT_WORKER_CODEX_HOME=/tmp/root-worker-codex-home \
CODEX_APP_SERVER_CMD="codex app-server --listen stdio://" \
pnpm --filter @my-codex/root-worker-prototype dev
```

## Build

```bash
pnpm --filter @my-codex/root-worker-prototype build
```

## Electron

```bash
pnpm --filter @my-codex/root-worker-prototype start
```
