import AppKit
import ApplicationServices
import Foundation

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

func focusedWindow(pid: pid_t) -> [String: Any]? {
  let app = AXUIElementCreateApplication(pid)
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &value) == .success else {
    return nil
  }
  let window = value as! AXUIElement
  var result: [String: Any] = [:]
  result["title"] = axString(window, kAXTitleAttribute)
  result["role"] = axString(window, kAXRoleAttribute)
  result["subrole"] = axString(window, kAXSubroleAttribute)
  result["position"] = axPoint(window, kAXPositionAttribute)
  result["size"] = axSize(window, kAXSizeAttribute)
  return result
}

func firstWindow(pid: pid_t) -> [String: Any]? {
  let app = AXUIElementCreateApplication(pid)
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value) == .success else {
    return nil
  }
  guard let windows = value as? [AXUIElement], let window = windows.first else {
    return nil
  }
  var result: [String: Any] = [:]
  result["title"] = axString(window, kAXTitleAttribute)
  result["role"] = axString(window, kAXRoleAttribute)
  result["subrole"] = axString(window, kAXSubroleAttribute)
  result["position"] = axPoint(window, kAXPositionAttribute)
  result["size"] = axSize(window, kAXSizeAttribute)
  return result
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
  if targetVisibility == "background" {
    response["limitations"] = [[
      "code": "background-observe-metadata-only",
      "message": "Background target observe is limited to app/window metadata; the screenshot remains the current desktop capture.",
    ]]
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

let command = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "observe"
let object = payload()

switch command {
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
case "key":
  pressKey(object)
case "hotkey":
  pressHotkey(object)
case "drag":
  drag(object)
default:
  error("Unsupported command: \(command)")
}
