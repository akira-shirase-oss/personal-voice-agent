@preconcurrency import AVFoundation
import Foundation
import Synchronization
import Testing
@testable import VoiceCore

/// The service binary the tests drive: `PVA_SESSION_BINARY`, else `$CARGO_TARGET_DIR/debug`,
/// else the repository's `target/debug`. Tests that need it fail without it, never skip.
let sessionBinary: URL = {
    let env = ProcessInfo.processInfo.environment
    if let path = env["PVA_SESSION_BINARY"], !path.isEmpty { return URL(fileURLWithPath: path) }
    if let dir = env["CARGO_TARGET_DIR"], !dir.isEmpty {
        let target = dir.hasPrefix("/") ? URL(fileURLWithPath: dir) : repoRoot.appending(path: dir)
        return target.appending(path: "debug/voice-session")
    }
    return repoRoot.appending(path: "target/debug/voice-session")
}()
var haveBinary: Bool { FileManager.default.isExecutableFile(atPath: sessionBinary.path) }
let binaryMissing: Comment = "voice-session not found at \(sessionBinary.path): run cargo build -p voice-session, or set PVA_SESSION_BINARY"

/// The temporary files and directories one test made; removed when that test ends.
final class TestScratch: Sendable {
    @TaskLocal static var current: TestScratch?
    let urls = Mutex<[URL]>([])

    /// `TMPDIR` when set (Foundation ignores it on macOS; `scripts/check.sh` gives the tests a
    /// private one), else the user's temp directory.
    static var root: URL {
        if let dir = ProcessInfo.processInfo.environment["TMPDIR"], !dir.isEmpty { return URL(fileURLWithPath: dir) }
        return FileManager.default.temporaryDirectory
    }

    /// A path in the temp directory that is removed when the current test ends.
    static func url(_ name: String) -> URL {
        let url = root.appending(path: "pva-swift-\(name)-\(UUID().uuidString)")
        if let scratch = current {
            scratch.urls.withLock { $0.append(url) }
        } else {
            Issue.record("a temporary path outside a .removesScratch suite would outlive its test: \(name)")
        }
        return url
    }
}

/// Every test in the suite gets its own scratch list, removed when the test ends, pass or fail:
/// data directories hold heard transcripts, and `scripts/check.sh` fails if one is left.
struct RemovesScratch: SuiteTrait, TestTrait, TestScoping {
    var isRecursive: Bool { true }
    func scopeProvider(for test: Test, testCase: Test.Case?) -> RemovesScratch? { testCase == nil ? nil : self }
    func provideScope(for test: Test, testCase: Test.Case?, performing function: @Sendable () async throws -> Void) async throws {
        let scratch = TestScratch()
        defer { for url in scratch.urls.withLock({ $0 }) { try? FileManager.default.removeItem(at: url) } }
        try await TestScratch.$current.withValue(scratch) { try await function() }
    }
}

extension Trait where Self == RemovesScratch {
    static var removesScratch: Self { RemovesScratch() }
}

