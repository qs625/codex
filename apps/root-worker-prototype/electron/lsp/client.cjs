const path = require("node:path");
const { spawn } = require("node:child_process");
const { fileURLToPath, pathToFileURL } = require("node:url");

class LspClient {
  constructor({ adapter, commandSpec, workspaceRoot }) {
    this.adapter = adapter;
    this.workspaceRoot = workspaceRoot;
    this.commandSpec = commandSpec;
    this.nextRequestId = 1;
    this.pendingRequests = new Map();
    this.readBuffer = Buffer.alloc(0);
    this.openDocuments = new Set();
    this.isInitialized = false;
    this.initializingPromise = null;
    this.process = spawn(commandSpec.command, commandSpec.args, {
      cwd: workspaceRoot,
      stdio: ["pipe", "pipe", "pipe"],
    });

    this.process.stdout.on("data", (chunk) => {
      this.handleStdout(chunk);
    });
    this.process.stderr.on("data", () => {});
    this.process.on("exit", () => {
      const error = new Error(`${adapter.serverLabel} exited unexpectedly.`);
      for (const pending of this.pendingRequests.values()) {
        pending.reject(error);
      }
      this.pendingRequests.clear();
      this.isInitialized = false;
      this.initializingPromise = null;
    });
  }

  async definition({ column, filePath, line, text }) {
    await this.initialize();
    await this.openDocument(filePath, text);

    const result = await this.request("textDocument/definition", {
      textDocument: { uri: pathToFileURL(filePath).href },
      position: {
        line: Math.max(0, line - 1),
        character: Math.max(0, column - 1),
      },
    });

    const locations = normalizeLocations(result);
    return locations.map((location) => ({
      path: fileURLToPath(location.uri),
      line: location.range.start.line + 1,
      column: location.range.start.character + 1,
    }));
  }

  async initialize() {
    if (this.isInitialized) {
      return;
    }

    if (!this.initializingPromise) {
      this.initializingPromise = (async () => {
        await this.request("initialize", {
          processId: process.pid,
          clientInfo: {
            name: "root-worker-prototype",
            version: "0.0.0",
          },
          rootUri: pathToFileURL(this.workspaceRoot).href,
          workspaceFolders: [
            {
              uri: pathToFileURL(this.workspaceRoot).href,
              name: path.basename(this.workspaceRoot),
            },
          ],
          capabilities: {
            textDocument: {
              definition: {
                dynamicRegistration: false,
                linkSupport: true,
              },
            },
            workspace: {
              workspaceFolders: true,
            },
          },
          initializationOptions: {},
        });
        this.notify("initialized", {});
        this.isInitialized = true;
      })();
    }

    return this.initializingPromise;
  }

  async openDocument(filePath, text) {
    const uri = pathToFileURL(filePath).href;
    if (this.openDocuments.has(uri)) {
      return;
    }

    this.notify("textDocument/didOpen", {
      textDocument: {
        uri,
        languageId: this.adapter.languageIdForFile(filePath),
        version: 1,
        text,
      },
    });
    this.openDocuments.add(uri);
  }

  request(method, params) {
    const id = this.nextRequestId++;
    const payload = {
      jsonrpc: "2.0",
      id,
      method,
      params,
    };

    return new Promise((resolve, reject) => {
      this.pendingRequests.set(id, { resolve, reject });
      this.writeMessage(payload);
    });
  }

  notify(method, params) {
    this.writeMessage({
      jsonrpc: "2.0",
      method,
      params,
    });
  }

  writeMessage(payload) {
    const body = Buffer.from(JSON.stringify(payload), "utf8");
    const header = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "utf8");
    this.process.stdin.write(Buffer.concat([header, body]));
  }

  handleStdout(chunk) {
    this.readBuffer = Buffer.concat([this.readBuffer, chunk]);

    while (true) {
      const headerEnd = this.readBuffer.indexOf("\r\n\r\n");
      if (headerEnd === -1) {
        return;
      }

      const headerText = this.readBuffer.subarray(0, headerEnd).toString("utf8");
      const contentLengthMatch = headerText.match(/Content-Length:\s*(\d+)/i);
      if (!contentLengthMatch) {
        this.readBuffer = Buffer.alloc(0);
        return;
      }

      const contentLength = Number.parseInt(contentLengthMatch[1], 10);
      const messageEnd = headerEnd + 4 + contentLength;
      if (this.readBuffer.length < messageEnd) {
        return;
      }

      const body = this.readBuffer.subarray(headerEnd + 4, messageEnd).toString("utf8");
      this.readBuffer = this.readBuffer.subarray(messageEnd);
      this.handleMessage(JSON.parse(body));
    }
  }

  handleMessage(message) {
    if (message.id == null) {
      return;
    }

    const pending = this.pendingRequests.get(message.id);
    if (!pending) {
      return;
    }

    this.pendingRequests.delete(message.id);
    if (message.error) {
      pending.reject(new Error(message.error.message ?? "Unknown LSP error"));
      return;
    }

    pending.resolve(message.result);
  }
}

function normalizeLocations(result) {
  if (!result) {
    return [];
  }

  if (Array.isArray(result)) {
    return result.map((location) => ({
      uri: location.targetUri ?? location.uri,
      range: location.targetSelectionRange ?? location.range,
    }));
  }

  return [
    {
      uri: result.targetUri ?? result.uri,
      range: result.targetSelectionRange ?? result.range,
    },
  ];
}

module.exports = {
  LspClient,
};
