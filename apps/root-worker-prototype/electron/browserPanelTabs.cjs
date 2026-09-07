function nextBrowserTabIdAfterClose(tabs, activeTabId, closedTabId) {
  const index = tabs.findIndex((tab) => tab.id === closedTabId);
  if (index === -1) {
    return activeTabId;
  }
  if (activeTabId !== closedTabId) {
    return activeTabId;
  }
  const remainingTabs = tabs.filter((tab) => tab.id !== closedTabId);
  if (remainingTabs.length === 0) {
    return null;
  }
  return remainingTabs[Math.min(index, remainingTabs.length - 1)].id;
}

function shouldDetachAttachedBrowserPanelView({
  attachedTabId,
  tabDestroyed,
  windowDestroyed,
}) {
  return Boolean(attachedTabId) && !windowDestroyed && !tabDestroyed;
}

module.exports = {
  nextBrowserTabIdAfterClose,
  shouldDetachAttachedBrowserPanelView,
};
