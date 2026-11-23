import AppKit
import ApplicationServices
import Carbon.HIToolbox

enum Limit {
    static let depth = 60
    static let controls = 200
    static let texts = 70
    static let visited = 6000
    static let seconds = 2.5
    static let name = 90
    static let value = 140
    static let label = 4
    static let chunk = 16
    static let messaging: Float = 1.5
    static let settle: UInt32 = 500_000
    static let key: UInt32 = 12_000
    static let listed = 30
}

struct Failure: Error {
    let text: String
}

let inputs: Set<String> = ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"]
let pressable: Set<String> = [
    "AXButton", "AXLink", "AXCheckBox", "AXRadioButton", "AXPopUpButton", "AXMenuButton", "AXSlider", "AXMenuItem",
    "AXDisclosureTriangle", "AXIncrementor", "AXSwitch", "AXToggle", "AXTab", "AXColorWell", "AXDockItem",
]
let rows: Set<String> = ["AXRow", "AXOutlineRow"]
let words: [String: String] = [
    "AXButton": "button", "AXLink": "link", "AXCheckBox": "checkbox", "AXRadioButton": "option",
    "AXPopUpButton": "pop-up", "AXMenuButton": "menu button", "AXSlider": "slider", "AXMenuItem": "menu item",
    "AXDisclosureTriangle": "disclosure", "AXIncrementor": "stepper", "AXSwitch": "switch", "AXToggle": "switch",
    "AXTab": "tab", "AXColorWell": "color", "AXTextField": "text field", "AXTextArea": "text area",
    "AXComboBox": "combo box", "AXSearchField": "search field", "AXRow": "row", "AXOutlineRow": "row",
    "AXDockItem": "dock item",
]
let keys: [String: Int] = [
    "a": kVK_ANSI_A, "b": kVK_ANSI_B, "c": kVK_ANSI_C, "d": kVK_ANSI_D, "e": kVK_ANSI_E, "f": kVK_ANSI_F,
    "g": kVK_ANSI_G, "h": kVK_ANSI_H, "i": kVK_ANSI_I, "j": kVK_ANSI_J, "k": kVK_ANSI_K, "l": kVK_ANSI_L,
    "m": kVK_ANSI_M, "n": kVK_ANSI_N, "o": kVK_ANSI_O, "p": kVK_ANSI_P, "q": kVK_ANSI_Q, "r": kVK_ANSI_R,
    "s": kVK_ANSI_S, "t": kVK_ANSI_T, "u": kVK_ANSI_U, "v": kVK_ANSI_V, "w": kVK_ANSI_W, "x": kVK_ANSI_X,
    "y": kVK_ANSI_Y, "z": kVK_ANSI_Z, "0": kVK_ANSI_0, "1": kVK_ANSI_1, "2": kVK_ANSI_2, "3": kVK_ANSI_3,
    "4": kVK_ANSI_4, "5": kVK_ANSI_5, "6": kVK_ANSI_6, "7": kVK_ANSI_7, "8": kVK_ANSI_8, "9": kVK_ANSI_9,
    "return": kVK_Return, "enter": kVK_Return, "tab": kVK_Tab, "space": kVK_Space, "delete": kVK_Delete,
    "backspace": kVK_Delete, "forwarddelete": kVK_ForwardDelete, "escape": kVK_Escape, "esc": kVK_Escape,
    "left": kVK_LeftArrow, "right": kVK_RightArrow, "up": kVK_UpArrow, "down": kVK_DownArrow,
    "home": kVK_Home, "end": kVK_End, "pageup": kVK_PageUp, "pagedown": kVK_PageDown,
    ",": kVK_ANSI_Comma, ".": kVK_ANSI_Period, "/": kVK_ANSI_Slash, ";": kVK_ANSI_Semicolon,
    "'": kVK_ANSI_Quote, "[": kVK_ANSI_LeftBracket, "]": kVK_ANSI_RightBracket, "\\": kVK_ANSI_Backslash,
    "-": kVK_ANSI_Minus, "=": kVK_ANSI_Equal, "`": kVK_ANSI_Grave,
    "f1": kVK_F1, "f2": kVK_F2, "f3": kVK_F3, "f4": kVK_F4, "f5": kVK_F5, "f6": kVK_F6,
    "f7": kVK_F7, "f8": kVK_F8, "f9": kVK_F9, "f10": kVK_F10, "f11": kVK_F11, "f12": kVK_F12,
]
let flags: [String: CGEventFlags] = [
    "cmd": .maskCommand, "command": .maskCommand, "shift": .maskShift, "option": .maskAlternate,
    "opt": .maskAlternate, "alt": .maskAlternate, "ctrl": .maskControl, "control": .maskControl,
    "fn": .maskSecondaryFn,
]

