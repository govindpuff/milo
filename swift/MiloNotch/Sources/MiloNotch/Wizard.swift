import AppKit
import ServiceManagement
import SwiftUI

enum Step {
    case welcome
    case key
    case access
    case voice
    case options
    case finish
}

enum Look {
    static let size = CGSize(width: 680, height: 620)
    static let ground = Color(red: 0.055, green: 0.055, blue: 0.065)
    static let glow = Color(red: 1.0, green: 0.42, blue: 0.24)
    static let ink = Color.white
    static let faint = Color.white.opacity(0.55)
    static let card = Color.white.opacity(0.06)
    static let line = Color.white.opacity(0.1)
    static let good = Color(red: 0.3, green: 0.85, blue: 0.5)
    static let corner: CGFloat = 12
}

struct Sound: Identifiable {
    let id: String
    let line: String
    let design: String
}

let sounds = [
    Sound(id: "Warm", line: "A relaxed, friendly man", design: ""),
    Sound(
        id: "Bright",
        line: "An upbeat, energetic woman",
        design:
            "A bright, upbeat young American woman with a clear, friendly, energetic voice, like a close friend who is happy to hear from you."
    ),
    Sound(
        id: "Calm",
        line: "A soft, unhurried woman",
        design: "A calm American woman with a low, soft, unhurried voice, warm and thoughtful, like a friend on a quiet evening."
    ),
    Sound(
        id: "Deep",
        line: "A smooth, easygoing man",
        design:
            "A relaxed American man in his forties with a deep, smooth, easygoing voice, like a late-night radio host talking to a friend."
    ),
]

extension Access {
    var icon: String {
        switch self {
        case .microphone: "mic.fill"
        case .accessibility: "cursorarrow.click.2"
        case .screen: "rectangle.on.rectangle"
        }
    }
}

struct Halftone: View {
    static let leg = 0.23
    static let tall = 0.6
    static let across = 46

    func mark(_ size: CGSize) -> Path {
        let leg = size.width * Self.leg
        let legs = [leg / 2, size.width / 2, size.width - leg / 2]
        let radius = (legs[1] - legs[0]) / 2
        let shoulder = leg / 2 + radius
        var path = Path()
        for (left, right) in zip(legs, legs.dropFirst()) {
            path.move(to: CGPoint(x: left, y: size.height))
            path.addLine(to: CGPoint(x: left, y: shoulder))
            path.addArc(
                center: CGPoint(x: (left + right) / 2, y: shoulder),
                radius: radius,
                startAngle: .degrees(180),
                endAngle: .degrees(360),
                clockwise: false
            )
            path.addLine(to: CGPoint(x: right, y: size.height))
        }
        return path.strokedPath(StrokeStyle(lineWidth: leg, lineJoin: .round))
    }

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30)) { time in
            Canvas { context, size in
                let now = time.date.timeIntervalSinceReferenceDate
                let gap = size.width / CGFloat(Self.across)
                let rise = gap * 0.866
                context.clip(to: mark(size))
                for row in 0...Int(size.height / rise) {
                    for column in 0...Self.across {
                        let x = (CGFloat(column) + (row % 2 == 0 ? 0 : 0.5)) * gap
                        let y = CGFloat(row) * rise
                        let wave = sin(now * 2.4 - Double(x / size.width) * 7 + Double(y / size.height) * 2.5)
                        let radius = gap * (0.33 + 0.13 * wave)
                        let dot = CGRect(x: x - radius, y: y - radius, width: 2 * radius, height: 2 * radius)
                        context.fill(Path(ellipseIn: dot), with: .color(Look.ink))
                    }
                }
            }
        }
        .aspectRatio(1 / Self.tall, contentMode: .fit)
    }
}

struct Face: View {
    @Environment(\.isEnabled) var enabled
    let label: ButtonStyleConfiguration.Label
    let filled: Bool
    let pressed: Bool

    var body: some View {
        label
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(filled ? Color.black : Look.ink)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .background(filled ? Look.ink : Color.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
            .opacity(enabled ? (pressed ? 0.7 : 1) : 0.3)
    }
}

struct Press: ButtonStyle {
    var filled = false

