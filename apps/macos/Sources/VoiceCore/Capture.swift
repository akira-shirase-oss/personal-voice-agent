// Live microphone capture: the thin path the automated tests cannot exercise (an agent
// cannot grant the microphone prompt). Everything after the tap — recognition, bounds,
// turn tagging — is shared with the fixture path and tested there. Covered by docs/DRILLS.md.

@preconcurrency import AVFoundation
import Foundation

public enum CaptureError: Error, Equatable, Sendable {
    case permissionDenied
    case noInputDevice
}

public final class MicrophoneCapture: @unchecked Sendable {
    private var engine: AVAudioEngine?
    public private(set) var voiceProcessing = false

    public init() {}

    /// Asks for microphone access (shows the system prompt the first time).
    public static func requestPermission() async -> Bool {
        await AVAudioApplication.requestRecordPermission()
    }

    public static var permissionGranted: Bool {
        AVAudioApplication.shared.recordPermission == .granted
    }

    /// Starts capturing. The engine exists only while push-to-talk is held, so the microphone
    /// is open only then. Each tap buffer is copied (the tap reuses its buffers).
    public func start(voiceProcessing wanted: Bool = true, onBuffer: @escaping @Sendable (PCM) -> Void) throws {
        guard Self.permissionGranted else { throw CaptureError.permissionDenied }
        stop()
        let engine = AVAudioEngine()
        let input = engine.inputNode
        // Echo cancellation where the device supports it (D-023); capture still works without.
        voiceProcessing = wanted && (try? input.setVoiceProcessingEnabled(true)) != nil
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else { throw CaptureError.noInputDevice }
        input.installTap(onBus: 0, bufferSize: 1_024, format: format) { buffer, _ in
            if let copy = Audio.copy(buffer) { onBuffer(copy) }
        }
        engine.prepare()
        try engine.start()
        self.engine = engine
    }

    public func stop() {
        guard let engine else { return }
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        self.engine = nil
    }
}
