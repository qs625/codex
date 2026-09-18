import AppKit
import ApplicationServices
import Foundation
import ScreenCaptureKit

func json(_ value: Any) {
  let data = try! JSONSerialization.data(withJSONObject: value, options: [])
  FileHandle.standardOutput.write(data)
  FileHandle.standardOutput.write("\n".data(using: .utf8)!)
}

func error(_ message: String) -> Never {
  json(["ok": false, "error": message])
  exit(1)
}

func payload() -> [String: Any] {
  guard CommandLine.arguments.count > 2 else { return [:] }
  guard let data = Data(base64Encoded: CommandLine.arguments[2]) else {
    error("Invalid payload encoding")
  }
  guard
    let decoded = try? JSONSerialization.jsonObject(with: data),
    let object = decoded as? [String: Any]
  else {
    error("Invalid payload JSON")
  }
  return object
}

func number(_ value: Any?) -> Double? {
  if let value = value as? Double { return value }
  if let value = value as? Int { return Double(value) }
  if let value = value as? NSNumber { return value.doubleValue }
  if let value = value as? String { return Double(value) }
  return nil
}

func pointFromPayload(_ object: [String: Any]) -> CGPoint {
  guard let x = number(object["x"]), let y = number(object["y"]) else {
    error("Action requires x and y")
  }
  return CGPoint(x: x, y: y)
}

func pointFromNestedPayload(_ object: [String: Any], _ key: String) -> CGPoint {
  guard let nested = object[key] as? [String: Any] else {
    error("Action requires \(key) point")
  }
  guard let x = number(nested["x"]), let y = number(nested["y"]) else {
    error("Action requires \(key).x and \(key).y")
  }
  return CGPoint(x: x, y: y)
}

func axString(_ element: AXUIElement, _ attribute: String) -> String? {
  var value: CFTypeRef?
  if AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success {
    return value as? String
  }
  return nil
}

func axPoint(_ element: AXUIElement, _ attribute: String) -> [String: Double]? {
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else {
    return nil
  }
  var point = CGPoint.zero
  guard let axValue = value, AXValueGetValue(axValue as! AXValue, .cgPoint, &point) else {
    return nil
  }
  return ["x": point.x, "y": point.y]
}

func axSize(_ element: AXUIElement, _ attribute: String) -> [String: Double]? {
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else {
    return nil
  }
  var size = CGSize.zero
  guard let axValue = value, AXValueGetValue(axValue as! AXValue, .cgSize, &size) else {
    return nil
  }
  return ["width": size.width, "height": size.height]
}

func axRect(_ element: AXUIElement) -> [String: Double]? {
  guard
    let position = axPoint(element, kAXPositionAttribute),
    let size = axSize(element, kAXSizeAttribute),
    let x = position["x"],
    let y = position["y"],
    let width = size["width"],
    let height = size["height"],
    width > 0,
    height > 0
  else {
    return nil
  }
  return ["x": x, "y": y, "width": width, "height": height]
}

func axAttributeSettable(_ element: AXUIElement, _ attribute: String) -> Bool {
  var settable = DarwinBoolean(false)
  return AXUIElementIsAttributeSettable(element, attribute as CFString, &settable) == .success &&
    settable.boolValue
}

func axWritable(_ element: AXUIElement, role: String?) -> Bool {
  let writableRoles = Set(["AXComboBox", "AXSearchField", "AXTextArea", "AXTextField"])
  return writableRoles.contains(role ?? "") && axAttributeSettable(element, kAXValueAttribute)
}

func windowSummary(_ window: AXUIElement, pid: pid_t? = nil) -> [String: Any] {
  var result: [String: Any] = [:]
  result["title"] = axString(window, kAXTitleAttribute)
  result["role"] = axString(window, kAXRoleAttribute)
  result["subrole"] = axString(window, kAXSubroleAttribute)
  result["position"] = axPoint(window, kAXPositionAttribute)
  result["size"] = axSize(window, kAXSizeAttribute)
  if let pid = pid, let windowId = windowIdentifier(pid: pid, summary: result) {
    result["windowId"] = windowId
  }
  return result
}

func focusedWindowElement(pid: pid_t) -> AXUIElement? {
  let app = AXUIElementCreateApplication(pid)
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &value) == .success else {
    return nil
  }
  return (value as! AXUIElement)
}

func focusedWindow(pid: pid_t) -> [String: Any]? {
  guard let window = focusedWindowElement(pid: pid) else {
    return nil
  }
  return windowSummary(window, pid: pid)
}

func firstWindowElement(pid: pid_t) -> AXUIElement? {
  let app = AXUIElementCreateApplication(pid)
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value) == .success else {
    return nil
  }
  guard let windows = value as? [AXUIElement], let window = windows.first else {
    return nil
  }
  return window
}

