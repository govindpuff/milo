import AppKit
import SwiftUI

enum Size {
    static let ear: CGFloat = 30
    static let bar: CGFloat = 180
    static let menu: CGFloat = 24
    static let open: CGFloat = 380
    static let drop: CGFloat = 190
    static let settle: TimeInterval = 0.4
    static let level = 3
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

struct Message: Decodable {
    let type: String
    let state: String?
    let id: UInt64?
    let card: Card?
}

struct Answer: Encodable {
    let id: UInt64
    let action: String
}

@MainActor
final class Model: ObservableObject {
    @Published var state = "starting"
    @Published var notch = CGSize(width: Size.bar, height: Size.menu)
    @Published var cards: [Card] = [] { didSet { changed() } }
    @Published var hovering = false { didSet { changed() } }
    var changed: () -> Void = {}
    var choose: (UInt64, String) -> Void = { _, _ in }
    var expanded: Bool { !cards.isEmpty || hovering }
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

struct Dot: View {
    let state: String

    var color: Color {
        switch state {
        case "listening": .green
        case "talking": .blue
        case "working": .orange
        default: .gray
        }
    }

    var body: some View {
        Circle().fill(color).frame(width: 7, height: 7)
    }
}

struct CardView: View {
    let card: Card
    let choose: (UInt64, String) -> Void

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
                    .lineLimit(3)
            }
            HStack(spacing: 8) {
                Spacer()
                ForEach(card.actions) { action in
                    Button(action.label) { choose(card.id, action.id) }
                        .buttonStyle(Pill(primary: action.primary ?? false))
                }
            }
            .padding(.top, 6)
        }
    }
}

struct NotchView: View {
    @ObservedObject var model: Model

    var body: some View {
        let open = model.expanded
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                Dot(state: model.state).frame(width: Size.ear)
                Spacer(minLength: 0)
                Text(model.cards.count > 1 ? "\(model.cards.count)" : "")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.7))
                    .frame(width: Size.ear)
            }
            .frame(height: model.notch.height)
            if open {
                Group {
                    if let card = model.cards.first {
                        CardView(card: card, choose: model.choose)
                    } else {
                        HStack {
                            Text("Milo").font(.system(size: 13, weight: .semibold)).foregroundStyle(.white)
                            Spacer()
                            Text(model.state.capitalized).font(.system(size: 12)).foregroundStyle(.white.opacity(0.6))
                        }
                    }
                }
                .padding(.horizontal, 18)
                .padding(.top, 6)
                .padding(.bottom, 14)
                .transition(.opacity)
            }
        }
        .frame(width: open ? Size.open : model.notch.width + 2 * Size.ear)
        .background(Color.black)
        .clipShape(
            UnevenRoundedRectangle(
                bottomLeadingRadius: open ? 22 : 10,
                bottomTrailingRadius: open ? 22 : 10,
                style: .continuous
            )
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .animation(.spring(response: 0.32, dampingFraction: 0.82), value: open)
        .animation(.spring(response: 0.32, dampingFraction: 0.82), value: model.cards.first?.id)
    }
}

final class Host: NSHostingView<NotchView> {
    var hover: (Bool) -> Void = { _ in }
    var area: NSTrackingArea?

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let area { removeTrackingArea(area) }
        let next = NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
            owner: self
        )
        addTrackingArea(next)
        area = next
    }

    override func mouseEntered(with event: NSEvent) {
        super.mouseEntered(with: event)
        hover(true)
    }

    override func mouseExited(with event: NSEvent) {
        super.mouseExited(with: event)
        hover(false)
    }
}

@MainActor
final class Controller {
    let model = Model()
    let panel: NSPanel

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
        let host = Host(rootView: NotchView(model: model))
        host.sizingOptions = []
        host.hover = { [model] inside in model.hovering = inside }
        panel.contentView = host
        model.changed = { [weak self] in self?.changed() }
        model.choose = { [weak self] id, action in self?.choose(id, action) }
        NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.place() }
        }
        place()
        panel.orderFrontRegardless()
    }

    func place() {
        guard let screen = NSScreen.screens.first(where: { $0.safeAreaInsets.top > 0 }) ?? NSScreen.main else { return }
        var notch = CGSize(width: Size.bar, height: max(screen.frame.maxY - screen.visibleFrame.maxY, Size.menu))
        if screen.safeAreaInsets.top > 0, let left = screen.auxiliaryTopLeftArea, let right = screen.auxiliaryTopRightArea {
            notch = CGSize(width: screen.frame.width - left.width - right.width, height: screen.safeAreaInsets.top)
        }
        if model.notch != notch { model.notch = notch }
        let open = model.expanded
        let width = open ? Size.open : notch.width + 2 * Size.ear
        let height = open ? notch.height + Size.drop : notch.height
        let frame = NSRect(x: screen.frame.midX - width / 2, y: screen.frame.maxY - height, width: width, height: height)
        panel.setFrame(frame, display: true)
    }

    func changed() {
        if model.expanded {
            place()
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + Size.settle) { [weak self] in
            MainActor.assumeIsolated {
                guard let self, !self.model.expanded else { return }
                self.place()
            }
        }
    }

    func choose(_ id: UInt64, _ action: String) {
        model.cards.removeAll { $0.id == id }
        guard let data = try? JSONEncoder().encode(Answer(id: id, action: action)) else { return }
        String(decoding: data, as: UTF8.self).withCString { reply?($0) }
    }

    func receive(_ data: Data) {
        guard let message = try? JSONDecoder().decode(Message.self, from: data) else { return }
        switch message.type {
        case "status":
            if let state = message.state { model.state = state }
        case "card":
            if let card = message.card { model.cards.append(card) }
        case "dismiss":
            if let id = message.id { model.cards.removeAll { $0.id == id } }
        default:
            break
        }
    }
}

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

@_cdecl("milo_notch_post")
public func post(message: UnsafePointer<CChar>) {
    let data = Data(String(cString: message).utf8)
    DispatchQueue.main.async {
        MainActor.assumeIsolated { controller?.receive(data) }
    }
}
