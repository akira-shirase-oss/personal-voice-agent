@preconcurrency import AVFoundation
import Foundation
import Synchronization
import Testing
@testable import VoiceCore

let playerFormat = Audio.monoFloat(22_050)

func frames(_ seconds: Double) -> AVAudioFrameCount { AVAudioFrameCount(seconds * playerFormat.sampleRate) }

extension TurnFencedPlayer {
    func render(_ n: AVAudioFrameCount) throws -> AVAudioPCMBuffer { try render(frames: n) }
}

/// A buffer scheduled on a running player node reaches its render thread asynchronously, so
/// sound may begin a few render quanta after scheduling (probed: it varies run to run).
/// Renders 512-frame blocks until a block has sound; returns the frames rendered first, or nil
/// if `limit` passes in silence. Tests never assume the first rendered frame is loud.
func renderUntilSound(_ render: (AVAudioFrameCount) throws -> AVAudioPCMBuffer, limit: Double = 0.5) throws -> Int? {
    var waited = 0
    while Double(waited) < limit * playerFormat.sampleRate {
        if Audio.peak(try render(512)) > 0.1 { return waited }
        waited += 512
    }
    return nil
}

func chunk(_ turn: UInt64, seq: UInt32 = 0, seconds: Double) -> AudioChunk {
    AudioChunk(turn: TurnID(turn), seq: seq, pcm: Audio.tone(seconds: seconds, format: playerFormat))
}

/// The bug the contract asks to reproduce first: a player that stops on interrupt but has no
/// notion of turns. A synthesis callback that lands after the interrupt is scheduled and plays.
final class NaivePlayer: @unchecked Sendable {
    let engine = AVAudioEngine()
    let node = AVAudioPlayerNode()
    init() throws {
        try engine.enableManualRenderingMode(.offline, format: playerFormat, maximumFrameCount: 4096)
        engine.attach(node)
        engine.connect(node, to: engine.mainMixerNode, format: playerFormat)
        try engine.start()
        node.play()
    }
    func enqueue(_ chunk: AudioChunk) { node.scheduleBuffer(chunk.pcm.buffer) }
    func interrupt() {
        node.stop()
        node.play()
    }
    func render(_ n: AVAudioFrameCount) throws -> AVAudioPCMBuffer {
        let out = AVAudioPCMBuffer(pcmFormat: engine.manualRenderingFormat, frameCapacity: n)!
        let block = AVAudioPCMBuffer(pcmFormat: engine.manualRenderingFormat, frameCapacity: 4096)!
        var done: AVAudioFrameCount = 0
        while done < n {
            _ = try engine.renderOffline(min(4096, n - done), to: block)
            out.floatChannelData![0].advanced(by: Int(done)).update(from: block.floatChannelData![0], count: Int(block.frameLength))
            done += block.frameLength
        }
        out.frameLength = done
        return out
    }
}

@Suite("Turn-fenced playback (deterministic, offline rendering)", .serialized)
struct PlayerTests {
    @Test("reproduce: without turn fencing, a late chunk from an interrupted turn plays")
    func staleChunkReproduced() throws {
        let p = try NaivePlayer()
        p.enqueue(chunk(1, seconds: 1))
        #expect(try renderUntilSound { try p.render($0) } != nil, "turn 1 is playing")
        p.interrupt()
        p.enqueue(chunk(1, seq: 1, seconds: 0.5))  // synthesis of turn 1 finishes after the interrupt
        let after = try p.render(frames(1))
        #expect(Audio.peak(after) > 0.1, "the bug: stale turn-1 audio plays after the interrupt")
    }

    @Test("prove: with the fence, the same late chunk is refused and nothing plays")
    func staleChunkCannotPlay() throws {
        let p = try TurnFencedPlayer(format: playerFormat, offline: true)
        #expect(p.enqueue(chunk(1, seconds: 1)) == .scheduled)
        #expect(try renderUntilSound { try p.render($0) } != nil, "turn 1 is playing")
        p.stop(fence: TurnID(2))
        #expect(p.enqueue(chunk(1, seq: 1, seconds: 0.5)) == .droppedStale)
        // A window twice the stale chunk's length: had it been scheduled, it would be heard.
        let after = try p.render(frames(1))
        #expect(Audio.peak(after) < 1e-4, "silence after the interrupt: no stale audio")
        // The next turn plays normally.
        #expect(p.enqueue(chunk(2, seconds: 0.5)) == .scheduled)
        #expect(try renderUntilSound { try p.render($0) } != nil, "turn 2 plays")
        #expect(p.stats.droppedStale == 1)
    }

