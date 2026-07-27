import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  AppErrorFallback,
  formatErrorBoundaryMessage,
} from "./AppErrorBoundary";

test("app error fallback renders a recoverable renderer crash message", () => {
  const markup = renderToStaticMarkup(
    <AppErrorFallback error={new Error("render exploded")} />,
  );

  assert.match(markup, /role="alert"/);
  assert.match(markup, /Root Worker needs a refresh/);
  assert.match(markup, /render exploded/);
  assert.match(markup, /Reload/);
});

test("formatErrorBoundaryMessage falls back when the error has no message", () => {
  assert.equal(
    formatErrorBoundaryMessage(new Error("")),
    "The renderer hit an unexpected error.",
  );
});