func tempDataDir(_ name: String) throws -> URL {
    try DataDirectory.prepare(TestScratch.url(name))
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

/// Hears a different phrase each turn (the last one repeats).
final class SequenceRecognizer: SpeechRecognizing, @unchecked Sendable {
    private let phrases: Mutex<[String]>
    init(_ phrases: [String]) { self.phrases = Mutex(phrases) }
    func begin() async throws -> any RecognitionSession {
        let text = phrases.withLock { p in p.count > 1 ? p.removeFirst() : p[0] }
        return try await ScriptedRecognizer(heard: HeardText(text: text, minConfidence: 0.95)).begin()
    }
}

/// Builds each prompt exactly as the on-device provider does (its recap of the last two
/// exchanges, then the heard text) and records it; answers like the test provider.
final class RecapProbeProvider: LanguageModeling, @unchecked Sendable {
    let modelID = "recap-probe-test-provider"
    let instructionsVersion = "test-1"
    let isTestProvider = true
    let prompts = Mutex<[String]>([])
    private let recap = Mutex<[(heard: String, reply: String)]>([])
    func availability() -> LMAvailability { .available }
    func forget() { recap.withLock { $0.removeAll() } }
    func respond(to heard: String, events: @escaping @Sendable (LMEvent) -> Void) async throws {
        let earlier = recap.withLock { $0 }
        prompts.withLock { $0.append(FoundationModelsProvider.prompt(heard, recap: earlier)) }
        try await DeterministicTestProvider().respond(to: heard, events: events)
        recap.withLock { r in
            r.append((heard, "You said: \(heard)"))
            if r.count > 2 { r.removeFirst(r.count - 2) }
        }
    }
}

@Suite("Conversation through the real session service", .serialized, .removesScratch)
struct ConversationTests {
    @Test("the session service binary is built") func binaryPresent() { #expect(haveBinary, binaryMissing) }

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

    /// A revocation the service never reads must still hold. `how`: "hung" (SIGSTOP, then the
    /// heartbeat replaces it), "killed" (SIGKILL before it reads), or "granted" (the replacement
    /// still reports granted: the token was re-created behind the app's back, as when the service
    /// created it for an earlier grant just before dying). In every case the replacement must
    /// not open capture on the next key press, and the app must show listening off.
    @Test("a revocation the service never reads still holds after it is replaced", arguments: ["hung", "killed", "granted"])
    func revocationTheServiceNeverRead(_ how: String) async throws {
        let config = try hungConfig("lost-revoke-\(how)", idle: .seconds(60))
        let c = try Conversation(config)
        let r = Recorder(c.events)
        let pump = OfflineRenderPump(c.player)
        defer { pump.stop() }
        try await startWithConsent(c, r)
        let pid = try #require(await c.servicePID)
        kill(pid, SIGSTOP)
        defer { kill(pid, SIGKILL) }
        await c.setCaptureConsent(false)  // queued in the pipe; the stopped service never reads it
        switch how {
        case "hung":
            // Holding the key keeps the heartbeat at its fast interval, so the hang is found
            // and the service replaced; the press itself is refused locally.
            await c.pushToTalkDown()
            #expect(await r.next(timeout: .seconds(4)) { if case .serviceUnresponsive = $0 { true } else { false } } != nil)
            await c.pushToTalkUp()
        case "killed":
            kill(pid, SIGKILL)
        default:
            FileManager.default.createFile(atPath: config.dataDir.appending(path: "consent.capture.granted").path, contents: nil, attributes: [.posixPermissions: 0o600])
            kill(pid, SIGKILL)
        }
        let ready = await r.next(timeout: .seconds(8), isReady)
        guard case .ready(_, let consent, _)? = ready else { Issue.record("no replacement service"); return }
        #expect(!consent.capture, "the replacement is shown revoked")
        if how == "granted" {
            #expect(await r.next(timeout: .seconds(3)) { if case .consent(let s) = $0 { !s.capture } else { false } } != nil, "revoked again and confirmed")
        }
        await c.pushToTalkDown()
        let opened = await r.next(timeout: .seconds(1), isCaptureOn)
        #expect(opened == nil, "capture opened after the user revoked it")
        await c.pushToTalkUp()
        await c.shutdown()
        // And the next start on this data directory is revoked too.
        let again = try Conversation(config)
        let r3 = Recorder(again.events)
        try await again.start()
        guard case .ready(_, let next, _)? = await r3.next(isReady) else { Issue.record("no ready"); return }
        #expect(!next.capture, "a later start is revoked")
        await again.shutdown()
    }

    /// A service that hangs before it says `ready` (in startup or recovery) answers no ping; the
    /// startup deadline finds it and it is replaced, instead of "starting" forever.
    @Test("a service that never becomes ready is replaced")
    func startupHangIsFound() async throws {
        let script = TestScratch.url("never-ready")
        try Data("#!/bin/sh\nexec sleep 30\n".utf8).write(to: script)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: script.path)
        var config = try hungConfig("never-ready", idle: .seconds(60))
        config.binary = script
        config.startupTimeout = .milliseconds(600)
        config.maxRestarts = 1
        let c = try Conversation(config)
        let r = Recorder(c.events)
        let t0 = ContinuousClock.now
        try await c.start()
        #expect(await r.next(timeout: .seconds(3)) { if case .serviceUnresponsive = $0 { true } else { false } } != nil, "the startup hang is found")
        #expect(ContinuousClock.now - t0 < .seconds(2))
        #expect(await r.next(timeout: .seconds(3)) { if case .serviceRestarting = $0 { true } else { false } } != nil, "and the service replaced")
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

    /// "Delete history" must reach the model too: its recap of the last two exchanges would
    /// otherwise quote deleted speech in the next prompt, and the reply could write it back
    /// into the new log. Revoking capture consent clears it as well.
    @Test("deleting history, or revoking consent, leaves no earlier heard text in the next prompt")
    func forgetClearsTheModelRecap() async throws {
        let probe = RecapProbeProvider()
        let secret = "my locker code is 4321"
        let recognizer = SequenceRecognizer([secret, "what time is it", "what time is it", secret, "what time is it"])
        let (c, r, pump) = try makeConversation("forget-recap", model: probe, recognizer: recognizer)
        defer { pump.stop() }
        try await startWithConsent(c, r)
        for _ in 0..<2 {
            try await talk(c, r)
            #expect(await r.next { closed($0) != nil } != nil)
        }
        #expect(probe.prompts.withLock { $0.last?.contains("4321") } == true, "the recap carries the earlier turn")
        await c.forgetHistory()
        #expect(await r.next { if case .forgotten = $0 { true } else { false } } != nil)
        try await talk(c, r)
        #expect(await r.next { closed($0) != nil } != nil)
        let afterForget = probe.prompts.withLock { $0.last ?? "" }
        #expect(!afterForget.contains("4321"), "deleted speech reached the model: \(afterForget)")
        // Revocation: speech heard before it must not reach a prompt after a re-grant.
        try await talk(c, r)
        #expect(await r.next { closed($0) != nil } != nil)
        await c.setCaptureConsent(false)
        #expect(await r.next { if case .consent(let s) = $0 { !s.capture } else { false } } != nil)
        await c.setCaptureConsent(true)
        #expect(await r.next { if case .consent(let s) = $0 { s.capture } else { false } } != nil)
        try await talk(c, r)
        #expect(await r.next { closed($0) != nil } != nil)
        let afterRevoke = probe.prompts.withLock { $0.last ?? "" }
        #expect(!afterRevoke.contains("4321"), "speech from before the revocation reached the model: \(afterRevoke)")
        await c.shutdown()
    }

    /// A failed deletion is shown, never silent: the service's `forget_failed` reaches the app as
    /// its own event (the window shows a banner for it).
    @Test("a deletion that fails is reported to the app")
    func forgetFailureIsReported() async throws {
        let dir = try tempDataDir("forget-fails")
        var config = ConversationConfig(
            binary: sessionBinary, dataDir: dir, recognizer: ScriptedRecognizer(heard: HeardText(text: "hi", minConfidence: 0.9)),
            model: DeterministicTestProvider(), synthesizer: ToneSynthesizer())
        config.offlinePlayback = true
        let c = try Conversation(config)
        let r = Recorder(c.events)
        try await startWithConsent(c, r)
        // An entry the service cannot delete: a directory inside the ledger.
        let stuck = dir.appending(path: "ledger/stuck.closed.log/inner")
        try FileManager.default.createDirectory(at: stuck, withIntermediateDirectories: true)
        await c.forgetHistory()
        #expect(await r.ever(timeout: .seconds(5)) { "\($0)".hasPrefix("forgetFailed") }, "the failure reaches the app as its own event")
        #expect(await !(r.seen.contains { if case .forgotten = $0 { true } else { false } }), "nothing is reported deleted")
        await c.shutdown()
    }

    /// A restart that cannot launch is reported, never left as "restarting" forever: a binary
    /// that has gone missing, and one that can no longer be executed.
    @Test("a restart that cannot launch the service reports why", arguments: ["missing", "broken"])
    func restartFailureIsReported(_ how: String) async throws {
        let copy = TestScratch.url("binary")
        try FileManager.default.copyItem(at: sessionBinary, to: copy)
        var config = ConversationConfig(
            binary: copy, dataDir: try tempDataDir("relaunch-\(how)"),
            recognizer: ScriptedRecognizer(heard: HeardText(text: "hi", minConfidence: 0.9)),
            model: DeterministicTestProvider(), synthesizer: ToneSynthesizer())
        config.offlinePlayback = true
        config.maxRestarts = 2
        let c = try Conversation(config)
        let r = Recorder(c.events)
        try await c.start()
        #expect(await r.next(isReady) != nil)
        try FileManager.default.removeItem(at: copy)
        if how == "broken" {
            // Executable, but not a program: the launch itself fails.
            try Data("not a program".utf8).write(to: copy)
            try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: copy.path)
        }
        await c.killServiceForDrill()
        #expect(await r.next { if case .serviceRestarting(1) = $0 { true } else { false } } != nil)
        let expected = how == "missing" ? "binary_missing" : "launch_failed"
        let reported = await r.ever(timeout: .seconds(5)) { if case .serviceFailed(let code) = $0 { code == expected } else { false } }
        let seen = await r.seen
        #expect(reported, "the failed relaunch is reported as \(expected): \(seen)")
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

@Suite("On-device language model", .removesScratch)
struct ModelTests {
    @Test("after a context overflow, retry only if nothing was streamed (no spliced replies)")
    func overflowRetry() {
        #expect(FoundationModelsProvider.mayRetryAfterOverflow(eventsSent: 0))
        #expect(!FoundationModelsProvider.mayRetryAfterOverflow(eventsSent: 1))
        #expect(!FoundationModelsProvider.mayRetryAfterOverflow(eventsSent: 3))
    }

    @Test("the recap keeps two exchanges, and forget clears it and drops a turn still running")
    func recapForget() {
        let recap = ConversationRecap(turns: 2, chars: 300)
        for i in 1...3 {
            let g = recap.begin().generation
            recap.remember("heard \(i)", "reply \(i)", generation: g)
        }
        #expect(recap.begin().recap.map(\.heard) == ["heard 2", "heard 3"])
        let running = recap.begin().generation  // a turn starts, then history is deleted
        recap.forget()
        #expect(recap.isEmpty)
        recap.remember("deleted speech", "its reply", generation: running)
        #expect(recap.isEmpty, "a turn that began before the deletion cannot write it back")
        let fresh = recap.begin()
        #expect(fresh.recap.isEmpty)
        #expect(FoundationModelsProvider.prompt("hi", recap: fresh.recap) == "The user said: «hi»")
        recap.remember(String(repeating: "x", count: 400), "ok", generation: fresh.generation)
        #expect(recap.begin().recap.first?.heard.count == 300)
    }

    @Test("the on-device provider's forget clears its recap")
    func providerForget() {
        let p = FoundationModelsProvider()
        p.recap.remember("my locker code is 4321", "noted", generation: p.recap.begin().generation)
        #expect(!p.recap.isEmpty)
        p.forget()
        #expect(p.recap.isEmpty)
    }

    static var available: Bool { FoundationModelsProvider().availability() == .available }

    @Test("FoundationModels streams kind first, then sentences, then the proposal", .enabled("on-device: FoundationModels is available") { ModelTests.available })
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

    @Test("end to end on this Mac: fixture audio → recognizer → service → model → voice", .enabled("on-device: speech assets, FoundationModels and a system voice") {
        await OnDevice.speechReady() && ModelTests.available && OnDevice.systemVoice
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
