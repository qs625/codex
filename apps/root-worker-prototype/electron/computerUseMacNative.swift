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

func observe() {
  let trusted = AXIsProcessTrusted()
  let cursor = CGEvent(source: nil)?.location ?? CGPoint.zero
  let frontmost = NSWorkspace.shared.frontmostApplication
  var activeApp: [String: Any] = [:]
  if let app = frontmost {
    activeApp["name"] = app.localizedName
    activeApp["bundleIdentifier"] = app.bundleIdentifier
    activeApp["processIdentifier"] = Int(app.processIdentifier)
    if trusted {
      activeApp["window"] = focusedWindow(pid: app.processIdentifier)
    }
  }
  json([
    "ok": true,
    "cursor": ["x": cursor.x, "y": cursor.y],
    "activeApp": activeApp,
    "accessibilityTrusted": trusted,
  ])
}

func postMouse(_ type: CGEventType, _ point: CGPoint) {
  let event = CGEvent(
    mouseEventSource: nil,
    mouseType: type,
    mouseCursorPosition: point,
    mouseButton: .left
  )
  event?.post(tap: .cghidEventTap)
}

func requireDesktopControlPermission() {
  guard AXIsProcessTrusted() else {
    error("Accessibility permission is required before controlling the desktop")
  }
}

func move(_ object: [String: Any]) {
  requireDesktopControlPermission()
  postMouse(.mouseMoved, pointFromPayload(object))
  json(["ok": true])
}

func click(_ object: [String: Any]) {
  requireDesktopControlPermission()
  let point = pointFromPayload(object)
  postMouse(.mouseMoved, point)
  postMouse(.leftMouseDown, point)
  usleep(45_000)
  postMouse(.leftMouseUp, point)
  json(["ok": true])
}

func typeText(_ object: [String: Any]) {
  requireDesktopControlPermission()
  guard let text = object["text"] as? String else {
    error("Type action requires text")
  }
  for scalar in text.unicodeScalars {
    var value = UniChar(scalar.value)
    guard let down = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true),
      let up = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false)
    else {
      error("Could not create keyboard event")
    }
    down.keyboardSetUnicodeString(stringLength: 1, unicodeString: &value)
    up.keyboardSetUnicodeString(stringLength: 1, unicodeString: &value)
    down.post(tap: .cghidEventTap)
    up.post(tap: .cghidEventTap)
  }
  json(["ok": true])
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
  guard let code = keyCodes[raw.lowercased()] else {
    error("Unsupported key: \(raw)")
  }
  let modifiers = object["modifiers"] as? [String] ?? []
  let eventFlags = flags(modifiers)
  guard
    let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true),
    let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false)
  else {
    error("Could not create keyboard event")
  }
  down.flags = eventFlags
  up.flags = eventFlags
  down.post(tap: .cghidEventTap)
  up.post(tap: .cghidEventTap)
  json(["ok": true])
}

let command = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "observe"
let object = payload()

switch command {
case "observe":
  observe()
case "move":
  move(object)
case "click":
  click(object)
case "type":
  typeText(object)
case "key":
  pressKey(object)
default:
  error("Unsupported command: \(command)")
}
