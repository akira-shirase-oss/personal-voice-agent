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
    /// The service refused to start: `store_corrupt` or `store_io` (a restart cannot fix them),
    /// or `already_running` (another running copy owns the data directory; the app keeps
    /// retrying and takes over when it is free).
    case serviceFailed(String)
    /// The service stopped answering (during a turn, between turns or on a key press); it is
    /// being replaced.
    case serviceUnresponsive
    /// The app fell behind and this many older events were discarded; state shown may be stale.
    case eventsDropped(Int)
    /// A `forget` was carried out (D-018): how many session logs and bytes were deleted.
    case forgotten(files: UInt32, bytes: UInt64)
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
    /// While a turn is open (or the key is held) the app pings the service this often, and
    /// treats it as hung when a ping goes unanswered for `heartbeatTimeout`.
    public var heartbeatInterval: Duration = .seconds(1)
    public var heartbeatTimeout: Duration = .seconds(3)
    /// Between turns, once the session is ready, it pings this often; every key press also
    /// pings at once, so a service that hung while idle is found before the microphone opens.
    public var idleHeartbeatInterval: Duration = .seconds(5)
    /// How long a ping may wait behind a `forget`, which deletes files before it answers.
    public var forgetTimeout: Duration = .seconds(30)
    /// On quit, how long the service has to close its session after stdin closes before it
    /// gets SIGTERM (then SIGKILL): the app never leaves a child holding the data directory.
    public var shutdownGrace: Duration = .seconds(1)
    /// While another running copy holds the data directory, retry after 250 ms, doubling up to
    /// this cap, for as long as it takes.
    public var blockedRetryCap: Duration = .seconds(5)
    /// Bound on conversation events waiting for the app to read them.
    public var eventBuffer = 4_096

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
    /// Older events discarded because the app fell behind (reported, never silent).
    private var droppedEvents = 0
    /// The process whose exit has been handled (its stream may also end without `.exited`).
    private var exitHandled: ObjectIdentifier?
    /// One per launch of the service; tags every chunk so a restarted session (which numbers
    /// turns from 1 again) is never confused with the one that died.
    private var epoch: UInt64 = 0
    private var stopping = false
    /// Set when the service reported an error a plain restart cannot fix.
    private var fatal: String?
    /// Consecutive launches refused with `already_running`.
    private var blockedAttempts = 0
    /// The heartbeat ping awaiting its pong, if any.
    private var pendingPing: (seq: UInt64, sentAt: ContinuousClock.Instant)?
    private var pingSeq: UInt64 = 0
    private var lastPingAt: ContinuousClock.Instant?
    /// The current service has said `ready`: it is pinged from then on, turn or no turn.
    private var sessionUp = false
    /// A `forget` was sent and not yet answered.
    private var forgetPending = false
    static let fatalCodes: Set<String> = ["already_running", "store_corrupt", "store_io"]
    private static let maxPendingBuffers = 512

    public init(_ config: ConversationConfig) throws {
        self.config = config
        player = try TurnFencedPlayer(format: Audio.monoFloat(config.playerSampleRate), offline: config.offlinePlayback)
        (events, emit) = AsyncStream.makeStream(of: ConversationEvent.self, bufferingPolicy: .bufferingNewest(config.eventBuffer))
    }

    // MARK: Lifecycle

    public func start() throws {
        config.model.prewarm()
        let p = try SessionProcess(binary: config.binary, dataDir: config.dataDir, limits: config.limits)
        epoch += 1
        player.beginEpoch(epoch)
        process = p
        pendingPing = nil
        lastPingAt = nil
        sessionUp = false
        forgetPending = false
        startHeartbeat(epoch)
        try p.send(.hello(protocol: protocolVersion, client: "voice-agent-macos/0.1.0"))
        Task { [weak self] in
            for await event in p.events { await self?.handle(event, from: p) }
            await self?.streamEnded(p)
        }
    }

    /// Once the service is ready, ping it — every interval while a turn is open or the key is
    /// held, every idle interval between turns. An unanswered ping past the timeout means it is
    /// hung (a stopped process answers nothing, not even an exit).
    private func startHeartbeat(_ e: UInt64) {
        let interval = config.heartbeatInterval
        Task { [weak self] in
            while true {
                try? await Task.sleep(for: interval)
                guard let self, await self.heartbeat(e) else { return }
            }
        }
    }

    /// One heartbeat tick; false ends the loop for this session.
    private func heartbeat(_ e: UInt64) -> Bool {
        guard e == epoch, !stopping, let p = process, p.isRunning else { return false }
        if let pending = pendingPing {
            let limit = forgetPending ? config.forgetTimeout : config.heartbeatTimeout
            guard clock.now - pending.sentAt > limit else { return true }
            // Hung: stop everything here, then replace it (the exit starts a new session).
            publish(.serviceUnresponsive)
            if let c = capturing { stopMicrophone(c) }
            player.stop(fence: TurnID(UInt64.max), epoch: epoch)
            p.kill()
            return false
        }
        guard sessionUp else { return true }
        let active = currentTurn != nil || capturing != nil || keyHeld
        let idleDue = lastPingAt.map { clock.now - $0 >= config.idleHeartbeatInterval } ?? true
        if active || idleDue { ping() }
        return true
    }

    private func ping() {
        pingSeq += 1
        pendingPing = (pingSeq, clock.now)
        lastPingAt = clock.now
        send(.ping(seq: pingSeq))
    }

    /// Ends the session and does not return until the service has exited: `shutdown`, then
    /// stdin closed, SIGTERM after `shutdownGrace`, SIGKILL last. A child left running (or hung)
    /// would hold the data directory's lock, and the next launch would be refused.
    public func shutdown() async {
        stopping = true
        for task in calls.values { task.cancel() }
        player.shutdown()
        guard let p = process else { return }
        try? p.send(.shutdown)
        await p.stop(grace: config.shutdownGrace)
    }

    /// Crash drill: kills the child with SIGKILL; the conversation restarts it.
    public func killServiceForDrill() { process?.kill() }
    public var servicePID: Int32? { process?.pid }

    /// Every event goes through here. If the bounded buffer must discard an older event, the
    /// app is told at once, as the newest event, how many have been lost so far.
    private func publish(_ e: ConversationEvent) {
        if case .dropped = emit.yield(e) {
            droppedEvents += 1
            if case .dropped = emit.yield(.eventsDropped(droppedEvents + 1)) { droppedEvents += 1 }
        }
    }

    /// The process's event stream ended: if its exit was not seen (the exit event itself can be
    /// lost when the buffer overflowed), handle it now.
    private func streamEnded(_ p: SessionProcess) {
        guard exitHandled != ObjectIdentifier(p) else { return }
        if p.overflowed { publish(.notice("service_messages_overflowed")) }
        handleExit(-1, of: p)
    }

    private func handleExit(_ status: Int32, of p: SessionProcess) {
        guard p === process, exitHandled != ObjectIdentifier(p) else { return }
        exitHandled = ObjectIdentifier(p)
        let blocked = fatal == "already_running"
        if !blocked || blockedAttempts == 0 { publish(.serviceExited(status)) }
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
        sessionUp = false
        guard !stopping else { return }
        if blocked {
            fatal = nil
            return waitForDataDirectory()
        }
        if let fatal {
            publish(.serviceFailed(fatal))
            return
        }
        guard restarts < config.maxRestarts else {
            publish(.serviceGaveUp)
            return
        }
        restarts += 1
        let attempt = restarts
        publish(.serviceRestarting(attempt: attempt))
        Task { [weak self] in
            // Bounded backoff: 100 ms, 200 ms, 400 ms, ...
            try? await Task.sleep(for: .milliseconds(100 << (attempt - 1)))
            try? await self?.start()
        }
    }

    /// `already_running`: another service holds the data directory. A stale orphan of this
    /// app (its parent died; it still holds the lock, perhaps hung) is stopped and replaced at
    /// once. Any other holder is another running copy: wait for it, with capped backoff, and
    /// never give up — the refusal is reported, not made permanent.
    private func waitForDataDirectory() {
        blockedAttempts += 1
        let attempt = blockedAttempts
        if let pid = ServiceLock.holder(dataDir: config.dataDir), ServiceLock.isStaleOrphan(pid, binary: config.binary) {
            publish(.notice("stale_service_stopped"))
            Task { [weak self] in
                _ = await ServiceLock.terminate(pid)
                try? await self?.relaunch()
            }
            return
        }
        if attempt == 3 { publish(.serviceFailed("already_running")) }
        let delay = min(Duration.milliseconds(250 << min(attempt - 1, 8)), config.blockedRetryCap)
        Task { [weak self] in
            try? await Task.sleep(for: delay)
            try? await self?.relaunch()
        }
    }

    private func relaunch() throws {
        guard !stopping else { return }
        try start()
    }

    // MARK: User actions

    public func setCaptureConsent(_ grant: Bool) {
        if !grant {
            stopLocalWork(fence: currentTurn.map { TurnID($0.raw + 1) })
            // Speech heard under the revoked consent must not reach a later prompt.
            config.model.forget()
        }
        send(.consent(scope: .capture, grant: grant, rightsReview: nil))
    }

    public func pushToTalkDown() {
        keyHeld = true
        // Barge-in: stop the assistant here before the service even hears about it (D-023).
        if let t = currentTurn { player.stop(fence: TurnID(t.raw + 1), epoch: epoch) }
        // Every press checks the service is alive; the microphone opens only on its answer.
        if sessionUp && pendingPing == nil { ping() }
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
        publish(.capture(turn, on: false))
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

    /// Deletes every heard transcript (capture consent's deletion scope). Refused mid-turn.
    public func forgetHistory() {
        forgetPending = true
        // The model's recap holds heard text too; it goes now, whatever the service answers.
        config.model.forget()
        send(.forget(scope: .capture))
    }

    private func send(_ m: ClientMessage) {
        do { try process?.send(m) } catch { publish(.notice("service_not_running")) }
    }

    // MARK: Service messages

    private func handle(_ event: SessionEvent, from p: SessionProcess) {
        guard p === process else { return }
        switch event {
        case .exited(let status): handleExit(status, of: p)
        case .undecodable: publish(.notice("undecodable_service_line"))
        case .message(let m): handle(m)
        }
    }

    private func handle(_ m: ServiceMessage) {
        switch m {
        case .ready(_, let session, let consent, let recovered):
            self.consent = consent
            sessionUp = true
            blockedAttempts = 0
            publish(.ready(session: session, consent: consent, recovered: recovered))
        case .turnOpened(let turn):
            currentTurn = turn
            publish(.turnOpened(turn))
        case .capture(let turn, on: true):
            beginRecognition(turn)
            if keyHeld {
                capturing = turn
                publish(.capture(turn, on: true))
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
            publish(.heard(turn, heard))
            runModel(call: call, turn: turn, heard: heard)
        case .ttsRequest(let call, let turn, let seq, let text, let voice, _):
            if timing[turn]?.firstTTSRequest == nil { timing[turn, default: TurnTiming()].firstTTSRequest = clock.now }
            publish(.speaking(turn, text))
            speak(call: call, turn: turn, seq: seq, text: text, voice: voice)
        case .cancel(let call):
            calls.removeValue(forKey: call)?.cancel()
            callTurn.removeValue(forKey: call)
        case .stopPlayback(let fence):
            stopLocalWork(fence: fence)
        case .confirmRequest(let turn, let digest, let heard, let tool, let slots, let reasons):
            publish(.confirm(turn: turn, digest: digest, heard: heard, tool: tool, slots: slots, reasons: reasons))
        case .turnClosed(let turn, let outcome):
            let t = timing.removeValue(forKey: turn) ?? TurnTiming()
            if currentTurn == turn { currentTurn = nil }
            recognition.removeValue(forKey: turn)
            // A session never handed to a finish (an interrupt or revocation during capture)
            // would otherwise wait forever with its model loaded: release it.
            if let unfinished = sessions.removeValue(forKey: turn) { Task { await unfinished.cancel() } }
            pendingAudio.removeValue(forKey: turn)
            synthesisTail.removeValue(forKey: turn)
            restarts = 0
            publish(.turnClosed(turn, outcome, t))
        case .degraded(let turn, let stage, let reason):
            publish(.degraded(turn, stage, reason))
        case .consentState(let state):
            consent = state
            publish(.consent(state))
        case .notice(let code, _):
            if code == "forget_refused_turn_active" { forgetPending = false }
            publish(.notice(code))
        case .flushed:
            publish(.notice("flushed"))
        case .pong(let seq):
            if pendingPing?.seq == seq { pendingPing = nil }
        case .forgotten(_, let files, let bytes):
            forgetPending = false
            config.model.forget()
            publish(.forgotten(files: files, bytes: bytes))
        case .error(let code, _):
            if Self.fatalCodes.contains(code) { fatal = code }
            if code == "forget_failed" { forgetPending = false }
            publish(.notice(code))
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
        guard recognition[turn] != nil else {
            // The turn closed while the recognizer was starting.
            Task { await session.cancel() }
            return
        }
        for pcm in buffered { session.append(pcm) }
        sessions[turn] = session
    }

    private func finishRecognition(call: CallID, turn: TurnID) {
        stopMicrophone(turn)
        // From here the finishing task owns the session (and cancels it if it is cancelled).
        sessions.removeValue(forKey: turn)
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