nonisolated(unsafe) var marked: [AXUIElement] = []
nonisolated(unsafe) var labels: [String] = []
nonisolated(unsafe) var woken: Set<pid_t> = []

func attribute(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}

func string(_ element: AXUIElement, _ name: String) -> String? {
    guard let value = attribute(element, name) else { return nil }
    if let text = value as? String { return text }
    if let number = value as? NSNumber { return number.stringValue }
    return nil
}

func element(_ parent: AXUIElement, _ name: String) -> AXUIElement? {
    guard let value = attribute(parent, name), CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
    return (value as! AXUIElement)
}

func elements(_ parent: AXUIElement, _ name: String) -> [AXUIElement] {
    (attribute(parent, name) as? [AXUIElement]) ?? []
}

func frame(_ element: AXUIElement) -> CGRect? {
    guard let position = attribute(element, kAXPositionAttribute), let size = attribute(element, kAXSizeAttribute),
        CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID()
    else { return nil }
    var origin = CGPoint.zero
    var extent = CGSize.zero
    AXValueGetValue(position as! AXValue, .cgPoint, &origin)
    AXValueGetValue(size as! AXValue, .cgSize, &extent)
    return CGRect(origin: origin, size: extent)
}

func tidy(_ text: String?, _ limit: Int) -> String {
    let flat = (text ?? "").split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    return flat.count > limit ? String(flat.prefix(limit)) + "…" : flat
}

func front() throws -> (NSRunningApplication, AXUIElement) {
    let system = AXUIElementCreateSystemWide()
    var pid: pid_t = 0
    if let focused = element(system, kAXFocusedApplicationAttribute) {
        AXUIElementGetPid(focused, &pid)
    }
    let running = NSRunningApplication(processIdentifier: pid) ?? NSWorkspace.shared.frontmostApplication
    guard let running else { throw Failure(text: "no app is in front") }
    let app = AXUIElementCreateApplication(running.processIdentifier)
    AXUIElementSetMessagingTimeout(app, Limit.messaging)
    if !woken.contains(running.processIdentifier) {
        woken.insert(running.processIdentifier)
        let manual = AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
        let enhanced = AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
        if manual == .success || enhanced == .success { usleep(Limit.settle) }
    }
    return (running, app)
}

func window(_ app: AXUIElement) -> AXUIElement? {
    element(app, kAXFocusedWindowAttribute) ?? element(app, kAXMainWindowAttribute)
        ?? elements(app, kAXWindowsAttribute).first
}

func gather(_ node: AXUIElement, _ depth: Int, _ into: inout [String]) {
    if depth > Limit.label || into.joined(separator: " ").count > Limit.name { return }
    for child in elements(node, kAXChildrenAttribute).prefix(12) {
        let role = string(child, kAXRoleAttribute) ?? ""
        if role == "AXStaticText" || role == "AXTextField" || role == "AXImage" || role == "AXHeading" {
            let text = tidy(string(child, kAXValueAttribute) ?? string(child, kAXTitleAttribute)
                ?? string(child, kAXDescriptionAttribute), Limit.name)
            if !text.isEmpty && !into.contains(text) { into.append(text) }
        }
        gather(child, depth + 1, &into)
    }
}

struct Walk {
    var lines: [String] = []
    var texts = 0
    var visited = 0
    var cut = false
    let bounds: CGRect?
    let until: Date
}

func kind(_ element: AXUIElement) -> String {
    let role = string(element, kAXRoleAttribute) ?? ""
    let sub = string(element, kAXSubroleAttribute) ?? ""
    if sub == "AXSearchField" { return "AXSearchField" }
    if sub == "AXTabButton" { return "AXTab" }
    if sub == "AXSwitch" || sub == "AXToggle" { return "AXSwitch" }
    if sub == "AXOutlineRow" { return "AXOutlineRow" }
    return role
}

