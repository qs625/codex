# Root Worker Prototype

Electron product shell for a root-agent plus worker-agent workspace.

## Run

```bash
pnpm install
pnpm --filter @my-codex/root-worker-prototype dev
```

This starts:

- Vite on `http://localhost:5173` for the Electron renderer
- Electron, which starts `../../codex-rs/target/debug/codex-app-server --listen stdio://` by default when that local build exists, otherwise it falls back to `codex-app-server --listen stdio://` from `PATH`
- the prototype defaults `CODEX_HOME` to `~/.codex-home`

Use this only when you specifically want the Vite dev server flow.

You can override the app-server command, Codex home, or workspace with:

```bash
CODEX_HOME=/tmp/my-codex-root-worker-ui-env/codex-home \
ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-ui-env/workspace \
CODEX_APP_SERVER_CMD="codex-app-server --listen stdio://" \
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

`start` now loads the built renderer from `dist/index.html` even when running from the source tree. If the build output is missing, Electron exits with an error telling you to run the build first.

For manual prototype iteration without Vite hot reload, use:

```bash
pnpm --filter @my-codex/root-worker-prototype build
pnpm --filter @my-codex/root-worker-prototype start
```
