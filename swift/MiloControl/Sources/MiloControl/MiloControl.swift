import AppKit
import ApplicationServices
import Carbon.HIToolbox
import ImageIO
import ScreenCaptureKit

enum Limit {
    static let depth = 60
    static let controls = 150
    static let texts = 60
    static let visited = 6000
    static let seconds = 2.5
    static let name = 90
    static let value = 140
    static let label = 4
    static let chunk = 16
    static let messaging: Float = 1.5
    static let settle: UInt32 = 500_000
    static let key: UInt32 = 12_000
    static let pressed: Float = 1.2
    static let listed = 30
    static let captured: CGFloat = 2400
    static let sent: CGFloat = 640_000
    static let quality = 0.72
    static let print = 24
    static let capture = 6.0
    static let launch = 4.0
    static let small: CGFloat = 5
    static let closer: CGFloat = 3
    static let build = 3.0
    static let brief: Float = 0.3
    static let swap: UInt32 = 10_000
    static let taken: UInt32 = 250_000
    static let glance: CGFloat = 320
    static let sought = 1500
    static let around = 24
    static let spared = 40
    static let thin: CGFloat = 3
    static let mine = CGSize(width: 1120, height: 760)
    static let read: UInt32 = 200_000
}

struct Failure: Error {
    let text: String
}