func children(_ node: AXUIElement, _ role: String) -> [AXUIElement] {
    if role == "AXTable" || role == "AXOutline" || role == "AXList" {
        let visible = elements(node, kAXVisibleRowsAttribute)
        if !visible.isEmpty { return visible }
        let shown = elements(node, kAXVisibleChildrenAttribute)
        if !shown.isEmpty { return shown }
    }
    return elements(node, kAXChildrenAttribute)
}

func visit(_ node: AXUIElement, _ depth: Int, _ quiet: Bool, _ walk: inout Walk) {
    if depth > Limit.depth { return }
    walk.visited += 1
    if walk.visited > Limit.visited || marked.count >= Limit.controls || Date() > walk.until {
        walk.cut = true
        return
    }
    let role = kind(node)
    if role == "AXMenuBar" { return }
    if let bounds = walk.bounds, let box = frame(node), box.width > 0, box.height > 0, !box.intersects(bounds) {
        return
    }
    var hush = quiet
    let control = inputs.contains(role) || pressable.contains(role) || rows.contains(role)
    if control {
        var name = tidy(string(node, kAXTitleAttribute), Limit.name)
        if name.isEmpty { name = tidy(string(node, kAXDescriptionAttribute), Limit.name) }
        if name.isEmpty, let titled = element(node, kAXTitleUIElementAttribute) {
            name = tidy(string(titled, kAXValueAttribute) ?? string(titled, kAXTitleAttribute), Limit.name)
        }
        if name.isEmpty { name = tidy(string(node, kAXPlaceholderValueAttribute), Limit.name) }
        if name.isEmpty && !inputs.contains(role) {
            var parts: [String] = []
            gather(node, 0, &parts)
            name = tidy(parts.joined(separator: " · "), Limit.name)
        }
        if name.isEmpty { name = tidy(string(node, kAXHelpAttribute), Limit.name) }
        if !name.isEmpty || inputs.contains(role) {
            marked.append(node)
            var line = "[\(marked.count)] \(words[role] ?? "control")"
            if !name.isEmpty { line += " \"\(name)\"" }
            if inputs.contains(role) || role == "AXSlider" || role == "AXPopUpButton" {
                let value = tidy(string(node, kAXValueAttribute), Limit.value)
                if !value.isEmpty && value != name { line += " containing \"\(value)\"" }
            }
            if role == "AXCheckBox" || role == "AXSwitch" || role == "AXRadioButton" || role == "AXTab" {
                let value = (attribute(node, kAXValueAttribute) as? NSNumber)?.intValue ?? 0
                line += value == 0 ? " (off)" : " (on)"
            }
            if (attribute(node, kAXSelectedAttribute) as? Bool) == true { line += " (selected)" }
            if (attribute(node, kAXFocusedAttribute) as? Bool) == true { line += " (focused)" }
            if (attribute(node, kAXEnabledAttribute) as? Bool) == false { line += " (disabled)" }
            walk.lines.append(line)
            labels.append("\(words[role] ?? "control")\(name.isEmpty ? "" : " \"\(name)\"")")
            hush = true
            if inputs.contains(role) { return }
        }
    } else if !quiet && (role == "AXStaticText" || role == "AXHeading") && walk.texts < Limit.texts {
        let text = tidy(string(node, kAXValueAttribute) ?? string(node, kAXTitleAttribute), Limit.value)
        if !text.isEmpty {
            walk.texts += 1
            walk.lines.append(role == "AXHeading" ? "heading: \(text)" : "text: \(text)")
            if role == "AXHeading" { hush = true }
        }
    }
    for child in children(node, role) {
        visit(child, depth + 1, hush, &walk)
    }
}

