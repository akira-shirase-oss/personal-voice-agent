@preconcurrency import AVFoundation
import Foundation
import Synchronization
import Testing
@testable import VoiceCore

enum OnDevice {
    static func speechReady() async -> Bool { await AppleSpeechRecognizer().availability() == .ready }
    static var systemVoice: Bool { AVSpeechSynthesisVoice(language: "en-US") != nil }
}

struct Utterance {
    let name: String
    let expected: String
}

func utterances() throws -> [Utterance] {
    try String(contentsOf: repoRoot.appending(path: "fixtures/audio/utterances.tsv"), encoding: .utf8)
        .split(separator: "\n").filter { !$0.hasPrefix("#") }
        .map { line in
            let parts = line.split(separator: "\t", maxSplits: 1).map(String.init)
            return Utterance(name: parts[0], expected: parts[1])
        }
}

/// Feeds a whole buffer through a recognition session the way a capture tap would.
func recognize(_ pcm: PCM, with recognizer: some SpeechRecognizing) async throws -> HeardText {
    let session = try await recognizer.begin()
    for slice in Audio.slices(pcm, frames: 1_600) { session.append(slice) }
    return try await session.finish()
}

@Suite("Transcripts")
struct TranscriptTests {
    @Test("spoken and written forms meet in one token form")
    func normalization() {
        #expect(Transcripts.tokens("Send $42 to Priya tomorrow.") == ["send", "42", "dollars", "to", "priya", "tomorrow"])
        #expect(Transcripts.tokens("Send forty two dollars to Priya tomorrow.") == ["send", "42", "dollars", "to", "priya", "tomorrow"])
        #expect(Transcripts.tokens("at 9:30") == ["at", "9", "30"])
        #expect(Transcripts.tokens("at nine thirty") == ["at", "9", "30"])
        #expect(Transcripts.tokens("one and two") == ["1", "and", "2"])
    }

    @Test("word error rate")
    func wer() {
        #expect(Transcripts.wordErrorRate(reference: "a b c d", hypothesis: "a b c d") == 0)
        #expect(Transcripts.wordErrorRate(reference: "a b c d", hypothesis: "a x c") == 0.5)
        #expect(Transcripts.wordErrorRate(reference: "a b", hypothesis: "") == 1)
    }
}

@Suite("Synthesis")
struct SynthesisTests {
    @Test("the tone synthesizer is deterministic")
    func tone() async throws {
        let count = Mutex(0)
        let frames = Mutex(0)
        try await ToneSynthesizer().synthesize(String(repeating: "x", count: 100), voice: .standard) { pcm in
            count.withLock { $0 += 1 }
            frames.withLock { $0 += pcm.frames }
        }
        #expect(frames.withLock { $0 } == 22_050)
        #expect(count.withLock { $0 } == 87)
    }

    @Test("the system voice renders to buffers, measured", .enabled("an en-US system voice is installed") { OnDevice.systemVoice })
    func systemVoice() async throws {
        let clock = ContinuousClock()
        let start = clock.now
        let first = Mutex<Duration?>(nil)
        let seconds = Mutex(0.0)
        try await SystemVoiceSynthesizer().synthesize("Here is a draft reply. Nothing has been sent.", voice: .standard) { pcm in
            first.withLock { if $0 == nil { $0 = clock.now - start } }
            seconds.withLock { $0 += pcm.seconds }
        }
        let firstBuffer = try #require(first.withLock { $0 })
        print("METRIC tts_first_buffer_ms=\(firstBuffer.components.seconds * 1000 + firstBuffer.components.attoseconds / 1_000_000_000_000_000) audio_s=\(seconds.withLock { $0 })")
        #expect(seconds.withLock { $0 } > 1 && seconds.withLock { $0 } < 8)
    }

    @Test("cancelling synthesis stops it promptly and delivers nothing afterwards", .enabled("an en-US system voice is installed") { OnDevice.systemVoice })
    func cancelSynthesis() async throws {
        let delivered = Mutex(0)
        let cancelledAt = Mutex<Int?>(nil)
        let long = String(repeating: "This is a long sentence that keeps going. ", count: 30)
        let task = Task {
            try await SystemVoiceSynthesizer().synthesize(long, voice: .standard) { _ in delivered.withLock { $0 += 1 } }
        }
        while delivered.withLock({ $0 }) == 0 { try await Task.sleep(for: .milliseconds(5)) }
        let clock = ContinuousClock()
        let t0 = clock.now
        task.cancel()
        let result = await task.result
        let took = clock.now - t0
        cancelledAt.withLock { $0 = delivered.withLock { $0 } }
        try await Task.sleep(for: .milliseconds(300))
        #expect(throws: SynthesizerError.cancelled) { try result.get() }
        #expect(took < .milliseconds(500), "cancel took \(took)")
        #expect(delivered.withLock { $0 } == cancelledAt.withLock { $0 }, "no buffer after the cancelled call returned")
    }
}

