import AppKit
import Carbon.HIToolbox
import SwiftUI
import os

enum Size {
    static let ear: CGFloat = 40
    static let bars = 5
    static let bar: CGFloat = 3
    static let gap: CGFloat = 2
    static let margin: CGFloat = 40
    static let reach: CGFloat = 4
    static let tick: TimeInterval = 1.0 / 30.0
    static let shift = 2
    static let decay: CGFloat = 0.7
    static let floor: Float = -52
    static let range: Float = 40
    static let plain: CGFloat = 180
    static let menu: CGFloat = 24
    static let open: CGFloat = 380
    static let drop: CGFloat = 240
    static let level = 3
    static let signature: OSType = 0x4D49_4C4F
}

enum Key: UInt32, CaseIterable {
    case approve = 1
    case decline = 2
    case mute = 3
    case talk = 4

    var code: Int {
        switch self {
        case .approve: kVK_ANSI_Y
        case .decline: kVK_ANSI_N
        case .mute: kVK_ANSI_M
        case .talk: kVK_Space
        }
    }

    var hint: String {
        switch self {
        case .approve: "⌃⌥Y"
        case .decline: "⌃⌥N"
        case .mute: "⌃⌥M"
        case .talk: "⌃⌥Space"
        }
    }
}

struct Choice: Decodable, Identifiable {
    let id: String
    let label: String
    let primary: Bool?
}

struct Card: Decodable, Identifiable {
    let id: UInt64
    let title: String
    let body: String?
    let actions: [Choice]
}

extension Card {
    var yes: Choice? { actions.first { $0.primary == true } }
    var no: Choice? { actions.first { $0.primary != true } }
}

struct Message: Decodable {
    let type: String
    let state: String?
    let on: Bool?
    let id: UInt64?
    let card: Card?
}

struct Answer: Encodable {
    let id: UInt64
    let action: String
}

struct Mute: Encodable {
    let type = "mute"
    let on: Bool
}

@MainActor
final class Model: ObservableObject {
    @Published var state = "starting"
    @Published var notch = CGSize(width: Size.plain, height: Size.menu)
    @Published var cards: [Card] = []
    @Published var hovering = false
    @Published var muted = false
    var toggle: () -> Void = {}
    var shape = CGSize.zero
    var choose: (UInt64, String) -> Void = { _, _ in }
    var expanded: Bool { !cards.isEmpty || hovering }
    var compact: CGSize { CGSize(width: notch.width + 2 * Size.ear, height: notch.height) }
}

@MainActor
final class Wave: ObservableObject {
    @Published var me = [CGFloat](repeating: 0, count: Size.bars)
    @Published var milo = [CGFloat](repeating: 0, count: Size.bars)
    var held: [CGFloat] = [0, 0]
    var ticks = 0

    func push(_ raw: [Float]) {
        for (i, value) in raw.enumerated() {
            let db = value > 0 ? 20 * log10(value) : Size.floor
            let level = CGFloat(min(max((db - Size.floor) / Size.range, 0), 1))
            held[i] = max(level, held[i] * Size.decay)
        }
        ticks += 1
        guard ticks % Size.shift == 0 else { return }
        me = slide(me, held[0])
        milo = slide(milo, held[1])
    }

    func slide(_ bars: [CGFloat], _ level: CGFloat) -> [CGFloat] {
        let value = level < 0.02 ? 0 : level
        if value == 0, bars.allSatisfy({ $0 == 0 }) { return bars }
        return Array(bars.dropFirst()) + [value]
    }
}

struct Bars: View {
    let levels: [CGFloat]
    let color: Color
    let height: CGFloat

    var body: some View {
        HStack(spacing: Size.gap) {
            ForEach(levels.indices, id: \.self) { i in
                Capsule()
                    .fill(color)
                    .frame(width: Size.bar, height: Size.bar + levels[i] * (height - Size.bar))
            }
        }
        .frame(height: height)
        .animation(.linear(duration: Size.tick * Double(Size.shift)), value: levels)
    }
}

struct Ears: View {
    @ObservedObject var wave: Wave
    let state: String
    let muted: Bool
    let notch: CGSize