let inputs: Set<String> = ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"]
let pressable: Set<String> = [
    "AXButton", "AXLink", "AXCheckBox", "AXRadioButton", "AXPopUpButton", "AXMenuButton", "AXSlider", "AXMenuItem",
    "AXDisclosureTriangle", "AXIncrementor", "AXSwitch", "AXToggle", "AXTab", "AXColorWell", "AXDockItem",
]
let rows: Set<String> = ["AXRow", "AXOutlineRow", "AXFile"]
let words: [String: String] = [
    "AXButton": "button", "AXLink": "link", "AXCheckBox": "checkbox", "AXRadioButton": "option",
    "AXPopUpButton": "pop-up", "AXMenuButton": "menu button", "AXSlider": "slider", "AXMenuItem": "menu item",
    "AXDisclosureTriangle": "disclosure", "AXIncrementor": "stepper", "AXSwitch": "switch", "AXToggle": "switch",
    "AXTab": "tab", "AXColorWell": "color", "AXTextField": "text field", "AXTextArea": "text area",
    "AXComboBox": "combo box", "AXSearchField": "search field", "AXRow": "row", "AXOutlineRow": "row",
    "AXDockItem": "dock item", "AXFile": "file",
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
nonisolated(unsafe) var boxes: [CGRect?] = []
nonisolated(unsafe) var aimed: pid_t?
nonisolated(unsafe) var pane = CGRect.zero
nonisolated(unsafe) var pane_id: CGWindowID = 0
nonisolated(unsafe) var shot: CGImage?

@_silgen_name("_AXUIElementGetWindow")
func _AXUIElementGetWindow(_ element: AXUIElement, _ id: UnsafeMutablePointer<CGWindowID>) -> AXError

typealias Post = @convention(c) (pid_t, CGEvent) -> Void
typealias Record = @convention(c) (UnsafeRawPointer, UnsafeRawPointer) -> Int32
typealias Front = @convention(c) (UnsafeMutableRawPointer) -> Int32
typealias Serial = @convention(c) (pid_t, UnsafeMutableRawPointer) -> Int32
typealias Owner = @convention(c) (UnsafeRawPointer, UnsafeMutablePointer<pid_t>) -> Int32

let skylight = dlopen("/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight", RTLD_LAZY)
let everywhere = UnsafeMutableRawPointer(bitPattern: -2)
let post: Post? = dlsym(skylight, "SLEventPostToPid").map { unsafeBitCast($0, to: Post.self) }
let record: Record? = dlsym(skylight, "SLPSPostEventRecordTo").map { unsafeBitCast($0, to: Record.self) }
let foremost: Front? = dlsym(skylight, "_SLPSGetFrontProcess").map { unsafeBitCast($0, to: Front.self) }
let owner: Owner? = dlsym(everywhere, "GetProcessPID").map { unsafeBitCast($0, to: Owner.self) }
let serial: Serial? = dlsym(everywhere, "GetProcessForPID").map { unsafeBitCast($0, to: Serial.self) }

final class Held: @unchecked Sendable {
    var image: CGImage?
}
nonisolated(unsafe) var woken: Set<pid_t> = []
nonisolated(unsafe) var fresh = false
nonisolated(unsafe) var pinned: CGWindowID?
nonisolated(unsafe) var roster: Set<CGWindowID> = []
nonisolated(unsafe) var nook: CGWindowID?
nonisolated(unsafe) var held: pid_t?
nonisolated(unsafe) var known: Set<pid_t> = []

func leader() -> pid_t? {
    var ahead = [UInt32](repeating: 0, count: 2)
    var pid: pid_t = 0
    guard let foremost, let owner, foremost(&ahead) == 0, owner(ahead, &pid) == 0, pid > 0 else {
        return NSWorkspace.shared.frontmostApplication?.processIdentifier
    }
    return pid
}

func apps() -> [NSRunningApplication] {
    var pids = Set(NSWorkspace.shared.runningApplications.map(\.processIdentifier))
    let listed = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
    for entry in listed {
        if let pid = (entry[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value { pids.insert(pid) }
    }
    return pids.compactMap { NSRunningApplication(processIdentifier: $0) }.filter { !$0.isTerminated }
}

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

func wake(_ pid: pid_t, _ app: AXUIElement) {
    AXUIElementSetMessagingTimeout(app, Limit.messaging)
    if woken.contains(pid) { return }
    woken.insert(pid)
    let manual = AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    let enhanced = AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
    if manual == .success || enhanced == .success { usleep(Limit.settle) }
    fresh = manual == .success
}

func front() throws -> (NSRunningApplication, AXUIElement) {
    if let aimed, let running = NSRunningApplication(processIdentifier: aimed), !running.isTerminated {
        let app = AXUIElementCreateApplication(aimed)
        wake(aimed, app)
        return (running, app)
    }
    throw Failure(text: "no app has been opened yet; start with open")
}

func parent(_ pid: pid_t) -> pid_t {
    var info = kinfo_proc()
    var size = MemoryLayout<kinfo_proc>.stride
    var name: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid]
    guard sysctl(&name, 4, &info, &size, nil, 0) == 0, size > 0 else { return 0 }
    return info.kp_eproc.e_ppid
}

func host() -> pid_t? {
    let regular = Set(apps().filter { $0.activationPolicy == .regular }.map(\.processIdentifier))
    var pid = getpid()
    for _ in 0..<Limit.listed {
        if regular.contains(pid) { return pid }
        pid = parent(pid)
        if pid <= 1 { return nil }
    }
    return nil
}

func around() -> [String: Any] {
    let ahead = leader()
    let home = host()
    var out: [[String: Any]] = []
    for running in apps() where running.activationPolicy == .regular {
        let pid = running.processIdentifier
        let app = AXUIElementCreateApplication(pid)
        let main = element(app, kAXFocusedWindowAttribute) ?? element(app, kAXMainWindowAttribute)
        let count = elements(app, kAXWindowsAttribute).count
        if count == 0 && pid != ahead { continue }
        out.append([
            "name": running.localizedName ?? "",
            "pid": Int(pid),
            "front": pid == ahead,
            "host": pid == home,
            "aimed": pid == aimed,
            "title": tidy(main.flatMap { string($0, kAXTitleAttribute) }, Limit.name),
        ])
    }
    return ["text": "", "apps": out]
}

func facing() throws -> String {
    guard let pid = leader(), let running = NSRunningApplication(processIdentifier: pid),
        running.activationPolicy == .regular
    else { throw Failure(text: "there is no window in front to look at") }
    aimed = pid
    pinned = nil
    return running.localizedName ?? ""
}

func kept() -> [String: Any] {
    guard let pid = aimed, let running = NSRunningApplication(processIdentifier: pid), !running.isTerminated else {
        aimed = nil
        pinned = nil
        return ["text": ""]
    }
    let app = AXUIElementCreateApplication(pid)
    if let had = pinned, !elements(app, kAXWindowsAttribute).contains(where: { number($0) == had }) {
        aimed = nil
        pinned = nil
        return ["text": ""]
    }
    return ["text": running.localizedName ?? ""]
}

func raise() throws -> String {
    let (running, app) = try front()
    if let target = window(app) {
        AXUIElementPerformAction(target, kAXRaiseAction as CFString)
    }
    forward(running.processIdentifier)
    return "brought \(running.localizedName ?? "it") to the front"
}

func number(_ window: AXUIElement) -> CGWindowID {
    var id: CGWindowID = 0
    _ = _AXUIElementGetWindow(window, &id)
    return id
}

func aim(_ request: [String: Any]) throws -> String {
    let what = (request["what"] as? String ?? "").trimmingCharacters(in: .whitespaces)
    let until = Date().addingTimeInterval(Limit.launch)
    let site = what.contains(".") && !what.contains(" ")
    while true {
        let running = apps().filter { $0.activationPolicy == .regular }
        var hit: NSRunningApplication?
        if site {
            let address = what.contains("://") ? what : "https://\(what)"
            if let url = URL(string: address), let browser = NSWorkspace.shared.urlForApplication(toOpen: url) {
                let id = Bundle(url: browser)?.bundleIdentifier
                hit = running.first { $0.bundleIdentifier == id }
            }
        } else {
            let name = what.lowercased()
            let file = { (app: NSRunningApplication) in
                app.bundleURL?.deletingPathExtension().lastPathComponent.lowercased() ?? ""
            }
            hit = running.first { $0.localizedName?.lowercased() == name || file($0) == name }
                ?? running.first { $0.localizedName?.lowercased().contains(name) == true || file($0).contains(name) }
                ?? running.first { ($0.localizedName?.lowercased()).map { $0.count > 2 && name.contains($0) } == true }
        }
        if let hit {
            if aimed != hit.processIdentifier { pinned = nil }
            aimed = hit.processIdentifier
            restore(hit.processIdentifier)
            return "working in \(hit.localizedName ?? what)"
        }
        if Date() > until || request["wait"] as? Bool == false {
            aimed = nil
            pinned = nil
            throw Failure(text: "\(what) is not open")
        }
        usleep(200_000)
    }
}

func installed(_ request: [String: Any]) throws -> String {
    let name = (request["what"] as? String ?? "").lowercased().trimmingCharacters(in: .whitespaces)
    guard !name.isEmpty else { throw Failure(text: "give the app's name") }
    let home = FileManager.default.homeDirectoryForCurrentUser.path
    var names: [String] = []
    for folder in ["/Applications", "/System/Applications", "/System/Applications/Utilities", "\(home)/Applications"] {
        for file in (try? FileManager.default.contentsOfDirectory(atPath: folder)) ?? [] where file.hasSuffix(".app") {
            names.append(String(file.dropLast(4)))
        }
    }
    let words = { (text: String) in Set(text.lowercased().split(whereSeparator: { !$0.isLetter && !$0.isNumber })) }
    let wanted = words(name)
    let hit = names.first { $0.lowercased() == name }
        ?? names.filter { wanted.isSubset(of: words($0)) }.min { $0.count < $1.count }
        ?? names.filter { $0.lowercased().contains(name) }.min { $0.count < $1.count }
    guard let hit else { throw Failure(text: "there is no app called \(name) on this Mac") }
    return hit
}

func window(_ app: AXUIElement) -> AXUIElement? {
    let all = elements(app, kAXWindowsAttribute)
    let ids = Set(all.map(number))
    if let pinned, let kept = all.first(where: { number($0) == pinned }) {
        let ahead = element(app, kAXFocusedWindowAttribute)
        guard let ahead, number(ahead) != pinned, !roster.contains(number(ahead)), ids.contains(number(ahead)) else {
            return kept
        }
        repin(ahead, ids)
        return ahead
    }
    let found = element(app, kAXFocusedWindowAttribute) ?? element(app, kAXMainWindowAttribute) ?? all.first
    if let found { repin(found, ids) } else { pinned = nil }
    return found
}

func repin(_ window: AXUIElement, _ ids: Set<CGWindowID>) {
    pinned = number(window)
    roster = ids
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
    var skip: AXUIElement?
    var around = false
    var hidden = 0
    var plain = 0
    var most = Limit.controls
    var holder = ""
    var unseen = 0
    var last = ""
    let bounds: CGRect?
    let until: Date
}

func page(_ window: AXUIElement) -> AXUIElement? {
    var queue = [window]
    var seen = 0
    while !queue.isEmpty && seen < Limit.sought {
        let node = queue.removeFirst()
        seen += 1
        let role = string(node, kAXRoleAttribute) ?? ""
        if role == "AXWebArea" { return node }
        if role == "AXMenuBar" { continue }
        queue.append(contentsOf: elements(node, kAXChildrenAttribute))
    }
    return nil
}

func loaded(_ page: AXUIElement?) -> Bool {
    guard let page else { return true }
    if let done = attribute(page, "AXLoaded") as? Bool { return done }
    if let progress = attribute(page, "AXLoadingProgress") as? NSNumber { return progress.doubleValue >= 1 }
    return true
}

func kind(_ element: AXUIElement) -> String {
    let role = string(element, kAXRoleAttribute) ?? ""
    let sub = string(element, kAXSubroleAttribute) ?? ""
    if sub == "AXSearchField" { return "AXSearchField" }
    if sub == "AXTabButton" { return "AXTab" }
    if sub == "AXSwitch" || sub == "AXToggle" { return "AXSwitch" }
    if sub == "AXOutlineRow" { return "AXOutlineRow" }
    if role == "AXImage", attribute(element, "AXFilename") != nil || attribute(element, "AXURL") != nil {
        return "AXFile"
    }
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

func popped(_ app: AXUIElement, _ window: AXUIElement) -> AXUIElement? {
    guard var node = element(app, kAXFocusedUIElementAttribute) else { return nil }
    var menu: AXUIElement?
    for _ in 0..<Limit.depth {
        let role = kind(node)
        if role == "AXMenuBar" || CFEqual(node, window) { return nil }
        if role == "AXMenu" { menu = node }
        guard let parent = element(node, kAXParentAttribute) else { break }
        node = parent
    }
    return menu
}

func shown(_ node: AXUIElement, _ walk: inout Walk) -> Bool {
    guard let bounds = walk.bounds, let box = frame(node) else { return true }
    let part = box.intersection(bounds)
    if !part.isNull, part.width >= Limit.thin, part.height >= Limit.thin { return true }
    walk.unseen += 1
    return false
}

func visit(_ node: AXUIElement, _ depth: Int, _ quiet: Bool, _ above: String, _ walk: inout Walk) {
    if depth > Limit.depth { return }
    walk.visited += 1
    if walk.visited > Limit.visited || marked.count >= walk.most || Date() > walk.until {
        walk.cut = true
        return
    }
    if let skip = walk.skip, CFEqual(skip, node) { return }
    let role = kind(node)
    if role == "AXMenuBar" { return }
    var above = above
    var inside: String?
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
        var label = "\(words[role] ?? "control")\(name.isEmpty ? "" : " \"\(name)\"")"
        let bare = label
        if !walk.holder.isEmpty && label != walk.holder && role == "AXButton" { label += " of \(walk.holder)" }
        let chosen = (attribute(node, kAXSelectedAttribute) as? Bool) == true
        let tab = role == "AXTab" || role == "AXRadioButton"
        if walk.around && !tab && !inputs.contains(role) { walk.plain += 1 }
        let full = walk.around && !tab && !inputs.contains(role) && walk.plain > Limit.around
        if full { walk.hidden += 1 }
        let echo = role == "AXLink" && !name.isEmpty && walk.holder.lowercased().contains(name.lowercased())
        if bare == above || full || echo {
            hush = true
        } else if (!name.isEmpty || inputs.contains(role)) && !shown(node, &walk) {
            hush = true
        } else if !name.isEmpty || inputs.contains(role) {
            above = bare
            if role == "AXTab" || role == "AXRadioButton" || rows.contains(role) { inside = bare }
            marked.append(node)
            boxes.append(frame(node))
            var line = "[\(marked.count)] \(bare)"

            if inputs.contains(role) || role == "AXSlider" || role == "AXPopUpButton" {
                let value = tidy(string(node, kAXValueAttribute), Limit.value)
                if !value.isEmpty && value != name { line += " containing \"\(value)\"" }
            }
            if role == "AXCheckBox" || role == "AXSwitch" || role == "AXRadioButton" || role == "AXTab" {
                let value = (attribute(node, kAXValueAttribute) as? NSNumber)?.intValue ?? 0
                line += value == 0 ? " (off)" : " (on)"
            }
            if chosen { line += " (selected)" }
            if (attribute(node, kAXFocusedAttribute) as? Bool) == true { line += " (focused)" }
            if (attribute(node, kAXEnabledAttribute) as? Bool) == false { line += " (disabled)" }
            walk.lines.append(line)
            walk.last = name
            labels.append(label)
            hush = true
            if inputs.contains(role) { return }
        }
    } else if !quiet && (role == "AXStaticText" || role == "AXHeading") && walk.texts < Limit.texts {
        var text = tidy(string(node, role == "AXHeading" ? kAXTitleAttribute : kAXValueAttribute), Limit.value)
        if text.isEmpty && role == "AXHeading" {
            var parts: [String] = []
            gather(node, 0, &parts)
            text = tidy(parts.joined(separator: " "), Limit.value)
        }
        if text.isEmpty && role != "AXHeading" { text = tidy(string(node, kAXTitleAttribute), Limit.value) }
        if !text.isEmpty && text != walk.last && shown(node, &walk) {
            walk.texts += 1
            walk.lines.append(role == "AXHeading" ? "heading: \(text)" : "text: \(text)")
            if role == "AXHeading" { hush = true }
        }
    }
    let outer = walk.holder
    if let inside { walk.holder = inside }
    for child in children(node, role) {
        visit(child, depth + 1, hush, above, &walk)
    }
    walk.holder = outer
}

func capture(_ id: CGWindowID, _ size: CGSize, _ longest: CGFloat = Limit.captured) throws -> CGImage {
    guard CGPreflightScreenCaptureAccess() else {
        throw Failure(
            text:
                "Screen Recording is turned off for Milo, so the screen cannot be seen. They need to turn it on in Milo's settings, under Permissions, and then start Milo again"
        )
    }
    let scale = min(2, longest / max(size.width, size.height, 1))
    let held = Held()
    let done = DispatchSemaphore(value: 0)
    Task.detached {
        defer { done.signal() }
        guard let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false),
            let window = content.windows.first(where: { $0.windowID == id })
        else { return }
        let setup = SCStreamConfiguration()
        setup.width = Int(size.width * scale)
        setup.height = Int(size.height * scale)
        setup.showsCursor = false
        setup.ignoreShadowsSingleWindow = true
        let filter = SCContentFilter(desktopIndependentWindow: window)
        held.image = try? await SCScreenshotManager.captureImage(contentFilter: filter, configuration: setup)
    }
    _ = done.wait(timeout: .now() + Limit.capture)
    guard let image = held.image else { throw Failure(text: "the window could not be seen; it may be minimized") }
    return image
}

