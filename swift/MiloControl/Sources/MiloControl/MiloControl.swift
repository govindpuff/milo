import AppKit
import ApplicationServices
import Carbon.HIToolbox
import CoreText
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
    static let listed = 30
    static let captured: CGFloat = 2400
    static let sent: CGFloat = 640_000
    static let quality = 0.72
    static let columns = 6
    static let rows = 4
    static let print = 24
    static let capture = 6.0
    static let launch = 4.0
    static let small: CGFloat = 5
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
nonisolated(unsafe) var boxes: [CGRect?] = []
nonisolated(unsafe) var aimed: pid_t?
nonisolated(unsafe) var pane = CGRect.zero
nonisolated(unsafe) var view: CGRect?
nonisolated(unsafe) var pane_id: CGWindowID = 0

@_silgen_name("_AXUIElementGetWindow")
func _AXUIElementGetWindow(_ element: AXUIElement, _ id: UnsafeMutablePointer<CGWindowID>) -> AXError

typealias Post = @convention(c) (pid_t, CGEvent) -> Void
typealias Record = @convention(c) (UnsafeRawPointer, UnsafeRawPointer) -> Int32
typealias Front = @convention(c) (UnsafeMutableRawPointer) -> Int32
typealias Serial = @convention(c) (pid_t, UnsafeMutableRawPointer) -> Int32

let skylight = dlopen("/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight", RTLD_LAZY)
let everywhere = UnsafeMutableRawPointer(bitPattern: -2)
let post: Post? = dlsym(skylight, "SLEventPostToPid").map { unsafeBitCast($0, to: Post.self) }
let record: Record? = dlsym(skylight, "SLPSPostEventRecordTo").map { unsafeBitCast($0, to: Record.self) }
let foremost: Front? = dlsym(skylight, "_SLPSGetFrontProcess").map { unsafeBitCast($0, to: Front.self) }
let serial: Serial? = dlsym(everywhere, "GetProcessForPID").map { unsafeBitCast($0, to: Serial.self) }

final class Held: @unchecked Sendable {
    var image: CGImage?
}
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

func wake(_ pid: pid_t, _ app: AXUIElement) {
    AXUIElementSetMessagingTimeout(app, Limit.messaging)
    if woken.contains(pid) { return }
    woken.insert(pid)
    let manual = AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    let enhanced = AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
    if manual == .success || enhanced == .success { usleep(Limit.settle) }
}

func front() throws -> (NSRunningApplication, AXUIElement) {
    if let aimed, let running = NSRunningApplication(processIdentifier: aimed), !running.isTerminated {
        let app = AXUIElementCreateApplication(aimed)
        wake(aimed, app)
        return (running, app)
    }
    let system = AXUIElementCreateSystemWide()
    var pid: pid_t = 0
    if let focused = element(system, kAXFocusedApplicationAttribute) {
        AXUIElementGetPid(focused, &pid)
    }
    let running = NSRunningApplication(processIdentifier: pid) ?? NSWorkspace.shared.frontmostApplication
    guard let running else { throw Failure(text: "no app is in front") }
    aimed = running.processIdentifier
    let app = AXUIElementCreateApplication(running.processIdentifier)
    wake(running.processIdentifier, app)
    return (running, app)
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
        let running = NSWorkspace.shared.runningApplications.filter { $0.activationPolicy == .regular }
        var hit: NSRunningApplication?
        if site {
            let address = what.contains("://") ? what : "https://\(what)"
            if let url = URL(string: address), let browser = NSWorkspace.shared.urlForApplication(toOpen: url) {
                let id = Bundle(url: browser)?.bundleIdentifier
                hit = running.first { $0.bundleIdentifier == id }
            }
        } else {
            let name = what.lowercased()
            hit = running.first { $0.localizedName?.lowercased() == name }
                ?? running.first { $0.localizedName?.lowercased().contains(name) == true }
        }
        if let hit {
            aimed = hit.processIdentifier
            view = nil
            return "working in \(hit.localizedName ?? what)"
        }
        if Date() > until { throw Failure(text: "\(what) is not open") }
        usleep(200_000)
    }
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
            boxes.append(frame(node))
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

