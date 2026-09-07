const test = require("node:test");
const assert = require("node:assert/strict");

const {
  nextBrowserTabIdAfterClose,
  shouldDetachAttachedBrowserPanelView,
} = require("./browserPanelTabs.cjs");

const tabs = [{ id: "tab-a" }, { id: "tab-b" }, { id: "tab-c" }];

test("nextBrowserTabIdAfterClose keeps active tab when closing background tab", () => {
  assert.equal(nextBrowserTabIdAfterClose(tabs, "tab-a", "tab-b"), "tab-a");
});

test("nextBrowserTabIdAfterClose selects the next neighbor for active middle tab", () => {
  assert.equal(nextBrowserTabIdAfterClose(tabs, "tab-b", "tab-b"), "tab-c");
});

test("nextBrowserTabIdAfterClose selects previous neighbor for active last tab", () => {
  assert.equal(nextBrowserTabIdAfterClose(tabs, "tab-c", "tab-c"), "tab-b");
});

test("nextBrowserTabIdAfterClose returns null when the last tab closes", () => {
  assert.equal(nextBrowserTabIdAfterClose([{ id: "tab-a" }], "tab-a", "tab-a"), null);
});

test("shouldDetachAttachedBrowserPanelView skips destroyed windows and tabs", () => {
  assert.equal(
    shouldDetachAttachedBrowserPanelView({
      attachedTabId: "tab-a",
      tabDestroyed: false,
      windowDestroyed: false,
    }),
    true,
  );
  assert.equal(
    shouldDetachAttachedBrowserPanelView({
      attachedTabId: "tab-a",
      tabDestroyed: false,
      windowDestroyed: true,
    }),
    false,
  );
  assert.equal(
    shouldDetachAttachedBrowserPanelView({
      attachedTabId: "tab-a",
      tabDestroyed: true,
      windowDestroyed: false,
    }),
    false,
  );
  assert.equal(
    shouldDetachAttachedBrowserPanelView({
      attachedTabId: null,
      tabDestroyed: false,
      windowDestroyed: false,
    }),
    false,
  );
});