func render(_ image: CGImage) -> Data? {
    let scale = (Limit.sent / max(CGFloat(image.width * image.height), 1)).squareRoot()
    let width = Int(CGFloat(image.width) * scale)
    let height = Int(CGFloat(image.height) * scale)
    guard width > 0, height > 0,
        let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
    else { return nil }
    context.interpolationQuality = .high
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    guard let drawn = context.makeImage() else { return nil }
    let data = NSMutableData()
    guard let out = CGImageDestinationCreateWithData(data, "public.jpeg" as CFString, 1, nil) else { return nil }
    CGImageDestinationAddImage(out, drawn, [kCGImageDestinationLossyCompressionQuality: Limit.quality] as CFDictionary)
    return CGImageDestinationFinalize(out) ? data as Data : nil
}

func print(_ image: CGImage) -> String {
    let side = Limit.print
    var pixels = [UInt8](repeating: 0, count: side * side)
    let context = CGContext(
        data: &pixels, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side,
        space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)
    context?.interpolationQuality = .medium
    context?.draw(image, in: CGRect(x: 0, y: 0, width: side, height: side))
    return pixels.map { String(format: "%02x", $0) }.joined()
}

func look(_ request: [String: Any]) throws -> [String: Any] {
    let (running, app) = try front()
    marked = []
    labels = []
    boxes = []
    var head = "App: \(running.localizedName ?? "unknown")"
    guard let target = window(app), let bounds = frame(target), bounds.width > 1, bounds.height > 1 else {
        return ["text": head + "\nIt has no window open. Use menu or press to open one."]
    }
    pane = bounds
    pane_id = number(target)
    let title = tidy(string(target, kAXTitleAttribute), Limit.name)
    if !title.isEmpty { head += "\nWindow: \(title)" }
    let until = Date().addingTimeInterval(Limit.seconds)
    var walk = Walk(bounds: bounds, until: until)
    var web = page(target)
    let patience = Date().addingTimeInterval(Limit.build)
    while fresh && web == nil && Date() < patience {
        usleep(Limit.read)
        web = page(target)
    }
    fresh = false
    walk.most = Limit.controls - Limit.spared
    if let web { visit(web, 0, false, "", &walk) }
    var rest = Walk(bounds: bounds, until: Date().addingTimeInterval(Limit.seconds / 2))
    rest.skip = web
    rest.around = web != nil
    visit(target, 0, web != nil, "", &rest)
    if web != nil && !rest.lines.isEmpty {
        walk.lines.append("Around the page, in the app's own window:")
    }
    walk.lines.append(contentsOf: rest.lines)
    if let now = element(app, kAXFocusedUIElementAttribute), inputs.contains(kind(now)),
        !marked.contains(where: { CFEqual($0, now) }),
        element(now, kAXWindowAttribute).map({ number($0) == pane_id }) ?? true
    {
        marked.append(now)
        boxes.append(frame(now))
        let word = words[kind(now)] ?? "text field"
        labels.append(word)
        let value = tidy(string(now, kAXValueAttribute), Limit.value)
        walk.lines.append(
            "[\(marked.count)] \(word)\(value.isEmpty ? "" : " containing \"\(value)\"") (focused) that is open for typing now"
        )
    }
    if let open = popped(app, target) {
        var extra = Walk(bounds: nil, until: Date().addingTimeInterval(Limit.seconds / 2))
        visit(open, 0, false, "", &extra)
        if !extra.lines.isEmpty {
            walk.lines.append("A menu is open on top of the window. Choose from it by number, or press escape to close it:")
            walk.lines.append(contentsOf: extra.lines)
        }
    }
    if rest.hidden > 0 { walk.lines.append("(\(rest.hidden) more buttons of the app are not listed)") }
    walk.cut = walk.cut || rest.cut
    if !loaded(web) { walk.lines.append("(the page is still loading)") }
    let image = try capture(pane_id, bounds.size)
    shot = image
    if walk.lines.isEmpty {
        head += "\nNo controls could be read in this window. Use the picture, and click what you see by describing it."
    } else {
        head += "\n" + walk.lines.joined(separator: "\n")
        if walk.cut { head += "\n(there is more in this window than is listed)" }
    }
    var out: [String: Any] = ["text": head, "print": print(image), "web": web != nil]
    if let data = render(image) {
        out["image"] = data.base64EncodedString()
        if let save = request["save"] as? String { try? data.write(to: URL(fileURLWithPath: save)) }
    }
    return out
}

