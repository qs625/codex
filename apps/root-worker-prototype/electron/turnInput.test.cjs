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
      {
        name: "example.png",
        mimeType: "image/png",
        bytes: Uint8Array.from([0, 0, 0]).buffer,
      },
      { name: "ignored.png", mimeType: "", bytes: null },
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
      url: "data:image/png;base64,AAAA",
    },
  ]);
});
