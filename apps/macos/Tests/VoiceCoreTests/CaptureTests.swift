// Capture's engine ownership. Live audio needs no permission here: the tap is installed on an
// engine, not opened — `attach` is the internal split so these run without the mic prompt.

@preconcurrency import AVFoundation
import Foundation
import Testing
@testable import VoiceCore

@Suite("Microphone capture and its engine", .serialized)
struct CaptureTests {
    /// The player's engine outlives a push-to-talk press: capture taps it, and `stop` removes
    /// only the tap — the engine keeps running for playback. A capture that stopped the shared
    /// engine would kill the assistant's voice mid-sentence.
    @Test("a shared engine is tapped and left running, never stopped")
    func sharedEngineSurvivesStop() throws {
        let shared = AVAudioEngine()
        try shared.start()
        let capture = MicrophoneCapture()
        try capture.attach(to: shared, owns: false, voiceProcessing: false) { _ in }
        #expect(capture.engine === shared)
        capture.stop()
        #expect(capture.engine == nil)
        #expect(shared.isRunning, "capture stopped an engine it does not own")
        shared.stop()
    }

    /// Without a shared engine, capture's own engine exists only while push-to-talk is held:
    /// started on `attach`, stopped on `stop`.
    @Test("a capture-owned engine starts and stops with capture")
    func ownedEngineFollowsCapture() throws {
        let owned = AVAudioEngine()
        let capture = MicrophoneCapture()
        try capture.attach(to: owned, owns: true, voiceProcessing: false) { _ in }
        #expect(owned.isRunning)
        capture.stop()
        #expect(!owned.isRunning, "the capture-owned engine kept running after stop")
    }
}
