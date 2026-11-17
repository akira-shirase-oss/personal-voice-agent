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
    func recentDegraded() -> (Stage, FailureReason)? {
        for e in seen.reversed() { if case .degraded(_, let stage, let reason) = e { return (stage, reason) } }
        return nil
    }

    /// Whether any event so far, or before the timeout, satisfies `match` (ignores the cursor).
    func ever(timeout: Duration, _ match: @Sendable (ConversationEvent) -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            if seen.contains(where: match) { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return seen.contains(where: match)
    }

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

/// Counts how many recognition sessions were cancelled (released).
final class CountingRecognizer: SpeechRecognizing, @unchecked Sendable {
    let cancels = Mutex(0)
    final class Session: RecognitionSession, @unchecked Sendable {
        let owner: CountingRecognizer
        init(_ owner: CountingRecognizer) { self.owner = owner }
        func append(_ pcm: PCM) -> Bool { true }
        func finish() async throws -> HeardText { HeardText(text: "hello", minConfidence: 0.9) }
        func cancel() async { owner.cancels.withLock { $0 += 1 } }
    }
    func begin() async throws -> any RecognitionSession { Session(self) }
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

    /// A hung service (SIGSTOP) must not keep the microphone open, even while the key is held:
    /// the app notices the missed heartbeat, stops capture and playback, and replaces it.
    @Test("a hung service is detected, capture stops, and the service is replaced")
    func hungService() async throws {
        var config = ConversationConfig(
            binary: sessionBinary, dataDir: try tempDataDir("hung"),
            recognizer: ScriptedRecognizer(heard: HeardText(text: "hello", minConfidence: 0.95)),
            model: DeterministicTestProvider(), synthesizer: ToneSynthesizer())
        config.offlinePlayback = true
        config.heartbeatInterval = .milliseconds(200)
        config.heartbeatTimeout = .milliseconds(800)
        let c = try Conversation(config)
        let r = Recorder(c.events)
        let pump = OfflineRenderPump(c.player)
        defer { pump.stop() }
        try await startWithConsent(c, r)
        await c.pushToTalkDown()  // held throughout
        #expect(await r.next(isCaptureOn) != nil)
        let pid = try #require(await c.servicePID)
        kill(pid, SIGSTOP)
        let stopped = ContinuousClock.now
        let off = await r.next(timeout: .seconds(4)) { if case .capture(_, on: false) = $0 { true } else { false } }
        #expect(off != nil, "capture stops although the key is still held")
        #expect(ContinuousClock.now - stopped < .seconds(3))
        #expect(await r.seen.contains { if case .serviceUnresponsive = $0 { true } else { false } })
        #expect(await r.next(timeout: .seconds(5), isReady) != nil, "a new service is ready")
        kill(pid, SIGKILL)
        await c.shutdown()
    }

    func hungConfig(_ name: String, idle: Duration) throws -> ConversationConfig {
        var config = ConversationConfig(
            binary: sessionBinary, dataDir: try tempDataDir(name),
            recognizer: ScriptedRecognizer(heard: HeardText(text: "hello", minConfidence: 0.95)),
            model: DeterministicTestProvider(), synthesizer: ToneSynthesizer())
        config.offlinePlayback = true
        config.heartbeatInterval = .milliseconds(200)
        config.heartbeatTimeout = .milliseconds(800)
        config.idleHeartbeatInterval = idle
        return config
    }

    /// A service that hangs between turns (SIGSTOP while idle, no key pressed) is found by the
    /// idle heartbeat and replaced; the turn after the replacement is served.
    @Test("a service hung between turns is detected and replaced without a key press")
    func hungBetweenTurns() async throws {
        let c = try Conversation(try hungConfig("hung-idle", idle: .milliseconds(400)))
        let r = Recorder(c.events)
        let pump = OfflineRenderPump(c.player)
        defer { pump.stop() }
        try await startWithConsent(c, r)
        try await talk(c, r)
        #expect(await r.next { closed($0) != nil } != nil, "a first turn completes")
        let pid = try #require(await c.servicePID)
        kill(pid, SIGSTOP)
        defer { kill(pid, SIGKILL) }
        let stopped = ContinuousClock.now
        let found = await r.next(timeout: .seconds(4)) { if case .serviceUnresponsive = $0 { true } else { false } }
        #expect(found != nil, "the idle service's hang is noticed")
        #expect(ContinuousClock.now - stopped < .seconds(3))
        #expect(await r.next(timeout: .seconds(5), isReady) != nil, "a new service is ready")
        try await talk(c, r)
        let (_, outcome, _) = try #require(await r.next { closed($0) != nil }.flatMap(closed))
        #expect(outcome == .completed, "the next turn is served")
        await c.shutdown()
    }

    /// Pressing Space on a service that hung while idle: the press pings it, the missed pong is
    /// reported within the timeout (the app shows it), the microphone never opens, and a new
    /// service is started.
    @Test("a key press on a service hung between turns is answered, never silently ignored")
    func keyPressOnHungService() async throws {
        let c = try Conversation(try hungConfig("hung-press", idle: .seconds(60)))
        let r = Recorder(c.events)
        let pump = OfflineRenderPump(c.player)
        defer { pump.stop() }
        try await startWithConsent(c, r)
        let pid = try #require(await c.servicePID)
        kill(pid, SIGSTOP)
        defer { kill(pid, SIGKILL) }
        try await Task.sleep(for: .milliseconds(300))
        let pressed = ContinuousClock.now
        await c.pushToTalkDown()
        let found = await r.next(timeout: .seconds(4)) { if case .serviceUnresponsive = $0 { true } else { false } }
        #expect(found != nil, "the press finds the hang")
        #expect(ContinuousClock.now - pressed < .seconds(2))
        #expect(!(await r.seen.contains(where: isCaptureOn)), "the microphone never opened")
        #expect(await r.next(timeout: .seconds(5), isReady) != nil, "a new service is ready")
        await c.pushToTalkUp()
        try await talk(c, r)
        let (_, outcome, _) = try #require(await r.next { closed($0) != nil }.flatMap(closed))
        #expect(outcome == .completed, "the next press is served")
        await c.shutdown()
    }

    /// A turn interrupted while capturing must release its recognizer (a `SpeechAnalyzer` keeps
    /// its model for the process lifetime and would otherwise wait forever).
    @Test("an interrupted capture releases its recognition session")
    func recognizerReleased() async throws {
        let recognizer = CountingRecognizer()
        let (c, r, pump) = try makeConversation("release", recognizer: recognizer)
        defer { pump.stop() }
        try await startWithConsent(c, r)
        await c.pushToTalkDown()
        #expect(await r.next(isCaptureOn) != nil)
        try await Task.sleep(for: .milliseconds(100))  // the session has started
        await c.interrupt()
        _ = await r.next { closed($0) != nil }
        let deadline = ContinuousClock.now + .seconds(1)
        while recognizer.cancels.withLock({ $0 }) == 0 && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(10)) }
        #expect(recognizer.cancels.withLock { $0 } == 1, "the interrupted turn's recognition session was released")
        await c.shutdown()
    }

    /// Conversation events the app has not read yet are bounded; if some are lost, the app is
    /// told how many, instead of silently missing (for example) a turn's close.
    @Test("lost conversation events are reported, not silent")
    func lostEventsReported() async throws {
        var config = ConversationConfig(
            binary: sessionBinary, dataDir: try tempDataDir("lost"),
            recognizer: ScriptedRecognizer(heard: HeardText(text: "hi", minConfidence: 0.9)),
            model: DeterministicTestProvider(), synthesizer: ToneSynthesizer())
        config.offlinePlayback = true
        config.eventBuffer = 8
        let c = try Conversation(config)
        try await c.start()
        for i in 0..<40 { await c.setCaptureConsent(i % 2 == 0) }
        try await Task.sleep(for: .milliseconds(500))  // nobody reads yet
        var consents = 0
        var reported = false
        let reader = Task {
            for await e in c.events {
                if case .consent = e { consents += 1 }
                if "\(e)".hasPrefix("eventsDropped") { reported = true }
                if consents >= 40 || reported { break }
            }
            return (consents, reported)
        }
        let stop = Task {
            try await Task.sleep(for: .seconds(3))
            reader.cancel()
        }
        let (got, told) = await reader.value
        stop.cancel()
        #expect(got >= 40 || told, "events were lost without a word: \(got) of 40 consent changes seen")
        await c.shutdown()
    }

    /// A second app on the same data directory: its service refuses to start, and the app says
    /// so instead of restarting it in a loop.
    @Test("a second instance reports 'already running' and does not restart its service")
    func secondInstance() async throws {
        let dir = try tempDataDir("second")
        func make() throws -> (Conversation, Recorder) {
            var config = ConversationConfig(
                binary: sessionBinary, dataDir: dir, recognizer: ScriptedRecognizer(heard: HeardText(text: "hi", minConfidence: 0.9)),
                model: DeterministicTestProvider(), synthesizer: ToneSynthesizer())
            config.offlinePlayback = true
            let c = try Conversation(config)
            return (c, Recorder(c.events))
        }
        let (first, r1) = try make()
        try await first.start()
        #expect(await r1.next(isReady) != nil)
        let (second, r2) = try make()
        try await second.start()
        #expect(await r2.next { if case .notice("already_running") = $0 { true } else { false } } != nil)
        #expect(await r2.next { if case .serviceExited = $0 { true } else { false } } != nil)
        let restarted = await r2.next(timeout: .seconds(1)) { if case .serviceRestarting = $0 { true } else { false } }
        #expect(restarted == nil, "the refused service is not restarted as if it had crashed")
        #expect(await r2.ever(timeout: .seconds(3)) { if case .serviceFailed("already_running") = $0 { true } else { false } }, "the refusal is reported")
        // The other copy is alive and has a live parent: never treated as an orphan.
        let firstPID = try #require(await first.servicePID)
        #expect(kill(firstPID, 0) == 0, "the running copy's service was left alone")
        #expect(await !(r2.seen.contains { if case .notice("stale_service_stopped") = $0 { true } else { false } }))
        // Not permanent: once the first copy quits, the second takes over.
        await first.shutdown()
        #expect(await r2.next(timeout: .seconds(8), isReady) != nil, "the waiting copy becomes ready")
        await second.shutdown()
    }

    /// Quitting while the service is hung (SIGSTOP): the app does not return from shutdown until
    /// its child has exited, so the next launch on the same data directory is not refused.
    @Test("quitting ends a hung service, and the next launch is not refused")
    func quitEndsAHungChild() async throws {
        let dir = try tempDataDir("quit-hung")
        func make() throws -> (Conversation, Recorder) {
            var config = ConversationConfig(
                binary: sessionBinary, dataDir: dir, recognizer: ScriptedRecognizer(heard: HeardText(text: "hi", minConfidence: 0.9)),
                model: DeterministicTestProvider(), synthesizer: ToneSynthesizer())
            config.offlinePlayback = true
            let c = try Conversation(config)
            return (c, Recorder(c.events))
        }
        let (first, r1) = try make()
        try await first.start()
        #expect(await r1.next(isReady) != nil)
        let pid = try #require(await first.servicePID)
        kill(pid, SIGSTOP)
        defer { kill(pid, SIGKILL) }
        let t0 = ContinuousClock.now
        await first.shutdown()
        #expect(ContinuousClock.now - t0 < .seconds(4), "quit is bounded")
        #expect(kill(pid, 0) != 0, "the hung service is gone when quit returns")
        let (next, r2) = try make()
        try await next.start()
        #expect(await r2.next(timeout: .seconds(5), isReady) != nil, "the relaunch is not refused")
        #expect(await !(r2.seen.contains { if case .notice("already_running") = $0 { true } else { false } }))
        await next.shutdown()
    }

    /// An earlier app process died and left its service behind, hung, holding the lock (its
    /// parent is now launchd). The relaunched app recognises its own stale orphan, stops it and
    /// starts normally instead of reporting "already running" forever.
    @Test("a stale orphan holding the data directory is stopped and replaced")
    func staleOrphanIsReplaced() async throws {
        let dir = try tempDataDir("orphan")
        // A background pipeline whose shell exits at once: the service is reparented to launchd.
        let sh = Process()
        sh.executableURL = URL(fileURLWithPath: "/bin/sh")
        sh.arguments = ["-c", "sleep 20 2>/dev/null | \"$0\" --data-dir \"$1\" >/dev/null 2>&1 & echo $!", sessionBinary.path, dir.path]
        let out = Pipe()
        sh.standardOutput = out
        try sh.run()
        sh.waitUntilExit()
        let text = String(decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        let orphan = try #require(Int32(text.trimmingCharacters(in: .whitespacesAndNewlines)))
        defer { kill(orphan, SIGKILL) }
        let deadline = ContinuousClock.now + .seconds(3)
        while ServiceLock.holder(dataDir: dir) != orphan && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(10)) }
        #expect(ServiceLock.holder(dataDir: dir) == orphan, "the orphan holds the lock")
        kill(orphan, SIGSTOP)  // hung: it would never let go
        #expect(ServiceLock.isStaleOrphan(orphan, binary: sessionBinary), "recognised as a stale orphan of this app")
        var config = ConversationConfig(
            binary: sessionBinary, dataDir: dir, recognizer: ScriptedRecognizer(heard: HeardText(text: "hi", minConfidence: 0.9)),
            model: DeterministicTestProvider(), synthesizer: ToneSynthesizer())
        config.offlinePlayback = true
        let c = try Conversation(config)
        let r = Recorder(c.events)
        try await c.start()
        #expect(await r.next(timeout: .seconds(6), isReady) != nil, "the relaunched app starts")
        #expect(kill(orphan, 0) != 0, "the stale orphan was stopped")
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
        // Deadlines long enough that machine load cannot expire them: this test proves the
        // path works, so any degraded turn is a broken path, never "the machine was busy".
        // How fast it is belongs to the bench (scripts/bench-e2e.sh), which records load.
        let generous = ["stt_finalize_ms": "30000", "lm_first_ms": "60000", "lm_total_ms": "120000", "tts_first_audio_ms": "30000"]
        let (c, r, pump) = try makeConversation(
            "e2e", model: FoundationModelsProvider(), synthesizer: SystemVoiceSynthesizer(), recognizer: AppleSpeechRecognizer(),
            limits: generous)
        defer { pump.stop() }
        try await startWithConsent(c, r)
        await c.pushToTalkDown()
        #expect(await r.next(isCaptureOn) != nil)
        let pcm = try Audio.read(repoRoot.appending(path: "fixtures/audio/capital-question.wav"))
        for slice in Audio.slices(pcm, frames: 1_600) { await c.feed(slice) }
        await c.pushToTalkUp()
        let (_, outcome, timing) = try #require(await r.next(timeout: .seconds(150)) { closed($0) != nil }.flatMap(closed))
        let degraded = await r.recentDegraded()
        #expect(outcome == .completed, "the on-device path failed: \(outcome), \(degraded.map { "\($0)" } ?? "no degraded event")")
        #expect(pump.loudBlocks.withLock { $0 } > 0, "the reply was heard")
        let heard = await r.seen.compactMap { if case .heard(_, let t) = $0 { t } else { nil } }.first ?? ""
        #expect(Transcripts.wordErrorRate(reference: "What is the capital of France?", hypothesis: heard) <= 0.2, "heard: \(heard)")
        func ms(_ d: Duration?) -> Double { d.map { Double($0.components.attoseconds) / 1e15 + Double($0.components.seconds) * 1000 } ?? -1 }
        print(
            "METRIC e2e_ms end_to_first_audio=\(ms(timing.endToFirstAudio)) stt=\(ms(timing.recognizerFinalize)) lm_first_sentence=\(ms(timing.modelFirstSentence)) tts_first_audio=\(ms(timing.synthesisFirstAudio))"
        )
        await c.shutdown()
    }
}
