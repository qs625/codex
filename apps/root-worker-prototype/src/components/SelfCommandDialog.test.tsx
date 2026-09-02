import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

(globalThis as typeof globalThis & { React: typeof React }).React = React;
const {
  SelfCommandDialog,
  isSelfCommandShortcut,
  isSelfCommandSubmitDisabled,
  normalizeSelfCommandText,
} = await import("./SelfCommandDialog");

const selfProject = {
  id: "/self" as const,
  path: "/self" as const,
  workspace: "/Users/example/.morpheus/source_workspace",
  hidden: true,
  system: true,
};

test("isSelfCommandShortcut accepts command or control P", () => {
  assert.equal(
    isSelfCommandShortcut({ ctrlKey: false, key: "p", metaKey: true }),
    true,
  );
  assert.equal(
    isSelfCommandShortcut({ ctrlKey: true, key: "P", metaKey: false }),
    true,
  );
  assert.equal(
    isSelfCommandShortcut({ ctrlKey: false, key: "p", metaKey: false }),
    false,
  );
  assert.equal(
    isSelfCommandShortcut({ ctrlKey: true, key: "f", metaKey: false }),
    false,
  );
});

test("self command submit state requires a project and non-empty text", () => {
  assert.equal(normalizeSelfCommandText("  fix packaging  "), "fix packaging");
  assert.equal(
    isSelfCommandSubmitDisabled({
      isSubmitting: false,
      project: selfProject,
      text: "  fix packaging  ",
    }),
    false,
  );
  assert.equal(
    isSelfCommandSubmitDisabled({
      isSubmitting: false,
      project: selfProject,
      text: "   ",
    }),
    true,
  );
  assert.equal(
    isSelfCommandSubmitDisabled({
      isSubmitting: false,
      project: null,
      text: "fix packaging",
    }),
    true,
  );
  assert.equal(
    isSelfCommandSubmitDisabled({
      isSubmitting: true,
      project: selfProject,
      text: "fix packaging",
    }),
    true,
  );
});

test("SelfCommandDialog renders packaged self target and input", () => {
  const markup = renderToStaticMarkup(
    <SelfCommandDialog
      error={null}
      isOpen={true}
      isSubmitting={false}
      onClose={() => {}}
      onSubmit={() => {}}
      onTextChange={() => {}}
      project={selfProject}
      text="fix packaging"
      unavailableMessage={null}
    />,
  );

  assert.match(markup, /role="dialog"/);
  assert.match(markup, /\/self/);
  assert.match(markup, /Morpheus self command/);
  assert.match(markup, /\/Users\/example\/.morpheus\/source_workspace/);
  assert.match(markup, /Start \/self/);
});

test("SelfCommandDialog renders unavailable and error states", () => {
  const markup = renderToStaticMarkup(
    <SelfCommandDialog
      error="packaged app required"
      isOpen={true}
      isSubmitting={false}
      onClose={() => {}}
      onSubmit={() => {}}
      onTextChange={() => {}}
      project={null}
      text="fix packaging"
      unavailableMessage="Self command is available after installing the packaged app."
    />,
  );

  assert.match(markup, /available after installing the packaged app/);
  assert.match(markup, /packaged app required/);
  assert.match(markup, /disabled=""/);
});

test("SelfCommandDialog renders nothing while closed", () => {
  const markup = renderToStaticMarkup(
    <SelfCommandDialog
      error={null}
      isOpen={false}
      isSubmitting={false}
      onClose={() => {}}
      onSubmit={() => {}}
      onTextChange={() => {}}
      project={selfProject}
      text="fix packaging"
      unavailableMessage={null}
    />,
  );

  assert.equal(markup, "");
});