func closer(_ request: [String: Any]) throws -> [String: Any] {
    guard let shot, let x = (request["x"] as? NSNumber)?.doubleValue,
        let y = (request["y"] as? NSNumber)?.doubleValue
    else { throw Failure(text: "there is no picture to look closer at") }
    let share = 1 / Limit.closer
    let left = min(max(x - share / 2, 0), 1 - share)
    let top = min(max(y - share / 2, 0), 1 - share)
    let area = CGRect(
        x: left * CGFloat(shot.width), y: top * CGFloat(shot.height), width: share * CGFloat(shot.width),
        height: share * CGFloat(shot.height))
    guard let piece = shot.cropping(to: area), let data = render(piece) else {
        throw Failure(text: "the picture could not be cut")
    }
    if let save = request["save"] as? String { try? data.write(to: URL(fileURLWithPath: save)) }
    return ["text": "", "image": data.base64EncodedString(), "left": left, "top": top, "share": share]
}

func glance() throws -> [String: Any] {
    let (_, app) = try front()
    guard let target = window(app), let bounds = frame(target), bounds.width > 1, bounds.height > 1 else {
        return ["text": "", "print": "", "loaded": true]
    }
    let image = try capture(number(target), bounds.size, Limit.glance)
    return ["text": "", "print": print(image), "loaded": loaded(page(target))]
}