func firstWindow(pid: pid_t) -> [String: Any]? {
  guard let window = firstWindowElement(pid: pid) else {
    return nil
  }
  return windowSummary(window, pid: pid)
}

func windowIdentifier(pid: pid_t, summary: [String: Any]) -> Int? {
  guard
    let info = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
      as? [[String: Any]]
  else {
    return nil
  }
  let title = summary["title"] as? String
  let position = summary["position"] as? [String: Double]
  let size = summary["size"] as? [String: Double]
  let candidates = info.filter { item in
    guard Int32(number(item[kCGWindowOwnerPID as String]) ?? -1) == pid else {
      return false
    }
    if let title = title, !title.isEmpty,
       let windowName = item[kCGWindowName as String] as? String,
       !windowName.isEmpty,
       windowName != title {
      return false
    }
    guard
      let bounds = item[kCGWindowBounds as String] as? [String: Any],
      let x = number(bounds["X"]),
      let y = number(bounds["Y"]),
      let width = number(bounds["Width"]),
      let height = number(bounds["Height"])
    else {
      return true
    }
    if
      let px = position?["x"],
      let py = position?["y"],
      let sw = size?["width"],
      let sh = size?["height"] {
      return abs(px - x) < 2 && abs(py - y) < 2 && abs(sw - width) < 2 && abs(sh - height) < 2
    }
    return true
  }
  guard candidates.count == 1 else {
    return nil
  }
  guard let windowId = number(candidates[0][kCGWindowNumber as String]) else {
    return nil
  }
  return Int(windowId)
}

func axChildren(_ element: AXUIElement) -> [AXUIElement] {
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &value) == .success else {
    return []
  }
  return value as? [AXUIElement] ?? []
}

func boundedText(_ value: String?, _ maxLength: Int = 160) -> String? {
  guard let value = value else {
    return nil
  }
  let trimmed = value
    .components(separatedBy: .whitespacesAndNewlines)
    .filter { !$0.isEmpty }
    .joined(separator: " ")
  guard !trimmed.isEmpty else {
    return nil
  }
  if trimmed.count <= maxLength {
    return trimmed
  }
  return String(trimmed.prefix(maxLength - 1)) + "…"
}

func axElementCandidate(_ element: AXUIElement) -> [String: Any]? {
  let role = axString(element, kAXRoleAttribute)
  let subrole = axString(element, kAXSubroleAttribute)
  let title = boundedText(axString(element, kAXTitleAttribute))
  let value = boundedText(axString(element, kAXValueAttribute))
  let description = boundedText(axString(element, kAXDescriptionAttribute))
  let includedRoles = Set([
    "AXButton",
    "AXCheckBox",
    "AXComboBox",
    "AXLink",
    "AXMenuButton",
    "AXPopUpButton",
    "AXRadioButton",
    "AXSearchField",
    "AXStaticText",
    "AXTextArea",
    "AXTextField",
  ])
  guard title != nil || value != nil || description != nil || includedRoles.contains(role ?? "") else {
    return nil
  }
  guard let bounds = axRect(element) else {
    return nil
  }
  let center: [String: Double] = [
    "x": (bounds["x"] ?? 0) + ((bounds["width"] ?? 0) / 2),
    "y": (bounds["y"] ?? 0) + ((bounds["height"] ?? 0) / 2),
  ]
  var result: [String: Any] = [
    "bounds": bounds,
    "center": center,
    "confidence": 0.85,
    "source": "macos-accessibility",
    "writable": axWritable(element, role: role),
  ]
  result["role"] = role
  result["subrole"] = subrole
  result["title"] = title
  result["value"] = value
  result["description"] = description
  return result
}

func accessibilityElements(window: AXUIElement, limit: Int) -> (elements: [[String: Any]], limitations: [[String: String]]) {
  guard limit > 0 else {
    return ([], [[
      "code": "accessibility-elements-truncated",
      "message": "Accessibility element output limit is 0; no candidates were returned.",
    ]])
  }
  var result: [[String: Any]] = []
  var limitations: [[String: String]] = []
  var queue: [(AXUIElement, Int)] = [(window, 0)]
  var visited = 0
  while !queue.isEmpty && visited < 600 {
    let (element, depth) = queue.removeFirst()
    visited += 1
    if let candidate = axElementCandidate(element) {
      if result.count >= limit {
        limitations.append([
          "code": "accessibility-elements-truncated",
          "message": "Accessibility element output reached the configured candidate limit before traversal completed.",
        ])
        break
      }
      result.append(candidate)
    }
    if depth >= 6 {
      continue
    }
    for child in axChildren(element).prefix(80) {
      queue.append((child, depth + 1))
    }
  }
  if visited >= 600 && !queue.isEmpty {
    limitations.append([
      "code": "accessibility-traversal-truncated",
      "message": "Accessibility traversal reached the safety visit cap before exhausting the target window tree.",
    ])
  }
  return (result, limitations)
}

