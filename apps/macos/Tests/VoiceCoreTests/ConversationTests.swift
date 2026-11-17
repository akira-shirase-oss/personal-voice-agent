@preconcurrency import AVFoundation
import Foundation
import Synchronization
import Testing
@testable import VoiceCore

let sessionBinary = repoRoot.appending(path: "target/debug/voice-session")
var haveBinary: Bool { FileManager.default.isExecutableFile(atPath: sessionBinary.path) }

func tempDataDir(_ name: String) throws -> URL {
    let url = FileManager.default.temporaryDirectory.appending(path: "pva-swift-\(name)-\(UUID().uuidString)")
    return try DataDirectory.prepare(url)
}

/// Collects a conversation's events so tests can wait for one.
actor Recorder {
    private(set) var seen: [ConversationEvent] = []
    private var cursor = 0
    init(_ events: AsyncStream<ConversationEvent>) {
        Task { for await e in events { await self.add(e) } }
    }
    private func add(_ e: ConversationEvent) { seen.append(e) }

    /// The next event (after the previous match) satisfying `match`, or nil at the timeout.
    func next(timeout: Duration = .seconds(10), _ match: @Sendable (ConversationEvent) -> Bool) async -> ConversationEvent? {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            while cursor < seen.count {
                let e = seen[cursor]
                cursor += 1
                if match(e) { return e }
            }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return nil
    }
}

typealias RenderPump = OfflineRenderPump

func makeConversation(
    _ name: String, heard: String = "what is the capital of France", model: any LanguageModeling = DeterministicTestProvider(),
    synthesizer: any SpeechSynthesizing = ToneSynthesizer(), recognizer: (any SpeechRecognizing)? = nil, limits: [String: String] = [:]
) throws -> (Conversation, Recorder, RenderPump) {
    var config = ConversationConfig(
        binary: sessionBinary, dataDir: try tempDataDir(name),
        recognizer: recognizer ?? ScriptedRecognizer(heard: HeardText(text: heard, minConfidence: 0.95)),
        model: model, synthesizer: synthesizer)
    config.offlinePlayback = true
    config.limits = limits
    let c = try Conversation(config)
    return (c, Recorder(c.events), OfflineRenderPump(c.player))
}

func isReady(_ e: ConversationEvent) -> Bool { if case .ready = e { true } else { false } }
func isCaptureOn(_ e: ConversationEvent) -> Bool { if case .capture(_, on: true) = e { true } else { false } }
func closed(_ e: ConversationEvent) -> (TurnID, Outcome, TurnTiming)? { if case .turnClosed(let t, let o, let timing) = e { (t, o, timing) } else { nil } }

func startWithConsent(_ c: Conversation, _ r: Recorder) async throws {
    try await c.start()
    #expect(await r.next(isReady) != nil, "service ready")
    await c.setCaptureConsent(true)
    #expect(await r.next { if case .consent(let s) = $0 { s.capture } else { false } } != nil)
}

/// One push-to-talk turn with a little fixture audio.
func talk(_ c: Conversation, _ r: Recorder) async throws {
    await c.pushToTalkDown()
    #expect(await r.next(isCaptureOn) != nil, "capture on")
    for slice in Audio.slices(Audio.tone(seconds: 0.3, format: Audio.monoFloat(16_000)), frames: 1_600) { await c.feed(slice) }
    await c.pushToTalkUp()
}

/// Stalls on its first turn (so a crash can land mid-turn), then answers like the test provider.
final class StallOnceProvider: LanguageModeling, @unchecked Sendable {
    let modelID = "stall-once-test-provider"
    let instructionsVersion = "test-1"
    let isTestProvider = true
    private let calls = Mutex(0)
    func availability() -> LMAvailability { .available }
    func respond(to heard: String, events: @escaping @Sendable (LMEvent) -> Void) async throws {
        let n = calls.withLock { c -> Int in
            c += 1
            return c
        }
        if n == 1 { try await Task.sleep(for: .seconds(3_600)) }
        try await DeterministicTestProvider().respond(to: heard, events: events)
    }
}