func target(_ request: [String: Any]) throws -> (AXUIElement, String, CGRect?) {
    guard let number = (request["number"] as? NSNumber)?.intValue else { throw Failure(text: "give a number") }
    guard number >= 1, number <= marked.count else {
        throw Failure(text: "there is no [\(number)] on the screen; use a number from the latest screen")
    }
    return (marked[number - 1], labels[number - 1], boxes[number - 1])
}

func psn(_ pid: pid_t) -> [UInt32]? {
    var out = [UInt32](repeating: 0, count: 2)
    guard let serial, serial(pid, &out) == 0 else { return nil }
    return out
}

func tell(_ process: [UInt32], _ window: CGWindowID, _ mode: UInt8) {
    guard let record else { return }
    var bytes = [UInt8](repeating: 0, count: 0xf8)
    bytes[0x04] = 0xf8
    bytes[0x08] = 0x0d
    bytes[0x8a] = mode
    withUnsafeBytes(of: window) { raw in
        for (offset, byte) in raw.enumerated() { bytes[0x3c + offset] = byte }
    }
    _ = record(process, bytes)
}

func crown(_ process: [UInt32], _ window: CGWindowID) {
    guard let record else { return }
    for stage in [UInt8(0x01), UInt8(0x02)] {
        var bytes = [UInt8](repeating: 0, count: 0xf8)
        bytes[0x04] = 0xf8
        bytes[0x08] = stage
        bytes[0x3a] = 0x10
        for offset in 0x20..<0x30 { bytes[offset] = 0xff }
        withUnsafeBytes(of: window) { raw in
            for (offset, byte) in raw.enumerated() { bytes[0x3c + offset] = byte }
        }
        _ = record(process, bytes)
    }
}

func borrow(_ pid: pid_t, _ work: () -> Void) {
    var ahead = [UInt32](repeating: 0, count: 2)
    guard let foremost, foremost(&ahead) == 0, let mine = psn(pid), mine != ahead else {
        work()
        return
    }
    var theirs: CGWindowID = 0
    if let other = leader() {
        let app = AXUIElementCreateApplication(other)
        AXUIElementSetMessagingTimeout(app, Limit.brief)
        if let focused = element(app, kAXFocusedWindowAttribute) { theirs = number(focused) }
    }
    tell(ahead, theirs, 0x02)
    usleep(Limit.swap)
    tell(mine, pane_id, 0x01)
    crown(mine, pane_id)
    usleep(Limit.taken)
    work()
    usleep(Limit.taken)
    tell(mine, pane_id, 0x02)
    usleep(Limit.swap)
    tell(ahead, theirs, 0x01)
}

func forward(_ pid: pid_t) {
    let app = AXUIElementCreateApplication(pid)
    if AXUIElementSetAttributeValue(app, kAXFrontmostAttribute as CFString, kCFBooleanTrue) != .success {
        NSRunningApplication(processIdentifier: pid)?.activate()
    }
}

func send(_ event: CGEvent, _ pid: pid_t) {
    if let post { post(pid, event) } else { event.postToPid(pid) }
}

func poke(_ point: CGPoint, _ pid: pid_t) {
    let source = CGEventSource(stateID: .hidSystemState)
    borrow(pid) {
        let kinds: [CGEventType] = [.mouseMoved, .leftMouseDown, .leftMouseUp]
        for kind in kinds {
            guard
                let event = CGEvent(
                    mouseEventSource: source, mouseType: kind, mouseCursorPosition: point, mouseButton: .left)
            else { continue }
            event.setIntegerValueField(.mouseEventClickState, value: kind == .mouseMoved ? 0 : 1)
            event.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(pane_id))
            event.setIntegerValueField(
                .mouseEventWindowUnderMousePointerThatCanHandleThisEvent, value: Int64(pane_id))
            event.setIntegerValueField(.eventTargetUnixProcessID, value: Int64(pid))
            send(event, pid)
            usleep(40_000)
        }
    }
}

