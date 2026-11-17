// The app side of a conversation (D-016): carries out the session service's provider
// requests with the on-device providers and reports results back. It decides nothing about
// what is said or executed — the service does — but it stops playback locally the moment the
// user interrupts (D-021), before the round trip.

@preconcurrency import AVFoundation
import Foundation
import Synchronization

public enum ConversationEvent: Sendable {
    case ready(session: String, consent: ConsentState, recovered: Recovered)
    case turnOpened(TurnID)
    case capture(TurnID, on: Bool)
    case heard(TurnID, String)
    case speaking(TurnID, String)
    case confirm(turn: TurnID, digest: String, heard: String, tool: String, slots: [Slot], reasons: [ConfirmReason])
    case turnClosed(TurnID, Outcome, TurnTiming)
    case degraded(TurnID?, Stage, FailureReason)
    case consent(ConsentState)
    case notice(String)
    case serviceExited(Int32)
    case serviceRestarting(attempt: Int)
    case serviceGaveUp
}

/// Per-turn timestamps on the continuous clock, for the latency evidence (END_GOAL).
public struct TurnTiming: Sendable, Equatable {
    public var pttUp: ContinuousClock.Instant?
    public var heardSent: ContinuousClock.Instant?
    public var firstSentence: ContinuousClock.Instant?
    public var firstTTSRequest: ContinuousClock.Instant?
    public var firstAudio: ContinuousClock.Instant?

    /// End of turn (push-to-talk release) → first audio buffer scheduled on the player.
    public var endToFirstAudio: Duration? { pttUp.flatMap { a in firstAudio.map { $0 - a } } }
    public var recognizerFinalize: Duration? { pttUp.flatMap { a in heardSent.map { $0 - a } } }
    public var modelFirstSentence: Duration? { heardSent.flatMap { a in firstSentence.map { $0 - a } } }
    public var synthesisFirstAudio: Duration? { firstTTSRequest.flatMap { a in firstAudio.map { $0 - a } } }
}

public struct ConversationConfig: Sendable {
    public var binary: URL
    public var dataDir: URL
    public var limits: [String: String] = [:]
    public var recognizer: any SpeechRecognizing
    public var model: any LanguageModeling
    public var synthesizer: any SpeechSynthesizing
    public var offlinePlayback = false
    public var playerSampleRate: Double = 22_050
    public var maxRestarts = 5
    /// The app's own bound on one push-to-talk capture, independent of the service.
    public var localCaptureMax: Duration = .seconds(60)

    public init(binary: URL, dataDir: URL, recognizer: any SpeechRecognizing, model: any LanguageModeling, synthesizer: any SpeechSynthesizing) {
        self.binary = binary
        self.dataDir = dataDir
        self.recognizer = recognizer
        self.model = model
        self.synthesizer = synthesizer
    }
}

/// Counts a chunk's scheduled buffers so `tts_finished` is sent once, after the last one played.
final class ChunkProgress: @unchecked Sendable {
    private let state = Mutex((scheduled: 0, played: 0, synthesized: false, finished: false, started: false))

    /// True for the first scheduled buffer.
    func scheduled() -> Bool {
        state.withLock { s in
            s.scheduled += 1
            defer { s.started = true }
            return !s.started
        }
    }
    var anyScheduled: Bool { state.withLock { $0.started } }
    /// True when this completes the chunk.
    func played() -> Bool { state.withLock { s in s.played += 1; return Self.done(&s) } }
    func synthesisEnded() -> Bool { state.withLock { s in s.synthesized = true; return Self.done(&s) } }
    private static func done(_ s: inout (scheduled: Int, played: Int, synthesized: Bool, finished: Bool, started: Bool)) -> Bool {
        guard s.synthesized, s.started, s.played >= s.scheduled, !s.finished else { return false }
        s.finished = true
        return true
    }
}