func textMatchHaystack(_ element: AXUIElement) -> String {
  return [
    axString(element, kAXTitleAttribute),
    axString(element, kAXValueAttribute),
    axString(element, kAXDescriptionAttribute),
  ]
    .compactMap { $0 }
    .joined(separator: " ")
    .lowercased()
}

func matchingWritableElements(window: AXUIElement, query: String, limit: Int) -> (matches: [(AXUIElement, [String: Any])], truncated: Bool) {
  var matches: [(AXUIElement, [String: Any])] = []
  var queue: [(AXUIElement, Int)] = [(window, 0)]
  var visited = 0
  let normalizedQuery = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
  while !queue.isEmpty && visited < 600 {
    let (element, depth) = queue.removeFirst()
    visited += 1
    if textMatchHaystack(element).contains(normalizedQuery),
       let candidate = axElementCandidate(element),
       candidate["writable"] as? Bool == true {
      if matches.count >= limit {
        return (matches, true)
      }
      matches.append((element, candidate))
    }
    if depth >= 6 {
      continue
    }
    for child in axChildren(element).prefix(80) {
      queue.append((child, depth + 1))
    }
  }
  return (matches, visited >= 600 && !queue.isEmpty)
}

func appSummary(_ app: NSRunningApplication, trusted: Bool, frontmost: Bool) -> [String: Any] {
  var result: [String: Any] = [:]
  result["name"] = app.localizedName
  result["bundleIdentifier"] = app.bundleIdentifier
  result["processIdentifier"] = Int(app.processIdentifier)
  result["frontmost"] = frontmost
  if trusted {
    result["window"] = frontmost
      ? focusedWindow(pid: app.processIdentifier)
      : firstWindow(pid: app.processIdentifier)
  }
  return result
}

func appMatches(_ app: NSRunningApplication, _ identifier: String) -> Bool {
  let expected = identifier.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
  guard !expected.isEmpty else { return false }
  return app.bundleIdentifier?.lowercased() == expected ||
    app.localizedName?.lowercased() == expected
}

func targetApplication(_ identifier: String?) -> NSRunningApplication? {
  guard let identifier = identifier else { return nil }
  return NSWorkspace.shared.runningApplications.first { app in
    appMatches(app, identifier)
  }
}

func observe(_ object: [String: Any]) {
  let trusted = AXIsProcessTrusted()
  let includePerception = (object["includePerception"] as? Bool) ?? true
  let perceptionLimit = min(max(Int(number(object["perceptionLimit"]) ?? 40), 0), 80)
  let cursor = CGEvent(source: nil)?.location ?? CGPoint.zero
  let frontmost = NSWorkspace.shared.frontmostApplication
  var frontmostApp: [String: Any] = [:]
  if let app = frontmost {
    frontmostApp = appSummary(app, trusted: trusted, frontmost: true)
  }
  let targetIdentifier = object["targetApp"] as? String
  let target = targetApplication(targetIdentifier)
  var targetApp: [String: Any]? = nil
  var targetVisibility = "unknown"
  if let target = target {
    let isFrontmost = frontmost?.processIdentifier == target.processIdentifier
    targetApp = appSummary(target, trusted: trusted, frontmost: isFrontmost)
    targetVisibility = isFrontmost ? "frontmost" : "background"
  } else if targetIdentifier == nil, let app = frontmost {
    targetApp = appSummary(app, trusted: trusted, frontmost: true)
    targetVisibility = "frontmost"
  } else if let targetIdentifier = targetIdentifier, let app = frontmost, appMatches(app, targetIdentifier) {
    targetApp = appSummary(app, trusted: trusted, frontmost: true)
    targetVisibility = "frontmost"
  }
  var response: [String: Any] = [
    "ok": true,
    "cursor": ["x": cursor.x, "y": cursor.y],
    "systemCursor": ["x": cursor.x, "y": cursor.y],
    "activeApp": frontmostApp,
    "frontmostApp": frontmostApp,
    "targetVisibility": targetVisibility,
    "accessibilityTrusted": trusted,
  ]
  if let targetApp = targetApp {
    response["targetApp"] = targetApp
  }
  var limitations: [[String: String]] = []
  var perception: [String: Any] = [
    "enabled": includePerception,
    "source": "macos-accessibility",
    "limit": perceptionLimit,
    "accessibilityElements": [],
    "limitations": [],
  ]
  var perceptionLimitations: [[String: String]] = []
  if !includePerception {
    perceptionLimitations.append([
      "code": "perception-disabled",
      "message": "Perception extraction was disabled for this observation.",
    ])
  } else if !trusted {
    perceptionLimitations.append([
      "code": "accessibility-permission-required",
      "message": "Accessibility permission is required for AX element extraction.",
    ])
  } else if targetVisibility != "frontmost" && targetVisibility != "background" {
    perceptionLimitations.append([
      "code": "target-unavailable",
      "message": "Target window crop and AX element extraction require a resolved target app.",
    ])
  } else if
    let app = targetVisibility == "background" ? target : frontmost,
    let window = targetVisibility == "background"
      ? firstWindowElement(pid: app.processIdentifier)
      : focusedWindowElement(pid: app.processIdentifier) {
    if let bounds = axRect(window) {
      perception["windowCrop"] = [
        "source": "target-window",
        "coordinateSpace": "screen",
        "bounds": bounds,
      ]
    } else {
      perceptionLimitations.append([
        "code": "target-window-bounds-unavailable",
        "message": "Target window bounds were not available for crop metadata.",
      ])
    }
    let extracted = accessibilityElements(window: window, limit: perceptionLimit)
    perception["accessibilityElements"] = extracted.elements
    perceptionLimitations.append(contentsOf: extracted.limitations)
  } else {
    perceptionLimitations.append([
      "code": "target-window-unavailable",
      "message": "The target window was not available through Accessibility.",
    ])
  }
  perception["limitations"] = perceptionLimitations
  response["perception"] = perception
  if !limitations.isEmpty {
    response["limitations"] = limitations
  }
  json(response)
}

