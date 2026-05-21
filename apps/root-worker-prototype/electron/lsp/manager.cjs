const fs = require("node:fs/promises");
const fsSync = require("node:fs");
const { execFile } = require("node:child_process");
const { promisify } = require("node:util");

const { adapterForFile } = require("./adapters.cjs");
const { LspClient } = require("./client.cjs");
const { resolveWorkspaceRoot } = require("./workspaceRoots.cjs");

const execFileAsync = promisify(execFile);

class LspManager {
  constructor() {
    this.clients = new Map();
    this.commandChecks = new Map();
  }

  async describeFile(filePath) {
    const adapter = adapterForFile(filePath);
    if (!adapter) {
      return {
        enabled: false,
        languageId: null,
        serverLabel: null,
        workspaceRoot: null,
        reason: "No LSP adapter is configured for this file type.",
      };
    }

    const rootResolution = await resolveWorkspaceRoot(adapter, filePath);
    if (!rootResolution.workspaceRoot) {
      return {
        enabled: false,
        languageId: adapter.languageIdForFile(filePath),
        serverLabel: adapter.serverLabel,
        workspaceRoot: null,
        reason: rootResolution.reason,
      };
    }

    const commandSpec = await this.findCommand(adapter);
    if (!commandSpec) {
      return {
        enabled: false,
        languageId: adapter.languageIdForFile(filePath),
        serverLabel: adapter.serverLabel,
        workspaceRoot: rootResolution.workspaceRoot,
        reason: `${adapter.serverLabel} is not available on PATH.`,
      };
    }

    return {
      enabled: true,
      languageId: adapter.languageIdForFile(filePath),
      serverLabel: adapter.serverLabel,
      workspaceRoot: rootResolution.workspaceRoot,
      reason: null,
    };
  }

  async definition({ column, filePath, line }) {
    const fileDescription = await this.describeFile(filePath);
    if (!fileDescription.enabled || !fileDescription.workspaceRoot) {
      return {
        enabled: false,
        locations: [],
        reason: fileDescription.reason,
      };
    }

    const adapter = adapterForFile(filePath);
    const commandSpec = await this.findCommand(adapter);
    if (!commandSpec) {
      return {
        enabled: false,
        locations: [],
        reason: `${adapter.serverLabel} is not available on PATH.`,
      };
    }

    const client = this.clientFor({
      adapter,
      commandSpec,
      workspaceRoot: fileDescription.workspaceRoot,
    });
    const text = await fs.readFile(filePath, "utf8");
    const locations = await client.definition({ column, filePath, line, text });

    return {
      enabled: true,
      locations,
      reason: null,
    };
  }

  clientFor({ adapter, commandSpec, workspaceRoot }) {
    const cacheKey = `${adapter.id}:${workspaceRoot}:${commandSpec.command}`;
    const existingClient = this.clients.get(cacheKey);
    if (existingClient) {
      return existingClient;
    }

    const client = new LspClient({
      adapter,
      commandSpec,
      onExit: () => {
        if (this.clients.get(cacheKey) === client) {
          this.clients.delete(cacheKey);
        }
      },
      workspaceRoot,
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
        const { stdout } = await execFileAsync(
          commandSpec.resolveCommand.command,
          commandSpec.resolveCommand.args,
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
      return fsSync.existsSync(commandSpec.availability.path);
    }

    if (!this.commandChecks.has(commandSpec.command)) {
      this.commandChecks.set(
        commandSpec.command,
        execFileAsync("which", [commandSpec.command])
          .then(() => true)
          .catch(() => false),
      );
    }

    return this.commandChecks.get(commandSpec.command);
  }
}

module.exports = {
  LspManager,
};
