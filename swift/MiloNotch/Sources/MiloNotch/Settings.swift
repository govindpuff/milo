import AVFoundation
import AppKit
import ApplicationServices
import ServiceManagement
import SwiftUI
import UniformTypeIdentifiers

struct Prefs: Codable, Equatable {
    var model = ""
    var voiceDesign = ""
    var voiceRef = ""
    var voiceText = ""
    var autoApprove = false
    var pushToTalk = false
    var headphones = false
    var listeningSounds = true
    var inputDevice = ""
    var outputDevice = ""
    var setup = false

    var provider: String { model.split(separator: "/", maxSplits: 1).first.map(String.init) ?? "" }
    var name: String { model.split(separator: "/", maxSplits: 1).dropFirst().first.map(String.init) ?? "" }

    func restarts(from old: Prefs) -> Bool {
        var same = self
        same.autoApprove = old.autoApprove
        same.pushToTalk = old.pushToTalk
        same.setup = old.setup
        return same != old
    }
}

struct Provider: Decodable, Identifiable {
    let id: String
    let keyed: Bool
}

struct Ask: Encodable {
    let type: String
    var provider: String?
    var key: String?
    var design: String?
}

struct Save: Encodable {
    let type = "save"
    let settings: Prefs
}

func named(_ provider: String) -> String {
    ["openai": "OpenAI", "anthropic": "Anthropic", "google": "Google", "openrouter": "OpenRouter"][provider] ?? provider
}

@MainActor
final class Desk: ObservableObject {
    @Published var saved = Prefs()
    @Published var draft = Prefs()
    @Published var providers: [Provider] = []
    @Published var inputs: [String] = []
    @Published var outputs: [String] = []
    @Published var models: [String: [String]] = [:]
    @Published var keys: [String: String] = [:]
    @Published var detail = ""
    @Published var voice = ""
    @Published var fresh = false
    @Published var previewing: String?
    var sound: NSSound?
    @Published var state = "starting"
    var ask: (Ask) -> Void = { _ in }
    var save: (Prefs) -> Void = { _ in }

    func keyed(_ provider: String) -> Bool {
        providers.first { $0.id == provider }?.keyed ?? false
    }

    func note(_ provider: String) -> String? {
        switch keys[provider] {
        case "checking": "Checking…"
        case "saved": "Saved."
        case "refused": "\(named(provider)) refused this key."
        case "failed": detail
        default: nil
        }
    }

    func preview(_ design: String) {
        sound?.stop()
        previewing = design
        ask(Ask(type: "preview", design: design))
    }

    func heard(_ design: String?, _ path: String?) {
        guard design == previewing else { return }
        previewing = nil
        guard let path else { return }
        sound = NSSound(contentsOfFile: path, byReference: true)
        sound?.play()
    }

    func hush() {
        previewing = nil
        sound?.stop()
    }

    func enter(_ provider: String, _ key: String) {
        keys[provider] = "checking"
        ask(Ask(type: "key", provider: provider, key: key))
    }
}

enum Access: CaseIterable, Identifiable {
    case microphone
    case accessibility
    case screen

    var id: Self { self }

    var title: String {
        switch self {
        case .microphone: "Microphone"
        case .accessibility: "Accessibility"
        case .screen: "Screen Recording"
        }
    }

    var why: String {
        switch self {
        case .microphone: "Milo hears you."
        case .accessibility: "Milo clicks and types in your apps."
        case .screen: "Milo sees the windows that it works in."
        }
    }

    var pane: String {
        switch self {
        case .microphone: "Privacy_Microphone"
        case .accessibility: "Privacy_Accessibility"
        case .screen: "Privacy_ScreenCapture"
        }
    }

    var granted: Bool {
        switch self {
        case .microphone: AVCaptureDevice.authorizationStatus(for: .audio) == .authorized
        case .accessibility: AXIsProcessTrusted()
        case .screen: CGPreflightScreenCaptureAccess()
        }
    }

    func request() {
        switch self {
        case .microphone:
            if AVCaptureDevice.authorizationStatus(for: .audio) == .notDetermined {
                AVCaptureDevice.requestAccess(for: .audio) { _ in }
                return
            }
        case .accessibility:
            _ = AXIsProcessTrustedWithOptions(["AXTrustedCheckOptionPrompt": true] as CFDictionary)
            return
        case .screen:
            if CGRequestScreenCaptureAccess() { return }
        }
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(pane)") {
            NSWorkspace.shared.open(url)
        }
    }
}