func tap(_ point: CGPoint, _ pid: pid_t) {
    let before = leader()
    let cursor = CGEvent(source: nil)?.location
    if before != pid {
        forward(pid)
        usleep(Limit.settle)
    }
    let source = CGEventSource(stateID: .hidSystemState)
    let kinds: [CGEventType] = [.mouseMoved, .leftMouseDown, .leftMouseUp]
    for kind in kinds {
        let event = CGEvent(mouseEventSource: source, mouseType: kind, mouseCursorPosition: point, mouseButton: .left)
        event?.post(tap: .cghidEventTap)
        usleep(40_000)
    }
    usleep(120_000)
    if let cursor { CGWarpMouseCursorPosition(cursor) }
    if let before, before != pid { forward(before) }
}

func under(_ point: CGPoint, _ app: AXUIElement) -> (AXUIElement, String)? {
    var found: AXUIElement?
    guard AXUIElementCopyElementAtPosition(app, Float(point.x), Float(point.y), &found) == .success, var node = found
    else { return nil }
    if let home = element(node, kAXWindowAttribute), number(home) != pane_id { return nil }
    for _ in 0..<Limit.label {
        let role = kind(node)
        if pressable.contains(role) || inputs.contains(role) || rows.contains(role) {
            var name = tidy(string(node, kAXTitleAttribute), Limit.name)
            if name.isEmpty { name = tidy(string(node, kAXDescriptionAttribute), Limit.name) }
            if name.isEmpty {
                var parts: [String] = []
                gather(node, 0, &parts)
                name = tidy(parts.joined(separator: " · "), Limit.name)
            }
            return (node, "\(words[role] ?? "control")\(name.isEmpty ? "" : " \"\(name)\"")")
        }
        guard let parent = element(node, kAXParentAttribute) else { break }
        node = parent
    }
    return nil
}

func holding(_ wanted: AXUIElement, _ box: CGRect?, _ pid: pid_t) -> Bool {
    guard let now = element(AXUIElementCreateApplication(pid), kAXFocusedUIElementAttribute) else { return true }
    if CFEqual(now, wanted) { return true }
    guard inputs.contains(kind(now)) else { return true }
    guard let box, let other = frame(now) else { return false }
    return other.intersects(box)
}

func focus(_ wanted: AXUIElement, _ box: CGRect?, _ pid: pid_t) -> Bool {
    _ = AXUIElementSetAttributeValue(wanted, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    usleep(120_000)
    if holding(wanted, box, pid) { return true }
    guard let box, box.width > 0, box.height > 0 else { return false }
    let point = CGPoint(x: box.midX, y: box.midY)
    for real in [false, true] {
        if real { tap(point, pid) } else { poke(point, pid) }
        usleep(220_000)
        if holding(wanted, box, pid) { return true }
    }
    return false
}

func tail(_ area: AXUIElement) {
    var end = CFRange(location: (string(area, kAXValueAttribute) ?? "").utf16.count, length: 0)
    if let range = AXValueCreate(.cfRange, &end) {
        AXUIElementSetAttributeValue(area, kAXSelectedTextRangeAttribute as CFString, range)
        usleep(Limit.key * 4)
    }
}

func strike(_ point: CGPoint, _ pid: pid_t, _ mode: String) {
    if mode == "real" { tap(point, pid) } else { poke(point, pid) }
}

func click(_ request: [String: Any]) throws -> String {
    let mode = request["mode"] as? String ?? ""
    let pid = try front().0.processIdentifier
    let element: AXUIElement
    let label: String
    var box: CGRect?
    if let x = (request["x"] as? NSNumber)?.doubleValue, let y = (request["y"] as? NSNumber)?.doubleValue {
        let point = CGPoint(x: pane.minX + pane.width * x, y: pane.minY + pane.height * y)
        guard pane.contains(point) else { throw Failure(text: "that spot is outside the window") }
        guard let hit = under(point, AXUIElementCreateApplication(pid)) else {
            strike(point, pid, mode == "real" ? "real" : "quiet")
            return "clicked that spot"
        }
        (element, label) = hit
        box = CGRect(x: point.x - 1, y: point.y - 1, width: 2, height: 2)
    } else {
        (element, label, box) = try target(request)
    }
    let role = kind(element)
    if !mode.isEmpty {
        guard let box, box.width > 0, box.height > 0 else { throw Failure(text: "it could not be clicked") }
        strike(CGPoint(x: box.midX, y: box.midY), pid, mode)
        return "clicked \(label)"
    }
    if inputs.contains(role) {
        guard focus(element, box, pid) else {
            throw Failure(text: "the cursor would not go into \(label); it is still in another field")
        }
        if role == "AXTextArea" { tail(element) }
        return "clicked \(label), and the cursor is in it"
    }
    if rows.contains(role) {
        if let box, box.width > 0, box.height > 0 {
            poke(CGPoint(x: box.minX + min(box.width / 2, 60), y: box.midY), pid)
            return "clicked \(label)"
        }
        if AXUIElementSetAttributeValue(element, kAXSelectedAttribute as CFString, kCFBooleanTrue) == .success {
            return "selected \(label)"
        }
    }
    var names: CFArray?
    AXUIElementCopyActionNames(element, &names)
    let actions = (names as? [String]) ?? []
    if actions.contains(kAXPressAction as String) {
        AXUIElementSetMessagingTimeout(element, Limit.pressed)
        let pressed = AXUIElementPerformAction(element, kAXPressAction as CFString)
        AXUIElementSetMessagingTimeout(element, 0)
        if pressed == .success { return "clicked \(label)" }
        if pressed == .cannotComplete { return "clicked \(label), and it opened something that waits for a choice" }
    }
    guard let box, box.width > 0, box.height > 0 else { throw Failure(text: "it could not be clicked") }
    poke(CGPoint(x: box.midX, y: box.midY), pid)
    return "clicked \(label)"
}

func stroke(_ code: Int, _ mask: CGEventFlags, _ pid: pid_t) {
    let source = CGEventSource(stateID: .hidSystemState)
    for down in [true, false] {
        guard let event = CGEvent(keyboardEventSource: source, virtualKey: CGKeyCode(code), keyDown: down) else {
            continue
        }
        event.flags = mask
        send(event, pid)
        usleep(Limit.key)
    }
}

func scribe(_ text: String, _ pid: pid_t, _ single: Bool = false) {
    let source = CGEventSource(stateID: .hidSystemState)
    borrow(pid) {
        for (index, line) in text.components(separatedBy: "\n").enumerated() {
            if index > 0 { stroke(kVK_Return, [], pid) }
            var units = Array(line.utf16)
            while !units.isEmpty {
                var take = min(single ? 1 : Limit.chunk, units.count)
                if take < units.count, UTF16.isLeadSurrogate(units[take - 1]) { take -= 1 }
                let piece = Array(units.prefix(take))
                units.removeFirst(take)
                for down in [true, false] {
                    guard let event = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down) else {
                        continue
                    }
                    event.flags = []
                    event.keyboardSetUnicodeString(stringLength: piece.count, unicodeString: piece)
                    send(event, pid)
                    usleep(Limit.key)
                }
            }
        }
    }
}

