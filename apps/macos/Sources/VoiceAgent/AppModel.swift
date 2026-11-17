// The app's state, on the main actor. It owns the conversation and the microphone and turns
// conversation events into what the window shows. Decisions stay in the session service.

import Foundation
import Observation
import VoiceCore

struct TurnRow: Identifiable {
    let id: TurnID
    var heard: String?
    var spoken: [String] = []
    var outcome: Outcome?
    var problem: String?
    var firstAudioMs: Int?
}

struct PendingConfirmation: Identifiable {
    let id = UUID()
    let turn: TurnID
    let digest: String
    let heard: String
    let tool: String
    let slots: [Slot]
    let reasons: [ConfirmReason]
}

@MainActor
@Observable
final class AppModel {
    enum Service: Equatable {
        case starting
        case ready
        case restarting(Int)
        case stopped(String)
    }

    var service: Service = .starting
    var consent = ConsentState(capture: false, personalVoice: false)
    var modelStatus = "Checking the on-device model…"
    var modelReady = false
    var speech: RecognizerAvailability = .unsupported
    var micGranted = MicrophoneCapture.permissionGranted
    var usingTestProvider = false
    var turns: [TurnRow] = []
    var confirmation: PendingConfirmation?
    var banner: String?
    var holding = false
    /// The microphone is actually capturing (the service said `capture on` while the key is held).
    var micOn = false
    var installingSpeech = false

    private var conversation: Conversation?
    private let capture = MicrophoneCapture()
    private var feeder: AsyncStream<PCM>.Continuation?
    private let recognizer = AppleSpeechRecognizer()
    private static let maxTurnsShown = 50

    func start() async {
        let args = CommandLine.arguments
        usingTestProvider = args.contains("--test-provider")
        let model: any LanguageModeling = usingTestProvider ? DeterministicTestProvider() : FoundationModelsProvider()
        switch model.availability() {
        case .available:
            modelReady = true
            modelStatus = usingTestProvider ? "Test provider (not a language model)" : "On-device model ready"
        case .unavailable(_, let detail):
            modelReady = false
            modelStatus = detail
        }
        speech = await recognizer.availability()
        let repo = ProcessInfo.processInfo.environment["PVA_REPO_ROOT"].map { URL(fileURLWithPath: $0) }
        guard let binary = SessionBinary.locate(repoRoot: repo) else {
            service = .stopped("The session service (voice-session) was not found.")
            return
        }
        let synthesizer = SystemVoiceSynthesizer()
        await synthesizer.warmUp()
        do {
            let dataDir = try DataDirectory.prepare(DataDirectory.standard)
            let config = ConversationConfig(binary: binary, dataDir: dataDir, recognizer: recognizer, model: model, synthesizer: synthesizer)
            let c = try Conversation(config)
            conversation = c
            try await c.start()
            Task { for await e in c.events { self.handle(e) } }
        } catch {
            service = .stopped("Could not start the session service.")
        }
    }

    // MARK: Actions

    func pushToTalkDown() {
        guard !holding, let conversation else { return }
        holding = true
        Task { await conversation.pushToTalkDown() }
    }

    func pushToTalkUp() {
        guard holding, let conversation else { return }
        holding = false
        // The microphone stops on the key, not on a reply from anything else.
        stopMicrophone()
        Task { await conversation.pushToTalkUp() }
    }

    /// Interrupt; with `eventUptime` (the key event's timestamp) the stop is measured on the
    /// live device and shown, for the drill (docs/DRILLS.md).
    func interrupt(eventUptime: TimeInterval? = nil) {
        guard let conversation else { return }
        let at = eventUptime ?? ProcessInfo.processInfo.systemUptime
        Task {
            await conversation.interrupt()
            try? await Task.sleep(for: .milliseconds(600))
            if let m = conversation.player.measureStop(since: at) {
                lastStop = m
                stopLog.append(m)
            }
        }
    }

    var lastStop: TurnFencedPlayer.StopMeasurement?
    var stopLog: [TurnFencedPlayer.StopMeasurement] = []

    func setConsent(_ grant: Bool) {
        guard let conversation else { return }
        if !grant { stopMicrophone() }
        Task { await conversation.setCaptureConsent(grant) }
    }

    func deleteHistory() {
        guard let conversation else { return }
        Task { await conversation.forgetHistory() }
    }

    func requestMicrophone() async {
        micGranted = await MicrophoneCapture.requestPermission()
    }

    func installSpeechModel() async {
        installingSpeech = true
        defer { installingSpeech = false }
        try? await recognizer.installAssets()
        speech = await recognizer.availability()
    }

    func resolve(_ c: PendingConfirmation, approve: Bool) {
        confirmation = nil
        guard let conversation else { return }
        Task { await conversation.confirm(turn: c.turn, digest: c.digest, approve: approve) }
    }

    func quit() async {
        stopMicrophone()
        await conversation?.shutdown()
    }

    // MARK: Microphone (thin; see docs/DRILLS.md)