func activate(_ object: [String: Any]) {
  guard let targetIdentifier = object["targetApp"] as? String else {
    error("Activate requires targetApp")
  }
  guard let target = targetApplication(targetIdentifier) else {
    error("Target app \(targetIdentifier) is not running")
  }
  let requested = target.activate(options: [.activateIgnoringOtherApps])
  let trusted = AXIsProcessTrusted()
  var waitedMs = 0
  var frontmost = NSWorkspace.shared.frontmostApplication
  while frontmost?.processIdentifier != target.processIdentifier && waitedMs < 1_000 {
    usleep(50_000)
    waitedMs += 50
    frontmost = NSWorkspace.shared.frontmostApplication
  }
  let activated = frontmost?.processIdentifier == target.processIdentifier
  var response: [String: Any] = [
    "ok": true,
    "activated": requested && activated,
    "waitedMs": waitedMs,
    "targetApp": appSummary(target, trusted: trusted, frontmost: activated),
  ]
  if let frontmost = frontmost {
    response["frontmostApp"] = appSummary(frontmost, trusted: trusted, frontmost: true)
  }
  if !requested {
    response["reason"] = "macOS declined target app activation"
  } else if !activated {
    response["reason"] = "Target app did not become frontmost after activation"
  }
  json(response)
}

func postMouse(_ type: CGEventType, _ point: CGPoint, button: CGMouseButton = .left, clickState: Int64 = 1) {
  let event = CGEvent(
    mouseEventSource: nil,
    mouseType: type,
    mouseCursorPosition: point,
    mouseButton: button
  )
  event?.setIntegerValueField(.mouseEventClickState, value: clickState)
  event?.post(tap: .cghidEventTap)
}

func currentCursor() -> CGPoint {
  CGEvent(source: nil)?.location ?? CGPoint.zero
}

func restoreCursorIfNeeded(_ before: CGPoint) -> (Bool, CGPoint) {
  let current = currentCursor()
  var restored = false
  if abs(current.x - before.x) > 0.5 || abs(current.y - before.y) > 0.5 {
    CGWarpMouseCursorPosition(before)
    restored = true
  }
  return (restored, currentCursor())
}

func cursorEvidence(_ before: CGPoint, _ restored: Bool, _ after: CGPoint) -> [String: Any] {
  [
    "systemCursorRestored": restored,
    "systemCursorBefore": ["x": before.x, "y": before.y],
    "systemCursorAfter": ["x": after.x, "y": after.y],
  ]
}

func requireDesktopControlPermission() {
  guard AXIsProcessTrusted() else {
    error("Accessibility permission is required before controlling the desktop")
  }
}

func move(_ object: [String: Any]) {
  requireDesktopControlPermission()
  json(["ok": true])
}