func look() throws -> String {
    let (running, app) = try front()
    marked = []
    labels = []
    let others = NSWorkspace.shared.runningApplications
        .filter { $0.activationPolicy == .regular && $0.processIdentifier != running.processIdentifier }
        .compactMap(\.localizedName)
    var head = "App in front: \(running.localizedName ?? "unknown")"
    guard let target = window(app) else {
        if !others.isEmpty { head += "\nOther open apps: \(others.joined(separator: ", "))" }
        return head + "\nIt has no window open."
    }
    let title = tidy(string(target, kAXTitleAttribute), Limit.name)
    if !title.isEmpty { head += "\nWindow: \(title)" }
    if !others.isEmpty { head += "\nOther open apps: \(others.joined(separator: ", "))" }
    var walk = Walk(bounds: frame(target), until: Date().addingTimeInterval(Limit.seconds))
    visit(target, 0, false, &walk)
    if walk.lines.isEmpty {
        return head + "\nNothing in this window can be read. Use press and menu instead."
    }
    var out = head + "\n" + walk.lines.joined(separator: "\n")
    if walk.cut { out += "\n(there is more in this window than is listed)" }
    return out
}

func target(_ request: [String: Any]) throws -> (AXUIElement, String) {
    guard let number = (request["number"] as? NSNumber)?.intValue else { throw Failure(text: "give a number") }
    guard number >= 1, number <= marked.count else {
        throw Failure(text: "there is no [\(number)] on the screen; use a number from the latest screen")
    }
    return (marked[number - 1], labels[number - 1])
}

func raise(_ element: AXUIElement) {
    var pid: pid_t = 0
    AXUIElementGetPid(element, &pid)
    if let running = NSRunningApplication(processIdentifier: pid), !running.isActive {
        running.activate()
        usleep(Limit.settle)
    }
}

func tap(_ element: AXUIElement) throws {
    guard let box = frame(element), box.width > 0, box.height > 0 else {
        throw Failure(text: "it could not be clicked")
    }
    raise(element)
    let point = CGPoint(x: box.midX, y: box.midY)
    let source = CGEventSource(stateID: .hidSystemState)
    let kinds: [CGEventType] = [.mouseMoved, .leftMouseDown, .leftMouseUp]
    for kind in kinds {
        let event = CGEvent(mouseEventSource: source, mouseType: kind, mouseCursorPosition: point, mouseButton: .left)
        event?.post(tap: .cghidEventTap)
        usleep(40_000)
    }
}

func click(_ request: [String: Any]) throws -> String {
    let (element, label) = try target(request)
    let role = kind(element)
    if (request["real"] as? Bool) == true {
        try tap(element)
        return "clicked \(label)"
    }
    if inputs.contains(role) {
        if AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue) != .success {
            try tap(element)
        }
        return "clicked \(label)"
    }
    if rows.contains(role) {
        if AXUIElementSetAttributeValue(element, kAXSelectedAttribute as CFString, kCFBooleanTrue) != .success {
            try tap(element)
        }
        return "selected \(label)"
    }
    var names: CFArray?
    AXUIElementCopyActionNames(element, &names)
    let actions = (names as? [String]) ?? []
    if actions.contains(kAXPressAction as String),
        AXUIElementPerformAction(element, kAXPressAction as CFString) == .success
    {
        return "clicked \(label)"
    }
    try tap(element)
    return "clicked \(label)"
}

func stroke(_ code: Int, _ mask: CGEventFlags, _ pid: pid_t?) {
    let source = CGEventSource(stateID: .hidSystemState)
    for down in [true, false] {
        guard let event = CGEvent(keyboardEventSource: source, virtualKey: CGKeyCode(code), keyDown: down) else {
            continue
        }
        event.flags = mask
        if let pid { event.postToPid(pid) } else { event.post(tap: .cghidEventTap) }
        usleep(Limit.key)
    }
}

func write(_ request: [String: Any]) throws -> String {
    guard let text = request["text"] as? String, !text.isEmpty else { throw Failure(text: "give the text to type") }
    var pid: pid_t = 0
    var place = "where the cursor is"
    if request["number"] is NSNumber {
        let (element, label) = try target(request)
        if AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue) != .success {
            try tap(element)
        }
        usleep(150_000)
        AXUIElementGetPid(element, &pid)
        place = "into \(label)"
    } else {
        pid = try front().0.processIdentifier
    }
    let source = CGEventSource(stateID: .hidSystemState)
    for (index, line) in text.components(separatedBy: "\n").enumerated() {
        if index > 0 { stroke(kVK_Return, [], pid) }
        var units = Array(line.utf16)
        while !units.isEmpty {
            var take = min(Limit.chunk, units.count)
            if take < units.count, UTF16.isLeadSurrogate(units[take - 1]) { take -= 1 }
            let piece = Array(units.prefix(take))
            units.removeFirst(take)
            for down in [true, false] {
                guard let event = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down) else { continue }
                event.flags = []
                event.keyboardSetUnicodeString(stringLength: piece.count, unicodeString: piece)
                event.postToPid(pid)
                usleep(Limit.key)
            }
        }
    }
    return "typed \"\(tidy(text, Limit.name))\" \(place)"
}