@Suite("Recognition on committed fixtures")
struct RecognitionTests {
    @Test("quiet and noisy fixtures: word error rate recorded", .enabled("on-device speech assets are installed") { await OnDevice.speechReady() })
    func fixtures() async throws {
        let recognizer = AppleSpeechRecognizer()
        var quietErrors = 0.0, noisyErrors = 0.0, words = 0.0
        var rows: [[String: Any]] = []
        for (i, u) in try utterances().enumerated() {
            let pcm = try Audio.read(repoRoot.appending(path: "fixtures/audio/\(u.name).wav"))
            let quiet = try await recognize(pcm, with: recognizer)
            let noisy = try await recognize(Audio.withNoise(pcm, snrDB: 10, seed: UInt64(i + 1)), with: recognizer)
            let n = Double(Transcripts.tokens(u.expected).count)
            let q = Transcripts.wordErrorRate(reference: u.expected, hypothesis: quiet.text)
            let z = Transcripts.wordErrorRate(reference: u.expected, hypothesis: noisy.text)
            quietErrors += q * n
            noisyErrors += z * n
            words += n
            rows.append([
                "name": u.name, "expected": u.expected, "quiet": quiet.text, "quiet_wer": q,
                "quiet_min_confidence": quiet.minConfidence ?? -1, "noisy_10db": noisy.text, "noisy_wer": z,
                "noisy_min_confidence": noisy.minConfidence ?? -1,
            ])
            print("EVAL \(u.name) quiet_wer=\(q) noisy_wer=\(z) conf=\(quiet.minConfidence ?? -1)/\(noisy.minConfidence ?? -1) heard=\"\(quiet.text)\"")
        }
        let quietWER = quietErrors / words, noisyWER = noisyErrors / words
        print("EVAL total quiet_wer=\(quietWER) noisy_10db_wer=\(noisyWER) words=\(Int(words))")
        if let out = ProcessInfo.processInfo.environment["PVA_EVAL_OUT"] {
            let report: [String: Any] = ["quiet_wer": quietWER, "noisy_10db_wer": noisyWER, "words": words, "utterances": rows]
            try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]).write(to: URL(fileURLWithPath: out))
        }
        // Regression guards for synthetic, studio-clean speech; real voices are owed (ROADMAP).
        #expect(quietWER <= 0.10)
        #expect(noisyWER <= 0.35)
    }

    @Test("synthesized speech round-trips through recognition", .enabled("speech assets and a system voice") { await OnDevice.speechReady() && OnDevice.systemVoice })
    func roundTrip() async throws {
        let text = "What is the capital of France?"
        let buffers = Mutex<[PCM]>([])
        try await SystemVoiceSynthesizer().synthesize(text, voice: .standard) { pcm in buffers.withLock { $0.append(pcm) } }
        let session = try await AppleSpeechRecognizer().begin()
        for pcm in buffers.withLock({ $0 }) { session.append(pcm) }
        let heard = try await session.finish()
        #expect(Transcripts.wordErrorRate(reference: text, hypothesis: heard.text) <= 0.2, "heard: \(heard.text)")
    }

    @Test("a flood of audio past the bound fails as overrun instead of growing", .enabled("on-device speech assets are installed") { await OnDevice.speechReady() })
    func overrun() async throws {
        let session = try await AppleSpeechRecognizer(maxQueuedBuffers: 4).begin()
        let pcm = try Audio.read(repoRoot.appending(path: "fixtures/audio/story-request.wav"))
        var refused = 0
        for _ in 0..<50 { for slice in Audio.slices(pcm, frames: 160) where !session.append(slice) { refused += 1 } }
        #expect(refused > 0)
        await #expect(throws: RecognizerError.overrun) { try await session.finish() }
    }

    @Test("a cancelled recognition returns promptly", .enabled("on-device speech assets are installed") { await OnDevice.speechReady() })
    func cancel() async throws {
        let session = try await AppleSpeechRecognizer().begin()
        let pcm = try Audio.read(repoRoot.appending(path: "fixtures/audio/capital-question.wav"))
        for slice in Audio.slices(pcm, frames: 1_600).prefix(5) { session.append(slice) }
        let clock = ContinuousClock()
        let t0 = clock.now
        await session.cancel()
        #expect(clock.now - t0 < .seconds(1))
    }
}