    var body: some View {
        let height = max(notch.height - 16, Size.bar)
        HStack(spacing: 0) {
            Group {
                if muted {
                    Image(systemName: "mic.slash.fill").font(.system(size: 12)).foregroundStyle(.red)
                } else {
                    Bars(levels: wave.me, color: .white, height: height)
                }
            }
            .frame(width: Size.ear)
            Spacer(minLength: 0)
            Bars(levels: wave.milo.reversed(), color: state == "working" ? .orange : .cyan, height: height)
                .frame(width: Size.ear)
        }
        .frame(width: notch.width + 2 * Size.ear, height: notch.height)
    }
}

struct Pill: ButtonStyle {
    let primary: Bool

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(primary ? Color.black : Color.white)
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .background(primary ? Color.white : Color.white.opacity(0.14), in: Capsule())
            .opacity(configuration.isPressed ? 0.6 : 1)
    }
}

struct CardView: View {
    let card: Card
    let waiting: Int
    let choose: (UInt64, String) -> Void

    func hint(_ action: Choice) -> String? {
        if action.id == card.yes?.id { return Key.approve.hint }
        if action.id == card.no?.id { return Key.decline.hint }
        return nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(card.title)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.white)
                .lineLimit(2)
            if let body = card.body, !body.isEmpty {
                Text(body)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.6))
                    .lineLimit(6)
            }
            HStack(spacing: 8) {
                if waiting > 0 {
                    Text("+\(waiting) more").font(.system(size: 11)).foregroundStyle(.white.opacity(0.5))
                }
                Spacer()
                ForEach(card.actions) { action in
                    Button {
                        choose(card.id, action.id)
                    } label: {
                        HStack(spacing: 6) {
                            Text(action.label)
                            if let hint = hint(action) {
                                Text(hint).opacity(0.45)
                            }
                        }
                    }
                    .buttonStyle(Pill(primary: action.primary ?? false))
                }
            }
            .padding(.top, 6)
        }
    }
}

struct NotchView: View {
    @ObservedObject var model: Model
    let wave: Wave

    var body: some View {
        let open = model.expanded
        VStack(spacing: 0) {
            Ears(wave: wave, state: model.state, muted: model.muted, notch: model.notch)
            if open {
                Group {
                    if let card = model.cards.first {
                        CardView(card: card, waiting: model.cards.count - 1, choose: model.choose)
                    } else {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Text("Milo").font(.system(size: 13, weight: .semibold)).foregroundStyle(.white)
                                Spacer()
                                Text(model.muted ? "Muted" : model.state.capitalized)
                                    .font(.system(size: 12))
                                    .foregroundStyle(.white.opacity(0.6))
                                Button(action: model.toggle) {
                                    Image(systemName: model.muted ? "mic.slash.fill" : "mic.fill")
                                        .font(.system(size: 12))
                                        .foregroundStyle(model.muted ? Color.red : Color.white)
                                        .frame(width: 28, height: 28)
                                        .background(Color.white.opacity(0.14), in: Circle())
                                }
                                .buttonStyle(.plain)
                                .help(model.muted ? "Unmute the microphone" : "Mute the microphone")
                            }
                            Text("\(Key.mute.hint) mute · hold \(Key.talk.hint) to talk")
                                .font(.system(size: 11))
                                .foregroundStyle(.white.opacity(0.4))
                        }
                    }
                }
                .padding(.horizontal, 18)
                .padding(.top, 6)
                .padding(.bottom, 14)
                .frame(width: Size.open)
                .transition(.opacity)
            }
        }
        .frame(width: open ? Size.open : model.compact.width)
        .background(Color.black)
        .clipShape(
            UnevenRoundedRectangle(
                bottomLeadingRadius: open ? 22 : 10,
                bottomTrailingRadius: open ? 22 : 10,
                style: .continuous
            )
        )
        .onGeometryChange(for: CGSize.self) { $0.size } action: { model.shape = $0 }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .animation(.spring(response: 0.38, dampingFraction: 0.86), value: open)
        .animation(.spring(response: 0.38, dampingFraction: 0.86), value: model.cards.first?.id)
    }
}

final class Host: NSHostingView<NotchView> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