func click(_ object: [String: Any]) {
  requireDesktopControlPermission()
  let point = pointFromPayload(object)
  let before = currentCursor()
  postMouse(.leftMouseDown, point)
  usleep(45_000)
  postMouse(.leftMouseUp, point)
  let (restored, afterRestore) = restoreCursorIfNeeded(before)
  var response = cursorEvidence(before, restored, afterRestore)
  response.merge([
    "ok": true,
    "method": "mouse-click",
    "button": "left",
    "clickCount": 1,
  ]) { _, new in new }
  json(response)
}

func doubleClick(_ object: [String: Any]) {
  requireDesktopControlPermission()
  let point = pointFromPayload(object)
  let before = currentCursor()
  postMouse(.leftMouseDown, point, clickState: 1)
  usleep(35_000)
  postMouse(.leftMouseUp, point, clickState: 1)
  usleep(65_000)
  postMouse(.leftMouseDown, point, clickState: 2)
  usleep(35_000)
  postMouse(.leftMouseUp, point, clickState: 2)
  let (restored, afterRestore) = restoreCursorIfNeeded(before)
  var response = cursorEvidence(before, restored, afterRestore)
  response.merge([
    "ok": true,
    "method": "mouse-click",
    "button": "left",
    "clickCount": 2,
  ]) { _, new in new }
  json(response)
}

func rightClick(_ object: [String: Any]) {
  requireDesktopControlPermission()
  let point = pointFromPayload(object)
  let before = currentCursor()
  postMouse(.rightMouseDown, point, button: .right)
  usleep(45_000)
  postMouse(.rightMouseUp, point, button: .right)
  let (restored, afterRestore) = restoreCursorIfNeeded(before)
  var response = cursorEvidence(before, restored, afterRestore)
  response.merge([
    "ok": true,
    "method": "mouse-click",
    "button": "right",
    "clickCount": 1,
  ]) { _, new in new }
  json(response)
}

func scroll(_ object: [String: Any]) {
  requireDesktopControlPermission()
  let point = pointFromPayload(object)
  let deltaX = Int32(number(object["deltaX"]) ?? 0)
  let deltaY = Int32(number(object["deltaY"]) ?? 0)
  let before = currentCursor()
  guard let event = CGEvent(
    scrollWheelEvent2Source: nil,
    units: .pixel,
    wheelCount: 2,
    wheel1: deltaY,
    wheel2: deltaX,
    wheel3: 0
  ) else {
    error("Could not create scroll event")
  }
  event.location = point
  event.post(tap: .cghidEventTap)
  let (restored, afterRestore) = restoreCursorIfNeeded(before)
  var response = cursorEvidence(before, restored, afterRestore)
  response.merge([
    "ok": true,
    "method": "scroll-wheel",
    "deltaX": Double(deltaX),
    "deltaY": Double(deltaY),
  ]) { _, new in new }
  json(response)
}

func drag(_ object: [String: Any]) {
  requireDesktopControlPermission()
  let from = pointFromNestedPayload(object, "from")
  let to = pointFromNestedPayload(object, "to")
  let before = currentCursor()
  postMouse(.leftMouseDown, from)
  let steps = 8
  for index in 1...steps {
    let progress = Double(index) / Double(steps)
    let point = CGPoint(
      x: from.x + ((to.x - from.x) * progress),
      y: from.y + ((to.y - from.y) * progress)
    )
    postMouse(.leftMouseDragged, point)
    usleep(12_000)
  }
  postMouse(.leftMouseUp, to)
  let (restored, afterRestore) = restoreCursorIfNeeded(before)
  var response = cursorEvidence(before, restored, afterRestore)
  response.merge([
    "ok": true,
    "method": "mouse-drag",
    "button": "left",
  ]) { _, new in new }
  json(response)
}

func typeText(_ object: [String: Any]) {
  requireDesktopControlPermission()
  guard let text = object["text"] as? String else {
    error("Type action requires text")
  }
  let pasteboard = NSPasteboard.general
  let saved = copyPasteboardItems(pasteboard)
  if let failure = saved.error {
    error(failure)
  }
  let savedItems = saved.items
  pasteboard.clearContents()
  guard pasteboard.setString(text, forType: .string) else {
    let restored = restorePasteboard(pasteboard, savedItems)
    error("Could not write type text to the pasteboard; pasteboardRestored=\(restored)")
  }
  if let failure = postKey("v", ["cmd"]) {
    let restored = restorePasteboard(pasteboard, savedItems)
    error("Could not send paste shortcut for type action: \(failure); pasteboardRestored=\(restored)")
  }
  usleep(120_000)
  let restored = restorePasteboard(pasteboard, savedItems)
  guard restored else {
    error("Typed text was sent, but the previous pasteboard contents could not be restored")
  }
  json([
    "ok": true,
    "method": "pasteboard-cmd-v",
    "characterCount": text.count,
    "pasteboardRestored": restored,
  ])
}

