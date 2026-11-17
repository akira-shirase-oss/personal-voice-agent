// PCM helpers shared by capture, recognition, synthesis, playback and tests.

@preconcurrency import AVFoundation
import Foundation

/// An immutable PCM buffer that may cross concurrency domains. Invariant: nobody writes to a
/// buffer after wrapping it, so sharing it read-only is safe.
public struct PCM: @unchecked Sendable {
    public let buffer: AVAudioPCMBuffer
    public init(_ buffer: AVAudioPCMBuffer) { self.buffer = buffer }
    public var frames: Int { Int(buffer.frameLength) }
    public var seconds: Double { Double(buffer.frameLength) / buffer.format.sampleRate }
}

/// Where a chunk belongs: the service session (an epoch the app counts, one per launch of
/// the service) and the turn within it. Ordered by epoch, then turn — a restarted service
/// numbers turns from 1 again, so the turn alone cannot order chunks across sessions.
public struct PlaybackPosition: Comparable, Hashable, Sendable, CustomStringConvertible {
    public let epoch: UInt64
    public let turn: TurnID
    public init(epoch: UInt64, turn: TurnID) {
        self.epoch = epoch
        self.turn = turn
    }
    public static func < (a: Self, b: Self) -> Bool { (a.epoch, a.turn.raw) < (b.epoch, b.turn.raw) }
    public var description: String { "\(epoch)/\(turn)" }
}

/// One playback chunk, always tagged with the session and turn it belongs to (D-009).
public struct AudioChunk: Sendable {
    public let epoch: UInt64
    public let turn: TurnID
    public let seq: UInt32
    public let pcm: PCM
    public init(epoch: UInt64 = 0, turn: TurnID, seq: UInt32, pcm: PCM) {
        self.epoch = epoch
        self.turn = turn
        self.seq = seq
        self.pcm = pcm
    }
    public var position: PlaybackPosition { PlaybackPosition(epoch: epoch, turn: turn) }
}

