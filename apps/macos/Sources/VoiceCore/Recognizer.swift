// Speech recognition behind a protocol (D-014): Apple's on-device SpeechAnalyzer with
// SpeechTranscriber, and a scripted recognizer for tests. Audio reaches a session as PCM
// buffers through a bounded stream, whether it came from the microphone or a fixture file.

@preconcurrency import AVFoundation
import Foundation
import Speech
import Synchronization

/// What the recognizer heard for one turn, verbatim, with its lowest segment confidence.
public struct HeardText: Equatable, Sendable {
    public let text: String
    public let minConfidence: Double?
    public init(text: String, minConfidence: Double?) {
        self.text = text
        self.minConfidence = minConfidence
    }
}

public enum RecognizerError: Error, Equatable, Sendable {
    case unavailable
    case assetsMissing
    case audioFormat
    /// More audio arrived than the bounded input queue holds.
    case overrun
    case cancelled

    public var reason: FailureReason {
        switch self {
        case .unavailable: .unavailable
        case .assetsMissing: .assetsMissing
        case .audioFormat: .audioFormat
        case .overrun: .tooLong
        case .cancelled: .cancelled
        }
    }
}

public protocol RecognitionSession: AnyObject, Sendable {
    /// Feeds one buffer. Never blocks; returns false (and the session will fail with
    /// `overrun`) when the bounded queue is full.
    @discardableResult func append(_ pcm: PCM) -> Bool
    /// Ends the input and returns the final transcript.
    func finish() async throws -> HeardText
    func cancel() async
}

public protocol SpeechRecognizing: Sendable {
    func begin() async throws -> any RecognitionSession
}

public enum RecognizerAvailability: Equatable, Sendable {
    case ready
    case assetsMissing
    case unsupported
}

public struct AppleSpeechRecognizer: SpeechRecognizing {
    public let locale: Locale
    /// Bound on queued input buffers (a capture tap delivers ~10–20 per second).
    public let maxQueuedBuffers: Int

    public init(locale: Locale = Locale(identifier: "en-US"), maxQueuedBuffers: Int = 1_024) {
        self.locale = locale
        self.maxQueuedBuffers = maxQueuedBuffers
    }

    private func transcriber() async throws -> SpeechTranscriber {
        guard SpeechTranscriber.isAvailable,
            let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale)
        else { throw RecognizerError.unavailable }
        return SpeechTranscriber(
            locale: supported, transcriptionOptions: [], reportingOptions: [], attributeOptions: [.transcriptionConfidence])
    }

    /// Whether this process can recognize now. `AssetInventory.status` reports `supported`
    /// until the process asks for an installation request; a nil request means the assets are
    /// installed and allocated (probed on macOS 27.2.1).
    public func availability() async -> RecognizerAvailability {
        guard let t = try? await transcriber() else { return .unsupported }
        return await Self.assetsReady(t) ? .ready : .assetsMissing
    }

    static func assetsReady(_ t: SpeechTranscriber) async -> Bool {
        if await AssetInventory.status(forModules: [t]) == .installed { return true }
        guard let request = try? await AssetInventory.assetInstallationRequest(supporting: [t]) else {
            return await AssetInventory.status(forModules: [t]) == .installed
        }
        _ = request
        return false
    }

    /// Downloads the on-device speech model for the locale (a user-initiated, one-time step).
    public func installAssets() async throws {
        let t = try await transcriber()
        if let request = try await AssetInventory.assetInstallationRequest(supporting: [t]) {
            try await request.downloadAndInstall()
        }
    }

    public func begin() async throws -> any RecognitionSession {
        let transcriber = try await transcriber()
        guard await Self.assetsReady(transcriber) else { throw RecognizerError.assetsMissing }
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) else {
            throw RecognizerError.audioFormat
        }
        // Keep the speech model loaded across turns instead of reloading it for each one.
        let analyzer = SpeechAnalyzer(modules: [transcriber], options: .init(priority: .userInitiated, modelRetention: .processLifetime))
        try await analyzer.prepareToAnalyze(in: format)
        let (stream, input) = AsyncStream.makeStream(of: AnalyzerInput.self, bufferingPolicy: .bufferingOldest(maxQueuedBuffers))
        let results = Task { () throws -> HeardText in
            var text = ""
            var minConfidence: Double?
            for try await result in transcriber.results {
                text += String(result.text.characters)
                for run in result.text.runs {
                    if let c = run.transcriptionConfidence { minConfidence = min(minConfidence ?? c, c) }
                }
            }
            return HeardText(text: text.trimmingCharacters(in: .whitespacesAndNewlines), minConfidence: minConfidence)
        }
        try await analyzer.start(inputSequence: stream)
        return AppleRecognitionSession(analyzer: analyzer, input: input, results: results, format: format)
    }
}

final class AppleRecognitionSession: RecognitionSession, @unchecked Sendable {
    private let analyzer: SpeechAnalyzer
    private let input: AsyncStream<AnalyzerInput>.Continuation
    private let results: Task<HeardText, Error>
    private let format: AVAudioFormat
    private struct State {
        var converter: FormatConverter?
        var overrun = false
        var finished = false
    }
    private let state = Mutex(State())

    init(analyzer: SpeechAnalyzer, input: AsyncStream<AnalyzerInput>.Continuation, results: Task<HeardText, Error>, format: AVAudioFormat) {
        self.analyzer = analyzer
        self.input = input
        self.results = results
        self.format = format
    }

    @discardableResult
    func append(_ pcm: PCM) -> Bool {
        state.withLock { s in
            guard !s.finished, !s.overrun else { return false }
            let buffer: PCM
            if pcm.buffer.format == format {
                buffer = pcm
            } else {
                if s.converter == nil { s.converter = FormatConverter(from: pcm.buffer.format, to: format) }
                guard let converted = try? s.converter?.convert(pcm) else {
                    s.overrun = true
                    return false
                }
                buffer = converted
            }
            if case .dropped = input.yield(AnalyzerInput(buffer: buffer.buffer)) {
                s.overrun = true
                return false
            }
            return true
        }
    }

    func finish() async throws -> HeardText {
        let overrun = state.withLock { s -> Bool in
            s.finished = true
            return s.overrun
        }
        input.finish()
        if overrun {
            await analyzer.cancelAndFinishNow()
            results.cancel()
            throw RecognizerError.overrun
        }
        try await analyzer.finalizeAndFinishThroughEndOfInput()
        return try await results.value
    }

    func cancel() async {
        state.withLock { $0.finished = true }
        input.finish()
        await analyzer.cancelAndFinishNow()
        results.cancel()
    }
}

/// A recognizer that "hears" a scripted text: for tests and CI, never for the app.
public struct ScriptedRecognizer: SpeechRecognizing {
    public let heard: HeardText
    public let delay: Duration
    public init(heard: HeardText, delay: Duration = .zero) {
        self.heard = heard
        self.delay = delay
    }

    public func begin() async throws -> any RecognitionSession { ScriptedSession(heard: heard, delay: delay) }

    final class ScriptedSession: RecognitionSession, @unchecked Sendable {
        let heard: HeardText
        let delay: Duration
        let buffers = Mutex(0)
        init(heard: HeardText, delay: Duration) {
            self.heard = heard
            self.delay = delay
        }
        func append(_ pcm: PCM) -> Bool {
            buffers.withLock { $0 += 1 }
            return true
        }
        func finish() async throws -> HeardText {
            try await Task.sleep(for: delay)
            return heard
        }
        func cancel() async {}
    }
}