func setText(_ object: [String: Any]) {
  guard AXIsProcessTrusted() else {
    error("Accessibility permission is required before setting AX text")
  }
  guard let targetIdentifier = object["targetApp"] as? String else {
    error("setText requires targetApp")
  }
  guard let query = object["query"] as? String, !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
    error("setText requires a non-empty query")
  }
  guard let text = object["text"] as? String else {
    error("setText requires text")
  }
  guard let target = targetApplication(targetIdentifier) else {
    error("Target app \(targetIdentifier) is not running")
  }
  guard let window = firstWindowElement(pid: target.processIdentifier) else {
    error("Target window was not available through Accessibility")
  }
  let frontmost = NSWorkspace.shared.frontmostApplication
  let targetVisibility = frontmost?.processIdentifier == target.processIdentifier ? "frontmost" : "background"
  let matched = matchingWritableElements(window: window, query: query, limit: 2)
  if matched.truncated {
    error("Writable AX text match is ambiguous because traversal was truncated")
  }
  if matched.matches.isEmpty {
    error("No unique writable AX text element matched query: \(query)")
  }
  if matched.matches.count > 1 {
    error("Multiple writable AX text elements matched query: \(query)")
  }
  let (element, candidate) = matched.matches[0]
  guard axAttributeSettable(element, kAXValueAttribute) else {
    error("Matched AX element is not writable")
  }
  let result = AXUIElementSetAttributeValue(element, kAXValueAttribute as CFString, text as CFTypeRef)
  guard result == .success else {
    error("AX set value failed: \(result.rawValue)")
  }
  json([
    "ok": true,
    "method": "accessibility-set-value",
    "targetVisibility": targetVisibility,
    "query": query,
    "matchStatus": "unique",
    "matchedElement": candidate,
    "characterCount": text.count,
  ])
}

let keyCodes: [String: CGKeyCode] = [
  "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9,
  "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19,
  "3": 20, "4": 21, "6": 22, "5": 23, "=": 24, "9": 25, "7": 26, "-": 27, "8": 28,
  "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35, "return": 36,
  "l": 37, "j": 38, "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45,
  "m": 46, ".": 47, "tab": 48, "space": 49, "`": 50, "delete": 51, "escape": 53,
  "left": 123, "right": 124, "down": 125, "up": 126
]

func flags(_ names: [String]) -> CGEventFlags {
  var result = CGEventFlags()
  for name in names {
    switch name.lowercased() {
    case "cmd", "command", "meta": result.insert(.maskCommand)
    case "shift": result.insert(.maskShift)
    case "alt", "option": result.insert(.maskAlternate)
    case "ctrl", "control": result.insert(.maskControl)
    default: break
    }
  }
  return result
}

func pressKey(_ object: [String: Any]) {
  requireDesktopControlPermission()
  guard let raw = object["key"] as? String else {
    error("Key action requires key")
  }
  let modifiers = object["modifiers"] as? [String] ?? []
  if let failure = postKey(raw, modifiers) {
    error(failure)
  }
  json(["ok": true, "method": "key-press", "key": raw, "modifiers": modifiers])
}

func pressHotkey(_ object: [String: Any]) {
  requireDesktopControlPermission()
  guard let raw = object["key"] as? String else {
    error("Hotkey action requires key")
  }
  let modifiers = object["modifiers"] as? [String] ?? []
  if let failure = postKey(raw, modifiers) {
    error(failure)
  }
  json(["ok": true, "method": "keyboard-shortcut", "key": raw, "modifiers": modifiers])
}

func postKey(_ raw: String, _ modifiers: [String]) -> String? {
  guard let code = keyCodes[raw.lowercased()] else {
    return "Unsupported key: \(raw)"
  }
  let eventFlags = flags(modifiers)
  guard
    let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true),
    let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false)
  else {
    return "Could not create keyboard event"
  }
  down.flags = eventFlags
  up.flags = eventFlags
  down.post(tap: .cghidEventTap)
  up.post(tap: .cghidEventTap)
  return nil
}