@Suite("Conversation through the real session service", .serialized, .enabled("voice-session is built (cargo build -p voice-session)") { haveBinary })
struct ConversationTests {
    @Test("a reply turn is heard, spoken and closed, with timing")
    func replyTurn() async throws {
        let (c, r, pump) = try makeConversation("reply")
        defer { pump.stop() }
        try await startWithConsent(c, r)
        try await talk(c, r)
        let heard = await r.next { if case .heard = $0 { true } else { false } }
        guard case .heard(_, let text)? = heard else { Issue.record("no heard event"); return }
        #expect(text == "what is the capital of France")
        let done = await r.next { closed($0) != nil }
        let (_, outcome, timing) = try #require(done.flatMap(closed))
        #expect(outcome == .completed)
        #expect(timing.endToFirstAudio != nil)
        #expect(pump.loudBlocks.withLock { $0 } > 0, "the reply was played")
        await c.shutdown()
    }

    /// Failure case: a stalled provider is cut off at its deadline; degraded, nothing invented.
    @Test("a stalled model degrades at the service deadline and nothing plays")
    func stalledModel() async throws {
        let (c, r, pump) = try makeConversation("stall", model: DeterministicTestProvider(mode: .stall), limits: ["lm_first_ms": "300"])
        defer { pump.stop() }
        try await startWithConsent(c, r)
        let t0 = ContinuousClock.now
        try await talk(c, r)
        let degraded = await r.next { if case .degraded(_, .lm, .deadline) = $0 { true } else { false } }
        #expect(degraded != nil)
        #expect(ContinuousClock.now - t0 < .seconds(3))
        let (_, outcome, _) = try #require(await r.next { closed($0) != nil }.flatMap(closed))
        #expect(outcome == .degraded)
        #expect(pump.loudBlocks.withLock { $0 } == 0, "no invented answer was spoken")
        await c.shutdown()
    }

    /// Failure case: interrupt mid-utterance — audio stops, late chunks never play.
    @Test("interrupting mid-utterance silences the turn and its late chunks")
    func interruptMidUtterance() async throws {
        let slow = ToneSynthesizer(secondsPerCharacter: 0.05)  // a ~1.5 s chunk per sentence
        let (c, r, pump) = try makeConversation("interrupt", synthesizer: slow)
        defer { pump.stop() }
        try await startWithConsent(c, r)
        try await talk(c, r)
        // Wait until the reply is audible.
        let deadline = ContinuousClock.now + .seconds(5)
        while pump.loudBlocks.withLock({ $0 }) == 0 && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        #expect(pump.loudBlocks.withLock { $0 } > 0)
        let interruptedAt = ContinuousClock.now
        await c.interrupt()
        let (_, outcome, _) = try #require(await r.next { closed($0) != nil }.flatMap(closed))
        #expect(outcome == .interrupted)
        // The second sentence's synthesis and any in-flight buffers belong to the old turn.
        try await Task.sleep(for: .seconds(2))
        let last = try #require(pump.lastLoud.withLock { $0 })
        let tail = last - interruptedAt
        print("METRIC e2e_offline_stop_last_loud_after_interrupt_ms=\(Double(tail.components.attoseconds) / 1e15 + Double(tail.components.seconds) * 1000)")
        #expect(tail < .milliseconds(200), "audio continued \(tail) after the interrupt")
        await c.shutdown()
    }

    /// Failure case: consent revoked mid-turn stops capture and blocks the next turn.
    @Test("revoking consent mid-turn stops capture and refuses push-to-talk")
    func revokeMidTurn() async throws {
        let (c, r, pump) = try makeConversation("revoke")
        defer { pump.stop() }
        try await startWithConsent(c, r)
        await c.pushToTalkDown()
        #expect(await r.next(isCaptureOn) != nil)
        await c.setCaptureConsent(false)
        #expect(await r.next { if case .capture(_, on: false) = $0 { true } else { false } } != nil)
        let (_, outcome, _) = try #require(await r.next { closed($0) != nil }.flatMap(closed))
        #expect(outcome == .consentRevoked)
        await c.pushToTalkDown()
        #expect(await r.next { if case .notice("consent_required") = $0 { true } else { false } } != nil)
        await c.shutdown()
    }

