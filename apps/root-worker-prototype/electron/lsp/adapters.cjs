const path = require("node:path");

const APP_ROOT = path.resolve(__dirname, "..", "..");

const ADAPTERS = [
  {
    id: "typescript",
    serverLabel: "TypeScript Language Server",
    commands: [
      {
        command: "pnpm",
        args: ["exec", "typescript-language-server", "--stdio"],
        cwd: APP_ROOT,
        availability: {
          type: "file",
          path: path.join(APP_ROOT, "node_modules", ".bin", "typescript-language-server"),
        },
      },
    ],
    extensions: new Set([".ts", ".tsx", ".mts", ".cts"]),
    languageIdForFile(filePath) {
      return filePath.endsWith(".tsx") ? "typescriptreact" : "typescript";
    },
    resolveWorkspaceRoot(filePath, findClosestMarker) {
      return (
        findClosestMarker(filePath, ["tsconfig.json", "jsconfig.json"]) ??
        findClosestMarker(filePath, ["package.json"])
      );
    },
  },
  {
    id: "javascript",
    serverLabel: "TypeScript Language Server",
    commands: [
      {
        command: "pnpm",
        args: ["exec", "typescript-language-server", "--stdio"],
        cwd: APP_ROOT,
        availability: {
          type: "file",
          path: path.join(APP_ROOT, "node_modules", ".bin", "typescript-language-server"),
        },
      },
    ],
    extensions: new Set([".js", ".jsx", ".mjs", ".cjs"]),
    languageIdForFile(filePath) {
      return filePath.endsWith(".jsx") ? "javascriptreact" : "javascript";
    },
    resolveWorkspaceRoot(filePath, findClosestMarker) {
      return (
        findClosestMarker(filePath, ["tsconfig.json", "jsconfig.json"]) ??
        findClosestMarker(filePath, ["package.json"])
      );
    },
  },
  {
    id: "rust",
    serverLabel: "rust-analyzer",
    commands: [
      {
        command: "rust-analyzer",
        args: [],
        resolveCommand: {
          command: "rustup",
          args: ["which", "rust-analyzer"],
        },
      },
    ],
    extensions: new Set([".rs"]),
    languageIdForFile() {
      return "rust";
    },
    async resolveWorkspaceRoot(filePath, findClosestMarker, { findCargoWorkspaceRoot }) {
      const crateRoot = await findClosestMarker(filePath, ["Cargo.toml"]);
      if (!crateRoot) {
        return null;
      }

      return (await findCargoWorkspaceRoot(crateRoot)) ?? crateRoot;
    },
  },
  {
    id: "python",
    serverLabel: "Pyright",
    commands: [
      { command: "basedpyright-langserver", args: ["--stdio"] },
      { command: "pyright-langserver", args: ["--stdio"] },
    ],
    extensions: new Set([".py"]),
    languageIdForFile() {
      return "python";
    },
    resolveWorkspaceRoot(filePath, findClosestMarker) {
      return (
        findClosestMarker(filePath, ["pyproject.toml", "setup.py"]) ??
        findClosestMarker(filePath, ["requirements.txt"])
      );
    },
  },
  {
    id: "go",
    serverLabel: "gopls",
    commands: [{ command: "gopls", args: [] }],
    extensions: new Set([".go"]),
    languageIdForFile() {
      return "go";
    },
    resolveWorkspaceRoot(filePath, findClosestMarker) {
      return findClosestMarker(filePath, ["go.mod"]);
    },
  },
];

function adapterForFile(filePath) {
  const extension = path.extname(filePath).toLowerCase();
  return ADAPTERS.find((adapter) => adapter.extensions.has(extension)) ?? null;
}

module.exports = {
  adapterForFile,
};