func copyPasteboardItems(_ pasteboard: NSPasteboard) -> (items: [NSPasteboardItem], error: String?) {
  guard let items = pasteboard.pasteboardItems else {
    return ([], nil)
  }
  var copies: [NSPasteboardItem] = []
  for item in items {
    if item.types.isEmpty {
      return ([], "Could not safely copy pasteboard item with no data types")
    }
    let copy = NSPasteboardItem()
    for type in item.types {
      if let data = item.data(forType: type) {
        guard copy.setData(data, forType: type) else {
          return ([], "Could not safely preserve pasteboard data for type \(type.rawValue)")
        }
        continue
      }
      if let value = item.propertyList(forType: type) {
        guard copy.setPropertyList(value, forType: type) else {
          return ([], "Could not safely preserve pasteboard property list for type \(type.rawValue)")
        }
        continue
      }
      if let string = fallbackStringForPasteboardItem(item, forType: type) {
        guard copy.setString(string, forType: type) else {
          return ([], "Could not safely preserve pasteboard string for type \(type.rawValue)")
        }
        continue
      }
      return ([], "Could not safely copy pasteboard data for type \(type.rawValue)")
    }
    copies.append(copy)
  }
  return (copies, nil)
}

func fallbackStringForPasteboardItem(_ item: NSPasteboardItem, forType targetType: NSPasteboard.PasteboardType) -> String? {
  let textTypes: [NSPasteboard.PasteboardType] = [
    .string,
    NSPasteboard.PasteboardType("public.utf8-plain-text"),
    NSPasteboard.PasteboardType("public.utf16-plain-text"),
    NSPasteboard.PasteboardType("public.utf16-external-plain-text"),
  ]
  guard textTypes.contains(targetType) else {
    return nil
  }
  for type in textTypes {
    if let string = item.string(forType: type) {
      return string
    }
    if let data = item.data(forType: type) {
      if let string = String(data: data, encoding: .utf8) {
        return string
      }
      if let string = String(data: data, encoding: .utf16) {
        return string
      }
    }
  }
  for type in item.types {
    if type == .rtf, let data = item.data(forType: type) {
      let attributed = try? NSAttributedString(
        data: data,
        options: [.documentType: NSAttributedString.DocumentType.rtf],
        documentAttributes: nil
      )
      if let string = attributed?.string {
        return string
      }
    }
  }
  return nil
}

func restorePasteboard(_ pasteboard: NSPasteboard, _ items: [NSPasteboardItem]) -> Bool {
  pasteboard.clearContents()
  if items.isEmpty {
    return true
  }
  return pasteboard.writeObjects(items)
}

func rectFromPayload(_ object: [String: Any]?) -> CGRect? {
  guard let object = object else {
    return nil
  }
  guard
    let x = number(object["x"]),
    let y = number(object["y"]),
    let width = number(object["width"]),
    let height = number(object["height"]),
    width > 0,
    height > 0
  else {
    return nil
  }
  return CGRect(x: x.rounded(), y: y.rounded(), width: width.rounded(), height: height.rounded())
}

enum NativeScreenshotError: Error {
  case unsupported(String)
}

func roundedPixelCount(_ value: CGFloat) -> Int {
  return max(1, Int(value.rounded()))
}

func screenScale(for rect: CGRect, on display: SCDisplay) -> CGFloat {
  guard display.frame.width > 0, display.frame.height > 0 else {
    return 1
  }
  let widthScale = CGFloat(display.width) / display.frame.width
  let heightScale = CGFloat(display.height) / display.frame.height
  return max(widthScale, heightScale, 1)
}

func shareableContent() throws -> SCShareableContent {
  let semaphore = DispatchSemaphore(value: 0)
  var resolvedContent: SCShareableContent?
  var resolvedError: Error?
  SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: true) {
    content,
    contentError in
    resolvedContent = content
    resolvedError = contentError
    semaphore.signal()
  }
  semaphore.wait()
  if let resolvedError = resolvedError {
    throw resolvedError
  }
  guard let resolvedContent = resolvedContent else {
    error("Screen Recording permission is required before observing the desktop")
  }
  return resolvedContent
}

func displayForRect(_ rect: CGRect?, in displays: [SCDisplay]) -> SCDisplay? {
  guard let rect = rect else {
    return displays.first
  }
  if let containing = displays.first(where: { $0.frame.intersects(rect) }) {
    return containing
  }
  return displays.first
}

func captureImage(contentFilter: SCContentFilter, configuration: SCStreamConfiguration) throws -> CGImage {
  guard #available(macOS 14.0, *) else {
    throw NativeScreenshotError.unsupported("Native screenshots require macOS 14 or newer")
  }
  let semaphore = DispatchSemaphore(value: 0)
  var resolvedImage: CGImage?
  var resolvedError: Error?
  SCScreenshotManager.captureImage(contentFilter: contentFilter, configuration: configuration) {
    image,
    captureError in
    resolvedImage = image
    resolvedError = captureError
    semaphore.signal()
  }
  semaphore.wait()
  if let resolvedError = resolvedError {
    throw resolvedError
  }
  guard let resolvedImage = resolvedImage else {
    error("Screen Recording permission is required before observing the desktop")
  }
  return resolvedImage
}