    /// Failure case: the service crashes; the app restarts it, recovery closes the open turn,
    /// and the next turn works.
    /// Failure case: the service crashes mid-turn; the app restarts it, recovery closes the
    /// open turn, and the next turn is actually spoken (the new session numbers turns from 1).
    @Test("a killed service is restarted, recovers its ledger, and serves the next turn")
    func crashRestart() async throws {
        let (c, r, pump) = try makeConversation("crash", model: StallOnceProvider())
        defer { pump.stop() }
        try await startWithConsent(c, r)
        try await talk(c, r)
        _ = await r.next { if case .heard = $0 { true } else { false } }
        await c.flush()
        _ = await r.next { if case .notice("flushed") = $0 { true } else { false } }
        await c.killServiceForDrill()  // turn 1 is open, waiting on the (stalled) model
        #expect(await r.next { if case .serviceExited = $0 { true } else { false } } != nil)
        #expect(await r.next { if case .serviceRestarting(1) = $0 { true } else { false } } != nil)
        let ready = await r.next(isReady)
        guard case .ready(_, let consent, let recovered)? = ready else { Issue.record("no ready after restart"); return }
        #expect(recovered.sessions == 1 && recovered.abandonedTurns == 1, "\(recovered)")
        #expect(consent.capture, "consent survived the crash")
        // The next turn — turn 1 of the new session — must complete and be heard.
        let loudBefore = pump.loudBlocks.withLock { $0 }
        try await talk(c, r)
        let (turn, outcome, _) = try #require(await r.next { closed($0) != nil }.flatMap(closed))
        #expect(turn == TurnID(1), "the new session numbers turns from 1")
        #expect(outcome == .completed, "the first turn after a restart is served")
        #expect(pump.loudBlocks.withLock { $0 } > loudBefore, "and it is heard")
        await c.shutdown()
    }

    /// The key comes up before the service's `capture on` arrives: the microphone must never
    /// open (it used to record until the service's capture limit).
    @Test("a quick tap never opens the microphone")
    func quickTap() async throws {
        let (c, r, pump) = try makeConversation("quicktap", limits: ["capture_max_ms": "3000"])
        defer { pump.stop() }
        try await startWithConsent(c, r)
        await c.pushToTalkDown()
        await c.pushToTalkUp()  // before any reply from the service
        let t0 = ContinuousClock.now
        #expect(await r.next(timeout: .seconds(2)) { closed($0) != nil } != nil, "the tapped turn closes promptly")
        #expect(ContinuousClock.now - t0 < .seconds(1))
        let opened = await r.seen.contains { if case .capture(_, on: true) = $0 { true } else { false } }
        #expect(!opened, "the microphone was turned on after the key had come up")
        await c.shutdown()
    }

    /// Holding the key past the app's own capture cap stops capture, whatever the service does.
    @Test("capture stops at the app's own cap, independent of the service")
    func localCaptureCap() async throws {
        var config = ConversationConfig(
            binary: sessionBinary, dataDir: try tempDataDir("localcap"),
            recognizer: ScriptedRecognizer(heard: HeardText(text: "hello", minConfidence: 0.95)),
            model: DeterministicTestProvider(), synthesizer: ToneSynthesizer())
        config.offlinePlayback = true
        config.limits = ["capture_max_ms": "30000"]
        config.localCaptureMax = .milliseconds(300)
        let c = try Conversation(config)
        let r = Recorder(c.events)
        let pump = OfflineRenderPump(c.player)
        defer { pump.stop() }
        try await startWithConsent(c, r)
        await c.pushToTalkDown()  // and never released
        #expect(await r.next(isCaptureOn) != nil)
        let on = ContinuousClock.now
        #expect(await r.next(timeout: .seconds(2)) { if case .capture(_, on: false) = $0 { true } else { false } } != nil)
        #expect(ContinuousClock.now - on < .milliseconds(800), "stopped by the local cap, not the service's 30 s")
        await c.shutdown()
    }

    @Test("an action proposal is refused, not executed (no tools exist in this release)")
    func actionRefused() async throws {
        let (c, r, pump) = try makeConversation("action", heard: "note buy milk")
        defer { pump.stop() }
        try await startWithConsent(c, r)
        try await talk(c, r)
        let spoken = await r.next { if case .speaking = $0 { true } else { false } }
        guard case .speaking(_, let text)? = spoken else { Issue.record("nothing spoken"); return }
        #expect(text.contains("can't do that here"), "\(text)")
        let (_, outcome, _) = try #require(await r.next { closed($0) != nil }.flatMap(closed))
        #expect(outcome == .refused)
        await c.shutdown()
    }
}

