// The turn-fenced player (D-009, D-021). Every chunk carries its turn; `stop(fence:)`
// stops playback and raises a fence below which no chunk is ever scheduled again.
//
// Why a late chunk cannot play: the fence check and `scheduleBuffer` happen under one lock,
// and `stop` raises the fence and stops the node under the same lock. A chunk either is
// scheduled before the stop (and the node's stop discards it) or is checked after it (and
// is refused). There is no window between check and schedule for a stop to fall into.

@preconcurrency import AVFoundation
import Foundation
import Synchronization

public enum EnqueueResult: Equatable, Sendable {
    case scheduled
    /// The chunk's turn is below the fence: it belongs to an interrupted turn.
    case droppedStale
    /// The bounded queue is full: more audio is queued than `maxQueuedSeconds`.
    case droppedFull
}

public struct PlayerStats: Equatable, Sendable {
    public var scheduled = 0
    public var droppedStale = 0
    public var droppedFull = 0
    public var stops = 0
}

public final class TurnFencedPlayer: @unchecked Sendable {
    public let format: AVAudioFormat
    public let engine = AVAudioEngine()
    private let node = AVAudioPlayerNode()
    private let maxQueuedFrames: AVAudioFramePosition
    private let offline: Bool

    private struct State {
        var fence = TurnID(0)
        var queuedFrames: AVAudioFramePosition = 0
        var generation: UInt64 = 0
        var stats = PlayerStats()
    }
    private let state = Mutex(State())
    /// Completion handlers hop here: `node.stop()` may call them while `state` is held, and
    /// the lock is not reentrant.
    private let callbacks = DispatchQueue(label: "voice.player.callbacks")

    /// - Parameters:
    ///   - offline: render with manual offline rendering (tests) instead of the output device.
    ///   - maxQueuedSeconds: bound on scheduled-but-unplayed audio.
    public init(format: AVAudioFormat, offline: Bool, maxQueuedSeconds: Double = 120) throws {
        self.format = format
        self.offline = offline
        maxQueuedFrames = AVAudioFramePosition(maxQueuedSeconds * format.sampleRate)
        if offline {
            try engine.enableManualRenderingMode(.offline, format: format, maximumFrameCount: 4096)
        }
        engine.attach(node)
        engine.connect(node, to: engine.mainMixerNode, format: format)
        // Offline, prepare() makes the first render quantum silent (probed 2026-11-16), which
        // would hide audio from the deterministic tests; the live engine prepares as usual.
        if !offline { engine.prepare() }
        try engine.start()
        node.play()
        if !offline { installStopMeter() }
    }

    /// Live only: host time (seconds since boot, the clock `NSEvent.timestamp` uses) at which
    /// the mixer last rendered a non-silent sample.
    private let lastLoud = Mutex<TimeInterval?>(nil)

    /// D-021 on the live device: from an interrupt event's timestamp to the last non-silent
    /// sample the mixer rendered, plus the output device's reported presentation latency.
    public struct StopMeasurement: Sendable, Equatable {
        public let renderedMs: Double
        public let deviceLatencyMs: Double
        public var totalMs: Double { renderedMs + deviceLatencyMs }
    }

    /// Call ~0.5 s after an interrupt whose event happened at `eventUptime`.
    public func measureStop(since eventUptime: TimeInterval) -> StopMeasurement? {
        guard !offline, let loud = lastLoud.withLock({ $0 }) else { return nil }
        return StopMeasurement(renderedMs: max(0, loud - eventUptime) * 1000, deviceLatencyMs: engine.outputNode.presentationLatency * 1000)
    }