func press(_ request: [String: Any]) throws -> String {
    guard let combo = request["keys"] as? String, !combo.isEmpty else { throw Failure(text: "give the keys") }
    var mask: CGEventFlags = []
    var code: Int?
    let parts = combo.lowercased().replacingOccurrences(of: " ", with: "").split(separator: "+").map(String.init)
    for part in parts.isEmpty ? ["+"] : parts {
        if let flag = flags[part] {
            mask.insert(flag)
        } else if let found = keys[part] {
            code = found
        } else {
            throw Failure(text: "\(part) is not a key; use names like cmd+n, return, escape, tab, down, pagedown")
        }
    }
    guard let code else { throw Failure(text: "\(combo) has no key, only modifiers") }
    stroke(code, mask, nil)
    return "pressed \(combo)"
}

func plain(_ title: String) -> String {
    title.lowercased().replacingOccurrences(of: "…", with: "").replacingOccurrences(of: "...", with: "")
        .trimmingCharacters(in: .whitespaces)
}

func menu(_ request: [String: Any]) throws -> String {
    guard let path = request["path"] as? String else { throw Failure(text: "give the menu path") }
    let wanted = path.split(separator: ">").map { plain(String($0)) }.filter { !$0.isEmpty }
    guard !wanted.isEmpty else { throw Failure(text: "give the menu path, like File > New Window") }
    let (_, app) = try front()
    guard var node = element(app, kAXMenuBarAttribute) else { throw Failure(text: "this app has no menu bar") }
    for (index, name) in wanted.enumerated() {
        var items = elements(node, kAXChildrenAttribute)
        if items.count == 1, kind(items[0]) == "AXMenu" { items = elements(items[0], kAXChildrenAttribute) }
        let titles = items.compactMap { string($0, kAXTitleAttribute) }.filter { !$0.isEmpty }
        guard let hit = items.first(where: { plain(string($0, kAXTitleAttribute) ?? "") == name })
            ?? items.first(where: { plain(string($0, kAXTitleAttribute) ?? "").hasPrefix(name) })
        else {
            let have = titles.prefix(Limit.listed).joined(separator: ", ")
            throw Failure(text: "there is no \(name) there; the choices are: \(have)")
        }
        if index + 1 == wanted.count {
            if (attribute(hit, kAXEnabledAttribute) as? Bool) == false {
                throw Failure(text: "\(path) is turned off right now")
            }
            guard AXUIElementPerformAction(hit, kAXPressAction as CFString) == .success else {
                throw Failure(text: "\(path) could not be chosen")
            }
        }
        node = hit
    }
    return "chose \(path)"
}

func handle(_ request: [String: Any]) throws -> String {
    let prompt = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
    guard AXIsProcessTrustedWithOptions(prompt) else {
        throw Failure(
            text:
                "Accessibility is turned off for the terminal app, so nothing on the screen can be used. They need to turn it on in System Settings, under Privacy and Security, Accessibility, and then start milo again"
        )
    }
    switch request["op"] as? String {
    case "look": return try look()
    case "click": return try click(request)
    case "type": return try write(request)
    case "press": return try press(request)
    case "menu": return try menu(request)
    default: throw Failure(text: "there is no such action")
    }
}

@_cdecl("milo_control_call")
public func call(message: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>? {
    let data = Data(String(cString: message).utf8)
    let request = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
    var answer: [String: Any]
    do {
        answer = ["ok": true, "text": try handle(request)]
    } catch let failure as Failure {
        answer = ["ok": false, "text": failure.text]
    } catch {
        answer = ["ok": false, "text": "it did not work"]
    }
    let out = (try? JSONSerialization.data(withJSONObject: answer)) ?? Data("{}".utf8)
    return strdup(String(decoding: out, as: UTF8.self))
}
