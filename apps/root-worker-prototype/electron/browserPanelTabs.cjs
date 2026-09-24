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

function shouldAttachBrowserPanelView({
  boundsVisible,
  tabMissing,
  tabDestroyed,
  panelVisible,
  windowDestroyed,
}) {
  return (
    !tabMissing &&
    panelVisible &&
    boundsVisible &&
    !windowDestroyed &&
    !tabDestroyed
  );
}

function isBrowserPanelTabAlreadyAttached({ attachedTabId, tabId }) {
  return Boolean(attachedTabId && tabId && attachedTabId === tabId);
}

async function closeBrowserPanelTabLifecycle(
  panel,
  tabId,
  { detachAttachedView, disposeTab, createTab, attachActiveView },
) {
  const index = panel.tabs.findIndex((tab) => tab.id === tabId);
  if (index === -1) {
    return false;
  }
  const tab = panel.tabs[index];
  const wasActive = panel.activeTabId === tab.id;
  const nextActiveTabId = nextBrowserTabIdAfterClose(
    panel.tabs,
    panel.activeTabId,
    tab.id,
  );
  if (wasActive) {
    detachAttachedView(panel, tab);
  }
  panel.tabs.splice(index, 1);
  if (wasActive) {
    panel.activeTabId = nextActiveTabId;
  }
  await disposeTab(panel, tab);
  if (panel.tabs.length === 0 && !panel.destroying) {
    createTab(panel, { activate: true });
    return true;
  }
  if (wasActive) {
    if (panel.visible) {
      attachActiveView(panel, { raise: true });
    }
  }
  return true;
}

module.exports = {
  closeBrowserPanelTabLifecycle,
  isBrowserPanelTabAlreadyAttached,
  nextBrowserTabIdAfterClose,
  shouldAttachBrowserPanelView,
  shouldDetachAttachedBrowserPanelView,
};
