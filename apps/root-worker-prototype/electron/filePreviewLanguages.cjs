const path = require("node:path");

function languageForFilePath(filePath) {
  const extension = path.extname(filePath).toLowerCase();
  switch (extension) {
    case ".cjs":
    case ".js":
    case ".jsx":
    case ".mjs":
      return "javascript";
    case ".ts":
    case ".tsx":
      return "typescript";
    case ".go":
      return "go";
    case ".rs":
      return "rust";
    case ".json":
      return "json";
    case ".md":
    case ".markdown":
      return "markdown";
    case ".css":
      return "css";
    case ".html":
      return "html";
    case ".yml":
    case ".yaml":
      return "yaml";
    case ".sh":
    case ".zsh":
    case ".fish":
      return "shell";
    default:
      return "plaintext";
  }
}

module.exports = {
  languageForFilePath,
};
