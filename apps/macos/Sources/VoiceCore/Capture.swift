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
    private(set) var engine: AVAudioEngine?
    private var ownsEngine = false
    public private(set) var voiceProcessing = false

    public init() {}

    /// Asks for microphone access (shows the system prompt the first time).
    public static func requestPermission() async -> Bool {
        await AVAudioApplication.requestRecordPermission()
    }

    public static var permissionGranted: Bool {
        AVAudioApplication.shared.recordPermission == .granted
    }

    /// Starts capturing. `on:` may share the player's engine, so voice processing can reference
    /// the assistant's own output for echo cancellation (D-023); a shared engine is only tapped
    /// — its owner starts and stops it. With no engine given, a capture-owned one exists only
    /// while push-to-talk is held, so the microphone is open only then.
    public func start(
        on shared: AVAudioEngine? = nil,
        voiceProcessing wanted: Bool = true,
        onBuffer: @escaping @Sendable (PCM) -> Void
    ) throws {
        guard Self.permissionGranted else { throw CaptureError.permissionDenied }
        try attach(to: shared ?? AVAudioEngine(), owns: shared == nil, voiceProcessing: wanted, onBuffer: onBuffer)
    }

    /// Installs the tap on `engine`. A capture-owned engine is prepared and started; a shared
    /// one is left to its owner (the player keeps it running). Each tap buffer is copied.
    func attach(to engine: AVAudioEngine, owns: Bool, voiceProcessing wanted: Bool, onBuffer: @escaping @Sendable (PCM) -> Void) throws {
        stop()
        let input = engine.inputNode
        // Echo cancellation where the device supports it (D-023); on the shared engine the
        // canceller can hear the output. Capture still works without it.
        voiceProcessing = wanted && (try? input.setVoiceProcessingEnabled(true)) != nil
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            if voiceProcessing { try? input.setVoiceProcessingEnabled(false); voiceProcessing = false }
            throw CaptureError.noInputDevice
        }
        input.installTap(onBus: 0, bufferSize: 1_024, format: format) { buffer, _ in
            if let copy = Audio.copy(buffer) { onBuffer(copy) }
        }
        if owns {
            engine.prepare()
            try engine.start()
        }
        self.engine = engine
        ownsEngine = owns
    }

    public func stop() {
        guard let engine else { return }
        engine.inputNode.removeTap(onBus: 0)
        if voiceProcessing { try? engine.inputNode.setVoiceProcessingEnabled(false); voiceProcessing = false }
        if ownsEngine { engine.stop() }
        self.engine = nil
        ownsEngine = false
    }
}