    @Test("the fence only rises: an older stop cannot reopen a newer fence")
    func fenceIsMonotonic() throws {
        let p = try TurnFencedPlayer(format: playerFormat, offline: true)
        p.stop(fence: TurnID(5))
        p.stop(fence: TurnID(3))
        #expect(p.fence == TurnID(5))
        #expect(p.enqueue(chunk(4, seconds: 0.1)) == .droppedStale)
    }

    @Test("stop takes effect at the next rendered sample (D-021, offline)")
    func stopLatencyOffline() throws {
        let p = try TurnFencedPlayer(format: playerFormat, offline: true)
        p.enqueue(chunk(1, seconds: 2))
        #expect(try renderUntilSound { try p.render($0) } != nil, "playing before the stop")
        p.stop(fence: TurnID(2))
        let after = try p.render(frames(0.5))
        let tail = Audio.lastNonSilent(after)
        #expect(tail == nil, "non-silent samples after stop: \(tail.map { $0 + 1 } ?? 0)")
        print("METRIC stop_offline_frames_after_stop=\(tail.map { $0 + 1 } ?? 0) sample_rate=\(Int(playerFormat.sampleRate))")
    }

    @Test("stop() itself is fast: wall time over 1,000 stops")
    func stopCallDuration() throws {
        let p = try TurnFencedPlayer(format: playerFormat, offline: true)
        var micros: [Double] = []
        for turn in 1...1_000 {
            p.enqueue(chunk(UInt64(turn), seconds: 0.05))
            let d = p.stop(fence: TurnID(UInt64(turn + 1)))
            micros.append(Double(d.components.attoseconds) / 1e12 + Double(d.components.seconds) * 1e6)
        }
        micros.sort()
        let p50 = micros[499], p99 = micros[989], max = micros[999]
        print("METRIC stop_call_us p50=\(p50) p99=\(p99) max=\(max) n=1000")
        // Budget: the stop call may use at most a quarter of the 200 ms stop target (D-021).
        #expect(p99 < 50_000, "stop() p99 \(p99) µs")
    }

    @Test("concurrent late chunks never survive a stop (200 rounds, 8 threads)")
    func concurrentStress() throws {
        let p = try TurnFencedPlayer(format: playerFormat, offline: true)
        let small = Audio.tone(seconds: 0.01, format: playerFormat)
        let violations = Mutex(0)
        for round in 1...200 {
            let turn = TurnID(UInt64(round))
            let stopped = Atomic<Bool>(false)
            DispatchQueue.concurrentPerform(iterations: 8) { worker in
                if worker == 0 {
                    p.stop(fence: TurnID(turn.raw + 1))
                    stopped.store(true, ordering: .sequentiallyConsistent)
                    return
                }
                for seq in 0..<25 {
                    let afterStop = stopped.load(ordering: .sequentiallyConsistent)
                    let result = p.enqueue(AudioChunk(turn: turn, seq: UInt32(seq), pcm: small))
                    if afterStop && result != .droppedStale { violations.withLock { $0 += 1 } }
                }
            }
            // Everything scheduled before the stop was discarded by it; everything after was
            // refused. 0.1 s covers all 175 refused-or-discarded 10 ms chunks' scheduling lag.
            #expect(Audio.peak(try p.render(frames(0.1))) < 1e-4, "round \(round): stale audio survived")
        }
        #expect(violations.withLock { $0 } == 0)
    }

    @Test("the queue is bounded")
    func bounded() throws {
        let p = try TurnFencedPlayer(format: playerFormat, offline: true, maxQueuedSeconds: 1)
        #expect(p.enqueue(chunk(1, seconds: 0.6)) == .scheduled)
        #expect(p.enqueue(chunk(1, seq: 1, seconds: 0.6)) == .droppedFull)
        p.stop(fence: TurnID(1))
        #expect(p.enqueue(chunk(1, seq: 2, seconds: 0.6)) == .scheduled, "a stop empties the queue's budget")
    }

    @Test("played is reported for chunks that played, never for chunks a stop discarded")
    func playedCallbacks() async throws {
        let p = try TurnFencedPlayer(format: playerFormat, offline: true)
        let played = Mutex<[UInt32]>([])
        p.enqueue(chunk(1, seq: 0, seconds: 0.1)) { played.withLock { $0.append(0) } }
        _ = try p.render(frames(0.3))
        try await Task.sleep(for: .milliseconds(100))
        p.enqueue(chunk(2, seq: 1, seconds: 1)) { played.withLock { $0.append(1) } }
        _ = try p.render(frames(0.1))
        p.stop(fence: TurnID(3))
        _ = try p.render(frames(0.1))
        try await Task.sleep(for: .milliseconds(100))
        #expect(played.withLock { $0 } == [0])
    }
}