enum Mark {
    static let side: CGFloat = 18
    static let leg: CGFloat = 3.6
    static let legs: [CGFloat] = [2.9, 9, 15.1]
    static let foot: CGFloat = 4.4
    static let shoulder: CGFloat = 9.1

    static var image: NSImage {
        let image = NSImage(size: NSSize(width: side, height: side), flipped: false) { _ in
            let path = NSBezierPath()
            path.lineWidth = leg
            path.lineJoinStyle = .round
            for (left, right) in zip(legs, legs.dropFirst()) {
                path.move(to: NSPoint(x: left, y: foot))
                path.line(to: NSPoint(x: left, y: shoulder))
                path.appendArc(
                    withCenter: NSPoint(x: (left + right) / 2, y: shoulder),
                    radius: (right - left) / 2,
                    startAngle: 180,
                    endAngle: 0,
                    clockwise: true
                )
                path.line(to: NSPoint(x: right, y: foot))
            }
            NSColor.black.setStroke()
            path.stroke()
            return true
        }
        image.isTemplate = true
        return image
    }
}

@MainActor
final class Menu: NSObject, NSMenuDelegate {
    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    let mute = NSMenuItem(title: "Mute Microphone", action: #selector(toggle), keyEquivalent: "")
    var muted: () -> Bool = { false }
    var flip: () -> Void = {}

    override init() {
        super.init()
        item.button?.image = Mark.image
        let menu = NSMenu()
        menu.delegate = self
        mute.target = self
        menu.addItem(mute)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit Milo", action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)
        item.menu = menu
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        mute.state = muted() ? .on : .off
    }

    @objc func toggle() {
        flip()
    }

    @objc func quit() {
        kill(getpid(), SIGINT)
    }
}

@MainActor
final class Controller {
    let model = Model()
    let menu = Menu()
    let wave = Wave()
    let panel: NSPanel
    var screen = NSRect.zero

    init() {
        panel = NSPanel(
            contentRect: .zero,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.isMovable = false
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.level = NSWindow.Level(rawValue: NSWindow.Level.mainMenu.rawValue + Size.level)
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        panel.ignoresMouseEvents = true
        let host = Host(rootView: NotchView(model: model, wave: wave))
        host.sizingOptions = []
        panel.contentView = host
        model.choose = { [weak self] id, action in self?.choose(id, action) }
        model.toggle = { [weak self] in self?.toggle() }
        menu.muted = { [weak self] in self?.model.muted ?? false }
        menu.flip = { [weak self] in self?.toggle() }
        NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.place() }
        }
        place()
        panel.orderFrontRegardless()
        let timer = Timer(timeInterval: Size.tick, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
        RunLoop.main.add(timer, forMode: .common)
        keys()
    }

    func place() {
        guard let screen = NSScreen.screens.first(where: { $0.safeAreaInsets.top > 0 }) ?? NSScreen.main else { return }
        var notch = CGSize(width: Size.plain, height: max(screen.frame.maxY - screen.visibleFrame.maxY, Size.menu))
        if screen.safeAreaInsets.top > 0, let left = screen.auxiliaryTopLeftArea, let right = screen.auxiliaryTopRightArea {
            notch = CGSize(width: screen.frame.width - left.width - right.width, height: screen.safeAreaInsets.top)
        }
        if model.notch != notch { model.notch = notch }
        self.screen = screen.frame
        let width = Size.open + Size.margin
        let height = notch.height + Size.drop
        let frame = NSRect(x: screen.frame.midX - width / 2, y: screen.frame.maxY - height, width: width, height: height)
        if panel.frame != frame { panel.setFrame(frame, display: true) }
    }

    func tick() {
        wave.push(levels.withLock { held in
            let now = held
            held = [0, 0]
            return now
        })
        var size = model.compact
        if model.expanded, model.shape.width > 0 { size = model.shape }
        let area = NSRect(
            x: screen.midX - size.width / 2,
            y: screen.maxY - size.height,
            width: size.width,
            height: size.height + Size.reach
        )
        let inside = area.contains(NSEvent.mouseLocation)
        if model.hovering != inside { model.hovering = inside }
        if panel.ignoresMouseEvents == inside { panel.ignoresMouseEvents = !inside }
    }