func captureDisplayImage(_ display: SCDisplay, rect: CGRect?) throws -> CGImage {
  let configuration = SCStreamConfiguration()
  if let rect = rect {
    let scale = screenScale(for: rect, on: display)
    configuration.sourceRect = rect
    configuration.width = roundedPixelCount(rect.width * scale)
    configuration.height = roundedPixelCount(rect.height * scale)
  } else {
    configuration.width = display.width
    configuration.height = display.height
  }
  return try captureImage(
    contentFilter: SCContentFilter(display: display, excludingWindows: []),
    configuration: configuration
  )
}

func captureDesktopImage(_ displays: [SCDisplay]) throws -> CGImage {
  guard !displays.isEmpty else {
    error("No display is available for screenshot")
  }
  if displays.count == 1 {
    return try captureDisplayImage(displays[0], rect: nil)
  }
  let union = displays.reduce(CGRect.null) { partial, display in
    partial.union(display.frame)
  }
  let width = roundedPixelCount(union.width)
  let height = roundedPixelCount(union.height)
  let colorSpace = CGColorSpaceCreateDeviceRGB()
  guard
    let context = CGContext(
      data: nil,
      width: width,
      height: height,
      bitsPerComponent: 8,
      bytesPerRow: 0,
      space: colorSpace,
      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )
  else {
    error("Failed to create screenshot canvas")
  }
  context.clear(CGRect(x: 0, y: 0, width: width, height: height))
  for display in displays {
    let image = try captureDisplayImage(display, rect: nil)
    let destination = CGRect(
      x: display.frame.minX - union.minX,
      y: union.maxY - display.frame.maxY,
      width: display.frame.width,
      height: display.frame.height
    )
    context.draw(image, in: destination)
  }
  guard let combined = context.makeImage() else {
    error("Failed to combine display screenshots")
  }
  return combined
}

func captureImage(bounds: [String: Any]?) throws -> CGImage {
  guard #available(macOS 14.0, *) else {
    throw NativeScreenshotError.unsupported("Native screenshots require macOS 14 or newer")
  }
  let content = try shareableContent()
  if let windowIdValue = number(bounds?["windowId"]) {
    guard
      let window = content.windows.first(where: {
        $0.windowID == CGWindowID(windowIdValue)
      })
    else {
      error("Window is no longer available for screenshot")
    }
    let configuration = SCStreamConfiguration()
    configuration.width = roundedPixelCount(window.frame.width)
    configuration.height = roundedPixelCount(window.frame.height)
    configuration.scalesToFit = false
    return try captureImage(
      contentFilter: SCContentFilter(desktopIndependentWindow: window),
      configuration: configuration
    )
  }

  let rect = rectFromPayload(bounds)
  if rect == nil {
    return try captureDesktopImage(content.displays)
  }
  guard let display = displayForRect(rect, in: content.displays) else {
    error("No display is available for screenshot")
  }
  return try captureDisplayImage(display, rect: rect)
}

func captureScreenshot(_ object: [String: Any]) {
  guard let tmpDir = object["tmpDir"] as? String, !tmpDir.isEmpty else {
    error("Screenshot requires tmpDir")
  }
  let bounds = object["bounds"] as? [String: Any]
  let image: CGImage
  do {
    image = try captureImage(bounds: bounds)
  } catch NativeScreenshotError.unsupported(let message) {
    error(message)
  } catch let captureError {
    error("Screen Recording permission is required before observing the desktop: \(captureError.localizedDescription)")
  }
  let bitmap = NSBitmapImageRep(cgImage: image)
  guard let data = bitmap.representation(using: .png, properties: [:]) else {
    error("Failed to encode screenshot as PNG")
  }
  let file = URL(fileURLWithPath: tmpDir)
    .appendingPathComponent("morpheus-computer-use-\(UUID().uuidString).png")
  do {
    try data.write(to: file, options: .atomic)
  } catch let writeError {
    error("Failed to write screenshot: \(writeError.localizedDescription)")
  }
  json([
    "ok": true,
    "screenshot": [
      "path": file.path,
      "mimeType": "image/png",
      "byteSize": data.count,
    ],
  ])
}

let command = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "observe"
let object = payload()

switch command {
case "screenshot":
  captureScreenshot(object)
case "observe":
  observe(object)
case "activate":
  activate(object)
case "move":
  move(object)
case "click":
  click(object)
case "doubleClick":
  doubleClick(object)
case "rightClick":
  rightClick(object)
case "scroll":
  scroll(object)
case "type":
  typeText(object)
case "setText":
  setText(object)
case "key":
  pressKey(object)
case "hotkey":
  pressHotkey(object)
case "drag":
  drag(object)
default:
  error("Unsupported command: \(command)")
}