public actor Conversation {
    public nonisolated let events: AsyncStream<ConversationEvent>
    private let emit: AsyncStream<ConversationEvent>.Continuation
    public nonisolated let player: TurnFencedPlayer
    private let config: ConversationConfig
    private let clock = ContinuousClock()

    private var process: SessionProcess?
    private var consent = ConsentState(capture: false, personalVoice: false)
    private var currentTurn: TurnID?
    /// The turn whose audio is being captured: the microphone is on exactly while this is set.
    private var capturing: TurnID?
    /// Whether push-to-talk is held, as the user's last key event says — the app's own truth,
    /// independent of the service's round trip.
    private var keyHeld = false
    private var recognition: [TurnID: Task<(any RecognitionSession)?, Never>] = [:]
    /// Sessions that have started; audio is appended to them synchronously, in order.
    private var sessions: [TurnID: any RecognitionSession] = [:]
    /// Audio captured before the session started (bounded).
    private var pendingAudio: [TurnID: [PCM]] = [:]
    private var calls: [CallID: Task<Void, Never>] = [:]
    private var synthesisTail: [TurnID: Task<Void, Never>] = [:]
    private var callTurn: [CallID: TurnID] = [:]
    private var timing: [TurnID: TurnTiming] = [:]
    private var restarts = 0
    /// One per launch of the service; tags every chunk so a restarted session (which numbers
    /// turns from 1 again) is never confused with the one that died.
    private var epoch: UInt64 = 0
    private var stopping = false
    private static let maxPendingBuffers = 512

    public init(_ config: ConversationConfig) throws {
        self.config = config
        player = try TurnFencedPlayer(format: Audio.monoFloat(config.playerSampleRate), offline: config.offlinePlayback)
        (events, emit) = AsyncStream.makeStream(of: ConversationEvent.self, bufferingPolicy: .bufferingNewest(4_096))
    }

    // MARK: Lifecycle

    public func start() throws {
        config.model.prewarm()
        let p = try SessionProcess(binary: config.binary, dataDir: config.dataDir, limits: config.limits)
        epoch += 1
        player.beginEpoch(epoch)
        process = p
        try p.send(.hello(protocol: protocolVersion, client: "voice-agent-macos/0.1.0"))
        Task { [weak self] in
            for await event in p.events { await self?.handle(event, from: p) }
        }
    }

    public func shutdown() {
        stopping = true
        try? process?.send(.shutdown)
        process?.close()
        for task in calls.values { task.cancel() }
        player.shutdown()
    }

    /// Crash drill: kills the child with SIGKILL; the conversation restarts it.
    public func killServiceForDrill() { process?.kill() }
    public var servicePID: Int32? { process?.pid }

    private func handleExit(_ status: Int32, of p: SessionProcess) {
        guard p === process else { return }
        emit.yield(.serviceExited(status))
        // The session is gone: stop its audio and drop all of its call state. The next
        // session reuses turn and call numbers from 1, so nothing of this one may linger.
        player.stop(fence: TurnID(UInt64.max), epoch: epoch)
        for task in calls.values { task.cancel() }
        for task in synthesisTail.values { task.cancel() }
        for session in sessions.values { Task { await session.cancel() } }
        calls.removeAll()
        callTurn.removeAll()
        synthesisTail.removeAll()
        recognition.removeAll()
        sessions.removeAll()
        pendingAudio.removeAll()
        timing.removeAll()
        if let c = capturing { stopMicrophone(c) }
        currentTurn = nil
        guard !stopping else { return }
        guard restarts < config.maxRestarts else {
            emit.yield(.serviceGaveUp)
            return
        }
        restarts += 1
        let attempt = restarts
        emit.yield(.serviceRestarting(attempt: attempt))
        Task { [weak self] in
            // Bounded backoff: 100 ms, 200 ms, 400 ms, ...
            try? await Task.sleep(for: .milliseconds(100 << (attempt - 1)))
            try? await self?.start()
        }
    }

    // MARK: User actions

    public func setCaptureConsent(_ grant: Bool) {
        if !grant { stopLocalWork(fence: currentTurn.map { TurnID($0.raw + 1) }) }
        send(.consent(scope: .capture, grant: grant, rightsReview: nil))
    }

    public func pushToTalkDown() {
        keyHeld = true
        // Barge-in: stop the assistant here before the service even hears about it (D-023).
        if let t = currentTurn { player.stop(fence: TurnID(t.raw + 1), epoch: epoch) }
        send(.pttDown)
    }

    /// Key up: capture stops here and now, before any reply from the service. If the service
    /// has not turned capture on yet, it never will be: see `.capture(on: true)`.
    public func pushToTalkUp() {
        keyHeld = false
        guard let t = capturing else { return }
        release(t)
    }

    /// Ends capture for `turn` locally and tells the service push-to-talk is up.
    private func release(_ turn: TurnID) {
        stopMicrophone(turn)
        timing[turn, default: TurnTiming()].pttUp = clock.now
        send(.pttUp(turn: turn))
    }

    private func stopMicrophone(_ turn: TurnID) {
        guard capturing == turn else { return }
        capturing = nil
        emit.yield(.capture(turn, on: false))
    }

    public func interrupt() {
        if let t = currentTurn { player.stop(fence: TurnID(t.raw + 1), epoch: epoch) }
        send(.interrupt)
    }

    public func confirm(turn: TurnID, digest: String, approve: Bool) {
        send(.confirm(turn: turn, digest: digest, approve: approve))
    }

    /// Captured audio for the capturing turn — from the microphone or a fixture. Bounded.
    public func feed(_ pcm: PCM) {
        guard let turn = capturing else { return }
        if let session = sessions[turn] {
            session.append(pcm)
        } else if pendingAudio[turn] != nil, pendingAudio[turn]!.count < Self.maxPendingBuffers {
            pendingAudio[turn]!.append(pcm)
        }
    }

    public func flush() { send(.flush) }

    private func send(_ m: ClientMessage) {
        do { try process?.send(m) } catch { emit.yield(.notice("service_not_running")) }
    }

    // MARK: Service messages

    private func handle(_ event: SessionEvent, from p: SessionProcess) {
        guard p === process else { return }
        switch event {
        case .exited(let status): handleExit(status, of: p)
        case .undecodable: emit.yield(.notice("undecodable_service_line"))
        case .message(let m): handle(m)
        }
    }

    private func handle(_ m: ServiceMessage) {
        switch m {
        case .ready(_, let session, let consent, let recovered):
            self.consent = consent
            emit.yield(.ready(session: session, consent: consent, recovered: recovered))
        case .turnOpened(let turn):
            currentTurn = turn
            emit.yield(.turnOpened(turn))
        case .capture(let turn, on: true):
            beginRecognition(turn)
            if keyHeld {
                capturing = turn
                emit.yield(.capture(turn, on: true))
                capLocally(turn)
            } else {
                // The key came up before the service answered: the microphone never opens,
                // and the (empty) turn ends at once.
                pendingAudio.removeValue(forKey: turn)
                timing[turn, default: TurnTiming()].pttUp = clock.now
                send(.pttUp(turn: turn))
            }
        case .capture(let turn, on: false):
            stopMicrophone(turn)
        case .sttRequest(let call, let turn, _):
            finishRecognition(call: call, turn: turn)
        case .lmRequest(let call, let turn, _, let heard):
            emit.yield(.heard(turn, heard))
            runModel(call: call, turn: turn, heard: heard)
        case .ttsRequest(let call, let turn, let seq, let text, let voice, _):
            if timing[turn]?.firstTTSRequest == nil { timing[turn, default: TurnTiming()].firstTTSRequest = clock.now }
            emit.yield(.speaking(turn, text))
            speak(call: call, turn: turn, seq: seq, text: text, voice: voice)
        case .cancel(let call):
            calls.removeValue(forKey: call)?.cancel()
            callTurn.removeValue(forKey: call)
        case .stopPlayback(let fence):
            stopLocalWork(fence: fence)
        case .confirmRequest(let turn, let digest, let heard, let tool, let slots, let reasons):
            emit.yield(.confirm(turn: turn, digest: digest, heard: heard, tool: tool, slots: slots, reasons: reasons))
        case .turnClosed(let turn, let outcome):
            let t = timing.removeValue(forKey: turn) ?? TurnTiming()
            if currentTurn == turn { currentTurn = nil }
            recognition.removeValue(forKey: turn)
            sessions.removeValue(forKey: turn)
            pendingAudio.removeValue(forKey: turn)
            synthesisTail.removeValue(forKey: turn)
            restarts = 0
            emit.yield(.turnClosed(turn, outcome, t))
        case .degraded(let turn, let stage, let reason):
            emit.yield(.degraded(turn, stage, reason))
        case .consentState(let state):
            consent = state
            emit.yield(.consent(state))
        case .notice(let code, _):
            emit.yield(.notice(code))
        case .flushed:
            emit.yield(.notice("flushed"))
        case .error(let code, _):
            emit.yield(.notice(code))
        }
    }

    /// Stops playback, raises the fence and cancels every provider call of older turns.
    private func stopLocalWork(fence: TurnID?) {
        guard let fence else { return }
        player.stop(fence: fence, epoch: epoch)
        for (call, turn) in callTurn where turn < fence {
            calls.removeValue(forKey: call)?.cancel()
            callTurn.removeValue(forKey: call)
        }
        for (turn, task) in synthesisTail where turn < fence {
            task.cancel()
            synthesisTail.removeValue(forKey: turn)
        }
        if let c = capturing, c < fence { stopMicrophone(c) }
    }

    // MARK: Providers

    /// The app's own capture limit: whatever the service does, capture ends here.
    private func capLocally(_ turn: TurnID) {
        let limit = config.localCaptureMax
        let epochAtStart = epoch
        Task { [weak self] in
            try? await Task.sleep(for: limit)
            await self?.localCapReached(turn, epochAtStart)
        }
    }

    private func localCapReached(_ turn: TurnID, _ e: UInt64) {
        guard e == epoch, capturing == turn else { return }
        release(turn)
    }

    private func beginRecognition(_ turn: TurnID) {
        pendingAudio[turn] = []
        let recognizer = config.recognizer
        recognition[turn] = Task { [weak self] in
            let session = try? await recognizer.begin()
            await self?.recognitionStarted(turn, session)
            return session
        }
    }

    private func recognitionStarted(_ turn: TurnID, _ session: (any RecognitionSession)?) {
        let buffered = pendingAudio.removeValue(forKey: turn) ?? []
        guard let session else { return }
        for pcm in buffered { session.append(pcm) }
        sessions[turn] = session
    }

    private func finishRecognition(call: CallID, turn: TurnID) {
        stopMicrophone(turn)
        let started = recognition[turn]
        let process = self.process
        track(call, turn, Task { [weak self] in
            guard let session = await started?.value else {
                try? process?.send(.sttFailed(call: call, reason: .assetsMissing))
                return
            }
            do {
                let heard = try await session.finish()
                let at = ContinuousClock.now
                await self?.mark(turn, .heard, at)
                try? process?.send(.sttFinal(call: call, text: heard.text, minConfidence: heard.minConfidence))
            } catch let e as RecognizerError {
                try? process?.send(.sttFailed(call: call, reason: e.reason))
            } catch {
                if Task.isCancelled { await session.cancel() } else { try? process?.send(.sttFailed(call: call, reason: .error)) }
            }
        })
    }

    /// A sendable recorder for callbacks running off the actor.
    private func marker(_ turn: TurnID) -> @Sendable (Mark, ContinuousClock.Instant) -> Void {
        { [weak self] mark, at in Task { await self?.mark(turn, mark, at) } }
    }

    private func runModel(call: CallID, turn: TurnID, heard: String) {
        let model = config.model
        let process = self.process
        let mark = marker(turn)
        track(call, turn, Task {
            do {
                try await model.respond(to: heard) { event in
                    guard !Task.isCancelled else { return }
                    switch event {
                    case .kind(let k): try? process?.send(.lmKind(call: call, kind: k))
                    case .sentence(let i, let text):
                        if i == 0 { mark(.firstSentence, ContinuousClock.now) }
                        try? process?.send(.lmSentence(call: call, index: i, text: text))
                    case .done(let p):
                        try? process?.send(.lmDone(call: call, proposal: p, model: model.modelID, instructions: model.instructionsVersion))
                    }
                }
            } catch LMError.failed(let reason) {
                if !Task.isCancelled { try? process?.send(.lmFailed(call: call, reason: reason)) }
            } catch {
                if !Task.isCancelled { try? process?.send(.lmFailed(call: call, reason: .error)) }
            }
        })
    }

    private func speak(call: CallID, turn: TurnID, seq: UInt32, text: String, voice: Voice) {
        let previous = synthesisTail[turn]
        let synthesizer = config.synthesizer
        let player = self.player
        let process = self.process
        let target = player.format
        let progress = ChunkProgress()
        let mark = marker(turn)
        let position = PlaybackPosition(epoch: epoch, turn: turn)
        let task = Task {
            // Chunks of one turn are synthesized in order; playback overlaps the next synthesis.
            await previous?.value
            guard !Task.isCancelled else { return }
            guard player.accepts(position) else {
                // Say so rather than go quiet: the service then fails the chunk at once (or
                // drops the reply as late) instead of waiting for its deadline.
                try? process?.send(.ttsFailed(call: call, reason: .cancelled))
                return
            }
            let converter = Mutex<FormatConverter?>(nil)
            do {
                try await synthesizer.synthesize(text, voice: voice) { pcm in
                    // No cancellation check here: this runs on the synthesizer's thread. The
                    // player's fence is what keeps a stale buffer from playing.
                    let ready: PCM
                    if pcm.buffer.format == target {
                        ready = pcm
                    } else {
                        let c = converter.withLock { c -> FormatConverter? in
                            if c == nil { c = FormatConverter(from: pcm.buffer.format, to: target) }
                            return c
                        }
                        guard let converted = try? c?.convert(pcm) else { return }
                        ready = converted
                    }
                    let result = player.enqueue(AudioChunk(epoch: position.epoch, turn: turn, seq: seq, pcm: ready)) {
                        if progress.played() { try? process?.send(.ttsFinished(call: call)) }
                    }
                    if result == .scheduled, progress.scheduled() {
                        mark(.firstAudio, ContinuousClock.now)
                        try? process?.send(.ttsStarted(call: call))
                    }
                }
                if progress.synthesisEnded() {
                    try? process?.send(.ttsFinished(call: call))
                } else if !progress.anyScheduled, !Task.isCancelled {
                    // Every buffer was refused by the fence: nothing will play.
                    try? process?.send(.ttsFailed(call: call, reason: .cancelled))
                }
            } catch {
                if !Task.isCancelled { try? process?.send(.ttsFailed(call: call, reason: .error)) }
            }
        }
        synthesisTail[turn] = task
        track(call, turn, task)
    }

    private func track(_ call: CallID, _ turn: TurnID, _ task: Task<Void, Never>) {
        calls[call] = task
        callTurn[call] = turn
    }

    enum Mark: Sendable { case heard, firstSentence, firstAudio }

    /// Records when something happened (taken where it happened, not after the actor hop).
    private func mark(_ turn: TurnID, _ mark: Mark, _ at: ContinuousClock.Instant) {
        var t = timing[turn, default: TurnTiming()]
        switch mark {
        case .heard: t.heardSent = t.heardSent ?? at
        case .firstSentence: t.firstSentence = t.firstSentence ?? at
        case .firstAudio: t.firstAudio = t.firstAudio ?? at
        }
        timing[turn] = t
    }
}
