const { spawn } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { createInterface } = require("node:readline");
const { EventEmitter } = require("node:events");

const DEFAULT_APP_SERVER_COMMAND = `${resolveWorkspaceAppServerBinary()} --listen stdio://`;
const DEFAULT_CODEX_HOME = resolvePrototypeCodexHome();

class AppServerClient extends EventEmitter {
  constructor() {
    super();
    this.child = null;
    this.pending = new Map();
    this.nextRequestId = 1;
    this.readyPromise = new Promise((resolve, reject) => {
      this.readyResolve = resolve;
      this.readyReject = reject;
    });
    this.start();
  }

  async ready() {
    return this.readyPromise;
  }

  async request(method, params) {
    if (method !== "initialize") {
      await this.ready();
    }
    return this.sendRequest(method, params);
  }

  async notify(method, params) {
    if (method !== "initialized") {
      await this.ready();
    }
    this.write({ method, params });
  }

  get status() {
    return {
      connected: this.child?.exitCode == null,
      pid: this.child?.pid ?? null,
    };
  }

  start() {
    const command = process.env.APP_SERVER_CMD ?? DEFAULT_APP_SERVER_COMMAND;
    const codexHome = process.env.CODEX_HOME ?? DEFAULT_CODEX_HOME;
    fs.mkdirSync(codexHome, { recursive: true });
    this.child = spawn(command, {
      cwd: process.cwd(),
      env: {
        ...process.env,
        CODEX_HOME: codexHome,
      },
      shell: true,
      stdio: "pipe",
    });

    this.child.stderr.on("data", (chunk) => {
      process.stderr.write(chunk);
    });

    this.child.on("exit", (code, signal) => {
      const reason = `app-server exited (${code ?? "null"} / ${signal ?? "null"})`;
      const error = new Error(reason);
      for (const pending of this.pending.values()) {
        pending.reject(error);
      }
      this.pending.clear();
      this.emit("status", { connected: false, reason });
    });

    const stdout = createInterface({ input: this.child.stdout });
    stdout.on("line", (line) => {
      if (!line.trim()) {
        return;
      }
      try {
        this.handleMessage(JSON.parse(line));
      } catch (error) {
        console.error("Failed to parse app-server message", line, error);
      }
    });

    void this.initialize();
  }

  async initialize() {
    try {
      const initializeResult = await this.request("initialize", {
        clientInfo: {
          name: "root_worker_prototype_electron",
          title: "Root Worker Prototype",
          version: "0.1.0",
        },
        capabilities: {
          experimentalApi: true,
        },
      });
      await this.notify("initialized", {});
      this.readyResolve();
      this.emit("status", {
        connected: true,
        initializeResult,
        pid: this.child?.pid ?? null,
      });
    } catch (error) {
      this.readyReject(
        error instanceof Error ? error : new Error(String(error)),
      );
    }
  }

  write(message) {
    if (!this.child) {
      throw new Error("app-server process is not running");
    }
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  sendRequest(method, params) {
    const id = this.nextRequestId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.write({ id, method, params });
    });
  }

  handleMessage(message) {
    if (!message || typeof message !== "object") {
      return;
    }

    if (typeof message.id === "number" && typeof message.method === "string") {
      this.write({
        id: message.id,
        error: {
          code: -32601,
          message: `Unsupported server request from app-server: ${message.method}`,
        },
      });
      return;
    }

    if (
      typeof message.id === "number" &&
      Object.prototype.hasOwnProperty.call(message, "result")
    ) {
      const pending = this.pending.get(message.id);
      if (!pending) {
        return;
      }
      this.pending.delete(message.id);
      pending.resolve(message.result);
      return;
    }

    if (
      typeof message.id === "number" &&
      Object.prototype.hasOwnProperty.call(message, "error")
    ) {
      const pending = this.pending.get(message.id);
      if (!pending) {
        return;
      }
      this.pending.delete(message.id);
      const errorPayload = message.error ?? {};
      pending.reject(
        new Error(
          `app-server request failed (${errorPayload.code ?? "unknown"}): ${errorPayload.message ?? "unknown error"}`,
        ),
      );
      return;
    }

    if (typeof message.method === "string") {
      this.emit("notification", {
        method: message.method,
        params: message.params,
      });
    }
  }
}

module.exports = {
  AppServerClient,
};

function resolveWorkspaceAppServerBinary() {
  for (
    let current = __dirname;
    current !== path.dirname(current);
    current = path.dirname(current)
  ) {
    const workspaceBinary = path.join(
      current,
      "codex-rs/target/debug/app-server",
    );
    if (fs.existsSync(workspaceBinary)) {
      return shellQuote(workspaceBinary);
    }
  }
  return "app-server";
}

function resolvePrototypeCodexHome() {
  return path.join(os.homedir(), ".codex-home");
}

function shellQuote(value) {
  return `"${value.replaceAll('"', '\\"')}"`;
}
