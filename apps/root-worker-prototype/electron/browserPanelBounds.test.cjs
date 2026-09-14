const test = require("node:test");
const assert = require("node:assert/strict");

const { normalizeBrowserBoundsUpdate } = require("./browserPanelBounds.cjs");

test("normalizeBrowserBoundsUpdate rounds and clamps bounds", () => {
  assert.deepEqual(
    normalizeBrowserBoundsUpdate({
      x: 10.2,
      y: -5,
      width: 652.6,
      height: 782.5,
      sequence: 4,
    }),
    {
      apply: true,
      bounds: {
        x: 10,
        y: 0,
        width: 653,
        height: 783,
      },
      sequence: 4,
    },
  );
});

test("normalizeBrowserBoundsUpdate ignores stale sequenced bounds", () => {
  assert.deepEqual(
    normalizeBrowserBoundsUpdate(
      {
        x: 821,
        y: 169,
        width: 768,
        height: 900,
        sequence: 7,
      },
      8,
    ),
    {
      apply: false,
      bounds: null,
      sequence: 8,
    },
  );
});

test("normalizeBrowserBoundsUpdate accepts newer and legacy unsequenced bounds", () => {
  assert.equal(normalizeBrowserBoundsUpdate({ sequence: 9 }, 8).apply, true);
  assert.deepEqual(
    normalizeBrowserBoundsUpdate(
      {
        x: 12,
        y: 34,
        width: 653,
        height: 783,
      },
      8,
    ),
    {
      apply: true,
      bounds: {
        x: 12,
        y: 34,
        width: 653,
        height: 783,
      },
      sequence: 8,
    },
  );
});