    func makeBody(configuration: Configuration) -> some View {
        Face(label: configuration.label, filled: filled, pressed: configuration.isPressed)
    }
}

struct Tile<Content: View>: View {
    var selected = false
    @ViewBuilder let content: Content

    var body: some View {
        content
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(selected ? Color.white.opacity(0.14) : Look.card, in: RoundedRectangle(cornerRadius: Look.corner))
            .overlay(
                RoundedRectangle(cornerRadius: Look.corner)
                    .strokeBorder(selected ? Look.ink.opacity(0.7) : Look.line)
            )
    }
}

struct Badge: View {
    let icon: String

    var body: some View {
        Image(systemName: icon)
            .font(.system(size: 15, weight: .medium))
            .foregroundStyle(Look.ink)
            .frame(width: 36, height: 36)
            .background(Color.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 9))
    }
}

struct Line<Trailing: View>: View {
    let icon: String
    let title: String
    let detail: String
    @ViewBuilder let trailing: Trailing

    var body: some View {
        Tile {
            HStack(spacing: 14) {
                Badge(icon: icon)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(.system(size: 14, weight: .semibold)).foregroundStyle(Look.ink)
                    Text(detail).font(.system(size: 12)).foregroundStyle(Look.faint)
                }
                Spacer(minLength: 12)
                trailing
            }
        }
    }
}

struct Heading: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: 8) {
            Text(title).font(.system(size: 26, weight: .bold)).foregroundStyle(Look.ink)
            Text(detail)
                .font(.system(size: 14))
                .foregroundStyle(Look.faint)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: 460)
    }
}

struct Wizard: View {
    @ObservedObject var desk: Desk
    @ObservedObject var grants: Grants
    let steps: [Step]
    let first: Bool
    let close: () -> Void
    @State var at = 0
    @State var key = ""
    @State var picked = sounds[0].id
    @State var custom = ""
    @State var login = SMAppService.mainApp.status == .enabled

    var step: Step { steps[at] }
    var id: String { desk.draft.provider }
    var started: Bool { desk.state != "starting" }

    var ready: Bool {
        switch step {
        case .key: desk.keyed(id) && !desk.draft.name.isEmpty
        case .access: grants.all
        case .finish: started
        default: true
        }
    }

    var next: String {
        switch step {
        case .welcome: "Get Started"
        case .finish: "Start Talking"
        default: "Continue"
        }
    }

    var provider: Binding<String> {
        Binding(
            get: { id },
            set: { chosen in
                desk.draft.model = chosen == desk.saved.provider ? desk.saved.model : "\(chosen)/"
                key = ""
                desk.ask(Ask(type: "models", provider: chosen))
            }
        )
    }

    var opens: Binding<Bool> {
        Binding(
            get: { login },
            set: { wanted in
                do {
                    if wanted {
                        try SMAppService.mainApp.register()
                    } else {
                        try SMAppService.mainApp.unregister()
                    }
                    login = wanted
                } catch {}
            }
        )
    }

    func design(_ sound: Sound) -> String {
        sound.design.isEmpty ? desk.voice : sound.design
    }

    var described: String { custom.trimmingCharacters(in: .whitespacesAndNewlines) }

    func advance() {
        guard ready else { return }
        desk.hush()
        if step == .finish {
            close()
            return
        }
        if steps[at + 1] == .finish { commit() }
        withAnimation(.easeInOut(duration: 0.25)) { at += 1 }
    }

    func commit() {
        if first {
            var prefs = desk.draft
            prefs.voiceDesign = described.isEmpty ? sounds.first { $0.id == picked }?.design ?? "" : described
            prefs.setup = true
            desk.save(prefs)
        }
        desk.ask(Ask(type: "ready"))
    }

    @ViewBuilder var welcome: some View {
        VStack(spacing: 28) {
            Halftone().frame(width: 230)
            Heading(
                title: "Meet Milo",
                detail: "Talk to your Mac like you'd talk to a friend. Milo talks back, and does things for you."
            )
        }
    }