@MainActor
final class Grants: ObservableObject {
    @Published var granted: [Access: Bool] = [:]
    @Published var asked: Set<Access> = []
    var clock: Timer?

    init() {
        refresh()
        let clock = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        RunLoop.main.add(clock, forMode: .common)
        self.clock = clock
    }

    var all: Bool { Access.allCases.allSatisfy { granted[$0] == true } }

    func refresh() {
        let now = Dictionary(uniqueKeysWithValues: Access.allCases.map { ($0, $0.granted) })
        if now != granted { granted = now }
    }

    func request(_ access: Access) {
        asked.insert(access)
        access.request()
    }
}

struct Permissions: View {
    @ObservedObject var grants: Grants
    let restart: () -> Void

    var body: some View {
        ForEach(Access.allCases) { access in
            let waits = access == .screen && grants.asked.contains(access)
            LabeledContent {
                if grants.granted[access] == true {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                } else if waits {
                    Button("Restart Milo", action: restart)
                } else {
                    Button("Allow") { grants.request(access) }
                }
            } label: {
                Text(access.title)
                Text(waits ? "Turn it on in System Settings, and then restart Milo." : access.why)
            }
        }
    }
}

struct Login: View {
    @State var on = SMAppService.mainApp.status == .enabled

    var body: some View {
        Toggle("Open at login", isOn: Binding(
            get: { on },
            set: { wanted in
                do {
                    if wanted {
                        try SMAppService.mainApp.register()
                    } else {
                        try SMAppService.mainApp.unregister()
                    }
                    on = wanted
                } catch {}
            }
        ))
    }
}

struct SettingsView: View {
    @ObservedObject var desk: Desk
    @ObservedObject var grants: Grants
    @State var key = ""

    var id: String { desk.draft.provider }