func bar(_ window: AXUIElement) -> AXUIElement? {
    var queue = [window]
    var seen = 0
    while !queue.isEmpty && seen < Limit.sought {
        let node = queue.removeFirst()
        seen += 1
        let role = string(node, kAXRoleAttribute) ?? ""
        if role == "AXTextField" || role == "AXComboBox" { return node }
        if role == "AXWebArea" || role == "AXMenuBar" { continue }
        queue.append(contentsOf: elements(node, kAXChildrenAttribute))
    }
    return nil
}

func select(_ field: AXUIElement, _ pid: pid_t) {
    let had = string(field, kAXValueAttribute) ?? ""
    if had.isEmpty { return }
    var all = CFRange(location: 0, length: had.utf16.count)
    let range = AXValueCreate(.cfRange, &all)
    if range == nil
        || AXUIElementSetAttributeValue(field, kAXSelectedTextRangeAttribute as CFString, range!) != .success
    {
        borrow(pid) { stroke(kVK_ANSI_A, .maskCommand, pid) }
    }
    usleep(Limit.key * 6)
}

func visit(_ request: [String: Any]) throws -> String {
    let what = (request["what"] as? String ?? "").trimmingCharacters(in: .whitespaces)
    let address = what.contains("://") ? what : "https://\(what)"
    guard let url = URL(string: address), let browser = NSWorkspace.shared.urlForApplication(toOpen: url),
        let id = Bundle(url: browser)?.bundleIdentifier
    else { throw Failure(text: "there is no browser to open \(what)") }
    _ = hold()
    let find = { apps().first(where: { $0.bundleIdentifier == id }) }
    guard let running = find() else {
        let setup = NSWorkspace.OpenConfiguration()
        setup.activates = false
        NSWorkspace.shared.open([url], withApplicationAt: browser, configuration: setup, completionHandler: nil)
        let until = Date().addingTimeInterval(Limit.capture)
        while Date() < until {
            usleep(Limit.read)
            guard let started = find() else { continue }
            let app = AXUIElementCreateApplication(started.processIdentifier)
            guard let first = element(app, kAXFocusedWindowAttribute) ?? elements(app, kAXWindowsAttribute).first
            else { continue }
            aimed = started.processIdentifier
            nook = number(first)
            pinned = nook
            restore(started.processIdentifier)
            return "opened \(what)"
        }
        throw Failure(text: "the browser did not open")
    }
    let pid = running.processIdentifier
    let app = AXUIElementCreateApplication(pid)
    wake(pid, app)
    aimed = pid
    let windows = { elements(app, kAXWindowsAttribute) }
    if nook == nil || !windows().contains(where: { number($0) == nook }) {
        let had = Set(windows().map(number))
        pinned = nil
        if let any = window(app), let bounds = frame(any) {
            pane = bounds
            pane_id = number(any)
        }
        borrow(pid) { stroke(kVK_ANSI_N, .maskCommand, pid) }
        let until = Date().addingTimeInterval(Limit.build)
        var made: CGWindowID?
        while made == nil && Date() < until {
            usleep(Limit.read)
            made = windows().map(number).first(where: { !had.contains($0) && $0 != 0 })
        }
        restore(pid, true)
        guard let made else { throw Failure(text: "the browser did not open a window") }
        nook = made
        if let fresh = windows().first(where: { number($0) == made }) {
            var size = Limit.mine
            if let value = AXValueCreate(.cgSize, &size) {
                AXUIElementSetAttributeValue(fresh, kAXSizeAttribute as CFString, value)
            }
        }
    }
    pinned = nook
    guard let target = window(app), let bounds = frame(target), bounds.width > 1 else {
        throw Failure(text: "the browser has no window")
    }
    pane = bounds
    pane_id = number(target)
    guard let bar = bar(target) else { throw Failure(text: "the browser's address field could not be found") }
    AXUIElementSetAttributeValue(bar, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    usleep(Limit.read)
    select(bar, pid)
    scribe(address, pid)
    borrow(pid) { stroke(kVK_ForwardDelete, [], pid) }
    usleep(Limit.read)
    borrow(pid) { stroke(kVK_Return, [], pid) }
    return "opened \(what)"
}

func hold() -> String {
    held = leader()
    known = Set(apps().map(\.processIdentifier))
    return "ready"
}

func restore(_ pid: pid_t, _ patient: Bool = false) {
    guard let held, held != pid else { return }
    let until = Date().addingTimeInterval(known.contains(pid) && !patient ? 0.3 : Limit.build)
    repeat {
        if leader() == pid {
            usleep(Limit.read)
            forward(held)
            return
        }
        usleep(100_000)
    } while Date() < until
}

func write(_ request: [String: Any]) throws -> String {
    guard let text = request["text"] as? String, !text.isEmpty else { throw Failure(text: "give the text to type") }
    let pid = try front().0.processIdentifier
    var place = "where the cursor is"
    var field: AXUIElement?
    if request["number"] is NSNumber {
        let (element, label, box) = try target(request)
        guard inputs.contains(string(element, kAXRoleAttribute) ?? "") else {
            throw Failure(
                text:
                    "\(label) is not something text can go into. Click the place where the text belongs first, and then type without a number"
            )
        }
        guard focus(element, box, pid) else {
            throw Failure(
                text:
                    "nothing was typed, because the cursor would not go into \(label); it is still in another field. Finish what that field needs first, like picking one of the choices it offers, or press tab"
            )
        }
        place = "into \(label)"
        field = element
        if kind(element) != "AXTextArea" {
            select(element, pid)
        } else {
            tail(element)
        }
    }
    let cursor = field ?? element(AXUIElementCreateApplication(pid), kAXFocusedUIElementAttribute)
    scribe(text, pid, cursor.map { !inputs.contains(kind($0)) } ?? true)
    usleep(Limit.read)
    let holder = field ?? element(AXUIElementCreateApplication(pid), kAXFocusedUIElementAttribute)
    if field == nil, let holder {
        let role = kind(holder)
        var name = tidy(string(holder, kAXTitleAttribute), Limit.name)
        if name.isEmpty { name = tidy(string(holder, kAXDescriptionAttribute), Limit.name) }
        if name.isEmpty { name = tidy(string(holder, kAXPlaceholderValueAttribute), Limit.name) }
        if let word = words[role] { place = "into the \(word)\(name.isEmpty ? "" : " \"\(name)\"") that has the cursor" }
    }
    let said = "typed \"\(tidy(text, Limit.name))\" \(place)"
    guard let holder, inputs.contains(kind(holder)), var now = string(holder, kAXValueAttribute) else { return said }
    let probe = tidy(text, 24).replacingOccurrences(of: "…", with: "").lowercased()
    let holds = { (value: String) in
        value.lowercased().contains(probe) || tidy(value, Limit.value).lowercased().contains(probe)
    }
    if !holds(now), field != nil {
        let whole = kind(holder) == "AXTextArea" ? now + text : text
        if AXUIElementSetAttributeValue(holder, kAXValueAttribute as CFString, whole as CFString) == .success {
            usleep(Limit.read)
            now = string(holder, kAXValueAttribute) ?? now
        }
    }
    let flat = tidy(now, Limit.value)
    if flat.isEmpty { return said }
    if holds(now) { return "\(said). It now contains \"\(flat)\"" }
    return
        "\(said), but the text did not go in. It contains \"\(flat)\". Click the control first, or try another way"
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
    let pid = try front().0.processIdentifier
    borrow(pid) { stroke(code, mask, pid) }
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
    let (running, app) = try front()
    let pid = running.processIdentifier
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
            var failure: Failure?
            borrow(pid) {
                if (attribute(hit, kAXEnabledAttribute) as? Bool) == false {
                    failure = Failure(text: "\(path) is turned off right now")
                } else if AXUIElementPerformAction(hit, kAXPressAction as CFString) != .success {
                    failure = Failure(text: "\(path) could not be chosen")
                }
            }
            if let failure { throw failure }
        }
        node = hit
    }
    return "chose \(path)"
}