    @ViewBuilder var connect: some View {
        VStack(spacing: 22) {
            Heading(title: "Connect a model", detail: "Milo thinks with a model from one of these providers. Paste your API key.")
            VStack(spacing: 10) {
                Picker("", selection: provider) {
                    ForEach(desk.providers) { Text(named($0.id)).tag($0.id) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                Tile {
                    HStack(spacing: 10) {
                        Image(systemName: "key.fill").foregroundStyle(Look.faint)
                        SecureField("", text: $key, prompt: Text(desk.keyed(id) ? "Saved" : "\(named(id)) API key"))
                            .textFieldStyle(.plain)
                            .font(.system(size: 14))
                            .onSubmit { if !key.isEmpty { desk.enter(id, key) } }
                        if desk.keyed(id), key.isEmpty {
                            Image(systemName: "checkmark.circle.fill").foregroundStyle(Look.good)
                        } else {
                            Button("Check") { desk.enter(id, key) }
                                .buttonStyle(Press())
                                .disabled(key.isEmpty || desk.keys[id] == "checking")
                        }
                    }
                }
                if let note = desk.note(id), desk.keys[id] != "saved" {
                    Text(note).font(.system(size: 12)).foregroundStyle(Look.faint)
                }
                if desk.keyed(id) {
                    Tile {
                        HStack {
                            Text("Model").font(.system(size: 14, weight: .semibold)).foregroundStyle(Look.ink)
                            Spacer()
                            SwiftUI.Menu(desk.draft.name.isEmpty ? "Choose a model" : desk.draft.name) {
                                ForEach(desk.models[id] ?? [], id: \.self) { model in
                                    Button(model) { desk.draft.model = "\(id)/\(model)" }
                                }
                            }
                            .menuStyle(.borderlessButton)
                            .fixedSize()
                        }
                    }
                }
            }
        }
        .onChange(of: desk.keys[id]) { _, state in
            guard state == "saved" else { return }
            key = ""
            desk.ask(Ask(type: "models", provider: id))
        }
    }

    @ViewBuilder var access: some View {
        VStack(spacing: 22) {
            Heading(title: "Give Milo access", detail: "Milo needs these three to hear you and to use your apps for you.")
            VStack(spacing: 10) {
                ForEach(Access.allCases) { access in
                    let waits = access == .screen && grants.asked.contains(access)
                    Line(
                        icon: access.icon,
                        title: access.title,
                        detail: waits ? "Turn it on in System Settings, and then restart Milo." : access.why
                    ) {
                        if grants.granted[access] == true {
                            Label("Granted", systemImage: "checkmark.circle.fill")
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(Look.good)
                        } else if waits {
                            Button("Restart Milo") { desk.ask(Ask(type: "restart")) }.buttonStyle(Press())
                        } else {
                            Button("Grant Access") { grants.request(access) }.buttonStyle(Press())
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder var voice: some View {
        VStack(spacing: 22) {
            Heading(title: "Choose a voice", detail: "Click a voice to hear it, or describe one of your own. A new voice takes a few seconds to make.")
            VStack(spacing: 10) {
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 10), GridItem(.flexible())], spacing: 10) {
                    ForEach(sounds) { sound in
                        let chosen = custom.isEmpty && picked == sound.id
                        Button {
                            picked = sound.id
                            custom = ""
                            desk.preview(design(sound))
                        } label: {
                            Tile(selected: chosen) {
                                HStack {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(sound.id).font(.system(size: 14, weight: .semibold)).foregroundStyle(Look.ink)
                                        Text(sound.line).font(.system(size: 12)).foregroundStyle(Look.faint)
                                    }
                                    Spacer()
                                    if chosen, desk.previewing == design(sound) {
                                        ProgressView().controlSize(.small)
                                    } else if chosen {
                                        Image(systemName: "speaker.wave.2.fill").foregroundStyle(Look.faint)
                                    }
                                }
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
                Tile(selected: !custom.isEmpty) {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: "pencil.line").foregroundStyle(Look.faint)
                        TextField("", text: $custom, prompt: Text("Or describe a voice, like a cheerful grandmother"), axis: .vertical)
                            .textFieldStyle(.plain)
                            .font(.system(size: 14))
                            .lineLimit(1...3)
                            .onSubmit { if !described.isEmpty { desk.preview(described) } }
                        if !described.isEmpty, desk.previewing == described {
                            ProgressView().controlSize(.small)
                        } else {
                            Button {
                                desk.preview(described)
                            } label: {
                                Image(systemName: "play.fill")
                            }
                            .buttonStyle(Press())
                            .disabled(described.isEmpty)
                        }
                    }
                }
            }
        }
        .onAppear {
            let design = desk.draft.voiceDesign
            if let known = sounds.first(where: { $0.design == design }) {
                picked = known.id
                desk.preview(self.design(known))
            } else {
                custom = design
                desk.preview(design)
            }
        }
    }

    @ViewBuilder var options: some View {
        VStack(spacing: 22) {
            Heading(title: "Make it yours", detail: "A few choices to start with. They're all in the settings later.")
            VStack(spacing: 10) {
                Line(icon: "headphones", title: "I use headphones", detail: "Milo then doesn't have to filter out its own voice.") {
                    Toggle("", isOn: $desk.draft.headphones).labelsHidden().toggleStyle(.switch)
                }
                Line(icon: "hand.tap.fill", title: "Push to talk", detail: "Milo listens only while you hold \(Key.talk.hint).") {
                    Toggle("", isOn: $desk.draft.pushToTalk).labelsHidden().toggleStyle(.switch)
                }
                Line(icon: "power", title: "Open at login", detail: "Milo is there whenever your Mac is on.") {
                    Toggle("", isOn: opens).labelsHidden().toggleStyle(.switch)
                }
            }
        }
    }

    @ViewBuilder var finish: some View {
        VStack(spacing: 28) {
            Halftone().frame(width: 170)
            if started {
                Heading(title: "Milo is ready", detail: "Just start talking. Milo lives in the notch at the top of your screen.")
            } else {
                Heading(
                    title: "Getting Milo ready",
                    detail: desk.fresh
                        ? "Downloading the voice and speech models. This can take several minutes."
                        : "This takes a moment."
                )
                ProgressView().controlSize(.small)
            }
            if first {
                Line(
                    icon: "globe",
                    title: "Let Milo use websites",
                    detail: "Open this address in Chrome, and allow remote debugging."
                ) {
                    Button("Copy Address") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString("chrome://inspect/#remote-debugging", forType: .string)
                    }
                    .buttonStyle(Press())
                }
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            Group {
                switch step {
                case .welcome: welcome
                case .key: connect
                case .access: access
                case .voice: voice
                case .options: options
                case .finish: finish
                }
            }
            .frame(maxWidth: 520, maxHeight: .infinity)
            .id(at)
            .transition(.opacity)
            HStack(spacing: 10) {
                HStack(spacing: 6) {
                    ForEach(steps.indices, id: \.self) { index in
                        Capsule()
                            .fill(index == at ? Look.ink : Color.white.opacity(0.18))
                            .frame(width: index == at ? 18 : 6, height: 6)
                    }
                }
                Spacer()
                if at > 0, step != .finish {
                    Button("Back") {
                        desk.hush()
                        withAnimation(.easeInOut(duration: 0.25)) { at -= 1 }
                    }
                    .buttonStyle(Press())
                }
                Button(action: advance) {
                    HStack(spacing: 8) {
                        Text(next)
                        Text("↵").opacity(0.45)
                    }
                }
                .buttonStyle(Press(filled: true))
                .keyboardShortcut(.defaultAction)
                .disabled(!ready)
            }
        }
        .padding(.horizontal, 32)
        .padding(.top, 44)
        .padding(.bottom, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(
            ZStack {
                Look.ground
                RadialGradient(
                    colors: [Look.glow.opacity(0.42), Look.glow.opacity(0)],
                    center: UnitPoint(x: 0.5, y: -0.1),
                    startRadius: 0,
                    endRadius: 460
                )
            }
        )
        .environment(\.colorScheme, .dark)
        .ignoresSafeArea()
        .onAppear { desk.ask(Ask(type: "settings")) }
    }
}