    var ids: [String] {
        let all = desk.providers.map(\.id)
        return all.contains(id) || id.isEmpty ? all : all + [id]
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

    var name: Binding<String> {
        Binding(get: { desk.draft.name }, set: { desk.draft.model = "\(id)/\($0)" })
    }

    var ready: Bool {
        let voiced = desk.draft.voiceRef.isEmpty || !desk.draft.voiceText.trimmingCharacters(in: .whitespaces).isEmpty
        return desk.draft != desk.saved && !desk.draft.name.isEmpty && voiced
    }

    func devices(_ all: [String], _ current: String) -> [String] {
        current.isEmpty || all.contains(current) ? all : all + [current]
    }

    func choose() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.wav]
        if panel.runModal() == .OK, let url = panel.url {
            desk.draft.voiceRef = url.path
        }
    }

    @ViewBuilder var model: some View {
        Section("Model") {
            Picker("Provider", selection: provider) {
                ForEach(ids, id: \.self) { Text(named($0)).tag($0) }
            }
            LabeledContent("API key") {
                SecureField("", text: $key, prompt: Text(desk.keyed(id) ? "Saved" : "Paste your key"))
                    .labelsHidden()
                Button("Save") { desk.enter(id, key) }
                    .disabled(key.isEmpty || desk.keys[id] == "checking")
            }
            if let note = desk.note(id) {
                Text(note).font(.callout).foregroundStyle(.secondary)
            }
            LabeledContent("Model") {
                TextField("", text: name).labelsHidden()
                SwiftUI.Menu("") {
                    ForEach(desk.models[id] ?? [], id: \.self) { model in
                        Button(model) { desk.draft.model = "\(id)/\(model)" }
                    }
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .disabled((desk.models[id] ?? []).isEmpty)
            }
        }
    }

    @ViewBuilder var sound: some View {
        Section("Voice") {
            TextField("Description", text: $desk.draft.voiceDesign, prompt: Text(desk.voice), axis: .vertical)
                .lineLimit(2...4)
            LabeledContent {
                if !desk.draft.voiceRef.isEmpty {
                    Button("Remove") {
                        desk.draft.voiceRef = ""
                        desk.draft.voiceText = ""
                    }
                }
                Button("Choose…", action: choose)
            } label: {
                Text("Voice to copy")
                Text(
                    desk.draft.voiceRef.isEmpty
                        ? "A WAV recording of 3 to 12 seconds."
                        : URL(fileURLWithPath: desk.draft.voiceRef).lastPathComponent
                )
            }
            if !desk.draft.voiceRef.isEmpty {
                TextField("Words in the recording", text: $desk.draft.voiceText, axis: .vertical)
            }
        }
    }

    @ViewBuilder var talk: some View {
        Section("Conversation") {
            Picker("Microphone", selection: $desk.draft.inputDevice) {
                Text("System default").tag("")
                ForEach(devices(desk.inputs, desk.draft.inputDevice), id: \.self) { Text($0).tag($0) }
            }
            Picker("Speakers", selection: $desk.draft.outputDevice) {
                Text("System default").tag("")
                ForEach(devices(desk.outputs, desk.draft.outputDevice), id: \.self) { Text($0).tag($0) }
            }
            Toggle(isOn: $desk.draft.headphones) {
                Text("Headphones")
                Text("Turns off echo cancellation.")
            }
            Toggle(isOn: $desk.draft.pushToTalk) {
                Text("Push to talk")
                Text("Milo starts muted. Hold \(Key.talk.hint) to talk.")
            }
            Toggle(isOn: $desk.draft.listeningSounds) {
                Text("Listening sounds")
                Text("Milo says things like mm-hmm while you talk.")
            }
        }
    }

    @ViewBuilder var acts: some View {
        Section("Actions") {
            Toggle(isOn: $desk.draft.autoApprove) {
                Text("Approve everything automatically")
                Text("Milo acts without asking first.")
            }
        }
    }

    @ViewBuilder var access: some View {
        Section("Permissions") {
            Permissions(grants: grants) { desk.ask(Ask(type: "restart")) }
        }
    }

    @ViewBuilder var login: some View {
        if Bundle.main.bundlePath.hasSuffix(".app") {
            Section {
                Login()
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            Form {
                model
                sound
                talk
                acts
                access
                login
            }
            .formStyle(.grouped)
            Divider()
            HStack {
                Spacer()
                Button("Revert") { desk.draft = desk.saved }
                    .disabled(desk.draft == desk.saved)
                Button(desk.draft.restarts(from: desk.saved) ? "Apply and Restart" : "Apply") { desk.save(desk.draft) }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!ready)
            }
            .padding(12)
        }
        .frame(width: 520, height: 660)
        .onAppear {
            desk.ask(Ask(type: "settings"))
            desk.ask(Ask(type: "models", provider: id))
        }
        .onChange(of: desk.keys[id]) { _, state in
            guard state == "saved" else { return }
            key = ""
            desk.ask(Ask(type: "models", provider: id))
        }
    }
}

@MainActor
final class Windows {
    var open: [String: NSWindow] = [:]

    func show(_ title: String, _ view: some View) {
        if open[title] == nil {
            let window = NSWindow(contentRect: .zero, styleMask: [.titled, .closable], backing: .buffered, defer: false)
            let host = NSHostingController(rootView: view)
            host.sizingOptions = .preferredContentSize
            window.title = title
            window.isReleasedWhenClosed = false
            window.contentViewController = host
            window.setContentSize(host.view.fittingSize)
            window.center()
            open[title] = window
        }
        NSApp.activate(ignoringOtherApps: true)
        open[title]?.makeKeyAndOrderFront(nil)
    }

    func wizard(_ view: some View) {
        open["Milo"]?.close()
        let window = NSWindow(
            contentRect: .zero,
            styleMask: [.titled, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        let host = NSHostingController(rootView: view)
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.isMovableByWindowBackground = true
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .darkAqua)
        host.sizingOptions = []
        for button in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            window.standardWindowButton(button)?.isHidden = true
        }
        window.contentViewController = host
        window.setContentSize(Look.size)
        window.center()
        open["Milo"] = window
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
    }

    func close(_ title: String) {
        open[title]?.close()
    }
}

@MainActor
func commands(quit: NSMenuItem) -> NSMenu {
    let main = NSMenu()
    let app = NSMenu()
    app.addItem(quit)
    let edit = NSMenu(title: "Edit")
    edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
    edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "Z")
    edit.addItem(.separator())
    edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
    edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
    edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
    edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
    edit.addItem(.separator())
    edit.addItem(withTitle: "Close Window", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
    for menu in [app, edit] {
        let item = NSMenuItem()
        item.submenu = menu
        main.addItem(item)
    }
    return main
}
