const path = require("node:path");

const FILE_PREVIEW_PROTOCOL = "morpheus-file-preview";

function pdfMimeForExtension(extension) {
  return extension.toLowerCase() === ".pdf" ? "application/pdf" : null;
}

function createPdfPreviewUrl(token, name) {
  return `${FILE_PREVIEW_PROTOCOL}://pdf/${encodeURIComponent(token)}/${encodeURIComponent(name)}`;
}

function buildPdfPreview(filePath, byteSize, token) {
  const mimeType = pdfMimeForExtension(path.extname(filePath));
  if (!mimeType) {
    return null;
  }

  return {
    path: filePath,
    mimeType,
    name: path.basename(filePath),
    byteSize,
    url: createPdfPreviewUrl(token, path.basename(filePath)),
  };
}

module.exports = {
  buildPdfPreview,
  createPdfPreviewUrl,
  FILE_PREVIEW_PROTOCOL,
  pdfMimeForExtension,
};