func capture(_ id: CGWindowID, _ size: CGSize) throws -> CGImage {
    guard CGPreflightScreenCaptureAccess() else {
        CGRequestScreenCaptureAccess()
        throw Failure(
            text:
                "Screen Recording is turned off for the terminal app, so the screen cannot be seen. They need to turn it on in System Settings, under Privacy and Security, Screen Recording, and then start milo again"
        )
    }
    let scale = min(2, Limit.captured / max(size.width, size.height, 1))
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

let tints: [CGColor] = [
    CGColor(red: 0.86, green: 0.1, blue: 0.1, alpha: 1), CGColor(red: 0.05, green: 0.4, blue: 0.9, alpha: 1),
    CGColor(red: 0.05, green: 0.55, blue: 0.2, alpha: 1), CGColor(red: 0.7, green: 0.2, blue: 0.8, alpha: 1),
    CGColor(red: 0.85, green: 0.45, blue: 0, alpha: 1), CGColor(red: 0, green: 0.55, blue: 0.6, alpha: 1),
]

func tag(_ context: CGContext, _ text: String, _ size: CGFloat, _ tint: CGColor, _ place: (CGSize) -> CGPoint) {
    let font = CTFontCreateWithName("Helvetica-Bold" as CFString, size, nil)
    let white = CGColor(red: 1, green: 1, blue: 1, alpha: 1)
    let styled = NSAttributedString(
        string: text,
        attributes: [
            NSAttributedString.Key(kCTFontAttributeName as String): font,
            NSAttributedString.Key(kCTForegroundColorAttributeName as String): white,
        ])
    let line = CTLineCreateWithAttributedString(styled)
    let bounds = CTLineGetBoundsWithOptions(line, .useOpticalBounds)
    let extent = CGSize(width: bounds.width + 6, height: size + 4)
    let origin = place(extent)
    context.setFillColor(tint)
    context.fill(CGRect(origin: origin, size: extent))
    context.textPosition = CGPoint(x: origin.x + 3, y: origin.y + 4)
    CTLineDraw(line, context)
}

func cells(_ area: CGRect) -> [(String, CGRect)] {
    var out: [(String, CGRect)] = []
    let width = area.width / CGFloat(Limit.columns)
    let height = area.height / CGFloat(Limit.rows)
    for row in 0..<Limit.rows {
        for column in 0..<Limit.columns {
            let name = "\(Character(UnicodeScalar(UInt8(65 + column))))\(row + 1)"
            let box = CGRect(
                x: area.minX + CGFloat(column) * width, y: area.minY + CGFloat(row) * height, width: width,
                height: height)
            out.append((name, box))
        }
    }
    return out
}

func render(_ image: CGImage, _ area: CGRect, _ grid: Bool) -> Data? {
    let ratio = CGFloat(image.width) / max(pane.width, 1)
    let local = CGRect(
        x: (area.minX - pane.minX) * ratio, y: (area.minY - pane.minY) * ratio, width: area.width * ratio,
        height: area.height * ratio)
    guard let piece = image.cropping(to: local) else { return nil }
    let scale = min((Limit.sent / max(area.width * area.height, 1)).squareRoot(), 3)
    let width = Int(area.width * scale)
    let height = Int(area.height * scale)
    guard width > 0, height > 0,
        let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
    else { return nil }
    context.interpolationQuality = .high
    context.draw(piece, in: CGRect(x: 0, y: 0, width: width, height: height))
    let place = { (box: CGRect) -> CGRect in
        CGRect(
            x: (box.minX - area.minX) * scale, y: CGFloat(height) - (box.maxY - area.minY) * scale,
            width: box.width * scale, height: box.height * scale)
    }
    if grid {
        for (index, (name, box)) in cells(area).enumerated() {
            let tint = tints[index % tints.count]
            let spot = place(box)
            context.setStrokeColor(tint)
            context.setLineWidth(1.5)
            context.stroke(spot)
            tag(context, name, 15, tint) { extent in
                CGPoint(x: spot.midX - extent.width / 2, y: spot.midY - extent.height / 2)
            }
        }
    } else {
        for (index, box) in boxes.enumerated() {
            guard let box, box.width >= Limit.small, box.height >= Limit.small, box.intersects(area) else { continue }
            let tint = tints[index % tints.count]
            let spot = place(box)
            context.setStrokeColor(tint)
            context.setLineWidth(1.5)
            context.stroke(spot)
            tag(context, "\(index + 1)", 11, tint) { extent in
                let above = spot.maxY + extent.height <= CGFloat(height)
                return CGPoint(x: max(0, spot.minX), y: above ? spot.maxY : spot.maxY - extent.height)
            }
        }
    }
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

func look(_ request: [String: Any], _ grid: Bool) throws -> [String: Any] {
    let (running, app) = try front()
    marked = []
    labels = []
    boxes = []
    var head = "App: \(running.localizedName ?? "unknown")"
    guard let target = window(app), let bounds = frame(target), bounds.width > 1, bounds.height > 1 else {
        view = nil
        return ["text": head + "\nIt has no window open. Use menu or press to open one."]
    }
    pane = bounds
    pane_id = number(target)
    let title = tidy(string(target, kAXTitleAttribute), Limit.name)
    if !title.isEmpty { head += "\nWindow: \(title)" }
    var walk = Walk(bounds: bounds, until: Date().addingTimeInterval(Limit.seconds))
    visit(target, 0, false, &walk)
    let image = try capture(pane_id, bounds.size)
    var area = bounds
    if grid {
        if let name = (request["cell"] as? String)?.uppercased().trimmingCharacters(in: .whitespaces), !name.isEmpty {
            let within = view.flatMap { $0.intersects(bounds) ? $0.intersection(bounds) : nil } ?? bounds
            guard let cell = cells(within).first(where: { $0.0 == name }) else {
                throw Failure(text: "there is no square \(name); the squares are A1 to F4")
            }
            area = cell.1
        }
        view = area
        head +=
            "\nThe picture shows \(area == bounds ? "the window" : "a closer view of one square") divided into squares named A1 to F4. To get closer, call grid with a square. To click the middle of a square, call click with that square."
    } else {
        if walk.lines.isEmpty {
            head += "\nNo controls could be read in this window. Use the picture, and grid to click by square."
        } else {
            head += "\n" + walk.lines.joined(separator: "\n")
            if walk.cut { head += "\n(there is more in this window than is listed)" }
        }
    }
    var out: [String: Any] = ["text": head, "print": print(image)]
    if let data = render(image, area, grid) {
        out["image"] = data.base64EncodedString()
        if let save = request["save"] as? String { try? data.write(to: URL(fileURLWithPath: save)) }
    }
    return out
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

func borrow(_ pid: pid_t, _ work: () -> Void) {
    var ahead = [UInt32](repeating: 0, count: 2)
    guard let foremost, foremost(&ahead) == 0, let mine = psn(pid), mine != ahead else {
        work()
        return
    }
    tell(mine, pane_id, 0x01)
    usleep(30_000)
    work()
    usleep(60_000)
    tell(mine, pane_id, 0x02)
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
    let before = NSWorkspace.shared.frontmostApplication
    let cursor = CGEvent(source: nil)?.location
    if before?.processIdentifier != pid {
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
    if let before, before.processIdentifier != pid { forward(before.processIdentifier) }
}

func strike(_ point: CGPoint, _ pid: pid_t, _ mode: String) {
    if mode == "real" { tap(point, pid) } else { poke(point, pid) }
}

func click(_ request: [String: Any]) throws -> String {
    let mode = request["mode"] as? String ?? ""
    let pid = try front().0.processIdentifier
    if let name = (request["cell"] as? String)?.uppercased().trimmingCharacters(in: .whitespaces), !name.isEmpty {
        guard let area = view, let cell = cells(area).first(where: { $0.0 == name }) else {
            throw Failure(text: "call grid first, then click one of its squares, A1 to F4")
        }
        strike(CGPoint(x: cell.1.midX, y: cell.1.midY), pid, mode)
        return "clicked the middle of square \(name)"
    }
    let (element, label, box) = try target(request)
    let role = kind(element)
    if !mode.isEmpty {
        guard let box, box.width > 0, box.height > 0 else { throw Failure(text: "it could not be clicked") }
        strike(CGPoint(x: box.midX, y: box.midY), pid, mode)
        return "clicked \(label)"
    }
    if inputs.contains(role) {
        if AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue) != .success,
            let box
        {
            poke(CGPoint(x: box.midX, y: box.midY), pid)
        }
        return "clicked \(label)"
    }
    if rows.contains(role),
        AXUIElementSetAttributeValue(element, kAXSelectedAttribute as CFString, kCFBooleanTrue) == .success
    {
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

func write(_ request: [String: Any]) throws -> String {
    guard let text = request["text"] as? String, !text.isEmpty else { throw Failure(text: "give the text to type") }
    let pid = try front().0.processIdentifier
    var place = "where the cursor is"
    if request["number"] is NSNumber {
        let (element, label, box) = try target(request)
        if AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue) != .success,
            let box
        {
            poke(CGPoint(x: box.midX, y: box.midY), pid)
        }
        usleep(150_000)
        place = "into \(label)"
    }
    let source = CGEventSource(stateID: .hidSystemState)
    borrow(pid) {
        for (index, line) in text.components(separatedBy: "\n").enumerated() {
            if index > 0 { stroke(kVK_Return, [], pid) }
            var units = Array(line.utf16)
            while !units.isEmpty {
                var take = min(Limit.chunk, units.count)
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

func handle(_ request: [String: Any]) throws -> [String: Any] {
    let prompt = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
    guard AXIsProcessTrustedWithOptions(prompt) else {
        throw Failure(
            text:
                "Accessibility is turned off for the terminal app, so nothing on the screen can be used. They need to turn it on in System Settings, under Privacy and Security, Accessibility, and then start milo again"
        )
    }
    switch request["op"] as? String {
    case "start":
        aimed = nil
        view = nil
        return ["text": "ready"]
    case "aim": return ["text": try aim(request)]
    case "look": return try look(request, false)
    case "grid": return try look(request, true)
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