    func choose(_ id: UInt64, _ action: String) {
        model.cards.removeAll { $0.id == id }
        send(Answer(id: id, action: action))
    }

    func toggle() {
        mute(!model.muted)
    }

    func mute(_ on: Bool) {
        guard model.muted != on else { return }
        model.muted = on
        send(Mute(on: on))
    }

    func key(_ key: Key, _ down: Bool) {
        switch key {
        case .approve:
            if down, let card = model.cards.first, let action = card.yes { choose(card.id, action.id) }
        case .decline:
            if down, let card = model.cards.first, let action = card.no { choose(card.id, action.id) }
        case .mute:
            if down { toggle() }
        case .talk:
            mute(!down)
        }
    }

    func keys() {
        let target = GetApplicationEventTarget()
        let kinds = [
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed)),
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyReleased)),
        ]
        InstallEventHandler(
            target,
            { _, event, _ in
                var id = EventHotKeyID()
                let status = GetEventParameter(
                    event,
                    EventParamName(kEventParamDirectObject),
                    EventParamType(typeEventHotKeyID),
                    nil,
                    MemoryLayout<EventHotKeyID>.size,
                    nil,
                    &id
                )
                guard status == noErr, let key = Key(rawValue: id.id) else { return OSStatus(eventNotHandledErr) }
                let down = GetEventKind(event) == UInt32(kEventHotKeyPressed)
                MainActor.assumeIsolated { controller?.key(key, down) }
                return noErr
            },
            kinds.count,
            kinds,
            nil,
            nil
        )
        for key in Key.allCases {
            var ref: EventHotKeyRef?
            RegisterEventHotKey(
                UInt32(key.code),
                UInt32(controlKey | optionKey),
                EventHotKeyID(signature: Size.signature, id: key.rawValue),
                target,
                0,
                &ref
            )
        }
    }

    func send(_ value: some Encodable) {
        guard let data = try? JSONEncoder().encode(value) else { return }
        String(decoding: data, as: UTF8.self).withCString { reply?($0) }
    }

    func receive(_ data: Data) {
        guard let message = try? JSONDecoder().decode(Message.self, from: data) else { return }
        switch message.type {
        case "status":
            if let state = message.state { model.state = state }
        case "card":
            guard let card = message.card else { return }
            if let at = model.cards.firstIndex(where: { $0.id == card.id }) {
                model.cards[at] = card
            } else {
                model.cards.insert(card, at: 0)
            }
        case "mute":
            mute(message.on ?? false)
        case "dismiss":
            if let id = message.id { model.cards.removeAll { $0.id == id } }
        default:
            break
        }
    }
}

let levels = OSAllocatedUnfairLock(initialState: [Float](repeating: 0, count: 2))
nonisolated(unsafe) var reply: (@convention(c) (UnsafePointer<CChar>) -> Void)?
@MainActor var controller: Controller?

@_cdecl("milo_notch_run")
public func run(callback: @convention(c) (UnsafePointer<CChar>) -> Void) {
    reply = callback
    MainActor.assumeIsolated {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        controller = Controller()
        app.run()
    }
}

@_cdecl("milo_notch_fail")
public func fail(message: UnsafePointer<CChar>) {
    let text = String(cString: message)
    DispatchQueue.main.async {
        MainActor.assumeIsolated {
            let alert = NSAlert()
            alert.alertStyle = .critical
            alert.messageText = "Milo stopped"
            alert.informativeText = text
            alert.addButton(withTitle: "Quit")
            NSApplication.shared.activate(ignoringOtherApps: true)
            alert.runModal()
            exit(1)
        }
    }
}

@_cdecl("milo_notch_post")
public func post(message: UnsafePointer<CChar>) {
    let data = Data(String(cString: message).utf8)
    DispatchQueue.main.async {
        MainActor.assumeIsolated { controller?.receive(data) }
    }
}

@_cdecl("milo_notch_level")
public func level(source: Int32, value: Float) {
    guard source >= 0, source < 2 else { return }
    levels.withLock { $0[Int(source)] = max($0[Int(source)], value) }
}