    private func installStopMeter() {
        let mixer = engine.mainMixerNode
        let format = mixer.outputFormat(forBus: 0)
        mixer.installTap(onBus: 0, bufferSize: 256, format: format) { [weak self] buffer, time in
            guard let self, let data = buffer.floatChannelData?[0] else { return }
            var last = -1
            for i in stride(from: Int(buffer.frameLength) - 1, through: 0, by: -1) where abs(data[i]) > 1e-3 {
                last = i
                break
            }
            guard last >= 0, time.isHostTimeValid else { return }
            let at = AVAudioTime.seconds(forHostTime: time.hostTime) + Double(last) / format.sampleRate
            self.lastLoud.withLock { $0 = at }
        }
    }

    public var fence: TurnID { state.withLock { $0.fence } }
    public var stats: PlayerStats { state.withLock { $0.stats } }

    /// Schedules a chunk unless its turn is below the fence or the queue is full.
    /// `played` runs once the chunk has finished playing (not when it was discarded by a stop).
    @discardableResult
    public func enqueue(_ chunk: AudioChunk, played: (@Sendable () -> Void)? = nil) -> EnqueueResult {
        state.withLock { s in
            if chunk.turn < s.fence {
                s.stats.droppedStale += 1
                return .droppedStale
            }
            let frames = AVAudioFramePosition(chunk.pcm.buffer.frameLength)
            if s.queuedFrames + frames > maxQueuedFrames {
                s.stats.droppedFull += 1
                return .droppedFull
            }
            s.queuedFrames += frames
            s.stats.scheduled += 1
            let generation = s.generation
            let callbacks = self.callbacks
            // Live: "played back" by the device. Offline there is no device, and only
            // "rendered" completions fire.
            let type: AVAudioPlayerNodeCompletionCallbackType = offline ? .dataRendered : .dataPlayedBack
            node.scheduleBuffer(chunk.pcm.buffer, completionCallbackType: type) { [weak self] _ in
                guard let player = self else { return }
                callbacks.async { player.completed(generation: generation, frames: frames, played: played) }
            }
            return .scheduled
        }
    }

    /// A chunk discarded by a stop belongs to an older generation: it was not "played".
    private func completed(generation: UInt64, frames: AVAudioFramePosition, played: (@Sendable () -> Void)?) {
        let stillCurrent = state.withLock { s -> Bool in
            guard s.generation == generation else { return false }
            s.queuedFrames = max(0, s.queuedFrames - frames)
            return true
        }
        if stillCurrent { played?() }
    }

    /// Stops playback now and refuses every chunk of a turn below `fence` from here on.
    /// Returns how long the stop took on this thread.
    @discardableResult
    public func stop(fence: TurnID) -> Duration {
        let clock = ContinuousClock()
        let started = clock.now
        state.withLock { s in
            s.fence = max(s.fence, fence)
            s.generation += 1
            s.queuedFrames = 0
            s.stats.stops += 1
            node.stop()  // discards every scheduled buffer
            node.play()  // ready for the next turn's chunks
        }
        return clock.now - started
    }

    /// Offline only: renders the next `frames` frames of output.
    public func render(frames: AVAudioFrameCount) throws -> AVAudioPCMBuffer {
        precondition(offline, "render(frames:) is for offline rendering")
        let out = AVAudioPCMBuffer(pcmFormat: engine.manualRenderingFormat, frameCapacity: frames)!
        var rendered: AVAudioFrameCount = 0
        let block = AVAudioPCMBuffer(pcmFormat: engine.manualRenderingFormat, frameCapacity: 4096)!
        while rendered < frames {
            let n = min(4096, frames - rendered)
            let status = try engine.renderOffline(n, to: block)
            guard status == .success else { throw PlayerError.render(status.rawValue) }
            for ch in 0..<Int(out.format.channelCount) {
                out.floatChannelData![ch].advanced(by: Int(rendered)).update(from: block.floatChannelData![ch], count: Int(block.frameLength))
            }
            rendered += block.frameLength
        }
        out.frameLength = rendered
        return out
    }

    public func shutdown() {
        node.stop()
        engine.stop()
    }
}

public enum PlayerError: Error, Equatable {
    case render(Int)
}
