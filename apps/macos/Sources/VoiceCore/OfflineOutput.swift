// Stands in for the output device when the player renders offline (tests, the bench, CI):
// pulls audio at about real time and records when sound was last heard.

@preconcurrency import AVFoundation
import Foundation
import Synchronization

public final class OfflineRenderPump: @unchecked Sendable {
    public let lastLoud = Mutex<ContinuousClock.Instant?>(nil)
    public let loudBlocks = Mutex(0)
    private var task: Task<Void, Never>?

    /// `realTime` paces 512-frame pulls to the sample rate; otherwise renders ~2x faster.
    public init(_ player: TurnFencedPlayer, realTime: Bool = false) {
        let blockSeconds = 512 / player.format.sampleRate
        let pause: Duration = realTime ? .microseconds(Int64(blockSeconds * 1e6)) : .milliseconds(10)
        task = Task.detached { [weak self] in
            while !Task.isCancelled {
                if let block = try? player.render(frames: 512), Audio.peak(block) > 0.05 {
                    self?.lastLoud.withLock { $0 = ContinuousClock.now }
                    self?.loudBlocks.withLock { $0 += 1 }
                }
                try? await Task.sleep(for: pause)
            }
        }
    }

    public func stop() { task?.cancel() }
}