let display = CGMainDisplayID()

func handle(_ request: [String: Any]) throws -> [String: Any] {
    _ = display
    guard AXIsProcessTrusted() else {
        throw Failure(
            text:
                "Accessibility is turned off for Milo, so nothing on the screen can be used. They need to turn it on in Milo's settings, under Permissions, and then start Milo again"
        )
    }
    switch request["op"] as? String {
    case "start": return kept()
    case "drop":
        aimed = nil
        pinned = nil
        return ["text": "ready"]
    case "around": return around()
    case "facing": return ["text": try facing()]
    case "raise": return ["text": try raise()]
    case "front":
        guard let pid = (request["pid"] as? NSNumber)?.int32Value else { throw Failure(text: "give the app") }
        forward(pid)
        return ["text": "brought it to the front"]
    case "media":
        let codes: [String: Int] = ["play": 16, "next": 17, "previous": 18]
        guard let code = codes[request["key"] as? String ?? ""] else { throw Failure(text: "there is no such key") }
        for down in [true, false] {
            let event = NSEvent.otherEvent(
                with: .systemDefined, location: .zero,
                modifierFlags: NSEvent.ModifierFlags(rawValue: down ? 0xa00 : 0xb00), timestamp: 0, windowNumber: 0,
                context: nil, subtype: 8, data1: (code << 16) | ((down ? 0xa : 0xb) << 8), data2: -1)
            event?.cgEvent?.post(tap: .cghidEventTap)
        }
        return ["text": "pressed the \(request["key"] as? String ?? "") key"]
    case "aim": return ["text": try aim(request)]
    case "visit": return ["text": try visit(request)]
    case "installed": return ["text": try installed(request)]
    case "hold": return ["text": hold()]
    case "glance": return try glance()
    case "closer": return try closer(request)
    case "look": return try look(request)
    case "peek":
        let (was, held) = (aimed, pinned)
        defer {
            aimed = was
            pinned = held
        }
        _ = try facing()
        return try look(request)
    case "click": return ["text": try click(request)]
    case "type": return ["text": try write(request)]
    case "press": return ["text": try press(request)]
    case "menu": return ["text": try menu(request)]
    default: throw Failure(text: "there is no such action")
    }
}

@_cdecl("milo_control_call")
public func call(message: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>? {
    let data = Data(String(cString: message).utf8)
    let request = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
    var answer: [String: Any]
    do {
        answer = try handle(request)
        answer["ok"] = true
    } catch let failure as Failure {
        answer = ["ok": false, "text": failure.text]
    } catch {
        answer = ["ok": false, "text": "it did not work"]
    }
    let out = (try? JSONSerialization.data(withJSONObject: answer)) ?? Data("{}".utf8)
    return strdup(String(decoding: out, as: UTF8.self))
}
