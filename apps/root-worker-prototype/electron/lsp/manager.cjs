const fs = require("node:fs/promises");
const fsSync = require("node:fs");
const { execFile } = require("node:child_process");
const { promisify } = require("node:util");

const { adapterForFile } = require("./adapters.cjs");
const { LspClient } = require("./client.cjs");
const { buildDesktopEnvironment } = require("../environment.cjs");
const { resolveWorkspaceRoot } = require("./workspaceRoots.cjs");

const execFileAsync = promisify(execFile);

class LspManager {
  constructor(options = {}) {
    this.clients = new Map();
    this.commandChecks = new Map();
    this.adapterForFile = options.adapterForFile ?? adapterForFile;
    this.resolveWorkspaceRoot =
      options.resolveWorkspaceRoot ?? resolveWorkspaceRoot;
    this.readFile = options.readFile ?? fs.readFile;
    this.execFileAsync = options.execFileAsync ?? execFileAsync;
    this.buildExecOptions = options.buildExecOptions ?? buildLspExecOptions;
    this.clientFactory =
      options.clientFactory ??
      (({ adapter, commandSpec, onExit, workspaceRoot }) =>
        new LspClient({
          adapter,
          commandSpec,
          onExit,
          workspaceRoot,
        }));
  }

  async describeFile(filePath) {
    const target = await this.resolveFileTarget(filePath);
    if (!target.enabled) {
      return target.toFileDescription();
    }

    const client = this.clientFor(target);
    void client.initialize().catch(() => {});
    return target.toFileDescription(client.getStatus());
  }

  async definition({ column, filePath, line }) {
    const target = await this.resolveFileTarget(filePath);
    if (!target.enabled) {
      return target.toDefinitionResponse();
    }

    const client = this.clientFor(target);
    const text = await this.readFile(filePath, "utf8");
    const locations = await client.definition({ column, filePath, line, text });

    return target.toDefinitionResponse(locations);
  }

  async status(filePath) {
    const target = await this.resolveFileTarget(filePath);
    if (!target.enabled) {
      return target.toStatusResponse();
    }

    const client = this.clientFor(target);
    void client.initialize().catch(() => {});
    return target.toStatusResponse(client.getStatus());
  }

  async resolveFileTarget(filePath) {
    const adapter = this.adapterForFile(filePath);
    if (!adapter) {
      return LspFileTarget.unavailable({
        filePath,
        languageId: null,
        lspStatus: {
          phase: "plain",
          detail: "No language server is configured for this file type.",
        },
        reason: "No LSP adapter is configured for this file type.",
        serverLabel: null,
        workspaceRoot: null,
      });
    }

    const languageId = adapter.languageIdForFile(filePath);
    const rootResolution = await this.resolveWorkspaceRoot(adapter, filePath);
    if (!rootResolution.workspaceRoot) {
      return LspFileTarget.unavailable({
        filePath,
        languageId,
        lspStatus: {
          phase: "plain",
          detail: rootResolution.reason,
        },
        reason: rootResolution.reason,
        serverLabel: adapter.serverLabel,
        workspaceRoot: null,
      });
    }

    const commandSpec = await this.findCommand(adapter);
    if (!commandSpec) {
      const reason = `${adapter.serverLabel} is not available on PATH.`;
      return LspFileTarget.unavailable({
        filePath,
        languageId,
        lspStatus: {
          phase: "unavailable",
          detail: reason,
        },
        reason,
        serverLabel: adapter.serverLabel,
        workspaceRoot: rootResolution.workspaceRoot,
      });
    }

    return LspFileTarget.available({
      adapter,
      commandSpec,
      filePath,
      languageId,
      serverLabel: adapter.serverLabel,
      workspaceRoot: rootResolution.workspaceRoot,
    });
  }

  clientFor(target) {
    const cacheKey = target.clientCacheKey();
    const existingClient = this.clients.get(cacheKey);
    if (existingClient) {
      return existingClient;
    }

    const client = this.clientFactory({
      adapter: target.adapter,
      commandSpec: target.commandSpec,
      onExit: () => {
        if (this.clients.get(cacheKey) === client) {
          this.clients.delete(cacheKey);
        }
      },
      workspaceRoot: target.workspaceRoot,
    });
    this.clients.set(cacheKey, client);
    return client;
  }

