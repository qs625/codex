const test = require("node:test");
const assert = require("node:assert/strict");

const {
  buildDesktopEnvironment,
  buildDesktopPath,
} = require("./environment.cjs");

test("macOS desktop path keeps existing entries first and appends common paths", () => {
  const desktopPath = buildDesktopPath("/custom/bin:/usr/bin:/custom/bin", {
    home: "/Users/alice",
    platform: "darwin",
    shellPath: "/shell/bin:/opt/homebrew/bin:/usr/bin",
  });
  const entries = desktopPath.split(":");

  assert.deepEqual(entries.slice(0, 4), [
    "/custom/bin",
    "/usr/bin",
    "/shell/bin",
    "/opt/homebrew/bin",
  ]);
  assert.equal(entries.filter((entry) => entry === "/custom/bin").length, 1);
  assert.ok(entries.includes("/usr/local/bin"));
  assert.ok(entries.includes("/Users/alice/.local/bin"));
  assert.ok(entries.includes("/Users/alice/.cargo/bin"));
});

test("macOS desktop path falls back to common paths when shell path is unavailable", () => {
  const desktopPath = buildDesktopPath("/usr/bin:/bin", {
    home: "/Users/alice",
    platform: "darwin",
    shellPath: "",
  });
  const entries = desktopPath.split(":");

  assert.deepEqual(entries.slice(0, 2), ["/usr/bin", "/bin"]);
  assert.ok(entries.includes("/opt/homebrew/bin"));
  assert.ok(entries.includes("/usr/local/bin"));
});

test("macOS desktop path falls back when shell probe fails", () => {
  const desktopPath = buildDesktopPath("/usr/bin", {
    platform: "darwin",
    readShellPath() {
      throw new Error("probe failed");
    },
  });

  assert.ok(desktopPath.split(":").includes("/opt/homebrew/bin"));
});

test("macOS desktop path tolerates missing HOME", () => {
  const desktopPath = buildDesktopPath("/usr/bin", {
    platform: "darwin",
    shellPath: "",
  });

  assert.ok(desktopPath.split(":").includes("/opt/homebrew/bin"));
});

test("non-macOS desktop path does not probe shell path", () => {
  const desktopPath = buildDesktopPath("/custom/bin:/usr/bin", {
    platform: "linux",
    readShellPath() {
      throw new Error("shell probe should not run");
    },
  });

  assert.equal(desktopPath, "/custom/bin:/usr/bin");
});

test("desktop environment preserves non-PATH variables", () => {
  const env = buildDesktopEnvironment(
    {
      HOME: "/Users/alice",
      PATH: "/usr/bin",
      TOKEN: "keep-me",
    },
    {
      platform: "darwin",
      shellPath: "/shell/bin",
    },
  );

  assert.equal(env.TOKEN, "keep-me");
  assert.ok(env.PATH.startsWith("/usr/bin:/shell/bin:"));
  assert.ok(env.PATH.includes("/opt/homebrew/bin"));
});