@Suite("On-device language model")
struct ModelTests {
    @Test("after a context overflow, retry only if nothing was streamed (no spliced replies)")
    func overflowRetry() {
        #expect(FoundationModelsProvider.mayRetryAfterOverflow(eventsSent: 0))
        #expect(!FoundationModelsProvider.mayRetryAfterOverflow(eventsSent: 1))
        #expect(!FoundationModelsProvider.mayRetryAfterOverflow(eventsSent: 3))
    }

    static var available: Bool { FoundationModelsProvider().availability() == .available }

    @Test("FoundationModels streams kind first, then sentences, then the proposal", .enabled("FoundationModels is available") { ModelTests.available })
    func streams() async throws {
        let events = Mutex<[LMEvent]>([])
        let clock = ContinuousClock()
        let t0 = clock.now
        let first = Mutex<Duration?>(nil)
        try await FoundationModelsProvider().respond(to: "What is the capital of France?") { e in
            first.withLock { if $0 == nil { $0 = clock.now - t0 } }
            events.withLock { $0.append(e) }
        }
        let got = events.withLock { $0 }
        guard case .kind(let kind)? = got.first, case .done(let proposal)? = got.last else {
            Issue.record("bad event order: \(got)")
            return
        }
        #expect(proposal.kind == kind)
        let sentences = got.compactMap { if case .sentence(_, let s) = $0 { s } else { nil } }
        if kind == .reply {
            #expect(!sentences.isEmpty)
            guard case .reply(let text) = proposal else { return }
            #expect(text.lowercased().contains("paris"), "\(text)")
        } else {
            #expect(sentences.isEmpty, "an action's reply is never streamed for speech")
        }
        print("METRIC lm_first_event_ms=\(first.withLock { $0 }.map { Double($0.components.attoseconds) / 1e15 + Double($0.components.seconds) * 1000 } ?? -1)")
    }

    @Test("end to end on this Mac: fixture audio → recognizer → service → model → voice", .enabled("speech assets, FoundationModels and a system voice") {
        await OnDevice.speechReady() && ModelTests.available && OnDevice.systemVoice && haveBinary
    })
    func endToEnd() async throws {
        let (c, r, pump) = try makeConversation(
            "e2e", model: FoundationModelsProvider(), synthesizer: SystemVoiceSynthesizer(), recognizer: AppleSpeechRecognizer())
        defer { pump.stop() }
        try await startWithConsent(c, r)
        await c.pushToTalkDown()
        #expect(await r.next(isCaptureOn) != nil)
        let pcm = try Audio.read(repoRoot.appending(path: "fixtures/audio/capital-question.wav"))
        for slice in Audio.slices(pcm, frames: 1_600) { await c.feed(slice) }
        await c.pushToTalkUp()
        let (_, outcome, timing) = try #require(await r.next(timeout: .seconds(30)) { closed($0) != nil }.flatMap(closed))
        func ms(_ d: Duration?) -> Double { d.map { Double($0.components.attoseconds) / 1e15 + Double($0.components.seconds) * 1000 } ?? -1 }
        // The invariant, whatever the machine's load: a turn either completes and is heard, or
        // degrades at a deadline and says nothing. How often it completes, and how fast, is the
        // bench's job (scripts/bench-e2e.sh records load average with every run).
        switch outcome {
        case .completed:
            #expect(pump.loudBlocks.withLock { $0 } > 0, "a completed turn was heard")
            print(
                "METRIC e2e_ms end_to_first_audio=\(ms(timing.endToFirstAudio)) stt=\(ms(timing.recognizerFinalize)) lm_first_sentence=\(ms(timing.modelFirstSentence)) tts_first_audio=\(ms(timing.synthesisFirstAudio))"
            )
        case .degraded:
            #expect(pump.loudBlocks.withLock { $0 } == 0, "a degraded turn speaks nothing")
            print("NOTE e2e turn degraded at a provider deadline; nothing was spoken (check uptime: the model may be starved)")
        default:
            Issue.record("unexpected outcome \(outcome)")
        }
        await c.shutdown()
    }
}
