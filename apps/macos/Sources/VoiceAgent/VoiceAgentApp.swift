// The SwiftUI shell (D-001). Hold Space (or the button) to talk, Escape to interrupt.

import AppKit
import SwiftUI
import VoiceCore

@main
struct VoiceAgentApp: App {
    @State private var model = AppModel()
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        WindowGroup("Personal Voice Agent") {
            ContentView(model: model)
                .frame(minWidth: 560, minHeight: 520)
                .task {
                    delegate.model = model
                    await model.start()
                }
        }
        .windowResizability(.contentMinSize)
    }
}

/// Closes the session cleanly on quit, so its ledger is marked closed (no recovery needed).
final class AppDelegate: NSObject, NSApplicationDelegate {
    @MainActor var model: AppModel?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Second guard behind F_SETNOSIGPIPE on the service pipe: no write may kill the app.
        signal(SIGPIPE, SIG_IGN)
        // An unbundled development run still gets a Dock icon and keyboard focus.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let model = MainActor.assumeIsolated({ self.model }) else { return .terminateNow }
        Task { @MainActor in
            await model.quit()
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

struct ContentView: View {
    @Bindable var model: AppModel
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            StatusBar(model: model)
            Divider()
            if model.usingTestProvider {
                Text("Test provider in use — replies are scripted, not from a language model.")
                    .font(.callout).padding(8).frame(maxWidth: .infinity).background(.yellow.opacity(0.25))
            }
            if let banner = model.banner {
                HStack {
                    Text(banner).font(.callout)
                    Spacer()
                    Button("Dismiss") { model.banner = nil }.buttonStyle(.borderless)
                }
                .padding(8).background(.orange.opacity(0.15))
            }
            Transcript(turns: model.turns)
            Divider()
            TalkBar(model: model)
        }
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onAppear { focused = true }
        .onKeyPress(.space, phases: [.down, .up]) { press in
            switch press.phase {
            case .down: model.pushToTalkDown()
            case .up: model.pushToTalkUp()
            default: break
            }
            return .handled
        }
        .onKeyPress(.escape) {
            model.interrupt(eventUptime: NSApp.currentEvent?.timestamp)
            return .handled
        }
        .sheet(item: $model.confirmation) { c in
            ConfirmationSheet(confirmation: c) { approve in model.resolve(c, approve: approve) }
        }
    }
}

struct StatusBar: View {
    @Bindable var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 16) {
                Label(serviceText, systemImage: model.service == .ready ? "checkmark.circle" : "exclamationmark.triangle")
                Label(model.modelStatus, systemImage: model.modelReady ? "cpu" : "cpu.fill")
                    .foregroundStyle(model.modelReady ? Color.primary : Color.orange)
            }
            HStack(spacing: 16) {
                Toggle("Listen while I hold Space", isOn: Binding(get: { model.consent.capture }, set: { model.setConsent($0) }))
                    .disabled(model.service != .ready)
                Button("Delete history") { model.deleteHistory() }
                    .disabled(model.service != .ready)
                    .help("Deletes every heard transcript kept on this Mac")
                if !model.micGranted {
                    Button("Allow microphone") { Task { await model.requestMicrophone() } }
                }
                switch model.speech {
                case .ready: EmptyView()
                case .assetsMissing:
                    Button(model.installingSpeech ? "Downloading speech model…" : "Download speech model") {
                        Task { await model.installSpeechModel() }
                    }
                    .disabled(model.installingSpeech)
                case .unsupported: Text("Speech recognition is unavailable on this Mac.").foregroundStyle(.orange)
                }
            }
        }
        .font(.callout)
        .padding(10)
    }

    var serviceText: String {
        switch model.service {
        case .starting: "Starting…"
        case .ready: "Ready — private, nothing leaves this Mac"
        case .restarting(let n): n == 0 ? "Session stopped — restarting" : "Restarting (attempt \(n))"
        case .stopped(let why): why
        }
    }
}

struct Transcript: View {
    let turns: [TurnRow]

    var body: some View {
        ScrollViewReader { proxy in
            List(turns) { turn in
                VStack(alignment: .leading, spacing: 4) {
                    Row(label: "Heard", text: turn.heard ?? "…", mono: true)
                    Row(label: "Draft", text: turn.spoken.isEmpty ? "—" : turn.spoken.joined(separator: " "), mono: false)
                    Row(label: "Executed", text: "Nothing (no actions in this version)", mono: false).foregroundStyle(.secondary)
                    HStack {
                        if let o = turn.outcome { Text(o.rawValue.replacingOccurrences(of: "_", with: " ")).font(.caption).bold() }
                        if let p = turn.problem { Text(p).font(.caption).foregroundStyle(.orange) }
                        Spacer()
                        if let ms = turn.firstAudioMs { Text("first audio \(ms) ms").font(.caption).foregroundStyle(.secondary) }
                    }
                }
                .padding(.vertical, 4)
                .id(turn.id)
            }
            .onChange(of: turns.count) { if let last = turns.last { proxy.scrollTo(last.id, anchor: .bottom) } }
        }
    }

    struct Row: View {
        let label: String
        let text: String
        let mono: Bool
        var body: some View {
            HStack(alignment: .firstTextBaseline) {
                Text(label).font(.caption).foregroundStyle(.secondary).frame(width: 64, alignment: .leading)
                Text(text).font(mono ? .body.monospaced() : .body).textSelection(.enabled)
            }
        }
    }
}

struct TalkBar: View {
    @Bindable var model: AppModel

    var body: some View {
        HStack {
            Text(model.holding ? "Listening… release to send" : "Hold Space to talk · Esc to stop")
                .foregroundStyle(.secondary)
            if let s = model.lastStop {
                Text(String(format: "last stop %.0f ms rendered + %.0f ms device (%d measured)", s.renderedMs, s.deviceLatencyMs, model.stopLog.count))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Stop") { model.interrupt() }.keyboardShortcut(.cancelAction)
            Text(model.holding ? "Release" : "Hold to talk")
                .padding(.horizontal, 14).padding(.vertical, 6)
                .background(model.holding ? Color.accentColor : Color.secondary.opacity(0.2), in: Capsule())
                .foregroundStyle(model.holding ? Color.white : Color.primary)
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { _ in model.pushToTalkDown() }
                        .onEnded { _ in model.pushToTalkUp() })
        }
        .padding(10)
    }
}

struct ConfirmationSheet: View {
    let confirmation: PendingConfirmation
    let decide: (Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Confirm before anything happens").font(.headline)
            Text("What I heard, exactly:").font(.caption).foregroundStyle(.secondary)
            Text(confirmation.heard).font(.body.monospaced()).textSelection(.enabled)
            Text("Proposed: \(confirmation.tool)").font(.callout)
            ForEach(confirmation.slots, id: \.name) { s in
                Text("• \(s.name) (\(s.kind.rawValue)): \(s.value)").font(.callout.monospaced())
            }
            Text("Why I'm asking: " + confirmation.reasons.map(\.rawValue).joined(separator: ", ")).font(.caption)
            HStack {
                Spacer()
                Button("Don't do it") { decide(false) }.keyboardShortcut(.cancelAction)
                Button("Approve") { decide(true) }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}