    private func startMicrophone() {
        guard let conversation else { return }
        let (stream, continuation) = AsyncStream.makeStream(of: PCM.self, bufferingPolicy: .bufferingOldest(1_024))
        feeder = continuation
        // One consumer, so buffers reach the recognizer in capture order.
        Task { for await pcm in stream { await conversation.feed(pcm) } }
        do {
            try capture.start { pcm in continuation.yield(pcm) }
            micOn = true
        } catch {
            banner = micGranted ? "The microphone could not start." : "Microphone access is off. Allow it to talk."
            continuation.finish()
        }
    }

    private func stopMicrophone() {
        micOn = false
        capture.stop()
        feeder?.finish()
        feeder = nil
    }

    // MARK: Events

    private func row(_ turn: TurnID, _ update: (inout TurnRow) -> Void) {
        if let i = turns.lastIndex(where: { $0.id == turn }) {
            update(&turns[i])
        } else {
            var r = TurnRow(id: turn)
            update(&r)
            turns.append(r)
            if turns.count > Self.maxTurnsShown { turns.removeFirst(turns.count - Self.maxTurnsShown) }
        }
    }

    private func handle(_ e: ConversationEvent) {
        switch e {
        case .ready(_, let consent, let recovered):
            service = .ready
            self.consent = consent
            if recovered.sessions > 0 {
                banner = "Recovered after an unexpected stop: \(recovered.abandonedTurns) unfinished turn(s) closed."
            }
        case .turnOpened(let t): row(t) { _ in }
        case .capture(_, let on): if on && holding { startMicrophone() } else { stopMicrophone() }
        case .heard(let t, let text): row(t) { $0.heard = text }
        case .speaking(let t, let text): row(t) { $0.spoken.append(text) }
        case .confirm(let turn, let digest, let heard, let tool, let slots, let reasons):
            confirmation = PendingConfirmation(turn: turn, digest: digest, heard: heard, tool: tool, slots: slots, reasons: reasons)
        case .turnClosed(let t, let outcome, let timing):
            row(t) { r in
                r.outcome = outcome
                if let d = timing.endToFirstAudio { r.firstAudioMs = Int(d.components.seconds * 1000 + d.components.attoseconds / 1_000_000_000_000_000) }
            }
            if confirmation?.turn == t { confirmation = nil }
        case .degraded(let t, let stage, let reason):
            let text = Self.describe(stage, reason)
            if let t { row(t) { $0.problem = text } } else { banner = text }
        case .consent(let state):
            consent = state
        case .notice(let code):
            if code == "consent_required" { banner = "Turn on “Listen while I hold Space” to talk." }
            if code == "forget_refused_turn_active" { banner = "Finish or stop the current turn, then delete history." }
            if code == "service_not_running" { banner = "The session is restarting; press Space again in a moment." }
            if code == "stale_service_stopped" { banner = "A session left running by an earlier run of the app was stopped." }
        case .serviceExited:
            service = .restarting(0)
            stopMicrophone()
        case .serviceRestarting(let attempt):
            service = .restarting(attempt)
        case .serviceGaveUp:
            service = .stopped("The session service keeps stopping. Quit and reopen the app.")
        case .forgotten(let files, _):
            turns.removeAll()
            banner = "History deleted: \(files) session log(s) removed from this Mac (backups made earlier are not touched)."
        case .forgetFailed:
            banner = "History could not be deleted completely; some of what you said may still be on this Mac. Try again."
        case .eventsDropped(let n):
            banner = "The window fell behind and missed \(n) update(s); what it shows may be out of date."
        case .serviceUnresponsive:
            stopMicrophone()
            service = .restarting(0)
            banner = "The session stopped responding; it is being restarted. Nothing was done."
        case .serviceFailed(let code):
            stopMicrophone()
            let why: String =
                switch code {
                case "already_running": "Another copy of the app is using its data; this one takes over when that one quits."
                case "store_corrupt": "The session record is damaged and was not changed. See docs/DRILLS.md."
                case "binary_missing": "The session service (voice-session) is missing. Reinstall the app."
                case "launch_failed": "The session service could not be started again. Quit and reopen the app."
                default: "The session record could not be opened (disk full or unreadable)."
                }
            service = .stopped(why)
        }
    }

    static func describe(_ stage: Stage, _ reason: FailureReason) -> String {
        let what: String =
            switch stage {
            case .stt: "Speech recognition"
            case .lm: "The on-device model"
            case .tts: "Speech output"
            case .tool: "A tool"
            case .ledger: "The session record"
            }
        let why: String =
            switch reason {
            case .deadline: "did not answer in time"
            case .unavailable: "is unavailable"
            case .modelNotReady: "is not ready yet"
            case .assetsMissing: "needs its model downloaded"
            case .guardrail: "declined to answer"
            case .contextWindow: "ran out of context"
            case .tooLong: "got more than it can take"
            case .ledgerFull: "is full"
            default: "failed"
            }
        return "\(what) \(why). Nothing was done."
    }
}