public enum Audio {
    /// Mono float32 at `sampleRate`, the format the player and synthesizer use.
    public static func monoFloat(_ sampleRate: Double) -> AVAudioFormat {
        AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: sampleRate, channels: 1, interleaved: false)!
    }

    /// A sine tone: a deterministic, obviously non-silent test signal.
    public static func tone(seconds: Double, hz: Double = 440, amplitude: Float = 0.5, format: AVAudioFormat) -> PCM {
        let frames = AVAudioFrameCount(seconds * format.sampleRate)
        let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames)!
        buffer.frameLength = frames
        let data = buffer.floatChannelData![0]
        for i in 0..<Int(frames) {
            data[i] = amplitude * Float(sin(2 * Double.pi * hz * Double(i) / format.sampleRate))
        }
        return PCM(buffer)
    }

    /// Peak absolute sample value of channel 0 over `range` (whole buffer by default).
    public static func peak(_ buffer: AVAudioPCMBuffer, from start: Int = 0, to end: Int? = nil) -> Float {
        guard let data = buffer.floatChannelData?[0] else { return 0 }
        let stop = min(end ?? Int(buffer.frameLength), Int(buffer.frameLength))
        var peak: Float = 0
        var i = start
        while i < stop {
            peak = max(peak, abs(data[i]))
            i += 1
        }
        return peak
    }

    /// Index of the last sample above `threshold`, or nil if every sample is below it.
    public static func lastNonSilent(_ buffer: AVAudioPCMBuffer, threshold: Float = 1e-4) -> Int? {
        guard let data = buffer.floatChannelData?[0] else { return nil }
        var i = Int(buffer.frameLength) - 1
        while i >= 0 {
            if abs(data[i]) > threshold { return i }
            i -= 1
        }
        return nil
    }

    /// Reads a whole audio file as mono float32 at its own sample rate.
    public static func read(_ url: URL) throws -> PCM {
        let file = try AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false)
        let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length))!
        try file.read(into: buffer)
        return PCM(buffer)
    }

    /// An independent copy of a buffer. Synthesizers and capture taps reuse theirs.
    public static func copy(_ src: AVAudioPCMBuffer) -> PCM? {
        guard let dst = AVAudioPCMBuffer(pcmFormat: src.format, frameCapacity: src.frameLength) else { return nil }
        dst.frameLength = src.frameLength
        let bytes = Int(src.frameLength) * Int(src.format.streamDescription.pointee.mBytesPerFrame)
        let from = UnsafeMutableAudioBufferListPointer(src.mutableAudioBufferList)
        let to = UnsafeMutableAudioBufferListPointer(dst.mutableAudioBufferList)
        for (s, d) in zip(from, to) {
            if let sp = s.mData, let dp = d.mData { dp.copyMemory(from: sp, byteCount: min(bytes, Int(s.mDataByteSize))) }
        }
        return PCM(dst)
    }

    /// Splits a buffer into consecutive slices of `frames` (as a capture tap would deliver).
    public static func slices(_ pcm: PCM, frames: Int) -> [PCM] {
        let src = pcm.buffer
        var out: [PCM] = []
        var start = 0
        while start < Int(src.frameLength) {
            let n = min(frames, Int(src.frameLength) - start)
            let slice = AVAudioPCMBuffer(pcmFormat: src.format, frameCapacity: AVAudioFrameCount(n))!
            slice.frameLength = AVAudioFrameCount(n)
            for ch in 0..<Int(src.format.channelCount) {
                slice.floatChannelData![ch].update(from: src.floatChannelData![ch].advanced(by: start), count: n)
            }
            out.append(PCM(slice))
            start += n
        }
        return out
    }

    /// Adds deterministic white noise at `snrDB` relative to the signal's RMS (a "noisy room"
    /// variant of a fixture, reproducible from `seed`).
    public static func withNoise(_ pcm: PCM, snrDB: Double, seed: UInt64) -> PCM {
        let src = pcm.buffer
        let n = Int(src.frameLength)
        let out = AVAudioPCMBuffer(pcmFormat: src.format, frameCapacity: src.frameLength)!
        out.frameLength = src.frameLength
        let s = src.floatChannelData![0]
        let o = out.floatChannelData![0]
        var energy = 0.0
        for i in 0..<n { energy += Double(s[i]) * Double(s[i]) }
        let rms = (energy / Double(max(n, 1))).squareRoot()
        let noiseRMS = rms / pow(10, snrDB / 20)
        var state = seed | 1
        for i in 0..<n {
            // xorshift64*, mapped to a uniform value in [-1, 1); uniform RMS is 1/sqrt(3).
            state ^= state >> 12
            state ^= state << 25
            state ^= state >> 27
            let r = Double((state &* 0x2545_F491_4F6C_DD1D) >> 11) / Double(1 << 53) * 2 - 1
            o[i] = s[i] + Float(r * noiseRMS * 3.0.squareRoot())
        }
        return PCM(out)
    }
}

/// Converts buffers to one target format (capture/synthesis rate → recognizer/player rate).
public final class FormatConverter: @unchecked Sendable {
    private let converter: AVAudioConverter
    public let output: AVAudioFormat
    private let lock = NSLock()

    public init?(from input: AVAudioFormat, to output: AVAudioFormat) {
        guard let c = AVAudioConverter(from: input, to: output) else { return nil }
        converter = c
        self.output = output
    }

    public func convert(_ pcm: PCM) throws -> PCM {
        lock.lock()
        defer { lock.unlock() }
        let ratio = output.sampleRate / pcm.buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(pcm.buffer.frameLength) * ratio + 1024)
        let out = AVAudioPCMBuffer(pcmFormat: output, frameCapacity: capacity)!
        var fed = false
        var error: NSError?
        let status = converter.convert(to: out, error: &error) { _, outStatus in
            if fed {
                outStatus.pointee = .noDataNow
                return nil
            }
            fed = true
            outStatus.pointee = .haveData
            return pcm.buffer
        }
        if status == .error { throw error ?? NSError(domain: "FormatConverter", code: 1) }
        return PCM(out)
    }
}
