const MAX_NOTIFICATION_TITLE_LENGTH = 80;
const MAX_NOTIFICATION_BODY_LENGTH = 160;

function showSystemNotification(payload, dependencies = {}) {
  const normalized = normalizeSystemNotificationPayload(payload);
  if (!normalized) {
    return { ok: false, reason: "invalidPayload" };
  }

  const Notification = dependencies.Notification;
  if (typeof Notification !== "function") {
    return { ok: false, reason: "unavailable" };
  }

  try {
    const notification = new Notification(normalized);
    if (typeof notification.show !== "function") {
      return { ok: false, reason: "unavailable" };
    }
    notification.show();
    return { ok: true };
  } catch {
    return { ok: false, reason: "showFailed" };
  }
}

function normalizeSystemNotificationPayload(payload) {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return null;
  }

  const title = normalizeNotificationText(
    payload.title,
    MAX_NOTIFICATION_TITLE_LENGTH,
  );
  if (!title) {
    return null;
  }

  const hasBody = Object.hasOwn(payload, "body") && payload.body != null;
  const body = hasBody
    ? normalizeNotificationText(payload.body, MAX_NOTIFICATION_BODY_LENGTH)
    : null;
  if (hasBody && !body) {
    return null;
  }
  return body ? { title, body } : { title };
}

function normalizeNotificationText(value, maxLength) {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.length <= maxLength) {
    return trimmed;
  }
  return `${trimmed.slice(0, maxLength - 3)}...`;
}

module.exports = {
  normalizeSystemNotificationPayload,
  showSystemNotification,
};