  async findCommand(adapter) {
    for (const commandSpec of adapter.commands) {
      const resolvedCommandSpec = await this.resolveCommandSpec(commandSpec);
      if (resolvedCommandSpec) {
        return resolvedCommandSpec;
      }
    }
    return null;
  }

  async resolveCommandSpec(commandSpec) {
    if (commandSpec.resolveCommand) {
      try {
        const { stdout } = await this.execFileAsync(
          commandSpec.resolveCommand.command,
          commandSpec.resolveCommand.args,
          this.buildExecOptions(),
        );
        const resolvedPath = stdout.trim();
        if (!resolvedPath) {
          return null;
        }

        return {
          ...commandSpec,
          command: resolvedPath,
        };
      } catch {
        return null;
      }
    }

    const isAvailable = await this.commandAvailable(commandSpec);
    return isAvailable ? commandSpec : null;
  }

  async commandAvailable(commandSpec) {
    if (commandSpec.availability?.type === "file") {
      const paths = commandSpec.availability.paths ?? [
        commandSpec.availability.path,
      ];
      return paths.some(
        (candidatePath) => candidatePath && fsSync.existsSync(candidatePath),
      );
    }

    if (!this.commandChecks.has(commandSpec.command)) {
      this.commandChecks.set(
        commandSpec.command,
        this.execFileAsync(
          "which",
          [commandSpec.command],
          this.buildExecOptions(),
        )
          .then(() => true)
          .catch(() => false),
      );
    }

    return this.commandChecks.get(commandSpec.command);
  }
}

class LspFileTarget {
  constructor({
    adapter = null,
    commandSpec = null,
    enabled,
    filePath,
    languageId,
    lspStatus = null,
    reason,
    serverLabel,
    workspaceRoot,
  }) {
    this.adapter = adapter;
    this.commandSpec = commandSpec;
    this.enabled = enabled;
    this.filePath = filePath;
    this.languageId = languageId;
    this.lspStatus = lspStatus;
    this.reason = reason;
    this.serverLabel = serverLabel;
    this.workspaceRoot = workspaceRoot;
  }

  static available({
    adapter,
    commandSpec,
    filePath,
    languageId,
    serverLabel,
    workspaceRoot,
  }) {
    return new LspFileTarget({
      adapter,
      commandSpec,
      enabled: true,
      filePath,
      languageId,
      reason: null,
      serverLabel,
      workspaceRoot,
    });
  }

  static unavailable({
    filePath,
    languageId,
    lspStatus,
    reason,
    serverLabel,
    workspaceRoot,
  }) {
    return new LspFileTarget({
      enabled: false,
      filePath,
      languageId,
      lspStatus,
      reason,
      serverLabel,
      workspaceRoot,
    });
  }

  clientCacheKey() {
    return `${this.adapter.id}:${this.workspaceRoot}:${this.commandSpec.command}`;
  }

  toFileDescription(lspStatus = this.lspStatus) {
    return {
      enabled: this.enabled,
      languageId: this.languageId,
      lspStatus,
      serverLabel: this.serverLabel,
      workspaceRoot: this.workspaceRoot,
      reason: this.reason,
    };
  }

  toStatusResponse(lspStatus = this.lspStatus) {
    return {
      enabled: this.enabled,
      lspStatus,
      reason: this.reason,
      workspaceRoot: this.workspaceRoot,
    };
  }

  toDefinitionResponse(locations = []) {
    return {
      enabled: this.enabled,
      locations,
      reason: this.reason,
    };
  }
}

module.exports = {
  LspManager,
  LspFileTarget,
  buildLspExecOptions,
};

function buildLspExecOptions(baseEnv = process.env, environmentOptions = {}) {
  return {
    env: buildDesktopEnvironment(baseEnv, environmentOptions),
  };
}
