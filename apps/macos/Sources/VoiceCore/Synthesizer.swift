// Speech output behind a protocol (D-014): AVSpeechSynthesizer system voices rendered to
// buffers (never straight to the speaker, so playback goes through the turn-fenced player),
// and a deterministic tone synthesizer for tests.

@preconcurrency import AVFoundation
import Foundation
import Synchronization

public enum SynthesizerError: Error, Equatable, Sendable {
    case cancelled
    case noVoice
}

public protocol SpeechSynthesizing: Sendable {
    /// Synthesizes `text`, delivering buffers in order to `onBuffer`. Returns when the last
    /// buffer was delivered. Cancelling the calling task stops synthesis promptly.
    func synthesize(_ text: String, voice: Voice, onBuffer: @escaping @Sendable (PCM) -> Void) async throws
}

/// The system's standard voice for a language — the first release's only voice (D-004).
public struct SystemVoiceSynthesizer: SpeechSynthesizing {
    public let language: String
    public init(language: String = "en-US") { self.language = language }

    public func synthesize(_ text: String, voice: Voice, onBuffer: @escaping @Sendable (PCM) -> Void) async throws {
        guard let systemVoice = AVSpeechSynthesisVoice(language: language) else { throw SynthesizerError.noVoice }
        let synthesizer = AVSpeechSynthesizer()
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = systemVoice
        let box = SynthesisBox(synthesizer)
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (done: CheckedContinuation<Void, Error>) in
                box.arm(done)
                synthesizer.write(utterance) { buffer in
                    guard let pcm = buffer as? AVAudioPCMBuffer else { return }
                    if pcm.frameLength == 0 {
                        box.finish(nil)
                    } else if box.isLive, let copy = Audio.copy(pcm) {
                        onBuffer(copy)
                    }
                }
            }
        } onCancel: {
            box.finish(SynthesizerError.cancelled)
        }
    }
}

/// Resumes the synthesis continuation exactly once — on the end-of-speech buffer or on
/// cancellation, whichever comes first — and stops the synthesizer when cancelled.
final class SynthesisBox: @unchecked Sendable {
    private let synthesizer: AVSpeechSynthesizer
    private let state = Mutex<(CheckedContinuation<Void, Error>?, Bool)>((nil, true))
    init(_ synthesizer: AVSpeechSynthesizer) { self.synthesizer = synthesizer }

    var isLive: Bool { state.withLock { $0.1 } }

    func arm(_ c: CheckedContinuation<Void, Error>) {
        let alreadyDone = state.withLock { s -> Bool in
            if !s.1 { return true }
            s.0 = c
            return false
        }
        if alreadyDone { c.resume(throwing: SynthesizerError.cancelled) }
    }

    func finish(_ error: Error?) {
        let c = state.withLock { s -> CheckedContinuation<Void, Error>? in
            s.1 = false
            defer { s.0 = nil }
            return s.0
        }
        if error != nil { synthesizer.stopSpeaking(at: .immediate) }
        if let error { c?.resume(throwing: error) } else { c?.resume() }
    }
}

/// Deterministic test synthesis: a tone whose length follows the text, delivered in
/// 256-frame buffers after `firstBufferDelay`. `stall` never delivers anything.
public struct ToneSynthesizer: SpeechSynthesizing {
    public let format: AVAudioFormat
    public let secondsPerCharacter: Double
    public let firstBufferDelay: Duration
    public let stall: Bool

    public init(sampleRate: Double = 22_050, secondsPerCharacter: Double = 0.01, firstBufferDelay: Duration = .zero, stall: Bool = false) {
        format = Audio.monoFloat(sampleRate)
        self.secondsPerCharacter = secondsPerCharacter
        self.firstBufferDelay = firstBufferDelay
        self.stall = stall
    }

    public func synthesize(_ text: String, voice: Voice, onBuffer: @escaping @Sendable (PCM) -> Void) async throws {
        if stall {
            try await Task.sleep(for: .seconds(3_600))
        }
        try await Task.sleep(for: firstBufferDelay)
        let whole = Audio.tone(seconds: max(0.05, Double(text.count) * secondsPerCharacter), format: format)
        for slice in Audio.slices(whole, frames: 256) {
            try Task.checkCancellation()
            onBuffer(slice)
        }
    }
}
