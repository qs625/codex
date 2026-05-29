const test = require("node:test");
const assert = require("node:assert/strict");

const { buildTurnInput } = require("./turnInput.cjs");

test("buildTurnInput uses app-server image url field", () => {
  const input = buildTurnInput({
    text: "  describe this  ",
    skills: [
      { name: "alpha", path: "/tmp/alpha" },
      { name: "", path: "/tmp/ignored" },
    ],
    images: [
      { name: "example.png", dataUrl: "data:image/png;base64,AAA" },
      { name: "ignored.png", dataUrl: "" },
    ],
  });

  assert.deepEqual(input, [
    {
      type: "skill",
      name: "alpha",
      path: "/tmp/alpha",
    },
    {
      type: "text",
      text: "describe this",
      text_elements: [],
    },
    {
      type: "image",
      url: "data:image/png;base64,AAA",
    },
  ]);
});
