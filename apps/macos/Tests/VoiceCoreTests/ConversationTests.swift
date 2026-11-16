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

/// Renders an offline player continuously (as the output device would) and remembers when
/// sound was last heard.
final class RenderPump: @unchecked Sendable {
    let lastLoud = Mutex<ContinuousClock.Instant?>(nil)
    let loudBlocks = Mutex(0)
    private var task: Task<Void, Never>?
    init(_ player: TurnFencedPlayer) {
        task = Task.detached { [weak self] in
            while !Task.isCancelled {
                if let block = try? player.render(frames: 512), Audio.peak(block) > 0.05 {
                    self?.lastLoud.withLock { $0 = ContinuousClock.now }
                    self?.loudBlocks.withLock { $0 += 1 }
                }
                try? await Task.sleep(for: .milliseconds(10))  // ~real time: 512 frames ≈ 23 ms
            }
        }
    }
    func stop() { task?.cancel() }
}

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
    return (c, Recorder(c.events), RenderPump(c.player))
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
    @Test("a killed service is restarted, recovers its ledger, and serves the next turn")
    func crashRestart() async throws {
        let (c, r, pump) = try makeConversation("crash", model: DeterministicTestProvider(mode: .stall))
        defer { pump.stop() }
        try await startWithConsent(c, r)
        try await talk(c, r)
        _ = await r.next { if case .heard = $0 { true } else { false } }
        await c.flush()
        _ = await r.next { if case .notice("flushed") = $0 { true } else { false } }
        await c.killServiceForDrill()
        #expect(await r.next { if case .serviceExited = $0 { true } else { false } } != nil)
        #expect(await r.next { if case .serviceRestarting(1) = $0 { true } else { false } } != nil)
        let ready = await r.next(isReady)
        guard case .ready(_, let consent, let recovered)? = ready else { Issue.record("no ready after restart"); return }
        #expect(recovered.sessions == 1 && recovered.abandonedTurns == 1, "\(recovered)")
        #expect(consent.capture, "consent survived the crash")
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
        #expect(outcome == .completed)
        func ms(_ d: Duration?) -> Double { d.map { Double($0.components.attoseconds) / 1e15 + Double($0.components.seconds) * 1000 } ?? -1 }
        print(
            "METRIC e2e_ms end_to_first_audio=\(ms(timing.endToFirstAudio)) stt=\(ms(timing.recognizerFinalize)) lm_first_sentence=\(ms(timing.modelFirstSentence)) tts_first_audio=\(ms(timing.synthesisFirstAudio))"
        )
        #expect(pump.loudBlocks.withLock { $0 } > 0)
        await c.shutdown()
    }
}
